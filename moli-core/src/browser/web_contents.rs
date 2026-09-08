use crate::{
    browser::{MainFrameSlotId, WebContentsId},
    page::{Page, RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind},
    runtime::NavigationEngine,
};

mod navigation_controller;
use navigation_controller::NavigationController;
pub use navigation_controller::PageNavigationHistoryEntry;
pub use navigation_controller::{
    HistoryTraversalDestination, InitialDocument, InitialDocumentCreator, InitialDocumentSnapshot,
    ResolvedHistoryTraversal,
};

mod document_host;
mod document_policy;
pub use document_policy::InheritedDocumentPolicy;
mod emulation_policy;
mod initial_document;
mod javascript_dialog;
pub use initial_document::InitialDocumentAdmission;
pub use initial_document::InitialDocumentBuildState;
pub use initial_document::{
    AdmittedInitialDocumentBuild, BuiltInitialDocument, CommittedInitialDocument,
    InitialDocumentBuildKey, InitialDocumentPageBuildWaiter,
};
mod navigation_commit;
pub use navigation_commit::AdmittedDocumentMaterialization;
mod navigation_history;
mod navigation_interception;
pub use navigation_interception::{
    ClaimedNavigationRequest, InterceptedNavigationLoad, InterceptedNavigationResponse,
    NavigationInterceptionPermit, NavigationRequestInterception,
};
mod navigation_load;
pub use navigation_history::SameDocumentNavigationCommitted;
pub use navigation_load::{AdmittedNavigationLoad, PreparedNavigationResponse};
mod network_request_policy;
pub use navigation_commit::{
    CommittedDocumentInfo, CommittedDocumentLifecycle, CommittedDocumentNavigation,
    DocumentNavigationDestination, PreparedDocumentNavigation, RetiringDocument,
};
mod page_surface;
mod paused_document_transfer;
mod resource_runtime;
mod session_storage;
#[cfg(test)]
mod tests;
mod window;
pub use document_host::DocumentLifecycleEvent;
pub use document_host::{DocumentCommitMetadata, DocumentCommitSnapshot, DocumentHost};
pub use emulation_policy::{EmulationPolicy, EmulationPolicyChange};
use javascript_dialog::JavaScriptDialogs;
pub use javascript_dialog::{
    JavaScriptDialogClosed, JavaScriptDialogError, JavaScriptDialogKey, JavaScriptDialogSnapshot,
};
pub use network_request_policy::NetworkRequestPolicy;
pub use network_request_policy::merge_extra_header_layers;
pub use page_surface::LIVE_DEVICE_METRICS_CLEAR_SCRIPT;
pub use page_surface::PageSurface;
pub use paused_document_transfer::{
    DocumentBodySource, OpenBodyStreamError, PausedDocumentTransfer,
    PausedResponsePreparedDocument, PausedStreamingDocumentResponse,
    PendingFetchResponseOpenedBodyStream, SyntheticDocumentResponseContext,
};
pub use session_storage::SessionStorageNamespace;
pub use window::{Window, WindowOpener};
pub use window::{WindowSurface, WindowSurfaceState};

/// Stable Browser page ownership, independent of DevTools bindings.
///
/// Owned by the physical BrowserContext, privately embedded in the Protocol
/// migration residence until the typed API cutover (Commit 24b).
/// Declaration order cancels pending work and retires the Document before
/// releasing the engine and storage. This owner is deliberately not Clone.
#[derive(Debug)]
pub struct WebContents {
    id: WebContentsId,
    navigation: NavigationController,
    // Dismiss modal renderer work before Document/Page teardown.
    pub(crate) javascript_dialogs: JavaScriptDialogs,
    pub(crate) main_frame: MainFrameSlot,
    navigation_engine: Option<NavigationEngine>,
    pub(crate) session_storage: SessionStorageNamespace,
    pub(crate) window: Window,
    pub(crate) crashed: bool,
    pub(crate) emulation_policy: EmulationPolicy,
    pub(crate) network_request_policy: NetworkRequestPolicy,
    fetch_subresource_interception: (bool, Option<crate::page::SubresourceResourceType>),
    pub(crate) network_offline: bool,
    pub(crate) tls_verify_host_override: Option<bool>,
    pub(crate) bypass_content_security_policy: bool,
    pub(crate) browser_identity_override: Option<moli_browser_profile::BrowserIdentityProfile>,
    pub(crate) locale_override: Option<String>,
    pub(crate) timezone_override: Option<String>,
}

impl Default for WebContents {
    fn default() -> Self {
        Self {
            id: WebContentsId::allocate(),
            navigation: NavigationController::default(),
            javascript_dialogs: JavaScriptDialogs::default(),
            main_frame: MainFrameSlot::default(),
            navigation_engine: None,
            session_storage: SessionStorageNamespace::default(),
            window: Window::default(),
            crashed: false,
            emulation_policy: EmulationPolicy::default(),
            network_request_policy: NetworkRequestPolicy::default(),
            fetch_subresource_interception: (false, None),
            network_offline: false,
            tls_verify_host_override: None,
            bypass_content_security_policy: false,
            browser_identity_override: None,
            locale_override: None,
            timezone_override: None,
        }
    }
}

impl WebContents {
    pub fn install_session_storage_namespace(&mut self, namespace: SessionStorageNamespace) {
        self.session_storage = namespace;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn fetch_subresource_interception(
        &self,
    ) -> (bool, Option<crate::page::SubresourceResourceType>) {
        self.fetch_subresource_interception
    }

    pub fn start_fetch_interception_update(
        &mut self,
        enabled: bool,
        resource_type: Option<crate::page::SubresourceResourceType>,
    ) -> Result<Option<crate::page::PendingPageCommand>, String> {
        // Install effective Browser policy even before the first Document, and
        // retain it if the outgoing renderer has already stopped accepting work.
        self.install_fetch_interception_policy(enabled, resource_type);
        self.main_frame
            .current_document
            .as_ref()
            .map(|document| {
                document
                    .page
                    .start_set_fetch_subresource_interception(enabled, resource_type)
            })
            .transpose()
            .map_err(|error| error.to_string())
    }

    pub fn install_fetch_interception_policy(
        &mut self,
        enabled: bool,
        resource_type: Option<crate::page::SubresourceResourceType>,
    ) {
        self.fetch_subresource_interception = (enabled, resource_type);
    }

    /// Retire Browser authority synchronously, then close the renderer without
    /// retaining any Context/registry borrow across await.
    pub fn begin_close(mut self) -> ClosingWebContents {
        self.navigation.clear_document_navigation_state();
        let page = self.replace_document(None);
        ClosingWebContents {
            page,
            _contents: self,
        }
    }

    pub fn performance_metric_snapshot(
        &self,
    ) -> Option<crate::page::RendererPerformanceMetricSnapshot> {
        Some(
            self.main_frame
                .current_document
                .as_ref()?
                .page
                .cached_performance_metric_snapshot(),
        )
    }

    /// Browser observation is independent of the DevTools command that
    /// produced this snapshot. The Page cache validates physical residence
    /// and revision; a rejected observation never invalidates a frozen reply.
    pub fn observe_renderer_page_state(
        &mut self,
        snapshot: &std::sync::Arc<moli_renderer_v8::RendererPageState>,
    ) -> bool {
        self.main_frame
            .current_document
            .as_mut()
            .is_some_and(|document| document.page.observe_renderer_page_state(snapshot))
    }

    pub fn apply_document_lifecycle(
        &mut self,
        event: RendererDocumentLifecycleEvent,
    ) -> Option<DocumentLifecycleEvent> {
        let document = self.main_frame.current_document.as_mut()?;
        let restarts = document
            .lifecycle
            .snapshot()
            .is_some_and(|snapshot| snapshot.epoch != event.epoch);
        if !document.lifecycle.observe(event) {
            return None;
        }
        if restarts
            || matches!(
                event.kind,
                RendererDocumentLifecycleEventKind::Terminated { .. }
            )
        {
            self.javascript_dialogs.clear();
        }
        if matches!(
            event.kind,
            RendererDocumentLifecycleEventKind::Started {
                reason: crate::page::RendererLifecycleStartReason::ExplicitDocumentOpen
                    | crate::page::RendererLifecycleStartReason::JavascriptDocumentReplacement
            }
        ) {
            self.navigation.mark_initial_empty_document_exited();
        }
        Some(DocumentLifecycleEvent::new(document.id, event))
    }

    pub fn replace_document(&mut self, next: Option<DocumentHost>) -> Option<Page> {
        self.navigation.cancel_initial_document_build();
        self.javascript_dialogs.clear();
        if let Some(document) = &next {
            self.navigation.seed_document_history((
                document.page.final_url().to_string(),
                document.page.document_title(),
            ));
        }
        self.main_frame.replace_document(next)
    }

    pub fn id(&self) -> WebContentsId {
        self.id
    }

    pub fn set_network_request_policy(&mut self, policy: NetworkRequestPolicy) {
        if let Some(engine) = self.navigation_engine.as_mut() {
            engine.set_cache_disabled(policy.cache_disabled);
        }
        self.network_request_policy = policy;
    }

    pub fn set_network_offline(&mut self, offline: bool) {
        self.network_offline = offline;
    }

    pub fn set_tls_verify_host_override(&mut self, enabled: Option<bool>) {
        self.tls_verify_host_override = enabled;
    }

    pub fn set_bypass_content_security_policy(&mut self, bypass: bool) {
        self.bypass_content_security_policy = bypass;
    }

    pub fn set_browser_identity_override(
        &mut self,
        identity: Option<moli_browser_profile::BrowserIdentityProfile>,
    ) {
        self.browser_identity_override = identity;
    }

    pub fn install_navigation_engine(&mut self, mut engine: NavigationEngine) {
        assert!(
            self.navigation_engine.is_none(),
            "WebContents must retain its first installed NavigationEngine"
        );
        engine.set_cache_disabled(self.network_request_policy.cache_disabled);
        engine.set_bypass_service_worker(self.network_request_policy.bypass_service_worker);
        self.navigation_engine = Some(engine);
    }

    pub fn set_locale_override(&mut self, locale: Option<String>) {
        self.locale_override = locale;
    }

    pub fn set_timezone_override(&mut self, timezone: Option<String>) {
        self.timezone_override = timezone;
    }
}

/// A move-owned Browser teardown participant, not a mutable WebContents handle.
/// Its Page retires before the engine and storage even if cleanup is cancelled.
pub struct ClosingWebContents {
    page: Option<Page>,
    _contents: WebContents,
}

impl ClosingWebContents {
    pub async fn close_async(mut self) {
        if let Some(page) = self.page.take() {
            let _ = page.close_async().await;
        }
    }
}

/// Stable main-frame slot; only the current Document is replaced on navigation.
#[derive(Debug)]
pub struct MainFrameSlot {
    id: MainFrameSlotId,
    pub current_document: Option<DocumentHost>,
}

impl Default for MainFrameSlot {
    fn default() -> Self {
        Self {
            id: MainFrameSlotId::allocate(),
            current_document: None,
        }
    }
}

impl MainFrameSlot {
    pub fn id(&self) -> MainFrameSlotId {
        self.id
    }

    fn replace_document(&mut self, next: Option<DocumentHost>) -> Option<Page> {
        std::mem::replace(&mut self.current_document, next).map(DocumentHost::retire)
    }
}
