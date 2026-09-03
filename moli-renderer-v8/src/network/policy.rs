use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use anyhow::{Result, anyhow};
use http::HeaderName;
use indexmap::IndexMap;
use moli_fetch::{Request, RequestCacheMode, url_pattern_matches};
use parking_lot::Mutex;

use crate::{protocol_types::OptionalResourceFetchMask, types::SubresourceResourceType};

const BLOCKED_BY_CLIENT_ERROR_TEXT: &str = "net::ERR_BLOCKED_BY_CLIENT";
static NEXT_MEMORY_CACHE_PARTITION_ID: AtomicU64 = AtomicU64::new(0);

fn next_memory_cache_partition_id() -> u64 {
    NEXT_MEMORY_CACHE_PARTITION_ID
        .fetch_add(1, Ordering::Relaxed)
        .checked_add(1)
        .expect("Page memory-cache partition id exhausted")
}

type SharedHeaderList = Arc<[(Box<str>, Box<str>)]>;
type SharedPatternList = Arc<[Box<str>]>;

#[derive(Debug, Clone)]
struct PageNetworkPolicyState {
    revision: u64,
    memory_cache_partition_id: u64,
    extra_http_headers: SharedHeaderList,
    blocked_url_patterns: SharedPatternList,
    optional_resource_fetch_mask: OptionalResourceFetchMask,
    subframe_loading_enabled: bool,
    author_styles_disabled: bool,
    bypass_service_worker: bool,
    cache_disabled: bool,
}

impl PageNetworkPolicyState {
    fn advance_revision(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("Page network policy revision exhausted");
    }
}

impl Default for PageNetworkPolicyState {
    fn default() -> Self {
        Self {
            revision: 0,
            memory_cache_partition_id: next_memory_cache_partition_id(),
            extra_http_headers: Arc::from([]),
            blocked_url_patterns: Arc::from([]),
            optional_resource_fetch_mask: OptionalResourceFetchMask::NONE,
            subframe_loading_enabled: true,
            author_styles_disabled: false,
            bypass_service_worker: false,
            cache_disabled: false,
        }
    }
}

#[derive(Debug, Default)]
struct PageNetworkConditionsState {
    revision: u64,
    offline: bool,
}

impl PageNetworkConditionsState {
    fn advance_revision(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("Page network conditions revision exhausted");
    }
}

/// Mutable network policy owned by one Page/target.
///
/// Clones intentionally share state. A committed Document and its child
/// Documents observe the same target policy, while a new target must call
/// [`Self::isolated_copy`] before it begins issuing requests.
///
/// Request configuration and network emulation conditions have different
/// capture boundaries. Headers, blocked URLs and loading policy are frozen
/// when a request begins. Offline emulation remains a live target condition,
/// matching Chromium's throttling-profile token: a request paused by DevTools
/// observes conditions in effect when it is actually resumed.
#[derive(Debug, Clone)]
pub struct PageNetworkPolicy {
    state: Arc<Mutex<PageNetworkPolicyState>>,
    network_conditions: Arc<Mutex<PageNetworkConditionsState>>,
}

impl Default for PageNetworkPolicy {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(PageNetworkPolicyState::default())),
            network_conditions: Arc::new(Mutex::new(PageNetworkConditionsState::default())),
        }
    }
}

impl PageNetworkPolicy {
    pub fn new(
        optional_resource_fetch_mask: OptionalResourceFetchMask,
        subframe_loading_enabled: bool,
        author_styles_disabled: bool,
    ) -> Self {
        let state = PageNetworkPolicyState {
            optional_resource_fetch_mask,
            subframe_loading_enabled,
            author_styles_disabled,
            ..PageNetworkPolicyState::default()
        };
        Self {
            state: Arc::new(Mutex::new(state)),
            network_conditions: Arc::new(Mutex::new(PageNetworkConditionsState::default())),
        }
    }

    /// Returns a policy with the same current values and independent mutable
    /// state. This is the target-creation boundary; ordinary `clone()` keeps
    /// sharing the original target policy.
    pub fn isolated_copy(&self) -> Self {
        Self::from_snapshot(self.snapshot())
    }

    /// Returns a Document-owned mutable view within the same Page target.
    ///
    /// Renderer Documents need independent request-policy state, but resource
    /// reuse across top-level navigation remains owned by the stable Page.
    pub fn isolated_same_page_copy(&self) -> Self {
        Self::from_snapshot_with_memory_cache_partition(
            self.snapshot(),
            self.memory_cache_partition_id(),
        )
    }

    pub fn shares_memory_cache_partition_with(&self, other: &Self) -> bool {
        self.memory_cache_partition_id() == other.memory_cache_partition_id()
    }

    /// Keeps this policy's current state while adopting another Page policy's
    /// stable renderer memory-cache capability.
    pub fn adopt_memory_cache_partition_from(&mut self, source: &Self) {
        let source_partition_id = source.memory_cache_partition_id();
        self.state.lock().memory_cache_partition_id = source_partition_id;
    }

    pub fn from_snapshot(snapshot: PageNetworkPolicySnapshot) -> Self {
        Self::from_snapshot_with_memory_cache_partition(snapshot, next_memory_cache_partition_id())
    }

    fn from_snapshot_with_memory_cache_partition(
        snapshot: PageNetworkPolicySnapshot,
        memory_cache_partition_id: u64,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(PageNetworkPolicyState {
                revision: snapshot.configuration_revision,
                memory_cache_partition_id,
                extra_http_headers: snapshot.extra_http_headers,
                blocked_url_patterns: snapshot.blocked_url_patterns,
                optional_resource_fetch_mask: snapshot.optional_resource_fetch_mask,
                subframe_loading_enabled: snapshot.subframe_loading_enabled,
                author_styles_disabled: snapshot.author_styles_disabled,
                bypass_service_worker: snapshot.bypass_service_worker,
                cache_disabled: snapshot.cache_disabled,
            })),
            network_conditions: Arc::new(Mutex::new(PageNetworkConditionsState {
                revision: snapshot.network_conditions_revision,
                offline: snapshot.network_offline,
            })),
        }
    }

    /// Freezes request configuration while retaining the target's live
    /// network-emulation conditions.
    ///
    /// A resource lease uses this view to keep its original backend, headers
    /// and blocked-URL policy across re-entry. It deliberately continues to
    /// observe `Network.emulateNetworkConditions`, including while a DevTools
    /// request-stage interception is paused.
    pub(crate) fn frozen_request_view(&self) -> Self {
        let snapshot = self.snapshot();
        Self {
            state: Arc::new(Mutex::new(PageNetworkPolicyState {
                revision: snapshot.configuration_revision,
                memory_cache_partition_id: self.memory_cache_partition_id(),
                extra_http_headers: snapshot.extra_http_headers,
                blocked_url_patterns: snapshot.blocked_url_patterns,
                optional_resource_fetch_mask: snapshot.optional_resource_fetch_mask,
                subframe_loading_enabled: snapshot.subframe_loading_enabled,
                author_styles_disabled: snapshot.author_styles_disabled,
                bypass_service_worker: snapshot.bypass_service_worker,
                cache_disabled: snapshot.cache_disabled,
            })),
            network_conditions: Arc::clone(&self.network_conditions),
        }
    }

    pub(crate) fn memory_cache_partition_id(&self) -> u64 {
        self.state.lock().memory_cache_partition_id
    }

    pub fn shares_state_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
            && Arc::ptr_eq(&self.network_conditions, &other.network_conditions)
    }

    /// Captures the current request configuration and network conditions.
    ///
    /// A normal request preparation retains this complete snapshot. A
    /// long-lived resource lease instead uses [`Self::frozen_request_view`] so
    /// that DevTools network conditions remain live while request
    /// configuration cannot drift across JavaScript/CDP re-entry.
    pub fn snapshot(&self) -> PageNetworkPolicySnapshot {
        let state = self.state.lock();
        let network_conditions = self.network_conditions.lock();
        PageNetworkPolicySnapshot {
            configuration_revision: state.revision,
            network_conditions_revision: network_conditions.revision,
            extra_http_headers: state.extra_http_headers.clone(),
            network_offline: network_conditions.offline,
            blocked_url_patterns: state.blocked_url_patterns.clone(),
            optional_resource_fetch_mask: state.optional_resource_fetch_mask,
            subframe_loading_enabled: state.subframe_loading_enabled,
            author_styles_disabled: state.author_styles_disabled,
            bypass_service_worker: state.bypass_service_worker,
            cache_disabled: state.cache_disabled,
        }
    }

    pub fn revision(&self) -> u64 {
        self.snapshot().revision()
    }

    pub fn set_extra_http_headers(&self, headers: &[(String, String)]) {
        let headers = headers
            .iter()
            .map(|(name, value)| {
                (
                    name.clone().into_boxed_str(),
                    value.clone().into_boxed_str(),
                )
            })
            .collect::<Vec<_>>();
        let headers: SharedHeaderList = Arc::from(headers);
        let mut state = self.state.lock();
        if state.extra_http_headers == headers {
            return;
        }
        state.extra_http_headers = headers;
        state.advance_revision();
    }

    pub fn set_network_offline(&self, offline: bool) {
        let mut conditions = self.network_conditions.lock();
        if conditions.offline == offline {
            return;
        }
        conditions.offline = offline;
        conditions.advance_revision();
    }

    pub fn set_blocked_url_patterns(&self, patterns: &[String]) {
        let patterns = patterns
            .iter()
            .map(|pattern| pattern.clone().into_boxed_str())
            .collect::<Vec<_>>();
        let patterns: SharedPatternList = Arc::from(patterns);
        let mut state = self.state.lock();
        if state.blocked_url_patterns == patterns {
            return;
        }
        state.blocked_url_patterns = patterns;
        state.advance_revision();
    }

    pub fn set_optional_resource_fetch_mask(&self, mask: OptionalResourceFetchMask) {
        let mut state = self.state.lock();
        if state.optional_resource_fetch_mask == mask {
            return;
        }
        state.optional_resource_fetch_mask = mask;
        state.advance_revision();
    }

    pub fn optional_resource_fetch_mask(&self) -> OptionalResourceFetchMask {
        self.state.lock().optional_resource_fetch_mask
    }

    pub fn set_optional_resource_fetch_enabled(
        &self,
        resource_type: SubresourceResourceType,
        enabled: bool,
    ) {
        let Some(resource) = OptionalResourceFetchMask::for_resource_type(resource_type) else {
            return;
        };
        let mut state = self.state.lock();
        let before = state.optional_resource_fetch_mask;
        state.optional_resource_fetch_mask.set(resource, enabled);
        if state.optional_resource_fetch_mask != before {
            state.advance_revision();
        }
    }

    pub fn optional_resource_fetch_enabled(&self, resource_type: SubresourceResourceType) -> bool {
        self.state
            .lock()
            .optional_resource_fetch_mask
            .allows(resource_type)
    }

    pub fn set_subframe_loading_enabled(&self, enabled: bool) {
        let mut state = self.state.lock();
        if state.subframe_loading_enabled == enabled {
            return;
        }
        state.subframe_loading_enabled = enabled;
        state.advance_revision();
    }

    pub fn subframe_loading_enabled(&self) -> bool {
        self.state.lock().subframe_loading_enabled
    }

    pub fn author_styles_disabled(&self) -> bool {
        self.state.lock().author_styles_disabled
    }

    pub fn set_bypass_service_worker(&self, bypass: bool) {
        let mut state = self.state.lock();
        if state.bypass_service_worker == bypass {
            return;
        }
        state.bypass_service_worker = bypass;
        state.advance_revision();
    }

    pub fn bypass_service_worker(&self) -> bool {
        self.state.lock().bypass_service_worker
    }

    pub fn set_cache_disabled(&self, disabled: bool) {
        let mut state = self.state.lock();
        if state.cache_disabled == disabled {
            return;
        }
        state.cache_disabled = disabled;
        state.advance_revision();
    }

    pub fn cache_disabled(&self) -> bool {
        self.state.lock().cache_disabled
    }
}

/// Immutable request-time view of one Page's network policy.
///
/// The `Arc`-backed lists make capture cheap while guaranteeing that a CDP
/// mutation cannot alter a request already being prepared.
#[derive(Debug, Clone)]
pub struct PageNetworkPolicySnapshot {
    configuration_revision: u64,
    network_conditions_revision: u64,
    extra_http_headers: SharedHeaderList,
    network_offline: bool,
    blocked_url_patterns: SharedPatternList,
    optional_resource_fetch_mask: OptionalResourceFetchMask,
    subframe_loading_enabled: bool,
    author_styles_disabled: bool,
    bypass_service_worker: bool,
    cache_disabled: bool,
}

impl PageNetworkPolicySnapshot {
    pub fn revision(&self) -> u64 {
        self.configuration_revision
            .saturating_add(self.network_conditions_revision)
    }

    pub fn network_offline(&self) -> bool {
        self.network_offline
    }

    pub fn optional_resource_fetch_mask(&self) -> OptionalResourceFetchMask {
        self.optional_resource_fetch_mask
    }

    pub fn subframe_loading_enabled(&self) -> bool {
        self.subframe_loading_enabled
    }

    pub fn author_styles_disabled(&self) -> bool {
        self.author_styles_disabled
    }

    pub fn bypass_service_worker(&self) -> bool {
        self.bypass_service_worker
    }

    pub fn cache_disabled(&self) -> bool {
        self.cache_disabled
    }

    pub(crate) fn blocks_url(&self, url: &url::Url) -> bool {
        self.blocked_url_patterns
            .iter()
            .any(|pattern| url_pattern_matches(pattern, url.as_str()))
    }

    pub(crate) fn apply_to_request(&self, mut request: Request) -> Result<Request> {
        if !request.uses_page_network_policy() {
            return Ok(request);
        }

        if self.network_offline {
            return Err(anyhow!("Network emulation offline"));
        }
        if self.blocks_url(&request.url) {
            return Err(anyhow!(BLOCKED_BY_CLIENT_ERROR_TEXT));
        }
        if self.author_styles_disabled
            && matches!(
                request.resource_type,
                moli_fetch::RequestResourceType::CssStyleSheet
                    | moli_fetch::RequestResourceType::LatePreloadCssStyleSheet
            )
        {
            return Err(anyhow!(BLOCKED_BY_CLIENT_ERROR_TEXT));
        }

        if self.cache_disabled {
            request = request.with_cache_mode(RequestCacheMode::Bypass);
        }

        if !self.extra_http_headers.is_empty() {
            request.request_headers = merge_page_network_policy_headers(
                &self.extra_http_headers,
                &request.request_headers,
            );
        }
        Ok(request)
    }
}

fn merge_page_network_policy_headers(
    context_headers: &[(Box<str>, Box<str>)],
    request_headers: &[(String, String)],
) -> Vec<(String, String)> {
    let mut merged = IndexMap::<String, (String, String)>::new();
    for (name, value) in context_headers {
        merged
            .entry(header_name_key(name))
            .or_insert_with(|| (name.to_string(), value.to_string()));
    }
    for (name, value) in request_headers {
        let key = header_name_key(name);
        merged.shift_remove(&key);
        merged.insert(key, (name.clone(), value.clone()));
    }
    merged.into_values().collect()
}

fn header_name_key(name: &str) -> String {
    HeaderName::from_bytes(name.as_bytes())
        .map(|name| name.as_str().to_owned())
        .unwrap_or_else(|_| name.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_policy_copy_preserves_values_without_sharing_mutations() {
        let policy = PageNetworkPolicy::new(OptionalResourceFetchMask::IMAGE, false, true);
        policy.set_extra_http_headers(&[("x-owner".to_owned(), "first".to_owned())]);
        let isolated = policy.isolated_copy();

        assert!(!policy.shares_state_with(&isolated));
        assert_eq!(
            isolated.optional_resource_fetch_mask(),
            OptionalResourceFetchMask::IMAGE
        );
        assert!(!isolated.subframe_loading_enabled());
        assert!(isolated.author_styles_disabled());

        isolated.set_network_offline(true);
        isolated.set_extra_http_headers(&[("x-owner".to_owned(), "second".to_owned())]);

        assert!(!policy.snapshot().network_offline());
        let original = policy
            .snapshot()
            .apply_to_request(
                Request::get("https://example.test/unwrap")
                    .unwrap()
                    .with_page_network_policy(),
            )
            .unwrap();
        assert_eq!(
            original.request_headers,
            vec![("x-owner".to_owned(), "first".to_owned())]
        );
    }

    #[test]
    fn author_style_policy_blocks_only_stylesheet_requests() {
        let policy = PageNetworkPolicy::new(OptionalResourceFetchMask::NONE, true, true);

        for resource_type in [
            moli_fetch::RequestResourceType::CssStyleSheet,
            moli_fetch::RequestResourceType::LatePreloadCssStyleSheet,
        ] {
            let error = policy
                .snapshot()
                .apply_to_request(
                    Request::get("https://example.test/page.css")
                        .unwrap()
                        .with_resource_type(resource_type)
                        .with_page_network_policy(),
                )
                .expect_err("disabled author styles must block CSS requests");
            assert_eq!(error.to_string(), BLOCKED_BY_CLIENT_ERROR_TEXT);
        }

        let script = policy
            .snapshot()
            .apply_to_request(
                Request::get("https://example.test/page.js")
                    .unwrap()
                    .with_resource_type(moli_fetch::RequestResourceType::Script)
                    .with_page_network_policy(),
            )
            .expect("disabling styles must not block scripts");
        assert_eq!(
            script.resource_type,
            moli_fetch::RequestResourceType::Script
        );
    }

    #[test]
    fn request_snapshot_does_not_observe_later_policy_mutation() {
        let policy = PageNetworkPolicy::default();
        policy.set_extra_http_headers(&[("x-policy-revision".to_owned(), "one".to_owned())]);
        let snapshot = policy.snapshot();

        policy.set_extra_http_headers(&[("x-policy-revision".to_owned(), "two".to_owned())]);
        policy.set_network_offline(true);

        let request = snapshot
            .apply_to_request(
                Request::get("https://example.test/snapshot")
                    .unwrap()
                    .with_page_network_policy(),
            )
            .unwrap();
        assert_eq!(
            request.request_headers,
            vec![("x-policy-revision".to_owned(), "one".to_owned())]
        );
        assert!(policy.snapshot().network_offline());
        assert!(policy.revision() > snapshot.revision());
    }

    #[test]
    fn cache_disabled_forces_page_requests_to_bypass_cache() {
        let policy = PageNetworkPolicy::default();
        let before = policy.revision();
        policy.set_cache_disabled(true);

        let request = policy
            .snapshot()
            .apply_to_request(
                Request::get("https://example.test/cache")
                    .unwrap()
                    .with_page_network_policy(),
            )
            .unwrap();

        assert_eq!(request.cache_mode(), RequestCacheMode::Bypass);
        assert!(policy.cache_disabled());
        assert!(policy.revision() > before);
    }

    #[test]
    fn frozen_request_view_keeps_configuration_but_observes_live_network_conditions() {
        let policy = PageNetworkPolicy::default();
        policy.set_extra_http_headers(&[("x-policy-revision".to_owned(), "one".to_owned())]);
        let request_view = policy.frozen_request_view();

        policy.set_extra_http_headers(&[("x-policy-revision".to_owned(), "two".to_owned())]);
        let request = request_view
            .snapshot()
            .apply_to_request(
                Request::get("https://example.test/frozen")
                    .unwrap()
                    .with_page_network_policy(),
            )
            .unwrap();
        assert_eq!(
            request.request_headers,
            vec![("x-policy-revision".to_owned(), "one".to_owned())],
            "request configuration must remain the one captured at registration"
        );

        policy.set_network_offline(true);
        assert!(
            request_view.snapshot().network_offline(),
            "a paused request must observe live DevTools network conditions"
        );
        policy.set_network_offline(false);
        assert!(
            !request_view.snapshot().network_offline(),
            "resuming online must update the same network-condition handle"
        );
    }
}
