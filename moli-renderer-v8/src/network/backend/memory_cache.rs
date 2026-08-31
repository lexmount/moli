//! Browser-resource-runtime scoped renderer memory cache.
//!
//! The cache deliberately outlives individual Document and Worker loaders. Its
//! retained-byte budget is shared by every consumer of one browser resource
//! runtime and must never be multiplied per execution context. Entries retain
//! the owning Page cache partition, so a DevTools cache bypass can replace one
//! Page's retained resource without rewriting a live peer Page's resource.

use std::sync::{Arc, Weak};

use indexmap::IndexMap;
use moli_fetch::{BrowserRequestMetadata, RawResponse, Request, RequestResourceType, Response};
use moli_http_cache::{cacheable_response_parts_policy, unix_now_ms};
use parking_lot::Mutex;
use tokio::sync::Notify;
use url::Url;

/// Strong-reference budget for renderer subresources.
///
/// Chromium's Linux MemoryCache currently keeps at most 15 MiB of strong
/// references and excludes resources whose decoded body exceeds 3 MiB.
/// Moli stores materialized text as both UTF-8 bytes and a `String`, so
/// the total budget below charges both representations instead of only the
/// encoded body.
const DEFAULT_RETAINED_BYTES_LIMIT: usize = 15 * 1024 * 1024;
const DEFAULT_RESOURCE_BODY_BYTES_LIMIT: usize = 3 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SharedMemoryResourceCacheDiagnostics {
    pub entry_count: usize,
    pub pending_script_entry_count: usize,
    pub completed_script_entry_count: usize,
    pub raw_subresource_entry_count: usize,
    pub retained_bytes: usize,
    pub retained_bytes_limit: usize,
    pub resource_body_bytes_limit: usize,
}

pub(in crate::network) type SharedScriptTextLoad = Arc<ScriptTextLoad>;
type ScriptTextLoadResult = std::result::Result<Response, String>;
type ScriptTextLoadCallback = Box<dyn FnOnce(ScriptTextLoadResult) + Send + 'static>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(in crate::network) struct ScriptTextCacheKey {
    page_cache_partition_id: u64,
    url: String,
    credentials_mode: String,
    site_context: String,
    site_for_cookies_origin: String,
    top_frame_origin: String,
    network_partition_key: Option<String>,
    storage_access_status: String,
    integrity: Option<String>,
    request_headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(in crate::network) struct RawSubresourceCacheKey {
    page_cache_partition_id: u64,
    url: String,
    resource_type: &'static str,
    credentials_mode: String,
    network_partition_key: Option<String>,
    cookie_context: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum MemoryCacheKey {
    ScriptText(ScriptTextCacheKey),
    RawSubresource(RawSubresourceCacheKey),
}

impl ScriptTextCacheKey {
    pub(in crate::network) fn for_page_cache_partition(mut self, partition_id: u64) -> Self {
        self.page_cache_partition_id = partition_id;
        self
    }
}

impl RawSubresourceCacheKey {
    pub(in crate::network) fn for_page_cache_partition(mut self, partition_id: u64) -> Self {
        self.page_cache_partition_id = partition_id;
        self
    }
}

enum MemoryCacheEntry {
    ScriptText {
        load: SharedScriptTextLoad,
        expires_at_unix_ms: Option<u64>,
        retained_bytes: usize,
    },
    RawSubresource {
        response: Box<RawResponse>,
        expires_at_unix_ms: u64,
        retained_bytes: usize,
    },
}

impl MemoryCacheEntry {
    fn retained_bytes(&self) -> usize {
        match self {
            Self::ScriptText { retained_bytes, .. }
            | Self::RawSubresource { retained_bytes, .. } => *retained_bytes,
        }
    }

    fn is_completed(&self) -> bool {
        match self {
            Self::ScriptText {
                expires_at_unix_ms, ..
            } => expires_at_unix_ms.is_some(),
            Self::RawSubresource { .. } => true,
        }
    }

    fn is_expired(&self, now_unix_ms: u64) -> bool {
        match self {
            Self::ScriptText {
                expires_at_unix_ms: Some(expires_at_unix_ms),
                ..
            }
            | Self::RawSubresource {
                expires_at_unix_ms, ..
            } => *expires_at_unix_ms <= now_unix_ms,
            Self::ScriptText {
                expires_at_unix_ms: None,
                ..
            } => false,
        }
    }
}

pub(in crate::network) struct SharedMemoryResourceCache {
    entries: IndexMap<MemoryCacheKey, MemoryCacheEntry>,
    retained_bytes: usize,
    retained_bytes_limit: usize,
    resource_body_bytes_limit: usize,
}

pub(in crate::network) enum ScriptTextCacheLookup {
    Owner(SharedScriptTextLoad),
    PendingWaiter(SharedScriptTextLoad),
    CompletedHit(Box<ScriptTextLoadResult>),
}

pub(in crate::network) struct ScriptTextLoad {
    state: Mutex<ScriptTextLoadState>,
    notify: Notify,
}

#[derive(Default)]
struct ScriptTextLoadState {
    result: Option<ScriptTextLoadResult>,
    next_consumer_id: u64,
    // Preserve the callback order of the former Vec implementation. Different
    // consumers may enqueue observable parser tasks when the shared load
    // completes, so cancellation must not make the survivors unordered.
    callbacks: IndexMap<u64, ScriptTextLoadCallback>,
    transport_cancel: Option<moli_fetch::FetchCancelHandle>,
}

/// Cancellable registration for one context waiting on a shared script load.
///
/// Dropping one lease removes only that callback. If it was the last pending
/// consumer, the shared transport is cancelled; other Documents or Workers
/// are never affected by a sibling consumer retiring.
pub(in crate::network) struct ScriptTextConsumerLease {
    load: Weak<ScriptTextLoad>,
    consumer_id: u64,
}

impl ScriptTextConsumerLease {
    pub(in crate::network) fn cancel(&self) {
        if let Some(load) = self.load.upgrade() {
            load.cancel_consumer(self.consumer_id);
        }
    }
}

impl Drop for ScriptTextConsumerLease {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl ScriptTextLoad {
    fn pending() -> SharedScriptTextLoad {
        Arc::new(Self {
            state: Mutex::new(ScriptTextLoadState::default()),
            notify: Notify::new(),
        })
    }

    fn try_result(&self) -> Option<ScriptTextLoadResult> {
        self.state.lock().result.clone()
    }

    pub(in crate::network) async fn wait(&self) -> ScriptTextLoadResult {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(result) = self.try_result() {
                return result;
            }
            notified.await;
        }
    }

    pub(in crate::network) fn wait_callback(
        self: &Arc<Self>,
        callback: ScriptTextLoadCallback,
    ) -> Option<ScriptTextConsumerLease> {
        let mut state = self.state.lock();
        if let Some(result) = state.result.clone() {
            drop(state);
            callback(result);
            return None;
        }
        state.next_consumer_id = state
            .next_consumer_id
            .checked_add(1)
            .expect("script text consumer id exhausted");
        let consumer_id = state.next_consumer_id;
        let previous = state.callbacks.insert(consumer_id, callback);
        debug_assert!(previous.is_none());
        Some(ScriptTextConsumerLease {
            load: Arc::downgrade(self),
            consumer_id,
        })
    }

    pub(in crate::network) fn attach_transport_cancel(
        &self,
        cancel_handle: moli_fetch::FetchCancelHandle,
    ) {
        let cancel_immediately = {
            let mut state = self.state.lock();
            if state.result.is_some() || state.callbacks.is_empty() {
                true
            } else {
                debug_assert!(
                    state.transport_cancel.is_none(),
                    "shared script load must own exactly one transport"
                );
                state.transport_cancel = Some(cancel_handle.clone());
                false
            }
        };
        if cancel_immediately {
            cancel_handle.cancel();
        }
    }

    fn cancel_consumer(&self, consumer_id: u64) {
        let transport_cancel = {
            let mut state = self.state.lock();
            if state.callbacks.shift_remove(&consumer_id).is_none()
                || state.result.is_some()
                || !state.callbacks.is_empty()
            {
                None
            } else {
                state.transport_cancel.take()
            }
        };
        if let Some(transport_cancel) = transport_cancel {
            transport_cancel.cancel();
        }
    }

    pub(in crate::network) fn finish(&self, result: ScriptTextLoadResult) {
        let callbacks = {
            let mut state = self.state.lock();
            if state.result.is_none() {
                state.result = Some(result.clone());
                state.transport_cancel.take();
                std::mem::take(&mut state.callbacks)
                    .into_values()
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            }
        };
        self.notify.notify_waiters();
        for callback in callbacks {
            callback(result.clone());
        }
    }
}

impl Default for SharedMemoryResourceCache {
    fn default() -> Self {
        Self::with_limits(
            DEFAULT_RETAINED_BYTES_LIMIT,
            DEFAULT_RESOURCE_BODY_BYTES_LIMIT,
        )
    }
}

impl SharedMemoryResourceCache {
    fn with_limits(retained_bytes_limit: usize, resource_body_bytes_limit: usize) -> Self {
        Self {
            entries: IndexMap::new(),
            retained_bytes: 0,
            retained_bytes_limit,
            resource_body_bytes_limit,
        }
    }

    pub(in crate::network) fn lookup_script_text(
        &mut self,
        key: ScriptTextCacheKey,
    ) -> ScriptTextCacheLookup {
        let cache_key = MemoryCacheKey::ScriptText(key);
        if let Some(entry) = self.remove_entry(&cache_key) {
            let MemoryCacheEntry::ScriptText {
                load,
                expires_at_unix_ms,
                retained_bytes,
            } = entry
            else {
                unreachable!("script cache key must map to a script cache entry");
            };

            if expires_at_unix_ms.is_some_and(|expires_at| expires_at <= unix_now_ms()) {
                return self.insert_pending_script(cache_key);
            }

            let result = load.try_result();
            self.insert_entry(
                cache_key,
                MemoryCacheEntry::ScriptText {
                    load: Arc::clone(&load),
                    expires_at_unix_ms,
                    retained_bytes,
                },
            );
            return match result {
                Some(result) => ScriptTextCacheLookup::CompletedHit(Box::new(result)),
                None => ScriptTextCacheLookup::PendingWaiter(load),
            };
        }

        self.insert_pending_script(cache_key)
    }

    pub(in crate::network) fn replace_script_text(
        &mut self,
        key: ScriptTextCacheKey,
    ) -> ScriptTextCacheLookup {
        let cache_key = MemoryCacheKey::ScriptText(key);
        self.remove_entry(&cache_key);
        self.insert_pending_script(cache_key)
    }

    fn insert_pending_script(&mut self, key: MemoryCacheKey) -> ScriptTextCacheLookup {
        let load = ScriptTextLoad::pending();
        self.insert_entry(
            key,
            MemoryCacheEntry::ScriptText {
                load: Arc::clone(&load),
                expires_at_unix_ms: None,
                retained_bytes: 0,
            },
        );
        ScriptTextCacheLookup::Owner(load)
    }

    /// Finalizes cache metadata before `ScriptTextLoad::finish()` invokes
    /// callbacks. This ordering lets reentrant callbacks observe either a
    /// complete retained entry or no cache entry, never a completed load with
    /// stale pending metadata.
    pub(in crate::network) fn complete_script_text(
        &mut self,
        key: &ScriptTextCacheKey,
        load: &SharedScriptTextLoad,
        request: &Request,
        result: &ScriptTextLoadResult,
    ) {
        let cache_key = MemoryCacheKey::ScriptText(key.clone());
        let Some(entry) = self.remove_entry(&cache_key) else {
            return;
        };
        let MemoryCacheEntry::ScriptText {
            load: cached_load,
            expires_at_unix_ms,
            retained_bytes,
        } = entry
        else {
            unreachable!("script cache key must map to a script cache entry");
        };
        if !Arc::ptr_eq(&cached_load, load) {
            self.insert_entry(
                cache_key,
                MemoryCacheEntry::ScriptText {
                    load: cached_load,
                    expires_at_unix_ms,
                    retained_bytes,
                },
            );
            return;
        }

        let Some(response) = result.as_ref().ok() else {
            return;
        };
        let Some(expires_at_unix_ms) = script_text_memory_cache_expiry(request, response) else {
            return;
        };
        if response.body_text().len() > self.resource_body_bytes_limit
            || response.body_bytes().len() > self.resource_body_bytes_limit
        {
            return;
        }

        let retained_bytes = script_text_retained_bytes(key, response);
        if retained_bytes > self.retained_bytes_limit {
            return;
        }
        self.insert_entry(
            cache_key,
            MemoryCacheEntry::ScriptText {
                load: cached_load,
                expires_at_unix_ms: Some(expires_at_unix_ms),
                retained_bytes,
            },
        );
        self.evict_to_budget();
    }

    pub(in crate::network) fn lookup_raw_subresource(
        &mut self,
        key: &RawSubresourceCacheKey,
    ) -> Option<RawResponse> {
        let cache_key = MemoryCacheKey::RawSubresource(key.clone());
        let entry = self.remove_entry(&cache_key)?;
        let MemoryCacheEntry::RawSubresource {
            response,
            expires_at_unix_ms,
            retained_bytes,
        } = entry
        else {
            unreachable!("raw cache key must map to a raw cache entry");
        };
        if expires_at_unix_ms <= unix_now_ms() {
            return None;
        }
        let hit = response.as_ref().clone();
        self.insert_entry(
            cache_key,
            MemoryCacheEntry::RawSubresource {
                response,
                expires_at_unix_ms,
                retained_bytes,
            },
        );
        Some(hit)
    }

    pub(in crate::network) fn insert_raw_subresource(
        &mut self,
        key: RawSubresourceCacheKey,
        response: RawResponse,
        expires_at_unix_ms: u64,
    ) {
        if response.body_bytes().len() > self.resource_body_bytes_limit {
            return;
        }
        let retained_bytes = raw_subresource_retained_bytes(&key, &response);
        if retained_bytes > self.retained_bytes_limit {
            return;
        }

        let cache_key = MemoryCacheKey::RawSubresource(key);
        self.remove_entry(&cache_key);
        self.insert_entry(
            cache_key,
            MemoryCacheEntry::RawSubresource {
                response: Box::new(response),
                expires_at_unix_ms,
                retained_bytes,
            },
        );
        self.evict_to_budget();
    }

    fn insert_entry(&mut self, key: MemoryCacheKey, entry: MemoryCacheEntry) {
        self.retained_bytes = self.retained_bytes.saturating_add(entry.retained_bytes());
        let replaced = self.entries.insert(key, entry);
        debug_assert!(
            replaced.is_none(),
            "cache entries must be removed before insert"
        );
    }

    fn remove_entry(&mut self, key: &MemoryCacheKey) -> Option<MemoryCacheEntry> {
        let entry = self.entries.shift_remove(key)?;
        self.retained_bytes = self.retained_bytes.saturating_sub(entry.retained_bytes());
        Some(entry)
    }

    fn evict_to_budget(&mut self) {
        let now_unix_ms = unix_now_ms();
        let mut expired_retained_bytes = 0usize;
        self.entries.retain(|_, entry| {
            if entry.is_expired(now_unix_ms) {
                expired_retained_bytes =
                    expired_retained_bytes.saturating_add(entry.retained_bytes());
                false
            } else {
                true
            }
        });
        self.retained_bytes = self.retained_bytes.saturating_sub(expired_retained_bytes);

        while self.retained_bytes > self.retained_bytes_limit {
            let Some(index) = self
                .entries
                .values()
                .position(MemoryCacheEntry::is_completed)
            else {
                break;
            };
            let (_, entry) = self
                .entries
                .shift_remove_index(index)
                .expect("completed cache entry index should remain valid");
            self.retained_bytes = self.retained_bytes.saturating_sub(entry.retained_bytes());
        }
    }

    pub(in crate::network) fn diagnostics(&self) -> SharedMemoryResourceCacheDiagnostics {
        let mut pending_script_entry_count = 0;
        let mut completed_script_entry_count = 0;
        let mut raw_subresource_entry_count = 0;
        for entry in self.entries.values() {
            match entry {
                MemoryCacheEntry::ScriptText {
                    expires_at_unix_ms: Some(_),
                    ..
                } => completed_script_entry_count += 1,
                MemoryCacheEntry::ScriptText {
                    expires_at_unix_ms: None,
                    ..
                } => pending_script_entry_count += 1,
                MemoryCacheEntry::RawSubresource { .. } => raw_subresource_entry_count += 1,
            }
        }
        SharedMemoryResourceCacheDiagnostics {
            entry_count: self.entries.len(),
            pending_script_entry_count,
            completed_script_entry_count,
            raw_subresource_entry_count,
            retained_bytes: self.retained_bytes,
            retained_bytes_limit: self.retained_bytes_limit,
            resource_body_bytes_limit: self.resource_body_bytes_limit,
        }
    }

    #[cfg(test)]
    fn contains_script_text(&self, key: &ScriptTextCacheKey) -> bool {
        self.entries
            .contains_key(&MemoryCacheKey::ScriptText(key.clone()))
    }

    #[cfg(test)]
    fn contains_raw_subresource(&self, key: &RawSubresourceCacheKey) -> bool {
        self.entries
            .contains_key(&MemoryCacheKey::RawSubresource(key.clone()))
    }
}

pub(in crate::network) fn script_text_request_is_memory_cacheable(request: &Request) -> bool {
    request.cache_mode().allows_memory_cache_store()
        && request.subresource_request_metadata().is_some()
        && request.method.eq_ignore_ascii_case("GET")
        && request.body.is_none()
        && request.auth().is_none()
        && request.follow_redirects
}

pub(in crate::network) fn script_text_cache_key(request: &Request) -> ScriptTextCacheKey {
    let browser_context = &request.cookie_context.browser_context;
    ScriptTextCacheKey {
        page_cache_partition_id: 0,
        url: request.url.as_str().to_owned(),
        credentials_mode: request.credentials_mode.as_ref().to_owned(),
        site_context: format!("{:?}", request.cookie_context.site_context),
        site_for_cookies_origin: script_cache_partition_url_component(
            browser_context.site_for_cookies_url.as_ref(),
            &request.url,
        ),
        top_frame_origin: script_cache_partition_url_component(
            browser_context
                .top_frame_origin_url
                .as_ref()
                .or(browser_context.site_for_cookies_url.as_ref()),
            &request.url,
        ),
        network_partition_key: request.network_partition_key().map(str::to_owned),
        storage_access_status: format!("{:?}", browser_context.storage_access_status),
        integrity: request
            .subresource_request_metadata()
            .and_then(|metadata| metadata.integrity.clone()),
        request_headers: request.request_headers.clone(),
    }
}

fn script_cache_partition_url_component(url: Option<&Url>, fallback_url: &Url) -> String {
    url.map(moli_url::origin_ascii_serialization)
        .unwrap_or_else(|| moli_url::origin_ascii_serialization(fallback_url))
}

pub(in crate::network) fn raw_subresource_memory_cache_key(
    request: &Request,
) -> Option<RawSubresourceCacheKey> {
    if !raw_subresource_request_is_memory_cacheable(request) {
        return None;
    }
    Some(RawSubresourceCacheKey {
        page_cache_partition_id: 0,
        url: request.url.as_str().to_owned(),
        resource_type: raw_subresource_cache_resource_type_key(request.resource_type),
        credentials_mode: request.credentials_mode.as_ref().to_owned(),
        network_partition_key: request.network_partition_key().map(str::to_owned),
        cookie_context: format!("{:?}", request.cookie_context),
    })
}

fn raw_subresource_cache_resource_type_key(resource_type: RequestResourceType) -> &'static str {
    match resource_type {
        RequestResourceType::Raw => "raw",
        RequestResourceType::CssStyleSheet => "stylesheet",
        RequestResourceType::Script
        | RequestResourceType::ParserBlockingScript
        | RequestResourceType::ClassicAsyncOrDeferScript
        | RequestResourceType::LatePreloadScript => "script",
        RequestResourceType::Image => "image",
        RequestResourceType::Font => "font",
        RequestResourceType::Media => "media",
        RequestResourceType::TextTrack => "text-track",
        _ => "other",
    }
}

fn raw_subresource_request_is_memory_cacheable(request: &Request) -> bool {
    request.cache_mode().allows_memory_cache_store()
        && raw_subresource_memory_cacheable_resource_type(request)
        && request.method.eq_ignore_ascii_case("GET")
        && request.body.is_none()
        && request.auth().is_none()
        && request.follow_redirects
        && raw_subresource_memory_cacheable_headers(request)
}

fn raw_subresource_memory_cacheable_resource_type(request: &Request) -> bool {
    matches!(
        request.resource_type,
        RequestResourceType::Image
            | RequestResourceType::Font
            | RequestResourceType::Media
            | RequestResourceType::TextTrack
            | RequestResourceType::CssStyleSheet
    ) || matches!(
        (request.resource_type, request.browser_request_metadata()),
        (
            RequestResourceType::Raw,
            Some(
                BrowserRequestMetadata::Fetch
                    | BrowserRequestMetadata::JsonModule
                    | BrowserRequestMetadata::Manifest
                    | BrowserRequestMetadata::StyleModule
                    | BrowserRequestMetadata::Xhr
            )
        )
    )
}

fn raw_subresource_memory_cacheable_headers(request: &Request) -> bool {
    request.request_headers.is_empty()
        || request.browser_request_metadata().is_some_and(|metadata| {
            matches!(
                metadata,
                BrowserRequestMetadata::Fetch
                    | BrowserRequestMetadata::JsonModule
                    | BrowserRequestMetadata::Manifest
                    | BrowserRequestMetadata::StyleModule
                    | BrowserRequestMetadata::Xhr
            ) && request
                .request_headers
                .iter()
                .all(|(name, _)| browser_cache_ignored_request_header(name))
        })
}

fn browser_cache_ignored_request_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "accept" | "accept-language" | "sec-fetch-dest" | "sec-fetch-mode" | "sec-fetch-site"
    )
}

pub(in crate::network) fn raw_subresource_memory_cache_expiry(
    request: &Request,
    response: &RawResponse,
) -> Option<u64> {
    let policy = cacheable_response_parts_policy(
        &request.url,
        &response.final_url,
        response.status,
        &response.headers,
        response.redirected,
    )?;
    let expires_at = policy.expires_at_unix_ms?;
    (expires_at > unix_now_ms()).then_some(expires_at)
}

fn script_text_memory_cache_expiry(request: &Request, response: &Response) -> Option<u64> {
    let policy = cacheable_response_parts_policy(
        &request.url,
        &response.final_url,
        response.status,
        &response.headers,
        response.redirected,
    )?;
    let expires_at = policy.expires_at_unix_ms?;
    (expires_at > unix_now_ms()).then_some(expires_at)
}

fn script_text_retained_bytes(key: &ScriptTextCacheKey, response: &Response) -> usize {
    script_text_key_retained_bytes(key)
        .saturating_add(response_head_retained_bytes(
            response.final_url.as_str(),
            &response.headers,
        ))
        .saturating_add(response.body_text().len())
        .saturating_add(response.body_bytes().len())
}

fn raw_subresource_retained_bytes(key: &RawSubresourceCacheKey, response: &RawResponse) -> usize {
    raw_subresource_key_retained_bytes(key)
        .saturating_add(response_head_retained_bytes(
            response.final_url.as_str(),
            &response.headers,
        ))
        .saturating_add(response.body_bytes().len())
}

fn response_head_retained_bytes(final_url: &str, headers: &[(String, String)]) -> usize {
    headers
        .iter()
        .fold(final_url.len(), |bytes, (name, value)| {
            bytes.saturating_add(name.len()).saturating_add(value.len())
        })
}

fn script_text_key_retained_bytes(key: &ScriptTextCacheKey) -> usize {
    let request_headers = key
        .request_headers
        .iter()
        .fold(0usize, |bytes, (name, value)| {
            bytes.saturating_add(name.len()).saturating_add(value.len())
        });
    key.url
        .len()
        .saturating_add(key.credentials_mode.len())
        .saturating_add(key.site_context.len())
        .saturating_add(key.site_for_cookies_origin.len())
        .saturating_add(key.top_frame_origin.len())
        .saturating_add(key.network_partition_key.as_deref().map_or(0, str::len))
        .saturating_add(key.storage_access_status.len())
        .saturating_add(key.integrity.as_deref().map_or(0, str::len))
        .saturating_add(request_headers)
}

fn raw_subresource_key_retained_bytes(key: &RawSubresourceCacheKey) -> usize {
    key.url
        .len()
        .saturating_add(key.resource_type.len())
        .saturating_add(key.credentials_mode.len())
        .saturating_add(key.network_partition_key.as_deref().map_or(0, str::len))
        .saturating_add(key.cookie_context.len())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use moli_fetch::{ResponseHead, ScriptFetchRequestMetadata};

    use super::*;

    fn script_request(url: &str) -> Request {
        Request::get(url)
            .expect("script request URL")
            .with_script_fetch_metadata(ScriptFetchRequestMetadata::default())
    }

    fn response(url: &str, body: &str) -> Response {
        Response::from_head_and_text_body(
            ResponseHead {
                final_url: Url::parse(url).expect("response URL"),
                status: 200,
                headers: vec![("cache-control".to_owned(), "max-age=60".to_owned())],
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            },
            body.to_owned(),
        )
    }

    fn raw_response(url: &str, body: &[u8]) -> RawResponse {
        RawResponse::from_head_and_body(
            ResponseHead {
                final_url: Url::parse(url).expect("response URL"),
                status: 200,
                headers: vec![("cache-control".to_owned(), "max-age=60".to_owned())],
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            },
            body.to_vec(),
        )
    }

    fn insert_script(
        cache: &mut SharedMemoryResourceCache,
        request: &Request,
        response: Response,
    ) -> SharedScriptTextLoad {
        let key = script_text_cache_key(request);
        let ScriptTextCacheLookup::Owner(load) = cache.lookup_script_text(key.clone()) else {
            panic!("new script key should own its load");
        };
        let result = Ok(response);
        cache.complete_script_text(&key, &load, request, &result);
        load.finish(result);
        load
    }

    #[test]
    fn cancelling_one_shared_script_consumer_preserves_its_sibling() {
        let load = ScriptTextLoad::pending();
        let delivered = Arc::new(AtomicUsize::new(0));
        let first_delivered = Arc::clone(&delivered);
        let first = load
            .wait_callback(Box::new(move |_| {
                first_delivered.fetch_add(1, Ordering::Relaxed);
            }))
            .expect("first pending consumer");
        let second_delivered = Arc::clone(&delivered);
        let second = load
            .wait_callback(Box::new(move |_| {
                second_delivered.fetch_add(1, Ordering::Relaxed);
            }))
            .expect("second pending consumer");
        let transport_cancel = moli_fetch::FetchCancelHandle::new();
        load.attach_transport_cancel(transport_cancel.clone());

        first.cancel();
        assert!(!transport_cancel.is_cancelled());
        load.finish(Err("terminal".to_owned()));

        assert_eq!(delivered.load(Ordering::Relaxed), 1);
        drop(second);
    }

    #[test]
    fn cancelling_last_shared_script_consumer_cancels_transport() {
        let load = ScriptTextLoad::pending();
        let first = load
            .wait_callback(Box::new(|_| {}))
            .expect("first pending consumer");
        let second = load
            .wait_callback(Box::new(|_| {}))
            .expect("second pending consumer");
        let transport_cancel = moli_fetch::FetchCancelHandle::new();
        load.attach_transport_cancel(transport_cancel.clone());

        first.cancel();
        assert!(!transport_cancel.is_cancelled());
        second.cancel();

        assert!(transport_cancel.is_cancelled());
    }

    #[test]
    fn completed_script_entries_are_evicted_in_lru_order() {
        let first_request = script_request("https://cache.test/first.js");
        let second_request = script_request("https://cache.test/second.js");
        let first_response = response(first_request.url.as_str(), &"a".repeat(256));
        let second_response = response(second_request.url.as_str(), &"b".repeat(256));
        let first_key = script_text_cache_key(&first_request);
        let second_key = script_text_cache_key(&second_request);
        let first_weight = script_text_retained_bytes(&first_key, &first_response);
        let second_weight = script_text_retained_bytes(&second_key, &second_response);
        let mut cache =
            SharedMemoryResourceCache::with_limits(first_weight.max(second_weight), usize::MAX);

        insert_script(&mut cache, &first_request, first_response);
        insert_script(&mut cache, &second_request, second_response);

        assert!(!cache.contains_script_text(&first_key));
        assert!(cache.contains_script_text(&second_key));
        assert!(cache.retained_bytes <= cache.retained_bytes_limit);
    }

    #[test]
    fn script_cache_hit_refreshes_lru_position() {
        let requests = [
            script_request("https://cache.test/first.js"),
            script_request("https://cache.test/second.js"),
            script_request("https://cache.test/third.js"),
        ];
        let responses = [
            response(requests[0].url.as_str(), &"a".repeat(128)),
            response(requests[1].url.as_str(), &"b".repeat(128)),
            response(requests[2].url.as_str(), &"c".repeat(128)),
        ];
        let keys: [ScriptTextCacheKey; 3] =
            std::array::from_fn(|index| script_text_cache_key(&requests[index]));
        let weights = [
            script_text_retained_bytes(&keys[0], &responses[0]),
            script_text_retained_bytes(&keys[1], &responses[1]),
            script_text_retained_bytes(&keys[2], &responses[2]),
        ];
        let mut cache = SharedMemoryResourceCache::with_limits(
            weights[0]
                .saturating_add(weights[1])
                .max(weights[0].saturating_add(weights[2])),
            usize::MAX,
        );

        insert_script(&mut cache, &requests[0], responses[0].clone());
        insert_script(&mut cache, &requests[1], responses[1].clone());
        assert!(matches!(
            cache.lookup_script_text(keys[0].clone()),
            ScriptTextCacheLookup::CompletedHit(_)
        ));
        insert_script(&mut cache, &requests[2], responses[2].clone());

        assert!(cache.contains_script_text(&keys[0]));
        assert!(!cache.contains_script_text(&keys[1]));
        assert!(cache.contains_script_text(&keys[2]));
    }

    #[test]
    fn oversized_script_completes_waiters_without_becoming_retained() {
        let request = script_request("https://cache.test/large.js");
        let key = script_text_cache_key(&request);
        let mut cache = SharedMemoryResourceCache::with_limits(1024, 32);
        let ScriptTextCacheLookup::Owner(load) = cache.lookup_script_text(key.clone()) else {
            panic!("new script key should own its load");
        };
        let result = Ok(response(request.url.as_str(), &"x".repeat(64)));

        cache.complete_script_text(&key, &load, &request, &result);
        load.finish(result);

        assert!(!cache.contains_script_text(&key));
        assert!(load.try_result().is_some());
    }

    #[test]
    fn decoded_script_limit_is_independent_from_original_byte_length() {
        let request = script_request("https://cache.test/expanded.js");
        let key = script_text_cache_key(&request);
        let mut cache = SharedMemoryResourceCache::with_limits(1024, 32);
        let ScriptTextCacheLookup::Owner(load) = cache.lookup_script_text(key.clone()) else {
            panic!("new script key should own its load");
        };
        let response = Response::from_head_and_body(
            ResponseHead {
                final_url: request.url.clone(),
                status: 200,
                headers: vec![("cache-control".to_owned(), "max-age=60".to_owned())],
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            },
            "x".repeat(64),
            vec![b'x'; 16],
        );
        let result = Ok(response);

        cache.complete_script_text(&key, &load, &request, &result);
        load.finish(result);

        assert!(!cache.contains_script_text(&key));
        assert!(load.try_result().is_some());
    }

    #[test]
    fn raw_and_script_entries_share_one_retained_byte_budget() {
        let script_request = script_request("https://cache.test/app.js");
        let script_response = response(script_request.url.as_str(), &"s".repeat(128));
        let script_key = script_text_cache_key(&script_request);
        let script_weight = script_text_retained_bytes(&script_key, &script_response);
        let raw_request = Request::get("https://cache.test/app.css")
            .expect("raw request")
            .with_page_network_policy()
            .with_resource_type(RequestResourceType::CssStyleSheet);
        let raw_key =
            raw_subresource_memory_cache_key(&raw_request).expect("cacheable raw request key");
        let raw_response = raw_response(raw_request.url.as_str(), &[b'r'; 128]);
        let raw_weight = raw_subresource_retained_bytes(&raw_key, &raw_response);
        let mut cache =
            SharedMemoryResourceCache::with_limits(script_weight.max(raw_weight), usize::MAX);

        insert_script(&mut cache, &script_request, script_response);
        cache.insert_raw_subresource(raw_key.clone(), raw_response, u64::MAX);

        assert!(!cache.contains_script_text(&script_key));
        assert!(cache.contains_raw_subresource(&raw_key));
        assert!(cache.retained_bytes <= cache.retained_bytes_limit);
    }

    #[test]
    fn expired_script_entry_is_not_returned_as_a_completed_hit() {
        let request = script_request("https://cache.test/expired.js");
        let key = script_text_cache_key(&request);
        let mut cache = SharedMemoryResourceCache::with_limits(usize::MAX, usize::MAX);
        let ScriptTextCacheLookup::Owner(load) = cache.lookup_script_text(key.clone()) else {
            panic!("new script key should own its load");
        };
        let result = Ok(response(request.url.as_str(), "expired"));
        cache.remove_entry(&MemoryCacheKey::ScriptText(key.clone()));
        cache.insert_entry(
            MemoryCacheKey::ScriptText(key.clone()),
            MemoryCacheEntry::ScriptText {
                load: Arc::clone(&load),
                expires_at_unix_ms: Some(0),
                retained_bytes: 0,
            },
        );
        load.finish(result);

        assert!(matches!(
            cache.lookup_script_text(key),
            ScriptTextCacheLookup::Owner(_)
        ));
    }

    #[test]
    fn diagnostics_report_one_shared_bounded_cache() {
        let script_request = script_request("https://cache.test/app.js");
        let script_response = response(script_request.url.as_str(), &"s".repeat(128));
        let raw_request = Request::get("https://cache.test/app.css")
            .expect("raw request")
            .with_page_network_policy()
            .with_resource_type(RequestResourceType::CssStyleSheet);
        let raw_key =
            raw_subresource_memory_cache_key(&raw_request).expect("cacheable raw request key");
        let raw_response = raw_response(raw_request.url.as_str(), &[b'r'; 128]);
        let mut cache = SharedMemoryResourceCache::with_limits(1024 * 1024, 4096);

        insert_script(&mut cache, &script_request, script_response);
        cache.insert_raw_subresource(raw_key, raw_response, u64::MAX);

        let diagnostics = cache.diagnostics();
        assert_eq!(diagnostics.entry_count, 2);
        assert_eq!(diagnostics.pending_script_entry_count, 0);
        assert_eq!(diagnostics.completed_script_entry_count, 1);
        assert_eq!(diagnostics.raw_subresource_entry_count, 1);
        assert!(diagnostics.retained_bytes > 0);
        assert!(diagnostics.retained_bytes <= diagnostics.retained_bytes_limit);
        assert_eq!(diagnostics.resource_body_bytes_limit, 4096);
    }

    #[test]
    fn insertion_prunes_expired_entries_before_lru_eviction() {
        let expired_request = script_request("https://cache.test/expired.js");
        let fresh_request = script_request("https://cache.test/fresh.js");
        let expired_key = script_text_cache_key(&expired_request);
        let fresh_key = script_text_cache_key(&fresh_request);
        let mut cache = SharedMemoryResourceCache::with_limits(usize::MAX, usize::MAX);

        let ScriptTextCacheLookup::Owner(expired_load) =
            cache.lookup_script_text(expired_key.clone())
        else {
            panic!("new expired script key should own its load");
        };
        cache.remove_entry(&MemoryCacheKey::ScriptText(expired_key.clone()));
        cache.insert_entry(
            MemoryCacheKey::ScriptText(expired_key.clone()),
            MemoryCacheEntry::ScriptText {
                load: expired_load,
                expires_at_unix_ms: Some(0),
                retained_bytes: 128,
            },
        );

        insert_script(
            &mut cache,
            &fresh_request,
            response(fresh_request.url.as_str(), "fresh"),
        );

        assert!(!cache.contains_script_text(&expired_key));
        assert!(cache.contains_script_text(&fresh_key));
        assert_eq!(
            cache.retained_bytes,
            cache
                .entries
                .values()
                .map(MemoryCacheEntry::retained_bytes)
                .sum::<usize>()
        );
    }
}
