//! Browser Basic authentication belongs to the network context, not libcurl's
//! URL userinfo handling. URL credentials require a challenge; accepted
//! identities can subsequently be sent within their protection space.

use std::{collections::VecDeque, fmt, sync::Arc};

use moli_header_field::{HeaderFieldTokenMode, HeaderFieldTokenizer, split_outside_quoted_strings};
use moli_storage_key::{MoliStorageKey, deserialize_serialized_storage_key, site_for_url};
use moli_url::WebOrigin;
use parking_lot::Mutex;
use url::Url;

use crate::{FetchConfig, Request, RequestAuth, RequestAuthScheme, RequestAuthTarget, RequestMode};

const MAX_ENTRIES: usize = 256;
const MAX_PATHS: usize = 10;

pub(crate) type SharedHttpAuthenticationCache = Arc<Mutex<HttpAuthenticationCache>>;

#[derive(Default)]
pub(crate) struct HttpAuthenticationCache {
    entries: VecDeque<Entry>,
}

#[derive(Clone, PartialEq, Eq)]
struct ProtectionSpace {
    origin: url::Origin,
    partition: String,
}

impl ProtectionSpace {
    fn new(request: &Request, url: &Url) -> Option<Self> {
        let origin = request.request_origin()?;
        let partition = if let Some(key) = request.network_partition_key() {
            if origin.is_opaque()
                && deserialize_serialized_storage_key(key)?
                    .opaque_nonce()
                    .is_none()
            {
                return None;
            }
            key.to_owned()
        } else {
            if origin.is_opaque() {
                return None;
            }
            let frame = Url::parse(origin.ascii_serialization()).ok()?;
            let context = &request.cookie_context.browser_context;
            let top_frame = context
                .top_frame_origin_url
                .as_ref()
                .or(context.site_for_cookies_url.as_ref())
                .unwrap_or(&frame);
            MoliStorageKey::from_url_and_top_level_site(&frame, site_for_url(top_frame), None)
                .serialized_storage_key()
        };
        Some(Self {
            origin: url.origin(),
            partition,
        })
    }
}

struct Identity {
    space: ProtectionSpace,
    realm: String,
    credentials: Arc<RequestAuth>,
}

struct Entry {
    identity: Arc<Identity>,
    paths: VecDeque<String>,
}

impl HttpAuthenticationCache {
    fn lookup(&mut self, space: &ProtectionSpace, path: &str) -> Option<Arc<Identity>> {
        let (index, _) = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                if entry.identity.space != *space {
                    return None;
                }
                entry
                    .paths
                    .iter()
                    .filter(|prefix| path.starts_with(prefix.as_str()))
                    .map(String::len)
                    .max()
                    .map(|length| (index, length))
            })
            .max_by_key(|(_, length)| *length)?;
        let entry = self.entries.remove(index)?;
        let identity = Arc::clone(&entry.identity);
        self.entries.push_front(entry);
        Some(identity)
    }

    fn store(&mut self, identity: Arc<Identity>, path: &str) {
        let mut entry = self
            .entries
            .iter()
            .position(|entry| {
                entry.identity.space == identity.space && entry.identity.realm == identity.realm
            })
            .and_then(|index| self.entries.remove(index))
            .unwrap_or_else(|| Entry {
                identity: Arc::clone(&identity),
                paths: VecDeque::new(),
            });
        entry.identity = identity;
        let directory = &path[..path.rfind('/').map_or(0, |index| index + 1)];
        if !entry
            .paths
            .iter()
            .any(|prefix| directory.starts_with(prefix.as_str()))
        {
            entry.paths.retain(|prefix| !prefix.starts_with(directory));
            entry.paths.push_front(directory.to_owned());
            entry.paths.truncate(MAX_PATHS);
        }
        self.entries.push_front(entry);
        self.entries.truncate(MAX_ENTRIES);
    }

    fn reject(&mut self, identity: &Arc<Identity>) {
        // An old in-flight rejection must not evict a newer accepted identity.
        self.entries
            .retain(|entry| !Arc::ptr_eq(&entry.identity, identity));
    }

    pub(crate) fn prepare_owned(
        cache: &SharedHttpAuthenticationCache,
        config: &FetchConfig,
        mut request: Request,
        url: &Url,
    ) -> Request {
        if request.request_origin().is_none()
            || request
                .configured_auth()
                .is_some_and(|auth| auth.target == RequestAuthTarget::Server)
        {
            request.http_authentication = None;
            return request;
        }
        if request
            .http_authentication
            .as_ref()
            .is_some_and(|snapshot| snapshot.matches(cache, config, &request, url))
        {
            return request;
        }
        let has_authorization = has_authorization(config, &request);
        let space = ProtectionSpace::new(&request, url);
        let action = if has_authorization
            || !request.allows_credentials_for_url(url)
            || !matches!(url.scheme(), "http" | "https")
        {
            Authentication::None
        } else if !url.username().is_empty() || url.password().is_some() {
            if matches!(
                request.request_mode,
                RequestMode::Cors | RequestMode::SameOrigin
            ) && request.has_cross_origin_url(url)
            {
                Authentication::None
            } else {
                Authentication::Url(Arc::new(RequestAuth {
                    target: RequestAuthTarget::Server,
                    scheme: RequestAuthScheme::Basic,
                    username: percent_encoding::percent_decode_str(url.username())
                        .decode_utf8_lossy()
                        .into_owned(),
                    password: percent_encoding::percent_decode_str(url.password().unwrap_or(""))
                        .decode_utf8_lossy()
                        .into_owned(),
                }))
            }
        } else {
            space
                .as_ref()
                .and_then(|space| cache.lock().lookup(space, url.path()))
                .map_or(Authentication::None, Authentication::Cached)
        };
        request.http_authentication = Some(Arc::new(PreparedHttpAuthentication {
            cache: Arc::clone(cache),
            url: url.clone(),
            origin: request.request_origin().cloned(),
            partition: request.network_partition_key().map(str::to_owned),
            top_frame: request
                .cookie_context
                .browser_context
                .top_frame_origin_url
                .clone(),
            site_for_cookies: request
                .cookie_context
                .browser_context
                .site_for_cookies_url
                .clone(),
            credentials_mode: request.credentials_mode,
            mode: request.request_mode,
            cross_origin: request.has_cross_origin_url(url),
            has_authorization,
            space,
            action,
        }));
        request
    }
}

#[derive(Clone)]
enum Authentication {
    None,
    Url(Arc<RequestAuth>),
    Challenged {
        realm: String,
        credentials: Arc<RequestAuth>,
    },
    Cached(Arc<Identity>),
}

#[derive(Clone)]
pub(crate) struct PreparedHttpAuthentication {
    cache: SharedHttpAuthenticationCache,
    url: Url,
    origin: Option<WebOrigin>,
    partition: Option<String>,
    top_frame: Option<Url>,
    site_for_cookies: Option<Url>,
    credentials_mode: crate::RequestCredentialsMode,
    mode: RequestMode,
    cross_origin: bool,
    has_authorization: bool,
    space: Option<ProtectionSpace>,
    action: Authentication,
}

impl fmt::Debug for PreparedHttpAuthentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PreparedHttpAuthentication")
    }
}

fn has_authorization(config: &FetchConfig, request: &Request) -> bool {
    request
        .request_headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        || config
            .default_request_headers()
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
}

impl PreparedHttpAuthentication {
    fn matches(
        &self,
        cache: &SharedHttpAuthenticationCache,
        config: &FetchConfig,
        request: &Request,
        url: &Url,
    ) -> bool {
        Arc::ptr_eq(&self.cache, cache)
            && self.url == *url
            && self.origin.as_ref() == request.request_origin()
            && self.partition.as_deref() == request.network_partition_key()
            && self.top_frame == request.cookie_context.browser_context.top_frame_origin_url
            && self.site_for_cookies == request.cookie_context.browser_context.site_for_cookies_url
            && self.credentials_mode == request.credentials_mode
            && self.mode == request.request_mode
            && self.cross_origin == request.has_cross_origin_url(url)
            && self.has_authorization == has_authorization(config, request)
    }

    pub(crate) fn credentials(&self) -> Option<&RequestAuth> {
        match &self.action {
            Authentication::None => None,
            Authentication::Url(credentials) | Authentication::Challenged { credentials, .. } => {
                Some(credentials)
            }
            Authentication::Cached(identity) => Some(&identity.credentials),
        }
    }

    pub(crate) fn preemptive_credentials(&self, url: &Url) -> Option<&RequestAuth> {
        if self.url != *url {
            return None;
        }
        match &self.action {
            Authentication::Challenged { credentials, .. } => Some(credentials),
            Authentication::Cached(identity) => Some(&identity.credentials),
            Authentication::None | Authentication::Url(_) => None,
        }
    }
}

pub(crate) struct HttpAuthenticationRetry {
    pub(crate) prepared: Arc<PreparedHttpAuthentication>,
    pub(crate) headers: Vec<(String, Vec<u8>)>,
}

pub(crate) enum AuthenticationHeaderAction {
    Forward,
    Defer,
    Retry,
    Replay(Vec<Vec<u8>>),
}

pub(crate) struct HttpAuthenticationResponse {
    prepared: Arc<PreparedHttpAuthentication>,
    status: u16,
    lines: Vec<Vec<u8>>,
    headers: Vec<(String, Vec<u8>)>,
    retry: Option<HttpAuthenticationRetry>,
    finalized: bool,
}

impl HttpAuthenticationResponse {
    pub(crate) fn new(request: &Request) -> Option<Self> {
        let prepared = request.http_authentication.as_ref()?;
        prepared.credentials()?;
        Some(Self {
            prepared: Arc::clone(prepared),
            status: 0,
            lines: Vec::new(),
            headers: Vec::new(),
            retry: None,
            finalized: false,
        })
    }

    pub(crate) fn take_retry(&mut self) -> Option<HttpAuthenticationRetry> {
        self.retry.take()
    }

    pub(crate) fn header(&mut self, data: &[u8]) -> AuthenticationHeaderAction {
        use AuthenticationHeaderAction::{Defer, Forward, Replay, Retry};
        if let Some(line) = crate::response::parse_http_response_status_line(data) {
            self.status = line.status;
            self.lines.clear();
            self.headers.clear();
            self.finalized = false;
        }
        if self.finalized {
            return Forward;
        }
        let defer = self.status == 401 && matches!(self.prepared.action, Authentication::Url(_));
        if defer {
            self.lines.push(data.to_vec());
            if let Some(header) = crate::headers::parse_http_response_header_line(data) {
                self.headers.push(header);
            }
        }
        if data != b"\r\n" && data != b"\n" {
            return if defer { Defer } else { Forward };
        }
        self.finalized = true;
        match &self.prepared.action {
            Authentication::Url(credentials) if defer => {
                if let Some(realm) = basic_realm(&self.headers) {
                    let mut prepared = (*self.prepared).clone();
                    prepared.action = Authentication::Challenged {
                        realm,
                        credentials: Arc::clone(credentials),
                    };
                    self.retry = Some(HttpAuthenticationRetry {
                        prepared: Arc::new(prepared),
                        headers: std::mem::take(&mut self.headers),
                    });
                    Retry
                } else {
                    Replay(std::mem::take(&mut self.lines))
                }
            }
            Authentication::Challenged { realm, credentials }
                if (200..400).contains(&self.status) =>
            {
                if let Some(space) = &self.prepared.space {
                    self.prepared.cache.lock().store(
                        Arc::new(Identity {
                            space: space.clone(),
                            realm: realm.clone(),
                            credentials: Arc::clone(credentials),
                        }),
                        self.prepared.url.path(),
                    );
                }
                Forward
            }
            Authentication::Cached(identity) if self.status == 401 => {
                self.prepared.cache.lock().reject(identity);
                Forward
            }
            _ => Forward,
        }
    }
}

/// A comma can separate challenges or auth-params. Tokenize params using the
/// shared HTTP field parser, preserving quoted commas/escapes and rejecting
/// malformed Basic challenges. Chromium accepts a missing realm as empty.
fn basic_realm(headers: &[(String, Vec<u8>)]) -> Option<String> {
    for (_, value) in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("www-authenticate"))
    {
        let value: String = value.iter().copied().map(char::from).collect();
        let mut basic = false;
        let mut valid = true;
        let mut realm = String::new();
        for part in split_outside_quoted_strings(&value, ',') {
            let mut parser = HeaderFieldTokenizer::new(part);
            let Some(token) = parser.consume_token(HeaderFieldTokenMode::Normal) else {
                valid = false;
                continue;
            };
            if !parser.consume('=') {
                if basic && valid {
                    return Some(realm);
                }
                basic = token.eq_ignore_ascii_case("Basic");
                valid = true;
                realm = String::new();
                if !basic || parser.is_consumed() {
                    continue;
                }
                let Some(name) = parser.consume_token(HeaderFieldTokenMode::Normal) else {
                    valid = false;
                    continue;
                };
                if !parser.consume('=') {
                    valid = false;
                    continue;
                }
                valid &= auth_parameter(&mut parser, name, &mut realm);
            } else if basic {
                valid &= auth_parameter(&mut parser, token, &mut realm);
            }
        }
        if basic && valid {
            return Some(realm);
        }
    }
    None
}

fn auth_parameter(parser: &mut HeaderFieldTokenizer<'_>, name: &str, realm: &mut String) -> bool {
    let Some(value) = parser.consume_token_or_quoted_string(HeaderFieldTokenMode::Normal) else {
        return false;
    };
    if !parser.is_consumed()
        || name.bytes().any(|byte| byte.is_ascii_control())
        || value
            .chars()
            .any(|character| character.is_ascii_control() && character != '\t')
    {
        return false;
    }
    if name.eq_ignore_ascii_case("realm") {
        *realm = value.into_owned();
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_challenges_use_http_quoted_parameter_grammar() {
        for (field, expected) in [
            ("Basic", Some("")),
            ("bAsIc realm=plain", Some("plain")),
            (
                "Digest realm=other, nonce=abc, Basic realm=\"a,b\\\"c\", charset=UTF-8",
                Some("a,b\"c"),
            ),
            ("Basic realm=old, REALM=new", Some("new")),
            ("Basic realm=\"", None),
            ("Basic realm=", None),
            ("Basic realm=\"ok\"junk", None),
            ("Basic realm=\"bad\u{7f}\"", None),
            ("Bearer token", None),
            ("NotBasic realm=other", None),
        ] {
            assert_eq!(
                basic_realm(&[("www-authenticate".into(), field.as_bytes().to_vec())]).as_deref(),
                expected,
                "{field:?}"
            );
        }
    }

    fn identity(partition: &str, origin: &str, realm: &str) -> Arc<Identity> {
        Arc::new(Identity {
            space: ProtectionSpace {
                origin: Url::parse(origin).unwrap().origin(),
                partition: partition.into(),
            },
            realm: realm.into(),
            credentials: Arc::new(RequestAuth {
                target: RequestAuthTarget::Server,
                scheme: RequestAuthScheme::Basic,
                username: "user".into(),
                password: "secret".into(),
            }),
        })
    }

    #[test]
    fn cache_uses_longest_directory_match_without_crossing_origin_or_partition() {
        let mut cache = HttpAuthenticationCache::default();
        let broad = identity("a", "https://one.test/", "broad");
        let narrow = identity("a", "https://one.test/", "narrow");
        cache.store(Arc::clone(&broad), "/shared/login");
        cache.store(Arc::clone(&narrow), "/shared/private/login");
        assert!(Arc::ptr_eq(
            &cache.lookup(&broad.space, "/shared/private/data").unwrap(),
            &narrow
        ));
        assert!(Arc::ptr_eq(
            &cache.lookup(&broad.space, "/shared/public").unwrap(),
            &broad
        ));
        for (partition, origin) in [
            ("b", "https://one.test/"),
            ("a", "http://one.test/"),
            ("a", "https://two.test/"),
        ] {
            assert!(
                cache
                    .lookup(
                        &identity(partition, origin, "").space,
                        "/shared/private/data"
                    )
                    .is_none()
            );
        }
        assert!(cache.lookup(&broad.space, "/sharedness/data").is_none());
        assert!(cache.lookup(&broad.space, "/shared").is_none());
    }

    #[test]
    fn old_rejection_does_not_remove_new_credentials_and_cache_is_bounded() {
        let mut cache = HttpAuthenticationCache::default();
        let old = identity("a", "https://one.test/", "realm");
        let new = identity("a", "https://one.test/", "realm");
        cache.store(Arc::clone(&old), "/login");
        cache.store(Arc::clone(&new), "/other/login");
        cache.reject(&old);
        assert!(Arc::ptr_eq(
            &cache.lookup(&new.space, "/data").unwrap(),
            &new
        ));
        cache.reject(&new);
        assert!(cache.lookup(&new.space, "/data").is_none());
        for index in 0..=MAX_ENTRIES {
            cache.store(
                identity(&index.to_string(), "https://one.test/", "realm"),
                "/login",
            );
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(
            cache
                .lookup(&identity("0", "https://one.test/", "realm").space, "/data")
                .is_none()
        );
        let value = identity("last", "https://one.test/", "realm");
        for index in 0..MAX_PATHS + 3 {
            cache.store(Arc::clone(&value), &format!("/{index}/login"));
        }
        assert_eq!(cache.entries.front().unwrap().paths.len(), MAX_PATHS);
        assert!(cache.lookup(&value.space, "/0/data").is_none());
        cache.store(Arc::clone(&value), "/login");
        assert_eq!(cache.entries.front().unwrap().paths.len(), 1);
        assert!(cache.lookup(&value.space, "/any/data").is_some());
    }

    #[test]
    fn prepared_authentication_keeps_one_selection_but_revalidates_security_inputs_and_owner() {
        let config = FetchConfig::default();
        let cache = SharedHttpAuthenticationCache::default();
        let url = Url::parse("https://one.test/data").unwrap();
        let request = Request::get_with_url(url.clone()).with_request_origin((&url).into());
        let selected = HttpAuthenticationCache::prepare_owned(&cache, &config, request, &url);
        let accepted = identity(
            ProtectionSpace::new(&selected, &url)
                .unwrap()
                .partition
                .as_str(),
            "https://one.test/",
            "realm",
        );
        cache.lock().store(accepted, "/login");
        let frozen =
            HttpAuthenticationCache::prepare_owned(&cache, &config, selected.clone(), &url);
        assert!(
            frozen.auth().is_none(),
            "cache selection must not change between renderer lookup and dispatch"
        );
        let mut fresh = selected.clone();
        fresh.http_authentication = None;
        let fresh = HttpAuthenticationCache::prepare_owned(&cache, &config, fresh, &url);
        assert!(fresh.auth().is_some());
        let other_owner = SharedHttpAuthenticationCache::default();
        assert!(
            HttpAuthenticationCache::prepare_owned(&other_owner, &config, fresh.clone(), &url)
                .auth()
                .is_none()
        );
        let mut omitted = fresh.clone();
        omitted.credentials_mode = crate::RequestCredentialsMode::Omit;
        assert!(
            HttpAuthenticationCache::prepare_owned(&cache, &config, omitted, &url)
                .auth()
                .is_none()
        );
        let mut explicit = fresh;
        explicit
            .request_headers
            .push(("Authorization".into(), b"Bearer author".to_vec()));
        assert!(
            HttpAuthenticationCache::prepare_owned(&cache, &config, explicit, &url)
                .auth()
                .is_none()
        );
    }

    #[test]
    fn opaque_origins_require_a_storage_nonce_for_session_reuse() {
        let url = Url::parse("https://one.test/data").unwrap();
        let request = Request::get_with_url(url.clone()).with_request_origin(WebOrigin::Opaque);
        assert!(ProtectionSpace::new(&request, &url).is_none());
        for key in [
            "null",
            "partition",
            "storage-key:v1;origin=null;top-level-site=https://one.test",
        ] {
            assert!(
                ProtectionSpace::new(
                    &request.clone().with_network_partition_key(Some(key.into())),
                    &url
                )
                .is_none()
            );
        }
        let key = |nonce| {
            MoliStorageKey::new(
                "null".into(),
                "https://one.test".into(),
                Some(moli_storage_key::OpaqueOriginNonce::new(nonce)),
                moli_storage_key::StoragePartitionRelation::FirstParty,
            )
            .serialized_storage_key()
        };
        let first = ProtectionSpace::new(
            &request.clone().with_network_partition_key(Some(key(1))),
            &url,
        )
        .unwrap();
        let second =
            ProtectionSpace::new(&request.with_network_partition_key(Some(key(2))), &url).unwrap();
        assert!(first != second);
    }
}
