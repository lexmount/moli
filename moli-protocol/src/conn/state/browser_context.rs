use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::PathBuf,
};

#[cfg(test)]
use moli_cookie_jar::{BrowserCookieStore, SharedBrowserCookieStore};
use moli_cookie_jar::{
    CookieSource, NetworkCookieRequestContext, StoredCookie, StoredCookieQueryReport,
};
#[cfg(test)]
use moli_core::browser::BrowserContextResourceStorageHandles;
#[cfg(test)]
use moli_core::browser::BrowserService;
use moli_core::browser::{
    BrowserContextHandle, BrowserContextId, BrowserContextPageStorageHandles,
    BrowserContextStoragePartitionHandles, BrowserHandle, ContextEmulationDefaults,
    ContextNetworkPolicy, EmulatedDeviceMetrics, EmulatedGeolocationOverrideState,
    EmulatedNetworkConditions, NavigationId, StoragePartitionKind,
};
use moli_core::runtime::{NavigationRuntimeConfig, RendererSharedWorkerRuntimeDiagnostics};
#[cfg(test)]
use moli_core::{
    network::SharedWebStorageStore,
    storage::{SharedIndexedDbManager, SharedStorageBucketStore},
};
use moli_shared_worker::SharedWorkerInstanceId;
use serde_json::{Value, json};

#[cfg(test)]
use crate::conn::cookie_manager_surface::BrowserContextCookieManagerSurface;

use super::{
    DevToolsSessionState,
    browser_identity::BrowserIdentityOverrideInputs,
    dedicated_worker_target::DedicatedWorkerTargetState,
    javascript_dialog::TargetPreparedJavaScriptDialog,
    page_agent_host::{PageAgentHost, PageAgentHostRegistry},
    page_slot::DocumentStartScript,
    service_worker_target::ServiceWorkerTargetState,
    shared_worker_target::SharedWorkerTargetState,
};

#[path = "../browser_document_commands.rs"]
mod browser_document_commands;
#[path = "../browser_web_contents_commands.rs"]
mod browser_web_contents_commands;
mod collection;
mod downloads;
#[cfg(test)]
mod initial_document_tests;
pub(in crate::conn) mod javascript_dialog;
mod navigation;
mod page_runtime;
pub(crate) use page_runtime::{
    BrowserAppManifestLoadPreparation, CompletedAppManifestLoadPreparation,
    CompletedAppManifestPublication, CompletedCaptureDocumentImage,
    CompletedCaptureDocumentScreencastFrame, CompletedCaptureDocumentSnapshot,
    CompletedChildFrameLifecycleWork, CompletedChildFrameNavigation,
    CompletedChildFrameTreeSnapshot, CompletedDocumentAutofillTrigger, CompletedDocumentBlobRead,
    CompletedDocumentCookieOwnerSnapshot, CompletedDocumentCspBypassUpdate,
    CompletedDocumentDiagnosticsSnapshot, CompletedDocumentFetchCommand,
    CompletedDocumentInputCommand, CompletedDocumentLifecycleStop, CompletedDocumentPolicyBatch,
    CompletedDocumentPolicyUpdate, CompletedDocumentResourceRuntimeUpdate,
    CompletedDocumentResourceTextSearch, CompletedDocumentStorageKeySnapshot,
    CompletedNavigationHistoryReset, CompletedNetworkResourceLoadPreparation,
    CompletedSetDocumentContent, CompletedTopLevelHistoryTraversal,
    CompletedTopLevelSameDocumentNavigation, DocumentFetchCommand, DocumentFetchCommandOutcome,
    DocumentPolicyUpdate, DocumentRuntimePolicyReconciliation, DocumentSnapshot, PageInputCommand,
    PendingAppManifestLoadPreparation, PendingAppManifestPublication, PendingCaptureDocumentImage,
    PendingCaptureDocumentScreencastFrame, PendingCaptureDocumentSnapshot,
    PendingChildFrameLifecycleWork, PendingChildFrameNavigation, PendingChildFrameTreeSnapshot,
    PendingDocumentAutofillTrigger, PendingDocumentBlobRead, PendingDocumentCookieOwnerSnapshot,
    PendingDocumentCspBypassUpdate, PendingDocumentDiagnosticsSnapshot,
    PendingDocumentFetchCommand, PendingDocumentInputCommand, PendingDocumentLifecycleStop,
    PendingDocumentPolicyBatch, PendingDocumentPolicyUpdate, PendingDocumentResourceRuntimeUpdate,
    PendingDocumentResourceTextSearch, PendingDocumentStorageKeySnapshot,
    PendingNavigationHistoryReset, PendingNetworkResourceLoadPreparation,
    PendingSetDocumentContent, PendingTopLevelHistoryTraversal,
    PendingTopLevelSameDocumentNavigation,
};
pub(in crate::conn) mod page_slot;
mod page_state;
mod permissions;
pub(in crate::conn) mod runtime_slot;
pub(in crate::conn) mod session;
#[cfg(test)]
mod tests;
mod workers;
pub(crate) use moli_core::browser::{OriginStorageUsage, SiteDataClearOptions};
pub(crate) use page_state::LoadedNavigationPageCommit;

/// DevTools projection for one Browser-owned Context.
pub struct BrowserContext {
    pub id: String,
    pub(crate) page_targets: PageAgentHostRegistry,
    /// Test-only cookie overrides, inherited by the first page fixture.
    #[cfg(test)]
    pub(crate) default_document_cookie_manager_surface: BrowserContextCookieManagerSurface,
    pub target_popup_ids: HashMap<String, u64>,
    pub(in crate::conn) automation_download_events_enabled: Option<bool>,
    pending_popup_javascript_dialogs: HashMap<u64, Vec<TargetPreparedJavaScriptDialog>>,
    pub(crate) shared_worker_targets: BTreeMap<SharedWorkerInstanceId, SharedWorkerTargetState>,
    pub(crate) dedicated_worker_targets: BTreeMap<u64, DedicatedWorkerTargetState>,
    pub(crate) service_worker_targets: BTreeMap<u64, ServiceWorkerTargetState>,
    pub(crate) service_worker_domain_sessions: BTreeSet<Option<String>>,
    browser_identity_inputs: BrowserIdentityOverrideInputs,
    pub(crate) next_default_document_start_script_id: u32,
    pub(crate) default_document_start_scripts: Vec<(String, DocumentStartScript)>,
    browser_context: BrowserContextHandle,
}

impl std::fmt::Debug for BrowserContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserContext")
            .field("browser_context_id", &self.browser_context.id())
            .field("id", &self.id)
            .field(
                "storage_partition_kind",
                &self.browser_context.storage_partition_kind_label(),
            )
            .field("active_target_id", &self.active_target_id())
            .field("active_session_id", &self.active_session_id())
            .field("has_loaded_page", &self.has_loaded_page())
            .finish_non_exhaustive()
    }
}

impl BrowserContext {
    pub(crate) fn download_policy(&self) -> Option<moli_core::browser::DownloadPolicy> {
        self.browser_context.download_policy()
    }

    pub(in crate::conn) fn set_download_policy(
        &mut self,
        policy: Option<moli_core::browser::DownloadPolicy>,
    ) {
        self.browser_context.set_download_policy(policy);
    }

    fn page_slot_for_target(&self, target_id: &str) -> Option<&super::page_slot::TargetPageSlot> {
        Some(self.page_targets.get(target_id)?.runtime_slot.page_slot())
    }

    fn page_slot_for_target_mut(
        &mut self,
        target_id: &str,
    ) -> Option<&mut super::page_slot::TargetPageSlot> {
        Some(
            self.page_targets
                .get_mut(target_id)?
                .runtime_slot
                .page_slot_mut(),
        )
    }

    pub fn browser_context_id(&self) -> BrowserContextId {
        self.browser_context.id()
    }

    #[cfg(test)]
    pub(crate) fn renderer_runtime_id_for_test(
        &self,
    ) -> moli_core::RendererBrowserContextRuntimeId {
        self.browser_context.renderer_runtime_id_for_test()
    }

    pub(crate) fn active_page_target(&self) -> &PageAgentHost {
        self.page_targets
            .active(self.browser_context.selected_web_contents_id())
            .expect("BrowserContext has no active page target")
    }

    pub(crate) fn active_page_target_mut(&mut self) -> &mut PageAgentHost {
        self.page_targets
            .active_mut(self.browser_context.selected_web_contents_id())
            .expect("BrowserContext has no active page target")
    }

    /// Parks a dialog only until the matching lightweight-popup target obtains
    /// a concrete protocol attachment.
    ///
    /// This is not a generic activity backlog: every value owns one popup id
    /// and one one-shot renderer completion. Removing the browser context or
    /// forgetting the popup mapping drops and dismisses the value.
    pub(crate) fn park_pending_popup_javascript_dialog(
        &mut self,
        dialog: TargetPreparedJavaScriptDialog,
    ) {
        let popup_id = dialog
            .popup_id()
            .expect("only lightweight-popup dialogs may enter popup attachment residence");
        self.pending_popup_javascript_dialogs
            .entry(popup_id)
            .or_default()
            .push(dialog);
    }

    pub(crate) fn take_pending_popup_javascript_dialogs(
        &mut self,
        popup_id: u64,
    ) -> Vec<TargetPreparedJavaScriptDialog> {
        self.pending_popup_javascript_dialogs
            .remove(&popup_id)
            .unwrap_or_default()
    }

    pub(crate) fn dismiss_pending_popup_javascript_dialogs(&mut self, popup_id: u64) {
        drop(self.take_pending_popup_javascript_dialogs(popup_id));
    }

    /// Isolated projection-unit fixture. Connection integration tests use its
    /// fixture factory so every Context belongs to that connection's Browser.
    #[cfg(test)]
    pub(crate) fn new(id: String) -> Self {
        let browser = BrowserService::start()
            .expect("standalone Browser owner should start")
            .handle();
        Self::new_with_browser_and_storage_partition_kind(
            &browser,
            id,
            BrowserContextStoragePartitionHandles::memory(),
            None,
            None,
            StoragePartitionKind::ProfileBacked,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_page_for_test(
        id: impl Into<String>,
        target_id: impl Into<String>,
    ) -> Self {
        let mut context = Self::new(id.into());
        context.bind_page_navigation_engines(Default::default(), None);
        context.set_active_target_id(target_id);
        context
    }

    #[cfg(test)]
    pub(crate) fn new_with_browser_for_test(
        browser: &BrowserHandle,
        id: impl Into<String>,
    ) -> Self {
        Self::new_with_browser_and_storage_partition_kind(
            browser,
            id.into(),
            BrowserContextStoragePartitionHandles::memory(),
            None,
            None,
            StoragePartitionKind::ProfileBacked,
        )
    }

    pub(crate) fn new_ephemeral_with_http_cache(
        browser: &BrowserHandle,
        id: String,
        http_cache_root: Option<PathBuf>,
        http_cache_max_bytes: Option<u64>,
    ) -> Self {
        Self::new_with_browser_and_storage_partition_kind(
            browser,
            id,
            BrowserContextStoragePartitionHandles::memory(),
            http_cache_root,
            http_cache_max_bytes,
            StoragePartitionKind::Ephemeral,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_storage_partition_and_http_cache(
        id: String,
        partition: BrowserContextStoragePartitionHandles,
        http_cache_root: Option<PathBuf>,
        http_cache_max_bytes: Option<u64>,
    ) -> Self {
        let browser = BrowserService::start()
            .expect("standalone Browser owner should start")
            .handle();
        Self::new_with_browser_and_storage_partition_kind(
            &browser,
            id,
            partition,
            http_cache_root,
            http_cache_max_bytes,
            StoragePartitionKind::ProfileBacked,
        )
    }

    pub(crate) fn new_with_storage_partition_handles_and_http_cache(
        browser: &BrowserHandle,
        id: String,
        partition: BrowserContextStoragePartitionHandles,
        http_cache_root: Option<PathBuf>,
        http_cache_max_bytes: Option<u64>,
    ) -> Self {
        Self::new_with_browser_and_storage_partition_kind(
            browser,
            id,
            partition,
            http_cache_root,
            http_cache_max_bytes,
            StoragePartitionKind::ProfileBacked,
        )
    }

    fn new_with_browser_and_storage_partition_kind(
        browser: &BrowserHandle,
        id: String,
        partition: BrowserContextStoragePartitionHandles,
        http_cache_root: Option<PathBuf>,
        http_cache_max_bytes: Option<u64>,
        kind: StoragePartitionKind,
    ) -> Self {
        let browser_context = browser
            .create_context(partition, kind, http_cache_root, http_cache_max_bytes)
            .expect("BrowserContext creation should succeed");

        Self {
            id,
            page_targets: PageAgentHostRegistry::default(),
            #[cfg(test)]
            default_document_cookie_manager_surface: BrowserContextCookieManagerSurface::default(),
            target_popup_ids: HashMap::new(),
            automation_download_events_enabled: None,
            pending_popup_javascript_dialogs: HashMap::new(),
            shared_worker_targets: BTreeMap::new(),
            dedicated_worker_targets: BTreeMap::new(),
            service_worker_targets: BTreeMap::new(),
            service_worker_domain_sessions: BTreeSet::new(),
            browser_identity_inputs: BrowserIdentityOverrideInputs::default(),
            next_default_document_start_script_id: 0,
            default_document_start_scripts: Vec::new(),
            browser_context,
        }
    }

    pub(crate) fn remove_from_browser(&self) -> Result<bool, String> {
        self.browser_context.remove()
    }

    pub(crate) fn bind_page_navigation_engines(
        &mut self,
        config: NavigationRuntimeConfig,
        renderer_output_transport_sender: Option<moli_core::RendererOutputTransportSender>,
    ) {
        self.browser_context
            .bind_page_navigation_engines(config, renderer_output_transport_sender);
    }

    pub(crate) fn set_renderer_output_transport_sender(
        &mut self,
        sender: moli_core::RendererOutputTransportSender,
    ) {
        self.browser_context
            .set_renderer_output_transport_sender(sender);
    }

    pub(crate) fn snapshot_profile_backed_cookies(&self) -> Option<Vec<StoredCookie>> {
        // The native Context can retire before its directory event is consumed.
        // Partition classification and snapshot must be one exact owner read.
        self.browser_context
            .snapshot_profile_backed_cookies()
            .ok()
            .flatten()
    }

    #[cfg(test)]
    pub(crate) fn is_profile_backed_storage_partition(&self) -> bool {
        self.browser_context.storage_partition_kind() == StoragePartitionKind::ProfileBacked
    }

    pub(crate) fn storage_partition_id(&self) -> &str {
        match self.browser_context.storage_partition_kind() {
            StoragePartitionKind::ProfileBacked => "default",
            StoragePartitionKind::Ephemeral => &self.id,
        }
    }

    pub(crate) fn storage_partition_kind_label(&self) -> &'static str {
        self.browser_context.storage_partition_kind_label()
    }

    #[cfg(test)]
    pub(crate) fn resource_storage_handles(&self) -> BrowserContextResourceStorageHandles {
        self.browser_context.resource_storage_handles_for_test()
    }

    pub(crate) fn page_storage_handles(&self) -> BrowserContextPageStorageHandles {
        self.browser_context
            .page_storage_handles(None)
            .expect("selected WebContents belongs to its BrowserContext")
    }

    pub(crate) fn page_storage_handles_for_target(
        &self,
        target_id: &str,
    ) -> Option<BrowserContextPageStorageHandles> {
        let handle = self.web_contents_handle_for_target(target_id)?;
        self.browser_context.page_storage_handles(Some(handle)).ok()
    }

    #[cfg(test)]
    pub(crate) fn with_cookie_store<R>(&self, f: impl FnOnce(&BrowserCookieStore) -> R) -> R {
        let store = self.browser_context.cookie_store_for_test();
        f(&store.lock())
    }

    #[cfg(test)]
    pub(crate) fn with_cookie_store_mut<R>(
        &self,
        f: impl FnOnce(&mut BrowserCookieStore) -> R,
    ) -> R {
        let store = self.browser_context.cookie_store_for_test();
        f(&mut store.lock())
    }

    #[cfg(test)]
    pub(crate) fn document_cookie_generation(&self) -> u64 {
        self.with_cookie_store(|store| store.document_cookie_generation())
    }

    pub(crate) fn observe_request_cookie_access_report(
        &self,
        request_url: &url::Url,
        request_context: NetworkCookieRequestContext,
    ) -> Option<StoredCookieQueryReport> {
        self.browser_context
            .observe_request_cookie_access_report(request_url, request_context)
    }

    pub(crate) fn storage_quota_for_origin(&self, origin: &str) -> (f64, bool) {
        self.browser_context.storage_quota_for_origin(origin)
    }

    pub(crate) fn set_storage_quota_override(&mut self, origin: String, quota: f64) {
        self.browser_context
            .set_storage_quota_override(origin, quota);
    }

    pub(crate) fn clear_storage_quota_override(&mut self, origin: &str) {
        self.browser_context.clear_storage_quota_override(origin);
    }

    pub(crate) fn storage_usage_for_origin(
        &self,
        serialized_origin: &str,
    ) -> Result<OriginStorageUsage, String> {
        self.browser_context
            .storage_usage_for_origin(serialized_origin)
    }

    pub(crate) fn selected_document_navigation_metadata(
        &self,
    ) -> Option<moli_core::browser::DocumentNavigationMetadata> {
        self.browser_context.selected_document_navigation_metadata()
    }

    pub(crate) fn set_javascript_dialog_handler_enabled(&self, enabled: bool) {
        self.browser_context
            .set_javascript_dialog_handler_enabled(enabled);
    }

    pub(crate) fn javascript_dialog_handler_enabled(&self) -> bool {
        self.browser_context.javascript_dialog_handler_enabled()
    }

    pub(crate) fn routes_renderer_browser_context_runtime(
        &self,
        runtime_id: moli_core::RendererBrowserContextRuntimeId,
    ) -> bool {
        self.browser_context
            .routes_renderer_browser_context_runtime(runtime_id)
    }

    pub(crate) fn target_id_for_renderer_owner_local_host_id(
        &self,
        owner_local_host_id: moli_core::RendererOwnerLocalHostId,
    ) -> Option<String> {
        let handle = self
            .browser_context
            .web_contents_for_renderer_owner(owner_local_host_id)?;
        self.page_targets
            .get_for_web_contents(handle.id())
            .map(|target| target.target_id().to_owned())
    }

    pub(crate) fn moli_memory_diagnostics(&self) -> Value {
        let target_infos = self.devtools_target_infos();
        let loaded_document_page_count = self.loaded_document_page_count();
        let pending_document_page_build_count = self.pending_document_page_build_count();
        let loaded_document_renderer_owner_count = self
            .loaded_document_renderer_owner_ids_for_diagnostics()
            .len();
        let estimated_document_isolate_count =
            loaded_document_page_count + pending_document_page_build_count;
        let page_target_pending_inspector_await_count =
            self.page_target_pending_inspector_await_count_for_diagnostics();
        let shared_worker_target_pending_inspector_await_count =
            self.shared_worker_target_pending_inspector_await_count_for_diagnostics();
        let service_worker_target_pending_inspector_await_count =
            self.service_worker_target_pending_inspector_await_count_for_diagnostics();
        let active_target = self
            .page_targets
            .active(self.browser_context.selected_web_contents_id());
        let runtime_session_diagnostics = active_target
            .map(|target| {
                let primary = target.devtools_sessions.primary();
                let attached_pending_inspector_await_count: usize = target
                    .devtools_sessions
                    .attached_states()
                    .map(DevToolsSessionState::pending_inspector_await_count)
                    .sum();
                json!({
                    "runtimeEnabled": primary.runtime_session_state.runtime_frontend_enabled,
                    "inspectorEnabled": primary.runtime_session_state.inspector_enabled,
                    "inspectorTargetCrashedDelivered": primary.runtime_session_state.inspector_target_crashed_delivered(),
                    "profilerCommandStateSource": "renderer-v8-inspector-agent",
                    "v8InspectorStateBytes": primary.inspector_session_state.v8_state.as_ref().map_or(0, |state| state.len()),
                    "attachedDevToolsSessionStateCount": target.devtools_sessions.attached_len(),
                    "pendingInspectorAwaitCount": target.devtools_sessions.pending_inspector_await_count(),
                    "primaryPendingInspectorAwaitCount": primary.pending_inspector_await_count(),
                    "attachedPendingInspectorAwaitCount": attached_pending_inspector_await_count,
                })
            })
            .unwrap_or(Value::Null);
        let page_session_diagnostics = active_target
            .map(|target| {
                let primary = target.devtools_sessions.primary();
                json!({
                    "pageLifecycleEvents": primary.page_session_state.page_lifecycle_events,
                    "logEnabled": primary.page_session_state.log_enabled,
                    "consoleEnabled": primary.console_output_session_state.console_enabled,
                    "performanceEnabled": primary
                        .page_session_state
                        .performance
                        .enabled(),
                    "performanceTimeDomain": primary
                        .page_session_state
                        .performance
                        .time_domain()
                        .as_str(),
                    "pageFontFamilyCount": primary.page_session_state.page_font_families.len(),
                })
            })
            .unwrap_or(Value::Null);
        let target_host_state_diagnostics = json!({
            "targetHostCount": self.page_targets.len(),
            "pageSessionStateCount": self.page_targets.iter()
                .filter(|target| self.has_non_default_session_state_for_target(target.target_id()))
                .count(),
            "targetOwnerStateWithPendingInspectorAwaitCount": self.page_targets.iter()
                .filter(|target| target.has_pending_inspector_awaits())
                .count(),
            "pendingInspectorAwaitCount": self.page_targets.iter()
                .map(|target| target.pending_inspector_await_count())
                .sum::<usize>(),
            "nonEmptyFetchStateCount": self.page_targets.iter()
                .filter(|target| !target.fetch_owner.pending_state().is_empty())
                .count(),
            "ownerStates": self.page_targets.iter().map(|target| json!({
                "targetId": target.target_id(),
                "ownerState": self.target_owner_diagnostics(target),
            })).collect::<Vec<_>>(),
        });
        json!({
            "id": self.id,
            "storagePartition": {
                "kind": self.storage_partition_kind_label(),
                "id": self.storage_partition_id(),
            },
            "hasActiveTarget": self.has_active_target(),
            "activeTargetId": self.active_target_id(),
            "hasActiveSession": self.has_active_session(),
            "activeLoadedPage": self.has_loaded_page(),
            "activePageAttachment": self.document_id().map(|attachment_id| json!({
                "id": attachment_id.get(),
                "targetId": self.active_target_id(),
            })),
            "backgroundTargetCount": self.background_target_count(),
            "backgroundLoadedPageCount": self
                .background_targets()
                .filter(|target| self.target_has_loaded_page(target.target_id()))
                .count(),
            "targetInfoCount": target_infos.len(),
            "attachedTargetInfoCount": target_infos
                .iter()
                .filter(|info| info.attached)
                .count(),
            "attachedTargetSessionCount": self
                .page_targets
                .iter()
                .map(|target| target.devtools_sessions.attached_len())
                .sum::<usize>(),
            "targetOpenerCount": self.browser_context.web_contents_window_counts().0,
            "targetOpenerFrameCount": self.page_targets.iter()
                .filter(|target| target.opener_frame_id.is_some()).count(),
            "targetCanAccessOpenerCount": self.browser_context.web_contents_window_counts().1,
            "targetWindowNameCount": self.browser_context.web_contents_window_counts().2,
            "defaultDocumentStartScriptCount": self.default_document_start_scripts.len(),
            "domRemoteObjectNodeCacheCount": active_target
                .map_or(0, |target| target.dom_remote_object_node_cache.len()),
            "sharedWorkerTargetCount": self.shared_worker_targets.len(),
            "serviceWorkerTargetCount": self.service_worker_targets.len(),
            "pendingInspectorAwaitCount": page_target_pending_inspector_await_count
                + shared_worker_target_pending_inspector_await_count
                + service_worker_target_pending_inspector_await_count,
            "pageTargetPendingInspectorAwaitCount": page_target_pending_inspector_await_count,
            "pageTargetWithPendingInspectorAwaitCount": self
                .page_target_with_pending_inspector_await_count_for_diagnostics(),
            "sharedWorkerTargetPendingInspectorAwaitCount": shared_worker_target_pending_inspector_await_count,
            "sharedWorkerTargetWithPendingInspectorAwaitCount": self
                .shared_worker_target_with_pending_inspector_await_count_for_diagnostics(),
            "serviceWorkerTargetPendingInspectorAwaitCount": service_worker_target_pending_inspector_await_count,
            "serviceWorkerTargetWithPendingInspectorAwaitCount": self
                .service_worker_target_with_pending_inspector_await_count_for_diagnostics(),
            "isolateScope": {
                "documentPageAccountingModel": "browser-context-page-count",
                "loadedDocumentPageCount": loaded_document_page_count,
                "loadedDocumentRendererOwnerCount": loaded_document_renderer_owner_count,
                "pendingDocumentPageBuildCount": pending_document_page_build_count,
                "estimatedDocumentIsolateCount": estimated_document_isolate_count,
                "sharedWorkerTargetCount": self.shared_worker_targets.len(),
                "serviceWorkerTargetCount": self.service_worker_targets.len(),
                "browserContextRuntime": self.browser_context.renderer_memory_diagnostics(),
            },
            "runtimeSession": runtime_session_diagnostics,
            "pageSession": page_session_diagnostics,
            "activeRuntimeSlot": active_target
                .map(|target| self.runtime_slot_diagnostics_for_target(target.target_id())),
            "activeFetch": active_target
                .map(|target| target.fetch_owner.moli_memory_diagnostics()),
            "activeOwnerState": active_target
                .map(|target| self.target_owner_diagnostics(target)),
            "targetHosts": target_host_state_diagnostics,
        })
    }

    fn target_owner_diagnostics(&self, target: &PageAgentHost) -> Value {
        let handle = self
            .web_contents_handle_for_target(target.target_id())
            .expect("live WebContents");
        let initial =
            self.browser_context
                .web_contents_initial_document_state(handle)
                .expect("live WebContents")
                .map(|document| {
                    let creator = document.creator().map(|creator| json!({
                "targetId": self.page_targets.iter()
                    .find(|target| target.web_contents_id() == creator.web_contents_id())
                    .map(PageAgentHost::target_id),
                "securityOrigin": creator.security_origin(),
                "secureContextType": creator.secure_context_type(),
            }));
                    json!({
                        "targetId": target.target_id(),
                        "initialUrl": document.initial_url(),
                        "creator": creator,
                        "materialized": document.materialized(),
                        "exited": document.exited(),
                        "pendingCrossDocumentNavigation": self.browser_context
                            .initial_document_has_pending_navigation(handle)
                            .unwrap_or(false),
                        "isOnInitialEmptyDocument": document.is_on_initial_empty_document(),
                    })
                });
        let mut diagnostics = target.owner_state.moli_memory_diagnostics();
        let window_surface = self
            .web_contents_window_surface(moli_core::browser::WebContentsHandle::new(
                self.browser_context.id(),
                target.web_contents_id(),
            ))
            .expect("live WebContents");
        diagnostics["initialEmptyDocument"] = json!(initial);
        diagnostics["windowSurfaceState"] = json!(window_surface.state.label());
        diagnostics["targetCrashed"] = json!(self.target_is_crashed(target.target_id()));
        diagnostics["isDefault"] = json!(
            target.owner_state.is_default()
                && self
                    .browser_context
                    .navigation_is_default(handle)
                    .unwrap_or(false)
                && !self.target_is_crashed(target.target_id())
                && window_surface == super::WindowSurface::default()
        );
        diagnostics
    }

    pub(crate) fn loaded_document_page_count(&self) -> usize {
        self.browser_context.loaded_document_count()
    }

    pub(crate) fn pending_document_page_build_count(&self) -> usize {
        self.page_targets
            .iter()
            .filter(|target| {
                self.target_has_pending_initial_document_page_build(target.target_id())
            })
            .count()
    }

    #[cfg(test)]
    pub(crate) fn assert_target_materialized_initial_empty_document_has_page(
        &self,
        target_id: &str,
    ) -> Result<(), String> {
        let Some(handle) = self.web_contents_handle_for_target(target_id) else {
            return Ok(());
        };
        if self
            .browser_context
            .web_contents_initial_document_state(handle)?
            .is_some_and(|initial| initial.is_on_initial_empty_document() && initial.materialized())
            && !self.browser_context.has_loaded_document(handle)
        {
            return Err(format!(
                "TargetInitialEmptyDocumentMissingPage: target {target_id} has materialized current initial empty document without loaded Page"
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn can_install_current_initial_empty_document_page(&self, target_id: &str) -> bool {
        let Some(handle) = self.web_contents_handle_for_target(target_id) else {
            return false;
        };
        !self.target_has_loaded_page(target_id)
            && !self
                .browser_context
                .has_pending_document_navigation(handle)
                .unwrap_or(true)
            && self
                .browser_context
                .is_on_initial_document(handle)
                .ok()
                .flatten()
                .unwrap_or(true)
    }

    /// Reports whether one exact Page target is still on its materialized
    /// initial empty Document and has a non-empty target URL left to load.
    ///
    /// This target-addressed query is used when the last debugger barrier is
    /// released by detaching its session, after that session can no longer be
    /// used as a routing key.
    pub(crate) fn target_needs_initial_document_navigation(&self, target_id: &str) -> bool {
        let Some(target) = self.page_target(target_id) else {
            return false;
        };
        let handle = self
            .web_contents_handle_for_target(target.target_id())
            .expect("live WebContents");
        let Some(initial_url) = self
            .browser_context
            .initial_document_url(handle)
            .ok()
            .flatten()
        else {
            return false;
        };
        target.target_url() != initial_url
            && !self
                .browser_context
                .initial_document_has_pending_navigation(handle)
                .unwrap_or(false)
    }

    pub(crate) fn loaded_document_renderer_owner_ids_for_diagnostics(&self) -> HashSet<u64> {
        self.browser_context.loaded_document_renderer_owner_ids()
    }

    pub(crate) fn pending_document_renderer_owner_ids_for_diagnostics(&self) -> HashSet<u64> {
        HashSet::new()
    }

    pub(crate) fn document_renderer_owner_ids_for_diagnostics(&self) -> HashSet<u64> {
        let mut owner_ids = self.loaded_document_renderer_owner_ids_for_diagnostics();
        owner_ids.extend(self.pending_document_renderer_owner_ids_for_diagnostics());
        owner_ids
    }

    pub(crate) fn dedicated_worker_running_worker_isolate_count_for_diagnostics(&self) -> usize {
        self.browser_context
            .dedicated_worker_running_isolate_count()
    }

    pub(crate) fn page_target_pending_inspector_await_count_for_diagnostics(&self) -> usize {
        self.page_targets
            .iter()
            .map(|target| target.pending_inspector_await_count())
            .sum()
    }

    pub(crate) fn has_pending_javascript_dialog(&self) -> bool {
        self.browser_context.has_pending_javascript_dialog()
    }

    pub(crate) fn page_target_with_pending_inspector_await_count_for_diagnostics(&self) -> usize {
        self.page_targets
            .iter()
            .filter(|target| target.has_pending_inspector_awaits())
            .count()
    }

    pub(crate) fn shared_worker_target_pending_inspector_await_count_for_diagnostics(
        &self,
    ) -> usize {
        self.shared_worker_targets
            .values()
            .map(SharedWorkerTargetState::pending_inspector_await_count_all_sessions)
            .sum()
    }

    pub(crate) fn shared_worker_target_with_pending_inspector_await_count_for_diagnostics(
        &self,
    ) -> usize {
        self.shared_worker_targets
            .values()
            .filter(|target| target.has_pending_inspector_awaits())
            .count()
    }

    pub(crate) fn service_worker_target_pending_inspector_await_count_for_diagnostics(
        &self,
    ) -> usize {
        self.service_worker_targets
            .values()
            .map(ServiceWorkerTargetState::pending_inspector_await_count_all_sessions)
            .sum()
    }

    pub(crate) fn service_worker_target_with_pending_inspector_await_count_for_diagnostics(
        &self,
    ) -> usize {
        self.service_worker_targets
            .values()
            .filter(|target| target.has_pending_inspector_awaits())
            .count()
    }

    pub(crate) fn shared_worker_runtime_diagnostics_for_diagnostics(
        &self,
    ) -> RendererSharedWorkerRuntimeDiagnostics {
        self.browser_context.shared_worker_runtime_diagnostics()
    }

    #[cfg(test)]
    pub(crate) fn start_document_navigation_for_active_target(
        &mut self,
        loader_id: String,
    ) -> Option<NavigationId> {
        let target_id = self.active_target_id()?.to_owned();
        self.start_document_navigation_for_target(&target_id, loader_id)
    }

    pub(crate) fn start_document_navigation_for_target(
        &mut self,
        target_id: &str,
        loader_id: String,
    ) -> Option<NavigationId> {
        self.page_targets.get(target_id)?;
        if self.has_pending_document_navigation_for_target(target_id)
            && let Some(previous_loader) = self
                .current_document_loader_id_for_target(target_id)
                .map(str::to_owned)
        {
            self.page_targets
                .get_mut(target_id)?
                .owner_state
                .page_resource_store
                .discard_uncommitted_loader(&previous_loader);
        }
        Some(self.begin_target_document_navigation(target_id, loader_id))
    }

    pub(crate) fn accepts_pending_document_navigation_event(&self, token: &NavigationId) -> bool {
        self.browser_context
            .accepts_any_pending_navigation_event(token)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn document_navigation_cancellation_handle(
        &self,
        token: &NavigationId,
    ) -> Option<moli_fetch::FetchCancelHandle> {
        self.browser_context
            .document_navigation_cancellation_handle_for_test(token)
    }

    pub(crate) fn arm_background_navigation_completion(
        &mut self,
        token: &NavigationId,
        additional_cancellation: Option<moli_fetch::FetchCancelHandle>,
    ) -> bool {
        self.browser_context
            .arm_background_navigation_completion(token, additional_cancellation)
    }

    pub(crate) fn settle_background_navigation_completion(&mut self, token: &NavigationId) -> bool {
        self.browser_context
            .settle_background_navigation_completion(token)
    }

    pub(crate) fn has_inflight_background_navigation(&self) -> bool {
        self.browser_context.has_inflight_background_navigation()
    }

    #[cfg(test)]
    pub(crate) fn accepts_document_body_completion_event(&self, token: &NavigationId) -> bool {
        self.browser_context
            .accepts_any_document_body_completion_for_test(token)
    }

    pub(crate) fn clear_pending_document_navigation_for_target_if_matches(
        &mut self,
        target_id: Option<&str>,
        navigation: &NavigationId,
    ) -> bool {
        let target_id = match target_id {
            Some(id) if self.page_targets.get(id).is_some() => id,
            _ => return false,
        };
        if !self.accepts_document_body_completion_event_for_target(target_id, navigation) {
            return false;
        }
        // The committed error document may retain an uncommitted response body.
        if let Some(loader_id) = self
            .current_document_loader_id_for_target(target_id)
            .map(str::to_owned)
        {
            self.page_targets
                .get_mut(target_id)
                .expect("live target")
                .owner_state
                .page_resource_store
                .discard_uncommitted_loader(&loader_id);
        }
        self.clear_pending_document_navigation_if_matches_for_target(target_id, navigation)
    }

    #[cfg(test)]
    pub(crate) fn commit_document_navigation_if_matches(&mut self, token: &NavigationId) {
        let target_id = self
            .page_targets
            .iter()
            .find(|target| {
                self.accepts_pending_document_navigation_event_for_target(target.target_id(), token)
            })
            .map(|target| target.target_id().to_owned());
        if let Some(target_id) = target_id {
            self.commit_pending_document_navigation_if_matches_for_target(&target_id, token);
        }
    }

    #[cfg(test)]
    pub(crate) fn clear_document_navigation_state_for_active_target(&mut self) {
        if let Some(target_id) = self.active_target_id_owned() {
            self.clear_document_navigation_state_for_target(&target_id);
        }
    }

    pub(crate) fn clear_site_data_for_origin(
        &mut self,
        origin: &url::Url,
        options: SiteDataClearOptions,
    ) -> Result<(), String> {
        self.browser_context
            .clear_site_data_for_origin(origin, options)
    }

    pub(crate) fn clear_site_data_for_storage_key(
        &mut self,
        storage_key: &moli_storage_key::MoliStorageKey,
        options: SiteDataClearOptions,
    ) -> Result<(), String> {
        self.browser_context
            .clear_site_data_for_storage_key(storage_key, options)
    }

    pub(crate) fn clear_http_cache(&self) -> Result<(), String> {
        self.browser_context.clear_http_cache()
    }

    #[cfg(test)]
    pub(crate) fn http_cache_configuration_for_test(&self) -> (Option<PathBuf>, Option<u64>) {
        self.browser_context.http_cache_configuration_for_test()
    }

    pub(crate) fn snapshot_cookies(&self) -> Vec<StoredCookie> {
        self.browser_context.snapshot_cookies()
    }

    pub(crate) fn store_cookie(
        &self,
        cookie: StoredCookie,
        request_url: Option<&url::Url>,
        source: CookieSource,
    ) -> moli_cookie_jar::StoredCookieSetReport {
        self.browser_context
            .store_cookie(cookie, request_url, source)
    }

    #[cfg(test)]
    pub(crate) fn store_response_cookie_headers_for_test(
        &self,
        response_url: &url::Url,
        response_headers: &[(String, String)],
    ) {
        self.with_cookie_store_mut(|store| {
            store.store_response_headers(response_url, response_headers);
        });
    }

    #[cfg(test)]
    pub(crate) fn cookie_store_for_test(&self) -> SharedBrowserCookieStore {
        self.browser_context.cookie_store_for_test()
    }

    #[cfg(test)]
    pub(crate) fn web_storage_store_for_test(&self) -> SharedWebStorageStore {
        self.browser_context.web_storage_store_for_test()
    }

    #[cfg(test)]
    pub(crate) fn session_storage_store_for_test(&self) -> SharedWebStorageStore {
        self.browser_context
            .selected_session_storage_store_for_test()
            .expect("active WebContents")
    }

    #[cfg(test)]
    pub(crate) fn indexed_db_manager_for_test(&self) -> SharedIndexedDbManager {
        self.browser_context.indexed_db_manager_for_test()
    }

    #[cfg(test)]
    pub(crate) fn storage_bucket_store_for_test(&self) -> SharedStorageBucketStore {
        self.browser_context.storage_bucket_store_for_test()
    }

    #[cfg(test)]
    pub(crate) fn replace_storage_bucket_store_for_test(
        &mut self,
        storage_bucket_store: SharedStorageBucketStore,
    ) {
        self.browser_context
            .replace_storage_bucket_store_for_test(storage_bucket_store);
    }

    #[cfg(test)]
    pub(crate) fn upsert_cookie_for_test(
        &self,
        cookie: StoredCookie,
    ) -> moli_cookie_jar::StoredCookieSetReport {
        self.store_cookie(cookie, None, CookieSource::Management)
    }

    #[cfg(test)]
    pub(crate) fn test_last_cookie_access_index(
        &self,
        domain: &str,
        path: &str,
        name: &str,
    ) -> Option<u64> {
        self.with_cookie_store(|store| store.test_last_access_index(domain, path, name))
    }

    #[cfg(test)]
    pub(crate) fn delete_cookies(
        &mut self,
        name: Option<&str>,
        domain: Option<&str>,
        path: Option<&str>,
        url_host: Option<&str>,
    ) {
        self.delete_cookies_with_partition_key(name, domain, path, url_host, None);
    }

    pub(crate) fn delete_cookies_with_partition_key(
        &mut self,
        name: Option<&str>,
        domain: Option<&str>,
        path: Option<&str>,
        url_host: Option<&str>,
        partition_key: Option<&moli_cookie_jar::StoredCookiePartitionKey>,
    ) {
        self.browser_context.delete_cookies_with_partition_key(
            name,
            domain,
            path,
            url_host,
            partition_key,
        );
    }

    pub(crate) fn active_target_id(&self) -> Option<&str> {
        self.page_targets
            .get_for_web_contents(self.browser_context.selected_web_contents_id()?)
            .map(PageAgentHost::target_id)
    }

    pub(crate) fn active_target_id_owned(&self) -> Option<String> {
        self.active_target_id().map(str::to_owned)
    }

    pub(crate) fn effective_active_browser_identity_override_owned(
        &self,
    ) -> Option<moli_browser_profile::BrowserIdentityProfile> {
        self.page_targets
            .active(self.browser_context.selected_web_contents_id())
            .and_then(|host| self.browser_identity_override_for_target(host.target_id()))
            .or_else(|| self.default_browser_identity_override_owned())
    }

    pub(crate) fn reported_active_user_agent_override(&self) -> Option<String> {
        self.page_targets
            .active(self.browser_context.selected_web_contents_id())
            .and_then(PageAgentHost::reported_user_agent_override)
            .map(str::to_owned)
            .or_else(|| {
                self.default_browser_identity_override()
                    .map(|identity| identity.user_agent().to_owned())
            })
    }

    pub(crate) fn default_browser_identity_override(
        &self,
    ) -> Option<moli_browser_profile::BrowserIdentityProfile> {
        self.browser_context.browser_identity_override()
    }

    pub(crate) fn default_browser_identity_override_owned(
        &self,
    ) -> Option<moli_browser_profile::BrowserIdentityProfile> {
        self.default_browser_identity_override()
    }

    pub(crate) fn set_default_user_agent_override(
        &mut self,
        user_agent: Option<String>,
        fallback: &moli_browser_profile::BrowserIdentityProfile,
    ) {
        self.browser_identity_inputs.user_agent = user_agent;
        self.browser_context
            .set_browser_identity_override(self.browser_identity_inputs.materialize(fallback));
    }

    pub(crate) fn set_default_locale_override(
        &mut self,
        locale: Option<String>,
        fallback: &moli_browser_profile::BrowserIdentityProfile,
    ) {
        self.browser_context
            .set_default_locale_override(locale.clone());
        self.browser_identity_inputs.accept_language = locale;
        self.browser_context
            .set_browser_identity_override(self.browser_identity_inputs.materialize(fallback));
    }

    #[cfg(test)]
    pub(crate) fn replace_default_browser_identity_override_for_test(
        &mut self,
        identity: moli_browser_profile::BrowserIdentityProfile,
    ) {
        self.browser_identity_inputs = BrowserIdentityOverrideInputs::from_profile(&identity);
        self.browser_context
            .set_browser_identity_override(Some(identity));
    }

    pub(crate) fn effective_active_locale_override_owned(&self) -> Option<String> {
        self.page_targets
            .active(self.browser_context.selected_web_contents_id())
            .and_then(|host| self.locale_override_for_target(host.target_id()))
            .or_else(|| self.emulation_defaults().locale.clone())
    }

    pub(crate) fn effective_active_timezone_override_owned(&self) -> Option<String> {
        self.page_targets
            .active(self.browser_context.selected_web_contents_id())
            .and_then(|host| self.timezone_override_for_target(host.target_id()))
            .or_else(|| self.emulation_defaults().timezone.clone())
    }

    pub(crate) fn effective_active_tls_verify_host_override(&self) -> Option<bool> {
        self.page_targets
            .active(self.browser_context.selected_web_contents_id())
            .and_then(|host| self.tls_verify_host_override_for_target(host.target_id()))
            .or(self.browser_context.network_policy().tls_verify_host)
    }

    pub(crate) fn emulation_defaults(&self) -> ContextEmulationDefaults {
        self.browser_context.emulation_defaults()
    }

    pub(crate) fn set_default_timezone_override(&mut self, timezone: Option<String>) {
        self.browser_context.set_default_timezone_override(timezone);
    }

    pub(crate) fn set_default_network_conditions(
        &mut self,
        conditions: Option<EmulatedNetworkConditions>,
    ) {
        self.browser_context
            .set_default_network_conditions(conditions);
    }

    pub(crate) fn set_default_geolocation_override(
        &mut self,
        geolocation: Option<EmulatedGeolocationOverrideState>,
    ) {
        self.browser_context
            .set_default_geolocation_override(geolocation);
    }

    pub(crate) fn set_default_device_metrics(&mut self, metrics: EmulatedDeviceMetrics) -> bool {
        self.browser_context.set_default_device_metrics(metrics)
    }

    pub(crate) fn network_policy(&self) -> ContextNetworkPolicy {
        self.browser_context.network_policy()
    }

    pub(crate) fn set_default_extra_headers(&mut self, headers: Vec<(String, String)>) {
        self.browser_context.set_default_extra_headers(headers);
    }

    pub(crate) fn set_network_policy(&mut self, policy: ContextNetworkPolicy) {
        self.browser_context.set_network_policy(policy);
    }

    pub(crate) fn set_tls_verify_host_override(&mut self, enabled: bool) {
        self.browser_context
            .set_context_tls_verify_host_override(enabled);
    }

    #[cfg(test)]
    pub(crate) fn set_http_proxy_override_for_test(&mut self, proxy: Option<String>) {
        self.browser_context.set_http_proxy_override_for_test(proxy);
    }

    pub(crate) fn effective_active_network_offline(
        &self,
        global_network_conditions: Option<EmulatedNetworkConditions>,
    ) -> bool {
        self.page_targets
            .active(self.browser_context.selected_web_contents_id())
            .and_then(|host| {
                self.target_emulation_policy(host.target_id())
                    .expect("live WebContents")
                    .network_conditions
            })
            .or(self.emulation_defaults().network_conditions)
            .or(global_network_conditions)
            .is_some_and(|conditions| !conditions.navigator_online())
    }

    pub(crate) fn effective_network_offline_for_target(
        &self,
        target_id: &str,
        global_network_conditions: Option<EmulatedNetworkConditions>,
    ) -> bool {
        self.page_target(target_id)
            .and_then(|state| {
                self.target_emulation_policy(state.target_id())
                    .expect("live WebContents")
                    .network_conditions
            })
            .or(self.emulation_defaults().network_conditions)
            .or(global_network_conditions)
            .is_some_and(|conditions| !conditions.navigator_online())
    }

    pub(crate) fn effective_locale_override_for_target_owned(
        &self,
        target_id: &str,
    ) -> Option<String> {
        self.page_target(target_id)
            .and_then(|state| self.locale_override_for_target(state.target_id()))
            .or_else(|| self.emulation_defaults().locale.clone())
    }

    pub(crate) fn has_active_target(&self) -> bool {
        self.page_targets
            .active(self.browser_context.selected_web_contents_id())
            .is_some()
    }

    pub(crate) fn is_active_target(&self, target_id: &str) -> bool {
        self.active_target_id() == Some(target_id)
    }

    pub(crate) fn set_active_target_id(&mut self, target_id: impl Into<String>) {
        let target_id = target_id.into();
        if self.page_targets.get(&target_id).is_none() {
            let inserted = self.register_web_contents_target(
                target_id.clone(),
                None,
                super::identity::TargetIdentityState::about_blank(),
                moli_core::browser::WebContentsCreation::default(),
                super::page_slot::TargetPageSlot::default(),
            );
            debug_assert!(inserted, "new page target id must be unique");
        }
        let handle = self
            .web_contents_handle_for_target(&target_id)
            .expect("registered page target must have WebContents");
        self.select_registered_web_contents(handle)
            .expect("registered WebContents must remain selectable");
    }

    pub(crate) fn rekey_active_target(&mut self, target_id: impl Into<String>) -> bool {
        let Some(previous) = self.active_target_id_owned() else {
            return false;
        };
        self.page_targets.rekey(&previous, target_id.into())
    }

    pub(crate) fn active_session_id(&self) -> Option<&str> {
        self.page_targets
            .active(self.browser_context.selected_web_contents_id())?
            .session_id()
    }

    pub(crate) fn active_session_id_owned(&self) -> Option<String> {
        self.active_session_id().map(str::to_owned)
    }

    pub(crate) fn has_active_session(&self) -> bool {
        self.active_session_id().is_some()
    }

    pub(crate) fn active_target_is_unclaimed_default_placeholder(
        &self,
        default_target_id: &str,
    ) -> bool {
        self.active_target_id() == Some(default_target_id)
            && !self.has_active_session()
            && !self.has_loaded_page()
            && self.background_target_count() == 0
            && self.shared_worker_targets.is_empty()
            && self.dedicated_worker_targets.is_empty()
            && self.service_worker_targets.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn attach_active_session(&mut self, session_id: impl Into<String>) {
        self.active_page_target_mut()
            .attach_session(session_id.into());
    }

    #[cfg(test)]
    pub(crate) fn detach_active_session(&mut self) -> Option<String> {
        self.page_targets
            .active_mut(self.browser_context.selected_web_contents_id())?
            .detach_session()
    }

    pub(crate) fn target_url(&self) -> &str {
        self.active_page_target().target_identity.url()
    }

    pub(crate) fn set_target_url(&mut self, url: String) {
        self.active_page_target_mut().target_identity.set_url(url);
    }

    #[cfg(test)]
    pub(crate) fn set_target_security_origin(&mut self, security_origin: String) {
        self.active_page_target_mut()
            .target_identity
            .set_security_origin(security_origin);
    }

    #[cfg(test)]
    pub(crate) fn set_target_secure_context_type(&mut self, secure_context_type: String) {
        self.active_page_target_mut()
            .target_identity
            .set_secure_context_type(secure_context_type);
    }
}
