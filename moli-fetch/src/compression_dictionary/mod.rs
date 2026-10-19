//! Compression dictionaries are network-context state, partitioned like the
//! requests which may use them. Bytes enter the store only after a readable
//! response completes; a header alone cannot register a dictionary.

mod fields;

use std::{borrow::Cow, collections::VecDeque, fmt, io::Read, sync::Arc};

use base64::{Engine, engine::general_purpose::STANDARD};
use moli_http_cache::{response_cache_policy, unix_now_ms};
use moli_storage_key::{MoliStorageKey, site_for_url};
use moli_url::{is_potentially_trustworthy_url, origin_ascii_serialization};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use url::Url;

use crate::{FetchConfig, Request, RequestMode};

pub(crate) use fields::RegistrationOptions;

const MAX_DICTIONARY_BYTES: usize = 64 * 1024 * 1024;
const MAX_STORE_BYTES: usize = 128 * 1024 * 1024;
const MAX_STORE_ENTRIES: usize = 256;

pub(crate) type SharedDictionaryStore = Arc<Mutex<DictionaryStore>>;

pub(crate) struct Dictionary {
    pub(crate) bytes: Arc<[u8]>,
    pub(crate) hash: [u8; 32],
    available: Vec<u8>,
    id: Option<Vec<u8>>,
}

impl fmt::Debug for Dictionary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Dictionary")
            .field("byte_length", &self.bytes.len())
            .field("available", &self.available)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DictionaryRequestSnapshot {
    url: Url,
    partition: Option<Partition>,
    destination: String,
    permitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Partition {
    top_frame: String,
    frame: String,
    network: String,
}

impl Partition {
    fn for_request(request: &Request, url: &Url) -> Option<Self> {
        let frame = match request.request_origin() {
            Some(origin) if origin.is_opaque() => return None,
            Some(origin) => origin.ascii_serialization().to_owned(),
            None => origin_ascii_serialization(url),
        };
        let top_frame_url = request
            .cookie_context
            .browser_context
            .top_frame_origin_url
            .as_ref()
            .or(request
                .cookie_context
                .browser_context
                .site_for_cookies_url
                .as_ref());
        // Top-level requests can use the implicit browser partition while a
        // same-context iframe carries its explicit StorageKey. Give the
        // default the same representation, retaining every explicit key,
        // including nonce, ancestor and caller-defined isolation components.
        let network = match request.network_partition_key() {
            Some(key) => key.to_owned(),
            None => {
                let frame_url = Url::parse(&frame).ok()?;
                MoliStorageKey::from_url_and_top_level_site(
                    &frame_url,
                    site_for_url(top_frame_url.unwrap_or(&frame_url)),
                    None,
                )
                .serialized_storage_key()
            }
        };
        Some(Self {
            top_frame: top_frame_url.map_or_else(|| frame.clone(), origin_ascii_serialization),
            frame,
            network,
        })
    }
}

#[derive(Debug)]
struct Entry {
    partition: Partition,
    origin: String,
    options: RegistrationOptions,
    dictionary: Arc<Dictionary>,
    expires: u64,
}

#[derive(Debug, Default)]
pub(crate) struct DictionaryStore {
    entries: VecDeque<Entry>,
    bytes: usize,
    generation: u64,
}

impl DictionaryStore {
    pub(crate) fn prepare<'a>(
        store: &SharedDictionaryStore,
        config: &FetchConfig,
        request: &'a Request,
        url: &Url,
    ) -> Cow<'a, Request> {
        Self::prepare_inner(store, config, Cow::Borrowed(request), url)
    }

    pub(crate) fn prepare_owned(
        store: &SharedDictionaryStore,
        config: &FetchConfig,
        request: Request,
        url: &Url,
    ) -> Request {
        Self::prepare_inner(store, config, Cow::Owned(request), url).into_owned()
    }

    fn prepare_inner<'a>(
        store: &SharedDictionaryStore,
        config: &FetchConfig,
        mut prepared: Cow<'a, Request>,
        url: &Url,
    ) -> Cow<'a, Request> {
        let headers =
            crate::blocking::outgoing_request_header_bytes_for_url(config, &prepared, url, None);
        let destination = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("sec-fetch-dest"))
            .and_then(|(_, value)| std::str::from_utf8(value).ok())
            .unwrap_or("");
        let key = DictionaryRequestSnapshot {
            url: url.clone(),
            partition: Partition::for_request(&prepared, url),
            destination: if destination == "empty" {
                ""
            } else {
                destination
            }
            .to_owned(),
            permitted: is_potentially_trustworthy_url(url)
                && matches!(url.scheme(), "http" | "https")
                && !(prepared.request_mode == RequestMode::NoCors
                    && prepared.has_cross_origin_url(url))
                && !prepared
                    .request_headers
                    .iter()
                    .any(|(name, _)| overrides_dictionary_negotiation(name))
                && !config
                    .default_request_headers()
                    .iter()
                    .any(|(name, _)| overrides_dictionary_negotiation(name)),
        };
        if prepared.compression_dictionary_snapshot.as_ref() == Some(&key)
            && prepared
                .compression_dictionary_store
                .as_ref()
                .is_some_and(|previous| Arc::ptr_eq(previous, store))
        {
            return prepared;
        }
        let request = prepared.to_mut();
        request.compression_dictionary = None;
        request.compression_dictionary_store = Some(Arc::clone(store));
        request.compression_dictionary_snapshot = Some(key.clone());
        if !key.permitted {
            return prepared;
        }
        let Some(partition) = key.partition else {
            return prepared;
        };
        let origin = origin_ascii_serialization(url);
        let now = unix_now_ms();
        let mut locked = store.lock();
        locked.retain(|entry| entry.expires > now);
        let selected = locked
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.partition == partition
                    && entry.origin == origin
                    && entry.options.matches(url, &key.destination)
            })
            .max_by_key(|(index, entry)| (entry.options.match_length(), *index))
            .map(|(index, _)| index);
        if let Some(index) = selected {
            let entry = locked
                .entries
                .remove(index)
                .expect("selected dictionary exists");
            request.compression_dictionary = Some(Arc::clone(&entry.dictionary));
            locked.entries.push_back(entry);
        }
        prepared
    }

    fn retain(&mut self, mut predicate: impl FnMut(&Entry) -> bool) {
        self.entries.retain(|entry| predicate(entry));
        self.bytes = self
            .entries
            .iter()
            .map(|entry| entry.dictionary.bytes.len())
            .sum();
    }

    fn clear_origin(&mut self, origin: &str) {
        self.retain(|entry| entry.origin != origin);
        // Cancel captures already in progress as well as retained entries.
        self.generation = self.generation.wrapping_add(1);
    }

    fn insert(&mut self, entry: Entry) {
        self.retain(|old| {
            !(old.partition == entry.partition
                && old.origin == entry.origin
                && old.options.same_scope(&entry.options))
        });
        self.bytes += entry.dictionary.bytes.len();
        self.entries.push_back(entry);
        while self.bytes > MAX_STORE_BYTES || self.entries.len() > MAX_STORE_ENTRIES {
            if let Some(old) = self.entries.pop_front() {
                self.bytes -= old.dictionary.bytes.len();
            }
        }
    }
}

/// Owns one response capture, including the clear-data generation at request
/// admission. Interrupted, opaque, stale and partial responses never commit.
pub(crate) struct DictionaryResponse {
    store: SharedDictionaryStore,
    request: ResponsePolicy,
    url: Url,
    generation: u64,
    status: u16,
    headers: Vec<(String, Vec<u8>)>,
    capture: Option<Capture>,
    headers_finalized: bool,
}

struct ResponsePolicy {
    partition: Option<Partition>,
    head: bool,
    credentials: bool,
    opaque: bool,
    cors: Option<(String, crate::RequestCredentialsMode)>,
}

impl ResponsePolicy {
    fn new(request: &Request, url: &Url) -> Self {
        Self {
            partition: Partition::for_request(request, url),
            head: request.method == "HEAD",
            credentials: request.allows_credentials_for_url(url),
            opaque: request.request_mode == RequestMode::NoCors
                && request.has_cross_origin_url(url),
            cors: request
                .request_origin()
                .filter(|_| {
                    request.request_mode == RequestMode::Cors && request.has_cross_origin_url(url)
                })
                .map(|origin| {
                    (
                        request.url_list(url).serialized_origin(origin),
                        request.credentials_mode,
                    )
                }),
        }
    }

    fn readable(&self, headers: &[(String, Vec<u8>)]) -> bool {
        !self.opaque
            && self.cors.as_ref().is_none_or(|(origin, mode)| {
                crate::validate_cors_response_for_origin(origin, headers, *mode).is_ok()
            })
    }
}

struct Capture {
    options: RegistrationOptions,
    partition: Partition,
    expires: u64,
    bytes: Vec<u8>,
}

impl DictionaryResponse {
    pub(crate) fn new(store: SharedDictionaryStore, request: &Request, url: Url) -> Self {
        let generation = store.lock().generation;
        Self {
            store,
            request: ResponsePolicy::new(request, &url),
            url,
            generation,
            status: 0,
            headers: Vec::new(),
            capture: None,
            headers_finalized: false,
        }
    }

    pub(crate) fn header(&mut self, data: &[u8]) {
        if let Some(status) = crate::response::parse_http_response_status_line(data) {
            self.status = status.status;
            self.headers.clear();
            self.capture = None;
            self.headers_finalized = false;
            return;
        }
        if self.headers_finalized {
            return;
        }
        if let Some(header) = crate::headers::parse_http_response_header_line(data) {
            self.headers.push(header);
            return;
        }
        if data.iter().any(|byte| !matches!(byte, b'\r' | b'\n')) || self.status < 200 {
            return;
        }
        self.headers_finalized = true;
        if !is_potentially_trustworthy_url(&self.url) {
            return;
        }
        if fields::clears_dictionaries(&self.headers) && self.request.credentials {
            self.store
                .lock()
                .clear_origin(&origin_ascii_serialization(&self.url));
        }
        self.begin_capture(None);
    }

    fn begin_capture(&mut self, fresh_until: Option<u64>) {
        if self.status != 200 || self.request.head || !self.request.readable(&self.headers) {
            return;
        }
        let Some(partition) = self.request.partition.clone() else {
            return;
        };
        let Some(options) = RegistrationOptions::parse(&self.url, &self.headers) else {
            return;
        };
        let policy = response_cache_policy(&self.headers);
        let Some(mut expires) = policy
            .expires_at_unix_ms
            .filter(|expires| *expires > unix_now_ms())
        else {
            return;
        };
        if let Some(deadline) = fresh_until {
            expires = expires.min(deadline);
        }
        if expires <= unix_now_ms() {
            return;
        }
        if !policy.store {
            return;
        }
        self.capture = Some(Capture {
            options,
            partition,
            expires,
            bytes: Vec::new(),
        });
    }

    pub(crate) fn decoded(&mut self, bytes: &[u8]) {
        let Some(capture) = self.capture.as_mut() else {
            return;
        };
        if bytes.len() > MAX_DICTIONARY_BYTES.saturating_sub(capture.bytes.len()) {
            self.capture = None;
        } else {
            capture.bytes.extend_from_slice(bytes);
        }
    }

    pub(crate) fn from_cached(
        store: SharedDictionaryStore,
        request: &Request,
        response: &crate::RawResponse,
        fresh_until: u64,
    ) {
        let Some(mut capture) = Self::cached(
            store,
            request,
            &response.final_url,
            response.status,
            &response.headers,
            Some(fresh_until),
        ) else {
            return;
        };
        capture.decoded(response.body_bytes());
        capture.finish();
    }

    pub(crate) fn cached(
        store: SharedDictionaryStore,
        request: &Request,
        url: &Url,
        status: u16,
        headers: &[(String, Vec<u8>)],
        fresh_until: Option<u64>,
    ) -> Option<Self> {
        if !is_potentially_trustworthy_url(url)
            || !headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("use-as-dictionary"))
        {
            return None;
        }
        let deadline = fresh_until.filter(|deadline| *deadline > unix_now_ms())?;
        let mut capture = Self::new(store, request, url.clone());
        capture.status = status;
        capture.headers = headers.to_vec();
        capture.begin_capture(Some(deadline));
        capture.capture.as_ref()?;
        Some(capture)
    }

    pub(crate) fn readable(&self) -> bool {
        self.request.readable(&self.headers)
    }

    pub(crate) fn finish(&mut self) {
        let Some(capture) = self.capture.take() else {
            return;
        };
        if capture.expires <= unix_now_ms() {
            return;
        }
        let hash: [u8; 32] = Sha256::digest(&capture.bytes).into();
        let available = format!(":{}:", STANDARD.encode(hash)).into_bytes();
        let dictionary = Arc::new(Dictionary {
            bytes: capture.bytes.into(),
            hash,
            available,
            id: capture.options.id_header(),
        });
        let mut store = self.store.lock();
        if store.generation != self.generation {
            return;
        }
        store.insert(Entry {
            partition: capture.partition,
            origin: origin_ascii_serialization(&self.url),
            options: capture.options,
            dictionary,
            expires: capture.expires,
        });
    }
}

pub(crate) struct DictionaryCachedReader<R> {
    inner: R,
    capture: Option<DictionaryResponse>,
    cancel: crate::FetchCancelHandle,
}

impl<R> DictionaryCachedReader<R> {
    pub(crate) fn new(
        inner: R,
        capture: Option<DictionaryResponse>,
        cancel: crate::FetchCancelHandle,
    ) -> Self {
        Self {
            inner,
            capture,
            cancel,
        }
    }
}

impl<R: Read> Read for DictionaryCachedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buffer)?;
        if buffer.is_empty() {
            return Ok(count);
        }
        if let Some(capture) = self.capture.as_mut() {
            if count == 0 {
                if !self.cancel.is_cancelled() {
                    capture.finish();
                }
                self.capture = None;
            } else {
                capture.decoded(&buffer[..count]);
            }
        }
        Ok(count)
    }
}

fn overrides_dictionary_negotiation(name: &str) -> bool {
    ["range", "available-dictionary", "dictionary-id"]
        .iter()
        .any(|value| name.eq_ignore_ascii_case(value))
}

pub(crate) fn append_request_headers(headers: &mut crate::RequestHeaders, dictionary: &Dictionary) {
    headers.push((
        "Available-Dictionary".to_owned(),
        dictionary.available.clone(),
    ));
    if let Some(id) = &dictionary.id {
        headers.push(("Dictionary-ID".to_owned(), id.clone()));
    }
}

/// Dictionary representations remain reusable after registration changes the
/// browser-generated negotiation fields. Other Vary fields remain in force.
pub fn cache_vary_header_names(url: &Url, headers: &[(String, Vec<u8>)]) -> Option<Vec<String>> {
    let mut names = moli_http_cache::response_vary_header_names(headers)?;
    if is_potentially_trustworthy_url(url) && RegistrationOptions::parse(url, headers).is_some() {
        names.retain(|name| name != "available-dictionary" && name != "accept-encoding");
    }
    Some(names)
}
