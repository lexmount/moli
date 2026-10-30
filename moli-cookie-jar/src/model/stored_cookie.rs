//! Stable cookie DTOs projected from the canonical cookie engine.

use cookie_store::utils::is_secure;
use cookie_store::{
    Cookie as StoreCookie, CookiePriority, CookieSourceScheme as CoreCookieSourceScheme, SameSite,
};
use time::OffsetDateTime;
use url::Url;

/// Stable browser-facing representation of a CHIPS partition key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StoredCookiePartitionKey {
    Site {
        top_level_site: String,
        has_cross_site_ancestor: bool,
    },
    Opaque {
        nonce: u64,
        has_cross_site_ancestor: bool,
    },
}

/// Partition scope for cookie queries and deletion, independent of the protocol.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum StoredCookiePartitionFilter {
    #[default]
    All,
    Unpartitioned,
    Partition(StoredCookiePartitionKey),
    /// Match each provided field of a serialized site identity, without URL parsing.
    /// Unpartitioned and opaque cookies have no serialized site identity.
    Serialized {
        top_level_site: Option<String>,
        has_cross_site_ancestor: Option<bool>,
    },
}

impl StoredCookiePartitionFilter {
    /// Match a complete stored identity, including keys that cannot be serialized.
    pub fn matches(&self, key: Option<&StoredCookiePartitionKey>) -> bool {
        match self {
            Self::All => true,
            Self::Unpartitioned => key.is_none(),
            Self::Partition(wanted) => key == Some(wanted),
            Self::Serialized {
                top_level_site,
                has_cross_site_ancestor,
            } => {
                let Some(StoredCookiePartitionKey::Site {
                    top_level_site: actual_site,
                    has_cross_site_ancestor: actual_ancestor,
                }) = key
                else {
                    return false;
                };
                top_level_site
                    .as_ref()
                    .is_none_or(|wanted| wanted == actual_site)
                    && has_cross_site_ancestor.is_none_or(|wanted| wanted == *actual_ancestor)
            }
        }
    }
}

/// Domain representation used by a cookie deletion filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieDomainMatch {
    Canonical,
    /// Match the serialized domain, including a domain cookie's leading dot.
    Serialized,
}

impl StoredCookiePartitionKey {
    pub fn site(top_level_site: String, has_cross_site_ancestor: bool) -> Self {
        Self::Site {
            top_level_site,
            has_cross_site_ancestor,
        }
    }

    pub fn opaque(nonce: u64, has_cross_site_ancestor: bool) -> Self {
        Self::Opaque {
            nonce,
            has_cross_site_ancestor,
        }
    }

    pub fn top_level_site(&self) -> Option<&str> {
        match self {
            Self::Site { top_level_site, .. } => Some(top_level_site),
            Self::Opaque { .. } => None,
        }
    }

    pub fn has_cross_site_ancestor(&self) -> bool {
        match self {
            Self::Site {
                has_cross_site_ancestor,
                ..
            }
            | Self::Opaque {
                has_cross_site_ancestor,
                ..
            } => *has_cross_site_ancestor,
        }
    }

    pub fn opaque_nonce(&self) -> Option<u64> {
        match self {
            Self::Site { .. } => None,
            Self::Opaque { nonce, .. } => Some(*nonce),
        }
    }

    pub fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque { .. })
    }
}

/// Serializable SameSite representation used at Moli crate boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieSameSite {
    Unspecified,
    None,
    Lax,
    Strict,
}

impl From<SameSite> for StoredCookieSameSite {
    fn from(value: SameSite) -> Self {
        match value {
            SameSite::Strict => Self::Strict,
            SameSite::Lax => Self::Lax,
            SameSite::None => Self::None,
        }
    }
}

/// Scheme of the request that originally created the cookie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieSourceScheme {
    Unset,
    NonSecure,
    Secure,
}

impl StoredCookieSourceScheme {
    /// Stable CDP/profile spelling for this source-scheme value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unset => "Unset",
            Self::NonSecure => "NonSecure",
            Self::Secure => "Secure",
        }
    }

    /// Classifies a URL into the source scheme used by browser cookie metadata.
    pub fn from_url(url: &Url) -> Self {
        if is_secure_scheme(url) {
            Self::Secure
        } else {
            Self::NonSecure
        }
    }

    /// Parses CDP/profile source-scheme strings, defaulting unknown values.
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some(value) if value.eq_ignore_ascii_case("secure") => Self::Secure,
            Some(value) if value.eq_ignore_ascii_case("nonsecure") => Self::NonSecure,
            _ => Self::Unset,
        }
    }
}

/// Browser-facing cookie DTO projected from the canonical cookie store.
///
/// The canonical matcher and quota logic live in `cookie_store`; this type is
/// the stable shape used by CDP, profile persistence, tests, and cross-crate
/// APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    /// True when the cookie was set without a Domain attribute.
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Absolute expiry time; `None` means a session cookie.
    pub expires: Option<OffsetDateTime>,
    pub same_site: StoredCookieSameSite,
    pub priority: Option<CookiePriority>,
    /// Partition identity when the cookie carried the `Partitioned` attribute.
    pub partition_key: Option<StoredCookiePartitionKey>,
    /// Scheme of the source request, independent from the Secure attribute.
    pub source_scheme: StoredCookieSourceScheme,
    /// Source port, or the browser sentinel used by imported/profile cookies.
    pub source_port: i32,
    /// Stable creation order projected from the canonical core cookie.
    pub creation_index: u64,
    /// Original creation time, retained across replacement and profile storage.
    pub creation_time: Option<OffsetDateTime>,
    /// Stable last-access order projected from the canonical core cookie.
    pub last_access_index: u64,
}

impl StoredCookie {
    /// Effective request mode, including the limited default-cookie POST allowance.
    pub fn effective_same_site(&self) -> super::StoredCookieEffectiveSameSite {
        super::StoredCookieEffectiveSameSite::for_cookie(
            self.same_site,
            self.creation_time,
            OffsetDateTime::now_utc(),
        )
    }

    /// Returns the eviction priority, defaulting omitted `Priority` to `Medium`.
    pub fn effective_priority(&self) -> CookiePriority {
        self.priority.unwrap_or(CookiePriority::Medium)
    }

    /// Checks URL visibility with the same domain/path/secure shape as cookies.
    ///
    /// The secure gate uses the store's secure-context predicate (cryptographic
    /// schemes plus loopback/localhost hosts) so that a `Secure` cookie matches
    /// an `http://127.0.0.1`/`http://localhost` URL the same way the cookie
    /// read path (`document.cookie`, request headers) treats it.
    pub fn matches(&self, url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };

        if self.secure && !is_secure(url) {
            return false;
        }

        if !domain_matches(host, &self.domain, self.host_only) {
            return false;
        }

        path_matches(url.path(), &self.path)
    }

    /// Returns true once the stored absolute expiry is in the past.
    pub fn is_expired(&self) -> bool {
        self.expires
            .is_some_and(|expiry| expiry <= OffsetDateTime::now_utc())
    }
}

/// Extracts an absolute expiry from a canonical cookie, ignoring session cookies.
pub fn cookie_expiration(cookie: &StoreCookie<'_>) -> Option<OffsetDateTime> {
    match cookie.expires {
        cookie_store::CookieExpiration::AtUtc(datetime) => Some(datetime),
        _ => None,
    }
}

/// Matches a request host against a canonical cookie domain.
pub fn domain_matches(request_host: &str, cookie_domain: &str, host_only: bool) -> bool {
    let request_host = request_host.to_ascii_lowercase();
    let cookie_domain = cookie_domain.trim_start_matches('.').to_ascii_lowercase();

    if host_only {
        return request_host == cookie_domain;
    }

    !cookie_domain.is_empty()
        && (request_host == cookie_domain || request_host.ends_with(&format!(".{cookie_domain}")))
}

pub(super) fn is_secure_scheme(url: &Url) -> bool {
    matches!(url.scheme(), "https" | "wss")
}

/// Matches a request path using RFC6265 cookie-path prefix rules.
pub fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    if request_path == cookie_path {
        return true;
    }

    if !request_path.starts_with(cookie_path) {
        return false;
    }

    cookie_path.ends_with('/')
        || request_path
            .as_bytes()
            .get(cookie_path.len())
            .is_some_and(|byte| *byte == b'/')
}

/// Detects control characters rejected by browser cookie parsing.
pub fn has_invalid_cookie_octets(cookie: &str) -> bool {
    cookie
        .chars()
        .any(|ch| ch == '\u{7f}' || (ch.is_control() && ch != '\t'))
}

pub(super) fn stored_source_scheme_from_core(
    source_scheme: CoreCookieSourceScheme,
) -> StoredCookieSourceScheme {
    match source_scheme {
        CoreCookieSourceScheme::Unset => StoredCookieSourceScheme::Unset,
        CoreCookieSourceScheme::NonSecure => StoredCookieSourceScheme::NonSecure,
        CoreCookieSourceScheme::Secure => StoredCookieSourceScheme::Secure,
    }
}

/// Converts a Moli source-scheme DTO back into the canonical store type.
pub fn core_source_scheme_from_stored(
    source_scheme: StoredCookieSourceScheme,
) -> CoreCookieSourceScheme {
    match source_scheme {
        StoredCookieSourceScheme::Unset => CoreCookieSourceScheme::Unset,
        StoredCookieSourceScheme::NonSecure => CoreCookieSourceScheme::NonSecure,
        StoredCookieSourceScheme::Secure => CoreCookieSourceScheme::Secure,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        StoredCookie, StoredCookieSameSite, StoredCookieSourceScheme, domain_matches, path_matches,
    };
    use url::Url;

    fn secure_cookie(domain: &str, host_only: bool) -> StoredCookie {
        StoredCookie {
            name: "nonec".to_owned(),
            value: "1".to_owned(),
            domain: domain.to_owned(),
            host_only,
            path: "/".to_owned(),
            secure: true,
            http_only: false,
            expires: None,
            same_site: StoredCookieSameSite::None,
            priority: None,
            partition_key: None,
            source_scheme: StoredCookieSourceScheme::Secure,
            source_port: -1,
            creation_index: 0,
            creation_time: None,
            last_access_index: 0,
        }
    }

    #[test]
    fn domain_matches_follow_chromium_cookie_util_cases() {
        assert!(domain_matches("example.com", "example.com", true));
        assert!(!domain_matches("www.example.com", "example.com", true));

        assert!(domain_matches("example.com", "example.com", false));
        assert!(domain_matches("www.example.com", "example.com", false));
        assert!(domain_matches("example.com", ".example.com", false));
        assert!(domain_matches("www.example.com", ".example.com", false));
        assert!(!domain_matches("example.com", ".www.example.com", false));
        assert!(!domain_matches("example.com", "www.example.com", false));
        assert!(!domain_matches("example.de", "example.com", false));
        assert!(!domain_matches("example.de.vu", "example.de", false));
    }

    #[test]
    fn path_matches_follow_chromium_cookie_util_cases() {
        assert!(path_matches("/", "/"));
        assert!(path_matches("/test", "/"));
        assert!(path_matches("/test/bar.html", "/"));
        assert!(!path_matches("", "/"));

        assert!(!path_matches("/", "/test"));
        assert!(path_matches("/test", "/test"));
        assert!(!path_matches("/testtest/", "/test"));
        assert!(path_matches("/test/bar.html", "/test"));
        assert!(path_matches("/test/sample/bar.html", "/test"));

        assert!(path_matches("/test", "/test"));
        assert!(!path_matches("/TEST", "/test"));
        assert!(!path_matches("/test", "/TEST"));
    }

    #[test]
    fn secure_cookie_matches_loopback_http_url_like_chromium() {
        let cookie = secure_cookie("127.0.0.1", true);
        let url = Url::parse("http://127.0.0.1:8765/").expect("loopback http url");
        assert!(
            cookie.matches(&url),
            "a Secure cookie must be visible for an http loopback URL"
        );

        let localhost_cookie = secure_cookie("localhost", true);
        let localhost_url = Url::parse("http://localhost/app").expect("localhost http url");
        assert!(localhost_cookie.matches(&localhost_url));

        let ipv6_url = Url::parse("http://[::1]:8765/").expect("ipv6 loopback http url");
        let ipv6_cookie = secure_cookie(ipv6_url.host_str().expect("ipv6 loopback host"), true);
        assert!(ipv6_cookie.matches(&ipv6_url));
    }

    #[test]
    fn secure_cookie_still_rejects_remote_http_url() {
        let cookie = secure_cookie("example.com", true);
        let url = Url::parse("http://example.com/").expect("remote http url");
        assert!(
            !cookie.matches(&url),
            "a Secure cookie must not be visible for a remote http URL"
        );

        let secure_url = Url::parse("https://example.com/").expect("https url");
        assert!(cookie.matches(&secure_url));
    }
}
