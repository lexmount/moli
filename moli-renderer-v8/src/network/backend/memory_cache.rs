//! Browser-resource-runtime scoped renderer memory cache.
//!
//! The cache deliberately outlives individual Document and Worker loaders. Its
//! retained-byte budget is shared by every consumer of one browser resource
//! runtime and must never be multiplied per execution context. Entries retain
//! the owning Page cache partition, so a DevTools cache bypass can replace one
//! Page's retained resource without rewriting a live peer Page's resource.

use std::sync::{Arc, Weak};

use indexmap::IndexMap;
use moli_fetch::{
    BrowserRequestMetadata, RawResponse, Request, RequestHeaders, RequestResourceType, Response,
};
use moli_http_cache::{
    HttpCacheRequestPolicy, cacheable_response_parts_policy, response_date, unix_now_ms,
    validation_headers_from_headers,
};
use parking_lot::Mutex;
use tokio::sync::oneshot;
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
pub(in crate::network) type ScriptResponseStartCallback = Box<dyn FnOnce() + Send + 'static>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(in crate::network) struct ScriptTextCacheKey {
    page_cache_partition_id: u64,
    url: String,
    credentials_mode: String,
    request_origin: Option<String>,
    request_mode: String,
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
    follow_redirects: bool,
    credentials_mode: String,
    request_origin: Option<String>,
    request_mode: String,
    referrer: Option<String>,
    network_partition_key: Option<String>,
    cookie_context: String,
    serialized_origin: String,
    has_cross_origin_url: bool,
}

pub(in crate::network) enum RawSubresourceCacheLookup {
    Local {
        response: Box<RawResponse>,
        expires_at_unix_ms: u64,
    },
    Revalidate(Box<RawResponse>),
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
        variants: Vec<RawSubresourceVariant>,
        retained_bytes: usize,
    },
}

struct RawSubresourceVariant {
    response: Box<RawResponse>,
    stored_at_unix_ms: u64,
    expires_at_unix_ms: u64,
    response_time: std::time::SystemTime,
    vary_headers: Vec<(String, Option<Vec<u8>>)>,
}

impl RawSubresourceVariant {
    fn matches(&self, request_headers: &RequestHeaders) -> bool {
        self.vary_headers
            .iter()
            .all(|(name, value)| *value == combined_request_header_value(request_headers, name))
    }

    fn retained_bytes(&self) -> usize {
        response_head_retained_bytes(self.response.final_url.as_str(), &self.response.headers)
            .saturating_add(self.response.body_bytes().len())
            .saturating_add(
                self.vary_headers
                    .iter()
                    .fold(0usize, |bytes, (name, value)| {
                        bytes
                            .saturating_add(name.len())
                            .saturating_add(value.as_ref().map_or(0, Vec::len))
                    }),
            )
    }
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
            } => *expires_at_unix_ms <= now_unix_ms,
            // Stale raw representations retain validators for the next
            // request. They remain subject to the same byte budget and LRU.
            Self::RawSubresource { .. } => false,
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
}

#[derive(Default)]
struct ScriptTextLoadState {
    result: Option<ScriptTextLoadResult>,
    response_started: bool,
    response_start_callbacks_in_progress: usize,
    next_consumer_id: u64,
    // Preserve the callback order of the former Vec implementation. Different
    // consumers may enqueue observable parser tasks when the shared load
    // completes, so cancellation must not make the survivors unordered.
    callbacks: IndexMap<u64, ScriptTextConsumerCallbacks>,
    transport_cancel: Option<moli_fetch::FetchCancelHandle>,
}

struct ScriptTextConsumerCallbacks {
    completion: ScriptTextLoadCallback,
    response_start: Option<ScriptResponseStartCallback>,
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
        })
    }

    fn try_result(&self) -> Option<ScriptTextLoadResult> {
        self.state.lock().result.clone()
    }

    pub(in crate::network) async fn wait(self: &Arc<Self>) -> ScriptTextLoadResult {
        let (sender, receiver) = oneshot::channel();
        let _consumer = self.wait_callback(Box::new(move |result| {
            let _ = sender.send(result);
        }));
        receiver
            .await
            .unwrap_or_else(|_| Err("script text consumer retired".to_owned()))
    }

    pub(in crate::network) fn wait_callback(
        self: &Arc<Self>,
        callback: ScriptTextLoadCallback,
    ) -> Option<ScriptTextConsumerLease> {
        self.wait_callback_with_response_start(callback, None)
    }

    pub(in crate::network) fn wait_callback_with_response_start(
        self: &Arc<Self>,
        callback: ScriptTextLoadCallback,
        mut response_start: Option<ScriptResponseStartCallback>,
    ) -> Option<ScriptTextConsumerLease> {
        let mut state = self.state.lock();
        if let Some(result) = state.result.clone() {
            drop(state);
            if result.is_ok()
                && let Some(response_start) = response_start
            {
                response_start();
            }
            callback(result);
            return None;
        }
        let ready_start = if state.response_started {
            response_start.take()
        } else {
            None
        };
        if ready_start.is_some() {
            state.response_start_callbacks_in_progress += 1;
        }
        state.next_consumer_id = state
            .next_consumer_id
            .checked_add(1)
            .expect("script text consumer id exhausted");
        let consumer_id = state.next_consumer_id;
        let previous = state.callbacks.insert(
            consumer_id,
            ScriptTextConsumerCallbacks {
                completion: callback,
                response_start,
            },
        );
        debug_assert!(previous.is_none());
        drop(state);
        if let Some(ready_start) = ready_start {
            // Register first so another consumer cannot cancel the transport
            // during this notification. Completion waits for it to return.
            ready_start();
            self.response_start_callback_finished();
        }
        Some(ScriptTextConsumerLease {
            load: Arc::downgrade(self),
            consumer_id,
        })
    }

    pub(in crate::network) fn notify_response_started(&self) {
        let callbacks = {
            let mut state = self.state.lock();
            if state.response_started || state.result.is_some() {
                return;
            }
            state.response_started = true;
            let callbacks = state
                .callbacks
                .values_mut()
                .filter_map(|consumer| consumer.response_start.take())
                .collect::<Vec<_>>();
            state.response_start_callbacks_in_progress += callbacks.len();
            callbacks
        };
        for callback in callbacks {
            callback();
            self.response_start_callback_finished();
        }
    }

    fn response_start_callback_finished(&self) {
        let completed = {
            let mut state = self.state.lock();
            state.response_start_callbacks_in_progress -= 1;
            state.take_ready_completions()
        };
        Self::deliver_completions(completed);
    }

    fn deliver_completions(
        completed: Option<(ScriptTextLoadResult, Vec<ScriptTextConsumerCallbacks>)>,
    ) {
        if let Some((result, callbacks)) = completed {
            for callback in callbacks {
                (callback.completion)(result.clone());
            }
        }
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
        let completed = {
            let mut state = self.state.lock();
            if state.result.is_some() {
                return;
            }
            state.result = Some(result);
            state.transport_cancel.take();
            state.take_ready_completions()
        };
        Self::deliver_completions(completed);
    }
}

impl ScriptTextLoadState {
    fn take_ready_completions(
        &mut self,
    ) -> Option<(ScriptTextLoadResult, Vec<ScriptTextConsumerCallbacks>)> {
        if self.response_start_callbacks_in_progress != 0 {
            return None;
        }
        let result = self.result.clone()?;
        let callbacks = std::mem::take(&mut self.callbacks).into_values().collect();
        Some((result, callbacks))
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
        request: &Request,
        request_headers: &RequestHeaders,
    ) -> Option<RawSubresourceCacheLookup> {
        if !request.cache_mode().allows_http_cache_lookup() {
            return None;
        }
        let cache_key = MemoryCacheKey::RawSubresource(key.clone());
        let MemoryCacheEntry::RawSubresource { variants, .. } = self.entries.get(&cache_key)?
        else {
            unreachable!("raw cache key must map to a raw cache entry");
        };
        // Prefer a matching Vary representation to an older default response,
        // then the most recent response Date, as required by RFC 9111. A
        // response without a usable Date uses its receipt time. Hits must not
        // rewrite the generation time or receipt order.
        let index = variants
            .iter()
            .enumerate()
            .filter(|(_, variant)| variant.matches(request_headers))
            .max_by_key(|(index, variant)| {
                (
                    !variant.vary_headers.is_empty(),
                    variant.response_time,
                    *index,
                )
            })
            .map(|(index, _)| index)?;
        let policy = HttpCacheRequestPolicy::from_headers(request_headers);
        let variant = &variants[index];
        let reusable = policy.allows_reuse_at(
            &variant.response.headers,
            variant.stored_at_unix_ms,
            Some(variant.expires_at_unix_ms),
            request.cache_mode().requires_http_cache_validation(),
            unix_now_ms(),
        );
        if !reusable && policy.only_if_cached {
            // A cache-only miss must not retire a representation that a
            // subsequent request with different constraints can still use.
            return None;
        }
        let entry = self.remove_entry(&cache_key)?;
        let MemoryCacheEntry::RawSubresource { mut variants, .. } = entry else {
            unreachable!("raw cache key must map to a raw cache entry");
        };
        if !reusable {
            // Retire the old entry before fetching its replacement. Failure,
            // cancellation or a non-storable response must not resurrect it
            // as a fresh hit for a later consumer.
            let mut variant = variants.remove(index);
            variant
                .response
                .final_url
                .set_fragment(request.url.fragment());
            variants.retain(|other| !other.matches(request_headers));
            self.insert_raw_subresource_variants(key.clone(), variants);
            return Some(RawSubresourceCacheLookup::Revalidate(variant.response));
        }
        let mut hit = variants[index].response.clone();
        let expires_at_unix_ms = variants[index].expires_at_unix_ms;
        hit.final_url.set_fragment(request.url.fragment());
        self.insert_raw_subresource_variants(key.clone(), variants);
        Some(RawSubresourceCacheLookup::Local {
            response: hit,
            expires_at_unix_ms,
        })
    }

    pub(in crate::network) fn remove_raw_subresource(
        &mut self,
        key: &RawSubresourceCacheKey,
        request_headers: &RequestHeaders,
    ) {
        let mut variants = self.take_raw_subresource_variants(key);
        variants.retain(|variant| !variant.matches(request_headers));
        self.insert_raw_subresource_variants(key.clone(), variants);
    }

    pub(in crate::network) fn insert_raw_subresource(
        &mut self,
        key: RawSubresourceCacheKey,
        response: RawResponse,
        expires_at_unix_ms: u64,
        request_headers: &RequestHeaders,
    ) {
        self.remove_raw_subresource(&key, request_headers);
        if response.body_bytes().len() > self.resource_body_bytes_limit {
            return;
        }
        let Some(vary_names) =
            moli_fetch::cache_vary_header_names(&response.final_url, &response.headers)
        else {
            return;
        };
        let variant = RawSubresourceVariant {
            stored_at_unix_ms: unix_now_ms(),
            response_time: response_date(&response.headers)
                .unwrap_or_else(std::time::SystemTime::now),
            response: Box::new(response),
            expires_at_unix_ms,
            vary_headers: vary_names
                .into_iter()
                .map(|name| {
                    let value = combined_request_header_value(request_headers, &name);
                    (name, value)
                })
                .collect(),
        };
        if raw_subresource_key_retained_bytes(&key).saturating_add(variant.retained_bytes())
            > self.retained_bytes_limit
        {
            return;
        }
        let mut variants = self.take_raw_subresource_variants(&key);
        variants.push(variant);
        self.insert_raw_subresource_variants(key, variants);
        self.evict_to_budget();
    }

    fn take_raw_subresource_variants(
        &mut self,
        key: &RawSubresourceCacheKey,
    ) -> Vec<RawSubresourceVariant> {
        match self.remove_entry(&MemoryCacheKey::RawSubresource(key.clone())) {
            Some(MemoryCacheEntry::RawSubresource { variants, .. }) => variants,
            None => Vec::new(),
            Some(_) => unreachable!("raw cache key must map to a raw cache entry"),
        }
    }

    fn insert_raw_subresource_variants(
        &mut self,
        key: RawSubresourceCacheKey,
        variants: Vec<RawSubresourceVariant>,
    ) {
        if variants.is_empty() {
            return;
        }
        let retained_bytes = variants.iter().fold(
            raw_subresource_key_retained_bytes(&key),
            |bytes, variant| bytes.saturating_add(variant.retained_bytes()),
        );
        self.insert_entry(
            MemoryCacheKey::RawSubresource(key),
            MemoryCacheEntry::RawSubresource {
                variants,
                retained_bytes,
            },
        );
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
                MemoryCacheEntry::RawSubresource { variants, .. } => {
                    raw_subresource_entry_count += variants.len()
                }
            }
        }
        SharedMemoryResourceCacheDiagnostics {
            entry_count: pending_script_entry_count
                + completed_script_entry_count
                + raw_subresource_entry_count,
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
        // These entries retain a complete response including its redirect
        // history. A resumed fetch must assemble its own history in transport.
        && request.redirect_chain().is_empty()
        && request.subresource_request_metadata().is_some()
        && request.method.eq_ignore_ascii_case("GET")
        && request.body.is_none()
        && request.auth().is_none()
        && request.transport_follows_redirects()
}

pub(in crate::network) fn script_text_cache_key(request: &Request) -> ScriptTextCacheKey {
    let browser_context = &request.cookie_context.browser_context;
    ScriptTextCacheKey {
        page_cache_partition_id: 0,
        url: request.url.as_str().to_owned(),
        credentials_mode: request.credentials_mode.as_ref().to_owned(),
        request_origin: request
            .request_origin()
            .map(|origin| origin.ascii_serialization().to_owned()),
        request_mode: request.request_mode.as_ref().to_owned(),
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
        request_headers: request.request_headers.to_byte_strings(),
    }
}

fn script_cache_partition_url_component(url: Option<&Url>, fallback_url: &Url) -> String {
    url.map(moli_url::origin_ascii_serialization)
        .unwrap_or_else(|| moli_url::origin_ascii_serialization(fallback_url))
}

pub(in crate::network) fn raw_subresource_memory_cache_key(
    request: &Request,
    request_headers: &RequestHeaders,
) -> Option<RawSubresourceCacheKey> {
    if !raw_subresource_request_is_memory_cacheable(request, request_headers) {
        return None;
    }
    let mut target_url = request.url.clone();
    target_url.set_fragment(None);
    Some(RawSubresourceCacheKey {
        page_cache_partition_id: 0,
        url: target_url.into(),
        resource_type: raw_subresource_cache_resource_type_key(request.resource_type),
        follow_redirects: request.transport_follows_redirects(),
        credentials_mode: request.credentials_mode.as_ref().to_owned(),
        request_origin: request
            .request_origin()
            .map(|origin| origin.ascii_serialization().to_owned()),
        request_mode: request.request_mode.as_ref().to_owned(),
        // Policies and explicit Fetch referrers can select different headers
        // without changing the origin or cookie initiator in this cache key.
        referrer: request.referrer_header_value(&request.url),
        network_partition_key: request.network_partition_key().map(str::to_owned),
        cookie_context: format!("{:?}", request.cookie_context),
        serialized_origin: request.serialized_origin(),
        has_cross_origin_url: request.has_cross_origin_url(&request.url),
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

fn raw_subresource_request_is_memory_cacheable(
    request: &Request,
    request_headers: &RequestHeaders,
) -> bool {
    request.cache_mode().allows_memory_cache_store()
        // A CORS-managed hop carries the logical request's earlier URLs. It
        // can cache this hop without treating the prefix as a followed
        // transport redirect; its origin/credential taint is part of the key.
        && (request.redirect_chain().is_empty() || !request.follow_redirects)
        && raw_subresource_memory_cacheable_resource_type(request)
        && request.method.eq_ignore_ascii_case("GET")
        && request.body.is_none()
        && request.auth().is_none()
        // The browser's CORS loop follows redirects itself. Its individual
        // transport requests can still reuse cached responses with an empty
        // URL list, in a separate bucket from automatically followed fetches.
        && request.redirect_mode() == moli_fetch::RequestRedirectMode::Follow
        && raw_subresource_memory_cacheable_headers(request_headers)
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
                    | BrowserRequestMetadata::TextModule
                    | BrowserRequestMetadata::Manifest
                    | BrowserRequestMetadata::StyleModule
                    | BrowserRequestMetadata::Xhr
            )
        )
    )
}

fn raw_subresource_memory_cacheable_headers(headers: &RequestHeaders) -> bool {
    !HttpCacheRequestPolicy::from_headers(headers).no_store
        && headers
            .iter()
            .all(|(name, _)| !memory_cache_sensitive_request_header(name))
}

fn memory_cache_sensitive_request_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "cookie"
            | "authorization"
            | "proxy-authorization"
            | "range"
            | "if-range"
            | "if-none-match"
            | "if-modified-since"
            | "if-match"
            | "if-unmodified-since"
    )
}

fn combined_request_header_value(headers: &RequestHeaders, name: &str) -> Option<Vec<u8>> {
    let mut values = headers
        .iter()
        .filter(|(field, _)| field.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim_ascii());
    let mut combined = values.next()?.to_vec();
    for value in values {
        combined.extend_from_slice(b", ");
        combined.extend_from_slice(value);
    }
    Some(combined)
}

pub(in crate::network) fn raw_subresource_memory_cache_expiry(
    request: &Request,
    response: &RawResponse,
) -> Option<u64> {
    // This cache retains complete representations, not Content-Range parts.
    if response.status == 206 {
        return None;
    }
    // Match ordinary fields against the same encoded builder as the native
    // request. Mutable transport-only fields need exact sent-value tracking
    // before a renderer representation can safely vary on them.
    if !moli_fetch::cache_vary_header_names(&response.final_url, &response.headers)?
        .iter()
        .all(|name| {
            name.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
                && !memory_cache_sensitive_request_header(name)
                && !name.starts_with("sec-ch-")
                && !matches!(
                    name.as_str(),
                    "connection"
                        | "keep-alive"
                        | "proxy-connection"
                        | "transfer-encoding"
                        | "te"
                        | "trailer"
                        | "upgrade"
                )
        })
    {
        return None;
    }
    let policy = cacheable_response_parts_policy(
        &request.url,
        &response.final_url,
        response.status,
        &response.headers,
        response.redirected,
    )?;
    policy
        .expires_at_unix_ms
        .or_else(|| (!validation_headers_from_headers(&response.headers).is_empty()).then_some(0))
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

#[cfg(test)]
fn raw_subresource_retained_bytes(key: &RawSubresourceCacheKey, response: &RawResponse) -> usize {
    raw_subresource_key_retained_bytes(key)
        .saturating_add(response_head_retained_bytes(
            response.final_url.as_str(),
            &response.headers,
        ))
        .saturating_add(response.body_bytes().len())
}

fn response_head_retained_bytes(final_url: &str, headers: &[(String, Vec<u8>)]) -> usize {
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
        .saturating_add(key.request_origin.as_deref().map_or(0, str::len))
        .saturating_add(key.request_mode.len())
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
        .saturating_add(key.request_origin.as_deref().map_or(0, str::len))
        .saturating_add(key.request_mode.len())
        .saturating_add(key.referrer.as_deref().map_or(0, str::len))
        .saturating_add(key.network_partition_key.as_deref().map_or(0, str::len))
        .saturating_add(key.cookie_context.len())
        .saturating_add(key.serialized_origin.len())
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
                status_text: None,
                final_url: Url::parse(url).expect("response URL"),
                status: 200,
                headers: vec![("cache-control".to_owned(), b"max-age=60".to_vec())],
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                cache_state: Default::default(),
                preload_state: Default::default(),
                negotiated_http_version: None,
            },
            body.to_owned(),
        )
    }

    fn raw_response(url: &str, body: &[u8]) -> RawResponse {
        RawResponse::from_head_and_body(
            ResponseHead {
                status_text: None,
                final_url: Url::parse(url).expect("response URL"),
                status: 200,
                headers: vec![("cache-control".to_owned(), b"max-age=60".to_vec())],
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                cache_state: Default::default(),
                preload_state: Default::default(),
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
    fn per_hop_cache_keeps_transport_redirects_and_client_origins_separate() {
        let initiator = Url::parse("https://cache.test/page").unwrap();
        let automatic = Request::get("https://cache.test/resource")
            .unwrap()
            .with_resource_type(RequestResourceType::Raw)
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
            .with_request_origin((&initiator).into())
            .with_initiator_url(&initiator);
        let per_hop = automatic.clone().with_single_hop();
        let key = raw_subresource_memory_cache_key(&per_hop, per_hop.headers())
            .expect("cacheable first CORS hop");
        let mut cache = SharedMemoryResourceCache::with_limits(usize::MAX, usize::MAX);
        cache.insert_raw_subresource(
            key.clone(),
            raw_response(per_hop.url.as_str(), b"cached"),
            u64::MAX,
            &per_hop.request_headers,
        );
        assert!(
            cache
                .lookup_raw_subresource(&key, &per_hop, &per_hop.request_headers)
                .is_some()
        );
        for isolated in [
            automatic,
            per_hop
                .clone()
                .with_request_origin(moli_url::WebOrigin::Opaque),
        ] {
            let isolated_key =
                raw_subresource_memory_cache_key(&isolated, &isolated.request_headers).unwrap();
            assert!(
                cache
                    .lookup_raw_subresource(&isolated_key, &isolated, &isolated.request_headers)
                    .is_none(),
                "a cached hop cannot replace an automatic redirect or another client's response"
            );
        }
        for mode in [
            moli_fetch::RequestRedirectMode::Manual,
            moli_fetch::RequestRedirectMode::Error,
        ] {
            assert!(
                raw_subresource_memory_cache_key(
                    &per_hop.clone().with_redirect_mode(mode),
                    &per_hop.request_headers
                )
                .is_none()
            );
        }
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
    fn shared_script_response_start_notifies_live_and_late_consumers_once() {
        let load = ScriptTextLoad::pending();
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscribe = |name| {
            let headers = Arc::clone(&events);
            let completion = Arc::clone(&events);
            load.wait_callback_with_response_start(
                Box::new(move |_| completion.lock().push((name, "complete"))),
                Some(Box::new(move || headers.lock().push((name, "headers")))),
            )
        };
        let retired = subscribe("retired").unwrap();
        let first = subscribe("first").unwrap();
        retired.cancel();
        load.notify_response_started();
        let late = subscribe("late").unwrap();
        load.notify_response_started();
        assert_eq!(*events.lock(), [("first", "headers"), ("late", "headers")]);
        load.finish(Ok(response("https://cache.test/app.js", "// script")));
        assert!(subscribe("completed").is_none());
        assert_eq!(
            *events.lock(),
            [
                ("first", "headers"),
                ("late", "headers"),
                ("first", "complete"),
                ("late", "complete"),
                ("completed", "headers"),
                ("completed", "complete"),
            ]
        );
        drop((first, late));
    }

    #[test]
    fn late_script_consumer_observes_headers_before_reentrant_completion() {
        let load = ScriptTextLoad::pending();
        load.notify_response_started();
        let events = Arc::new(Mutex::new(Vec::new()));
        let headers = Arc::clone(&events);
        let completion = Arc::clone(&events);
        let finished = Arc::clone(&load);
        let consumer = load.wait_callback_with_response_start(
            Box::new(move |_| completion.lock().push("complete")),
            Some(Box::new(move || {
                finished.finish(Err("body failed during header notification".to_owned()));
                headers.lock().push("headers");
            })),
        );
        assert!(consumer.is_some());
        assert_eq!(*events.lock(), ["headers", "complete"]);
    }

    #[test]
    fn late_script_consumer_keeps_transport_live_during_header_notification() {
        let load = ScriptTextLoad::pending();
        let owner = load
            .wait_callback(Box::new(|_| panic!("retired owner")))
            .unwrap();
        let transport = moli_fetch::FetchCancelHandle::new();
        load.attach_transport_cancel(transport.clone());
        load.notify_response_started();
        let during = Arc::clone(&load);
        let events = Arc::new(Mutex::new(Vec::new()));
        let headers = Arc::clone(&events);
        let completion = Arc::clone(&events);
        let surviving = load.wait_callback_with_response_start(
            Box::new(move |_| completion.lock().push("complete")),
            Some(Box::new(move || {
                owner.cancel();
                assert!(!transport.is_cancelled());
                during.finish(Err("body completed during header notification".to_owned()));
                headers.lock().push("headers");
            })),
        );
        assert!(surviving.is_some());
        assert_eq!(*events.lock(), ["headers", "complete"]);
    }

    #[tokio::test]
    async fn cancelling_callback_consumer_preserves_an_async_script_waiter() {
        use std::{
            future::Future,
            task::{Context, Waker},
        };
        let load = ScriptTextLoad::pending();
        let callback = load
            .wait_callback(Box::new(|_| panic!("retired callback")))
            .unwrap();
        let waiter = load.wait();
        tokio::pin!(waiter);
        assert!(
            waiter
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        let transport = moli_fetch::FetchCancelHandle::new();
        load.attach_transport_cancel(transport.clone());
        callback.cancel();
        assert!(!transport.is_cancelled());
        load.finish(Err("terminal".to_owned()));
        assert_eq!(waiter.await.unwrap_err(), "terminal");
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
                status_text: None,
                final_url: request.url.clone(),
                status: 200,
                headers: vec![("cache-control".to_owned(), b"max-age=60".to_vec())],
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                cache_state: Default::default(),
                preload_state: Default::default(),
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
        let raw_key = raw_subresource_memory_cache_key(&raw_request, &raw_request.request_headers)
            .expect("cacheable raw request key");
        let raw_response = raw_response(raw_request.url.as_str(), &[b'r'; 128]);
        let raw_weight = raw_subresource_retained_bytes(&raw_key, &raw_response);
        let mut cache =
            SharedMemoryResourceCache::with_limits(script_weight.max(raw_weight), usize::MAX);

        insert_script(&mut cache, &script_request, script_response);
        cache.insert_raw_subresource(
            raw_key.clone(),
            raw_response,
            u64::MAX,
            &raw_request.request_headers,
        );

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
        let raw_key = raw_subresource_memory_cache_key(&raw_request, &raw_request.request_headers)
            .expect("cacheable raw request key");
        let raw_response = raw_response(raw_request.url.as_str(), &[b'r'; 128]);
        let mut cache = SharedMemoryResourceCache::with_limits(1024 * 1024, 4096);

        insert_script(&mut cache, &script_request, script_response);
        cache.insert_raw_subresource(
            raw_key,
            raw_response,
            u64::MAX,
            &raw_request.request_headers,
        );

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

    #[test]
    fn raw_cache_vary_values_are_charged_to_the_shared_byte_budget() {
        let mut request = Request::get("https://cache.test/variant")
            .unwrap()
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        request.request_headers =
            RequestHeaders::from_bytes(vec![("X-Mode".into(), vec![b'x'; 16 * 1024])]);
        let key = raw_subresource_memory_cache_key(&request, &request.request_headers).unwrap();
        let mut response = raw_response(request.url.as_str(), b"body");
        response.headers.push(("vary".into(), b"X-Mode".to_vec()));
        let mut cache = SharedMemoryResourceCache::with_limits(4096, 4096);
        cache.insert_raw_subresource(key.clone(), response, u64::MAX, &request.request_headers);
        assert!(
            cache
                .lookup_raw_subresource(&key, &request, &request.request_headers)
                .is_none()
        );
        assert_eq!(cache.diagnostics().retained_bytes, 0);
        assert_eq!(cache.diagnostics().raw_subresource_entry_count, 0);
    }

    #[test]
    fn raw_cache_vary_variants_share_one_resource_lru_budget() {
        let mut request = Request::get("https://cache.test/variant")
            .unwrap()
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        let key = raw_subresource_memory_cache_key(&request, &request.request_headers).unwrap();
        let mut cache = SharedMemoryResourceCache::with_limits(5000, 4096);
        for (index, value) in ["one", "two", "three"].iter().enumerate() {
            request.request_headers =
                RequestHeaders::from_bytes(vec![("X-Mode".into(), value.as_bytes().to_vec())]);
            let mut response = raw_response(request.url.as_str(), &vec![b'x'; 1800]);
            response.headers.push(("vary".into(), b"X-Mode".to_vec()));
            cache.insert_raw_subresource(key.clone(), response, u64::MAX, &request.request_headers);
            let diagnostics = cache.diagnostics();
            assert!(diagnostics.retained_bytes <= 5000);
            if index < 2 {
                assert_eq!(diagnostics.raw_subresource_entry_count, index + 1);
            }
        }
        // Eviction applies to the resource bucket, without multiplying the
        // shared budget for its representations or losing retained-byte charges.
        assert_eq!(cache.diagnostics().raw_subresource_entry_count, 0);
        assert_eq!(cache.diagnostics().retained_bytes, 0);
    }
}
