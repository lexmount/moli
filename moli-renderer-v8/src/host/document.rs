use std::cell::RefCell;

use crate::DocumentCookieBrowserContextSnapshot;
use crate::dom::native::DocumentReadyState;
use moli_cookie_jar::{
    BrowserCookieFacadeContext, BrowserCookieStorageAccessStatus,
    StoredCookieBrowserContextValueSource, StoredCookieExclusionReason, StoredCookieFacadeStatus,
    StoredCookieSetRejectionReason, StoredCookieSetReport, StoredCookieSetStatus, same_site_urls,
};
use moli_cookie_jar::{BrowserCookieFacadeContextOverrides, BrowserCookieFacadeOverrides};

use super::*;

pub(crate) const WINDOW_EVENT_SLOT: &str = "__moliWindowEvent";
pub(crate) use crate::context_bootstrap::EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT as EVENT_STOP_IMMEDIATE_SLOT;
pub(crate) use crate::context_bootstrap::{
    EVENT_DISPATCHING_SLOT, EVENT_PASSIVE_SLOT, EVENT_STOP_PROPAGATION_SLOT,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ChildWindowEventTarget {
    child_handle: NativeNodeId,
    owner: crate::frame_owner_model::FrameDocumentTaskOwner,
}

impl ChildWindowEventTarget {
    pub(crate) fn new(
        child_handle: NativeNodeId,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> Self {
        Self {
            child_handle,
            owner,
        }
    }

    pub(crate) fn child_handle(self) -> NativeNodeId {
        self.child_handle
    }

    pub(crate) fn owner(self) -> crate::frame_owner_model::FrameDocumentTaskOwner {
        self.owner
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PopupWindowEventTarget {
    pub(crate) document_owner: crate::window_document_identity::LightweightPopupDocumentOwner,
    pub(crate) local_window_id: crate::window_document_identity::LightweightPopupLocalWindowId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum EventTargetHandle {
    Window,
    ChildWindow(ChildWindowEventTarget),
    PopupWindow(PopupWindowEventTarget),
    Node(NativeNodeId),
}

impl EventTargetHandle {
    pub(crate) fn is_window(self) -> bool {
        matches!(
            self,
            Self::Window | Self::ChildWindow(_) | Self::PopupWindow(_)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SelectorDebugSnapshot {
    pub(crate) query_selector: u32,
    pub(crate) query_selector_all: u32,
    pub(crate) matches: u32,
    pub(crate) closest: u32,
}

#[derive(Debug, Default)]
pub(crate) struct SelectorDebugCounters {
    query_selector: Cell<u32>,
    query_selector_all: Cell<u32>,
    matches: Cell<u32>,
    closest: Cell<u32>,
}

impl SelectorDebugCounters {
    pub(crate) fn record_query_selector(&self) {
        self.query_selector.set(self.query_selector.get() + 1);
    }

    pub(crate) fn record_query_selector_all(&self) {
        self.query_selector_all
            .set(self.query_selector_all.get() + 1);
    }

    pub(crate) fn record_matches(&self) {
        self.matches.set(self.matches.get() + 1);
    }

    pub(crate) fn record_closest(&self) {
        self.closest.set(self.closest.get() + 1);
    }

    pub(crate) fn snapshot(&self) -> SelectorDebugSnapshot {
        SelectorDebugSnapshot {
            query_selector: self.query_selector.get(),
            query_selector_all: self.query_selector_all.get(),
            matches: self.matches.get(),
            closest: self.closest.get(),
        }
    }
}

#[derive(Debug)]
pub struct HostDocumentState {
    url: Url,
    ready_state: DocumentReadyState,
    // Shared browser/backend cookie state. Network, CDP and document-facing
    // cookie access all ultimately converge on this jar, but the document does
    // not own its policy or cache semantics here.
    pub cookie_store: Option<SharedBrowserCookieStore>,
    // Blink keeps document-facing cookie cache policy at the document/facade
    // boundary, not in the shared backend. Keep the same ownership split here
    // so two same-URL documents never share JS cookie visibility by accident.
    // This caches only the current document's `document.cookie` string view.
    document_cookie_cache: RefCell<Option<DocumentCookieCacheEntry>>,
    // Document-local browser boundary for cookie access. This owns JS-visible
    // capability, browser-context defaults/overrides, facade generation and
    // telemetry, all of which may differ between two documents sharing the
    // same backend cookie store.
    cookie_facade: DocumentCookieFacadeState,
    active_element: Option<NativeNodeId>,
    replace_on_close: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DocumentCookieCacheEntry {
    url: Url,
    cookie_string: String,
    generation: u64,
    facade_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DocumentCookieFacadeState {
    view_generation: u64,
    cookies_enabled: Cell<bool>,
    browser_context: DocumentCookieBrowserContextState,
    telemetry: DocumentCookieFacadeTelemetry,
}

impl DocumentCookieFacadeState {
    fn new(default_url: Url) -> Self {
        Self {
            view_generation: 0,
            cookies_enabled: Cell::new(true),
            browser_context: DocumentCookieBrowserContextState::new(default_url),
            telemetry: DocumentCookieFacadeTelemetry::default(),
        }
    }

    fn cookies_enabled_preference(&self) -> bool {
        self.cookies_enabled.get()
    }

    fn browser_context(&self) -> BrowserCookieFacadeContext {
        self.browser_context.effective_context()
    }

    fn browser_context_sources(&self) -> &DocumentCookieBrowserContextState {
        &self.browser_context
    }

    fn invalidate(&mut self) {
        self.view_generation = self.view_generation.wrapping_add(1);
    }

    fn view_generation(&self) -> u64 {
        self.view_generation
    }

    fn telemetry_snapshot(&self) -> DocumentCookieFacadeTelemetrySnapshot {
        self.telemetry.snapshot()
    }

    fn apply_facade_overrides(&mut self, overrides: &BrowserCookieFacadeOverrides) -> bool {
        let mut changed = false;
        if let Some(enabled) = overrides.cookies_enabled
            && self.cookies_enabled.replace(enabled) != enabled
        {
            changed = true;
        }
        if self
            .browser_context
            .apply_overrides(&overrides.browser_context_overrides())
        {
            changed = true;
        }
        if changed {
            self.invalidate();
        }
        changed
    }

    fn clear_facade_overrides(&mut self) -> bool {
        let mut changed = false;
        if !self.cookies_enabled.replace(true) {
            changed = true;
        }
        if self.browser_context.clear_overrides() {
            changed = true;
        }
        if changed {
            self.invalidate();
        }
        changed
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct DocumentCookieFacadeTelemetry {
    first_operation: Cell<Option<DocumentCookieFirstOperation>>,
    last_cache_lookup_result: Cell<Option<DocumentCookieCacheLookupResult>>,
    last_cookie_access_was_set: Cell<Option<bool>>,
    cache_hits: Cell<u64>,
    store_reads: Cell<u64>,
    blocked_reads: Cell<u64>,
    unavailable_reads: Cell<u64>,
    applied_writes: Cell<u64>,
    rejected_writes: Cell<u64>,
    facade_blocked_writes: Cell<u64>,
}

impl DocumentCookieFacadeTelemetry {
    fn snapshot(&self) -> DocumentCookieFacadeTelemetrySnapshot {
        DocumentCookieFacadeTelemetrySnapshot {
            first_operation: self.first_operation.get(),
            last_cache_lookup_result: self.last_cache_lookup_result.get(),
            last_operation_was_set: self.last_cookie_access_was_set.get(),
            cache_hits: self.cache_hits.get(),
            store_reads: self.store_reads.get(),
            blocked_reads: self.blocked_reads.get(),
            unavailable_reads: self.unavailable_reads.get(),
            applied_writes: self.applied_writes.get(),
            rejected_writes: self.rejected_writes.get(),
            facade_blocked_writes: self.facade_blocked_writes.get(),
        }
    }

    fn bump(cell: &Cell<u64>) {
        cell.set(cell.get().wrapping_add(1));
    }

    fn record_first_operation(&self, operation: DocumentCookieFirstOperation) {
        if self.first_operation.get().is_none() {
            self.first_operation.set(Some(operation));
        }
    }

    fn record_cookie_enabled_probe(&self) {
        self.record_first_operation(DocumentCookieFirstOperation::CookiesEnabled);
    }

    fn record_cache_lookup(&self, was_hit: bool) {
        self.record_first_operation(DocumentCookieFirstOperation::Get);
        let result = match (was_hit, self.last_cookie_access_was_set.get()) {
            (false, None) => DocumentCookieCacheLookupResult::CacheMissFirstAccess,
            (false, Some(false)) => DocumentCookieCacheLookupResult::CacheMissAfterGet,
            (false, Some(true)) => DocumentCookieCacheLookupResult::CacheMissAfterSet,
            (true, Some(false) | None) => DocumentCookieCacheLookupResult::CacheHitAfterGet,
            (true, Some(true)) => DocumentCookieCacheLookupResult::CacheHitAfterSet,
        };
        self.last_cache_lookup_result.set(Some(result));
        self.last_cookie_access_was_set.set(Some(false));
    }

    fn record_cache_hit(&self) {
        self.record_cache_lookup(true);
        Self::bump(&self.cache_hits);
    }

    fn record_store_read(&self) {
        self.record_cache_lookup(false);
        Self::bump(&self.store_reads);
    }

    fn record_blocked_read(&self) {
        self.record_first_operation(DocumentCookieFirstOperation::Get);
        self.last_cookie_access_was_set.set(Some(false));
        Self::bump(&self.blocked_reads);
    }

    fn record_unavailable_read(&self) {
        self.record_first_operation(DocumentCookieFirstOperation::Get);
        self.last_cookie_access_was_set.set(Some(false));
        Self::bump(&self.unavailable_reads);
    }

    fn record_applied_write(&self) {
        self.record_first_operation(DocumentCookieFirstOperation::Set);
        self.last_cookie_access_was_set.set(Some(true));
        Self::bump(&self.applied_writes);
    }

    fn record_rejected_write(&self) {
        self.record_first_operation(DocumentCookieFirstOperation::Set);
        self.last_cookie_access_was_set.set(Some(true));
        Self::bump(&self.rejected_writes);
    }

    fn record_facade_blocked_write(&self) {
        Self::bump(&self.facade_blocked_writes);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DocumentCookieBrowserContextState {
    default_url: Url,
    site_for_cookies_override: Option<Url>,
    top_frame_origin_override: Option<Url>,
    storage_access_override: Option<BrowserCookieStorageAccessStatus>,
}

impl DocumentCookieBrowserContextState {
    fn new(default_url: Url) -> Self {
        Self {
            default_url,
            site_for_cookies_override: None,
            top_frame_origin_override: None,
            storage_access_override: None,
        }
    }

    fn effective_context(&self) -> BrowserCookieFacadeContext {
        BrowserCookieFacadeContext::default()
            .with_site_for_cookies_url(
                self.site_for_cookies_override
                    .as_ref()
                    .unwrap_or(&self.default_url),
            )
            .with_top_frame_origin_url(
                self.top_frame_origin_override
                    .as_ref()
                    .unwrap_or(&self.default_url),
            )
            .with_storage_access_status(
                self.storage_access_override
                    .unwrap_or(BrowserCookieStorageAccessStatus::None),
            )
    }

    fn site_for_cookies_source(&self) -> StoredCookieBrowserContextValueSource {
        if self.site_for_cookies_override.is_some() {
            StoredCookieBrowserContextValueSource::FacadeOverride
        } else {
            StoredCookieBrowserContextValueSource::FacadeDefault
        }
    }

    fn top_frame_origin_source(&self) -> StoredCookieBrowserContextValueSource {
        if self.top_frame_origin_override.is_some() {
            StoredCookieBrowserContextValueSource::FacadeOverride
        } else {
            StoredCookieBrowserContextValueSource::FacadeDefault
        }
    }

    fn storage_access_source(&self) -> StoredCookieBrowserContextValueSource {
        if self.storage_access_override.is_some() {
            StoredCookieBrowserContextValueSource::FacadeOverride
        } else {
            StoredCookieBrowserContextValueSource::FacadeDefault
        }
    }

    fn clear_overrides(&mut self) -> bool {
        let changed = self.site_for_cookies_override.is_some()
            || self.top_frame_origin_override.is_some()
            || self.storage_access_override.is_some();
        if changed {
            self.site_for_cookies_override = None;
            self.top_frame_origin_override = None;
            self.storage_access_override = None;
        }
        changed
    }

    fn apply_overrides(&mut self, overrides: &BrowserCookieFacadeContextOverrides) -> bool {
        let previous = (
            self.site_for_cookies_override.clone(),
            self.top_frame_origin_override.clone(),
            self.storage_access_override,
        );
        self.site_for_cookies_override = overrides.site_for_cookies_url.clone();
        self.top_frame_origin_override = overrides.top_frame_origin_url.clone();
        self.storage_access_override = overrides.storage_access_status;
        previous
            != (
                self.site_for_cookies_override.clone(),
                self.top_frame_origin_override.clone(),
                self.storage_access_override,
            )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DocumentCookieReadSource {
    Disabled,
    StorageAccessBlocked,
    StoreUnavailable,
    Store,
    Cache,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DocumentCookieReadResult {
    pub(super) value: String,
    pub(super) source: DocumentCookieReadSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DocumentCookieSetOutcome {
    Disabled,
    StorageAccessBlocked,
    StoreUnavailable,
    Applied,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentCookiePolicyBlockReason {
    CookiesDisabled,
    StorageAccessBlocked,
}

impl DocumentCookiePolicyBlockReason {
    fn read_source(self) -> DocumentCookieReadSource {
        match self {
            Self::CookiesDisabled => DocumentCookieReadSource::Disabled,
            Self::StorageAccessBlocked => DocumentCookieReadSource::StorageAccessBlocked,
        }
    }

    fn exclusion_reason(self) -> StoredCookieExclusionReason {
        match self {
            Self::CookiesDisabled => StoredCookieExclusionReason::CookiesDisabled,
            Self::StorageAccessBlocked => StoredCookieExclusionReason::StorageAccessBlocked,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DocumentCookieSetResult {
    pub(super) outcome: DocumentCookieSetOutcome,
    pub(super) report: Option<StoredCookieSetReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentCookieCapabilitySnapshot {
    pub cookies_enabled_preference: bool,
    pub facade_status: StoredCookieFacadeStatus,
    pub view_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentCookieCacheStatus {
    NoEntry,
    Reusable,
    PolicyBlocked,
    StoreUnavailable,
    UrlMismatch,
    StoreGenerationMismatch,
    FacadeGenerationMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentCookieCacheSnapshot {
    pub status: DocumentCookieCacheStatus,
    pub cached_url: Option<Url>,
    pub cached_store_generation: Option<u64>,
    pub current_store_generation: Option<u64>,
    pub cached_facade_generation: Option<u64>,
    pub current_facade_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentCookieFirstOperation {
    Get,
    Set,
    CookiesEnabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentCookieCacheLookupResult {
    CacheMissFirstAccess,
    CacheHitAfterGet,
    CacheHitAfterSet,
    CacheMissAfterGet,
    CacheMissAfterSet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentCookieFacadeTelemetrySnapshot {
    pub first_operation: Option<DocumentCookieFirstOperation>,
    pub last_cache_lookup_result: Option<DocumentCookieCacheLookupResult>,
    pub last_operation_was_set: Option<bool>,
    pub cache_hits: u64,
    pub store_reads: u64,
    pub blocked_reads: u64,
    pub unavailable_reads: u64,
    pub applied_writes: u64,
    pub rejected_writes: u64,
    pub facade_blocked_writes: u64,
}

impl HostDocumentState {
    pub fn new(url: Url) -> Self {
        let cookie_facade = DocumentCookieFacadeState::new(url.clone());
        Self {
            url,
            ready_state: DocumentReadyState::Loading,
            cookie_store: None,
            document_cookie_cache: RefCell::new(None),
            cookie_facade,
            active_element: None,
            replace_on_close: false,
        }
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    pub(crate) fn set_url(&mut self, url: Url) {
        self.url = url;
    }

    pub(crate) fn ready_state(&self) -> DocumentReadyState {
        self.ready_state
    }

    pub(crate) fn set_ready_state(&mut self, state: DocumentReadyState) {
        self.ready_state = state;
    }

    pub(crate) fn active_element(&self) -> Option<NativeNodeId> {
        self.active_element
    }

    pub(crate) fn set_active_element(&mut self, handle: Option<NativeNodeId>) {
        self.active_element = handle;
    }

    pub(crate) fn cookie_for_url(&self, url: &Url) -> String {
        self.cookie_read_result_for_url(url).value
    }

    fn cookie_read_result_for_url(&self, url: &Url) -> DocumentCookieReadResult {
        if let Some(block_reason) = self.document_cookie_policy_block_reason_for_url(url) {
            self.cookie_facade.telemetry.record_blocked_read();
            return DocumentCookieReadResult {
                value: String::new(),
                source: block_reason.read_source(),
            };
        }
        let Some(cookie_store) = self.cookie_store.as_ref() else {
            self.cookie_facade.telemetry.record_unavailable_read();
            return DocumentCookieReadResult {
                value: String::new(),
                source: DocumentCookieReadSource::StoreUnavailable,
            };
        };
        let mut cookie_store = cookie_store.lock();
        cookie_store.purge_expired();
        let generation = cookie_store.document_cookie_generation();
        if let Some(cache) = self.document_cookie_cache.borrow().as_ref()
            && cache.url == *url
            && cache.generation == generation
            && cache.facade_generation == self.cookie_facade.view_generation()
        {
            self.cookie_facade.telemetry.record_cache_hit();
            return DocumentCookieReadResult {
                value: cache.cookie_string.clone(),
                source: DocumentCookieReadSource::Cache,
            };
        }
        let cookie_string =
            cookie_store.document_cookie_with_context(url, &self.cookie_browser_context());
        *self.document_cookie_cache.borrow_mut() = Some(DocumentCookieCacheEntry {
            url: url.clone(),
            cookie_string: cookie_string.clone(),
            generation,
            facade_generation: self.cookie_facade.view_generation(),
        });
        self.cookie_facade.telemetry.record_store_read();
        DocumentCookieReadResult {
            value: cookie_string,
            source: DocumentCookieReadSource::Store,
        }
    }

    pub(super) fn cookies_enabled(&self) -> bool {
        self.cookie_facade.cookies_enabled_preference()
    }

    pub fn browser_cookie_enabled(&self) -> bool {
        self.cookie_facade.telemetry.record_cookie_enabled_probe();
        self.document_cookie_capability_snapshot()
            .facade_status
            .cookie_access_enabled
    }

    pub(crate) fn set_cookie_for_url(&mut self, url: &Url, cookie: &str) {
        let _ = self.set_cookie_with_result_for_url(url, cookie);
    }

    fn set_cookie_with_result_for_url(
        &mut self,
        url: &Url,
        cookie: &str,
    ) -> DocumentCookieSetResult {
        let owner = self.document_cookie_owner_snapshot_for_url(url);
        if let Some(rejection_reason) = owner.write_capability.primary_rejection_reason {
            self.cookie_facade.telemetry.record_rejected_write();
            self.cookie_facade.telemetry.record_facade_blocked_write();
            return DocumentCookieSetResult {
                outcome: match rejection_reason {
                    StoredCookieSetRejectionReason::CookiesDisabled => {
                        DocumentCookieSetOutcome::Disabled
                    }
                    StoredCookieSetRejectionReason::StorageAccessBlocked => {
                        DocumentCookieSetOutcome::StorageAccessBlocked
                    }
                    StoredCookieSetRejectionReason::StoreUnavailable => {
                        DocumentCookieSetOutcome::StoreUnavailable
                    }
                    _ => DocumentCookieSetOutcome::Rejected,
                },
                report: Some(rejected_document_cookie_set_report(rejection_reason)),
            };
        }
        let Some(cookie_store) = self.cookie_store.as_ref() else {
            unreachable!(
                "document cookie owner contract marked write enabled but no backend store was attached"
            );
        };
        let report = {
            let mut cookie_store = cookie_store.lock();
            cookie_store.set_document_cookie_with_context_report(
                url,
                cookie,
                &self.cookie_browser_context(),
            )
        };
        if report.is_accepted() {
            self.cookie_facade.telemetry.record_applied_write();
            self.invalidate_cookie_cache();
        } else {
            self.cookie_facade.telemetry.record_rejected_write();
        }
        DocumentCookieSetResult {
            outcome: if report.is_accepted() {
                DocumentCookieSetOutcome::Applied
            } else {
                DocumentCookieSetOutcome::Rejected
            },
            report: Some(report),
        }
    }

    pub fn document_cookie_capability_snapshot(&self) -> DocumentCookieCapabilitySnapshot {
        self.document_cookie_capability_snapshot_for_url(&self.url)
    }

    pub(crate) fn document_cookie_capability_snapshot_for_url(
        &self,
        url: &Url,
    ) -> DocumentCookieCapabilitySnapshot {
        DocumentCookieCapabilitySnapshot {
            cookies_enabled_preference: self.cookies_enabled(),
            facade_status: self.document_cookie_facade_status_for_url(url),
            view_generation: self.cookie_facade.view_generation(),
        }
    }

    pub fn document_cookie_telemetry_snapshot(&self) -> DocumentCookieFacadeTelemetrySnapshot {
        self.cookie_facade.telemetry_snapshot()
    }

    pub fn document_cookie_browser_context_snapshot(&self) -> DocumentCookieBrowserContextSnapshot {
        let browser_context = self.cookie_browser_context();
        let sources = self.cookie_facade.browser_context_sources();
        DocumentCookieBrowserContextSnapshot {
            site_for_cookies_url: browser_context.site_for_cookies_url,
            site_for_cookies_source: sources.site_for_cookies_source(),
            top_frame_origin_url: browser_context.top_frame_origin_url,
            top_frame_origin_source: sources.top_frame_origin_source(),
            storage_access_status: browser_context.storage_access_status,
            storage_access_source: sources.storage_access_source(),
        }
    }

    pub(crate) fn document_cookie_cache_snapshot_for_url(
        &self,
        url: &Url,
    ) -> DocumentCookieCacheSnapshot {
        let current_facade_generation = self.cookie_facade.view_generation();
        let cache = self.document_cookie_cache.borrow();
        let cached_url = cache.as_ref().map(|entry| entry.url.clone());
        let cached_store_generation = cache.as_ref().map(|entry| entry.generation);
        let cached_facade_generation = cache.as_ref().map(|entry| entry.facade_generation);

        if self
            .document_cookie_policy_block_reason_for_url(url)
            .is_some()
        {
            return DocumentCookieCacheSnapshot {
                status: DocumentCookieCacheStatus::PolicyBlocked,
                cached_url,
                cached_store_generation,
                current_store_generation: self
                    .cookie_store
                    .as_ref()
                    .map(|store| store.lock().document_cookie_generation()),
                cached_facade_generation,
                current_facade_generation,
            };
        }

        let Some(cookie_store) = self.cookie_store.as_ref() else {
            return DocumentCookieCacheSnapshot {
                status: DocumentCookieCacheStatus::StoreUnavailable,
                cached_url,
                cached_store_generation,
                current_store_generation: None,
                cached_facade_generation,
                current_facade_generation,
            };
        };
        let mut cookie_store = cookie_store.lock();
        cookie_store.purge_expired();
        let current_store_generation = Some(cookie_store.document_cookie_generation());

        let Some(cache) = cache.as_ref() else {
            return DocumentCookieCacheSnapshot {
                status: DocumentCookieCacheStatus::NoEntry,
                cached_url: None,
                cached_store_generation: None,
                current_store_generation,
                cached_facade_generation: None,
                current_facade_generation,
            };
        };
        let current_store_generation_value = cookie_store.document_cookie_generation();

        let status = if cache.url != *url {
            DocumentCookieCacheStatus::UrlMismatch
        } else if cache.generation != current_store_generation_value {
            DocumentCookieCacheStatus::StoreGenerationMismatch
        } else if cache.facade_generation != current_facade_generation {
            DocumentCookieCacheStatus::FacadeGenerationMismatch
        } else {
            DocumentCookieCacheStatus::Reusable
        };

        DocumentCookieCacheSnapshot {
            status,
            cached_url: Some(cache.url.clone()),
            cached_store_generation: Some(cache.generation),
            current_store_generation,
            cached_facade_generation: Some(cache.facade_generation),
            current_facade_generation,
        }
    }

    pub(crate) fn apply_cookie_facade_overrides(
        &mut self,
        overrides: &BrowserCookieFacadeOverrides,
    ) {
        // Keep one atomic browser-boundary seam for document-cookie capability
        // and browser-context ownership. Blink's facade treats these as one
        // browser policy surface, so callers should not need to sequence
        // multiple setters just to express one embedder/DevTools decision.
        self.apply_cookie_facade_update(|facade| facade.apply_facade_overrides(overrides));
    }

    pub(crate) fn clear_cookie_facade_overrides(&mut self) {
        self.apply_cookie_facade_update(|facade| facade.clear_facade_overrides());
    }

    pub fn invalidate_cookie_cache(&mut self) {
        // Treat browser-boundary policy/context changes as a first-class cache
        // version, separate from the shared backend's cookie generation. That
        // makes freshness explicit and avoids coupling document-cache validity
        // purely to jar mutations.
        self.cookie_facade.invalidate();
        self.clear_document_cookie_cache();
    }

    fn document_cookie_policy_block_reason_for_url(
        &self,
        url: &Url,
    ) -> Option<DocumentCookiePolicyBlockReason> {
        if !self.cookies_enabled() {
            return Some(DocumentCookiePolicyBlockReason::CookiesDisabled);
        }

        if self.document_cookie_storage_access_blocked_for_url(url) {
            return Some(DocumentCookiePolicyBlockReason::StorageAccessBlocked);
        }

        None
    }

    fn apply_cookie_facade_update(
        &mut self,
        update: impl FnOnce(&mut DocumentCookieFacadeState) -> bool,
    ) -> bool {
        let changed = update(&mut self.cookie_facade);
        if changed {
            self.clear_document_cookie_cache();
        }
        changed
    }

    fn clear_document_cookie_cache(&self) {
        *self.document_cookie_cache.borrow_mut() = None;
    }

    fn document_cookie_facade_status_for_url(&self, url: &Url) -> StoredCookieFacadeStatus {
        let policy_block_reason = self.document_cookie_policy_block_reason_for_url(url);
        let store_available = self.cookie_store.as_ref().is_some();
        let mut blocked_reasons = Vec::new();
        if !store_available {
            blocked_reasons.push(StoredCookieExclusionReason::StoreUnavailable);
        }
        if let Some(policy_block_reason) = policy_block_reason {
            let exclusion_reason = policy_block_reason.exclusion_reason();
            if !blocked_reasons.contains(&exclusion_reason) {
                blocked_reasons.push(exclusion_reason);
            }
        }

        StoredCookieFacadeStatus {
            cookie_access_enabled: store_available && blocked_reasons.is_empty(),
            store_available,
            blocked_reasons,
        }
    }

    fn document_cookie_storage_access_blocked_for_url(&self, url: &Url) -> bool {
        let browser_context = self.cookie_browser_context();
        if browser_context.storage_access_status == BrowserCookieStorageAccessStatus::Granted {
            return false;
        }

        // Blink treats Storage Access / site-for-cookies policy as facade
        // ownership, not backend storage policy. Keep the same split here:
        // `document.cookie` is blocked when the embedding browser context is
        // cross-site, but the underlying cookie jar and diagnostics remain
        // observable to the facade for tooling and cache invalidation.
        [
            browser_context.site_for_cookies_url.as_ref(),
            browser_context.top_frame_origin_url.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|context_url| !same_site_urls(url, context_url, true))
    }

    pub fn cookie_browser_context(&self) -> BrowserCookieFacadeContext {
        self.cookie_facade.browser_context()
    }

    pub(crate) fn resolve_url(&self, value: &str) -> String {
        if value.is_empty() {
            return String::new();
        }

        self.url()
            .join(value)
            .or_else(|_| Url::parse(value))
            .map(|url| url.to_string())
            .unwrap_or_else(|_| value.to_owned())
    }

    pub(crate) fn open_document(&mut self) {
        self.replace_on_close = true;
        self.active_element = None;
    }

    pub(crate) fn close_live_document_stream(&mut self) -> bool {
        if !self.replace_on_close {
            return false;
        }
        self.replace_on_close = false;
        self.active_element = None;
        true
    }

    pub(crate) fn replace_on_close(&self) -> bool {
        self.replace_on_close
    }
}

fn rejected_document_cookie_set_report(
    rejection_reason: StoredCookieSetRejectionReason,
) -> StoredCookieSetReport {
    StoredCookieSetReport {
        status: StoredCookieSetStatus::Rejected(rejection_reason),
        rejection_reasons: vec![rejection_reason],
        warning_reasons: Vec::new(),
        effective_same_site: None,
    }
}
