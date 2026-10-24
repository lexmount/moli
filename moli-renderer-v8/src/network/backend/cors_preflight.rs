use std::{collections::BTreeMap, time::Duration, time::Instant};

use moli_fetch::{Request, RequestCredentialsMode};
use moli_web_mime::response_header_values;

const MAX_ENTRIES: usize = 1024;
const MAX_URL_LENGTH: usize = 1024;

/// Permissions from a successfully checked preflight response. Keep the
/// original credentials mode: a credentialed grant can serve an omit request,
/// but its literal `*` permissions must not turn into wildcards on reuse.
#[derive(Debug)]
pub(crate) struct CorsPreflightResult {
    credentials: bool,
    methods: Option<Vec<String>>,
    headers: Option<Vec<String>>,
    max_age: Duration,
}

impl CorsPreflightResult {
    pub(crate) fn new(
        credentials_mode: RequestCredentialsMode,
        methods: Option<Vec<String>>,
        headers: Option<Vec<String>>,
        response_headers: &[(String, Vec<u8>)],
    ) -> Self {
        // Chromium uses a five-second default and a two-hour upper bound.
        // Parse the combined field so duplicate or malformed values cannot
        // accidentally select a longer lifetime.
        let max_age = response_header_values(response_headers, "access-control-max-age")
            .join(", ")
            .parse::<i64>()
            .unwrap_or(5)
            .clamp(0, 7200) as u64;
        Self {
            credentials: credentials_mode == RequestCredentialsMode::Include,
            methods,
            headers,
            max_age: Duration::from_secs(max_age),
        }
    }

    pub(crate) fn validate_request(
        &self,
        requested_method: &str,
        request_headers: &[(String, String)],
    ) -> Result<(), String> {
        if !moli_fetch::is_cors_safelisted_method(requested_method) {
            let Some(allow_methods) = self.methods.as_ref() else {
                return Err(format!(
                    "CORS preflight failed: no Access-Control-Allow-Methods for {requested_method}"
                ));
            };
            if !allow_methods
                .iter()
                .any(|method| method == requested_method || (!self.credentials && method == "*"))
            {
                return Err(format!(
                    "CORS preflight failed: Access-Control-Allow-Methods `{}` does not allow {requested_method}",
                    allow_methods.join(",")
                ));
            }
        }

        let unsafe_header_names = moli_fetch::cors_unsafe_request_header_names(request_headers);
        if unsafe_header_names.is_empty() {
            return Ok(());
        }
        let Some(allow_headers) = self.headers.as_ref() else {
            return Err(format!(
                "CORS preflight failed: no Access-Control-Allow-Headers for {}",
                unsafe_header_names.join(",")
            ));
        };
        let wildcard = !self.credentials && allow_headers.iter().any(|name| name == "*");
        for header_name in unsafe_header_names {
            // Authorization always needs an explicit permission.
            if !allow_headers
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&header_name))
                && (!wildcard || header_name == "authorization")
            {
                return Err(format!(
                    "CORS preflight failed: Access-Control-Allow-Headers `{}` does not allow {header_name}",
                    allow_headers.join(",")
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct CorsPreflightCacheKey {
    partition: Option<String>,
    initiator: String,
    origin: String,
    url: String,
}

impl CorsPreflightCacheKey {
    pub(crate) fn for_request(request: &Request) -> Option<Self> {
        if request.request_mode != moli_fetch::RequestMode::Cors
            || !matches!(request.url.scheme(), "http" | "https")
            || !request.has_cross_origin_url(&request.url)
        {
            return None;
        }
        let initiator = request.request_origin()?;
        // Opaque origins need the existing storage-key nonce. A plain HTTP
        // caller without that identity cannot safely share a `null` grant.
        if initiator.is_opaque()
            && request
                .network_partition_key()
                .and_then(moli_storage_key::deserialize_serialized_storage_key)
                .is_none_or(|key| key.opaque_nonce().is_none())
        {
            return None;
        }
        if request.url.as_str().len() >= MAX_URL_LENGTH {
            return None;
        }
        Some(Self {
            partition: request.network_partition_key().map(str::to_owned),
            initiator: initiator.ascii_serialization().to_owned(),
            // Redirect-tainted Origin:null must not authorize an untainted
            // request to the same URL in the same storage partition.
            origin: request.serialized_origin(),
            url: request.url.as_str().to_owned(),
        })
    }
}

#[derive(Debug)]
struct CacheEntry {
    result: CorsPreflightResult,
    expires: Instant,
}

/// Browser-context memory only; independent of HTTP response caching and of
/// individual Document/Worker lifetimes. Early eviction is permitted by Fetch.
#[derive(Debug, Default)]
pub(crate) struct CorsPreflightCache {
    entries: BTreeMap<CorsPreflightCacheKey, CacheEntry>,
}

impl CorsPreflightCache {
    pub(crate) fn can_skip(
        &mut self,
        request: &Request,
        headers: &[(String, String)],
        now: Instant,
    ) -> bool {
        if !request.cache_mode().allows_memory_cache_lookup() {
            return false;
        }
        let Some(key) = CorsPreflightCacheKey::for_request(request) else {
            return false;
        };
        let Some(entry) = self.entries.get(&key) else {
            return false;
        };
        if entry.expires > now
            && (entry.result.credentials
                || request.credentials_mode != RequestCredentialsMode::Include)
            && entry
                .result
                .validate_request(&request.method, headers)
                .is_ok()
        {
            return true;
        }
        // Like Chromium, retire an expired or insufficient grant before
        // issuing a new preflight, including when that preflight later fails.
        self.entries.remove(&key);
        false
    }

    pub(crate) fn insert(&mut self, request: &Request, result: CorsPreflightResult, now: Instant) {
        if !request.cache_mode().allows_memory_cache_store() {
            return;
        }
        let Some(key) = CorsPreflightCacheKey::for_request(request) else {
            return;
        };
        self.entries.retain(|_, entry| entry.expires > now);
        if result.max_age.is_zero() {
            self.entries.remove(&key);
            return;
        }
        if self.entries.len() == MAX_ENTRIES && !self.entries.contains_key(&key) {
            self.entries.pop_first();
        }
        let expires = now + result.max_age;
        self.entries.insert(key, CacheEntry { result, expires });
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn clear_for_origin(&mut self, origin: &url::Url) {
        // Chromium's filtered cache clearing matches the initiating origin,
        // across network partitions, rather than the target resource URL.
        let origin = origin.origin().ascii_serialization();
        self.entries.retain(|key, _| key.initiator != origin);
    }

    pub(crate) fn remove(&mut self, request: &Request) {
        if let Some(key) = CorsPreflightCacheKey::for_request(request) {
            self.remove_key(&key);
        }
    }

    pub(crate) fn remove_key(&mut self, key: &CorsPreflightCacheKey) {
        self.entries.remove(key);
    }
}

#[cfg(test)]
mod tests;
