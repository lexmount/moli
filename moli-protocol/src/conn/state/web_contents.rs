use moli_core::{
    browser::{MainFrameSlotId, WebContentsId},
    page::{Page, RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind},
    runtime::NavigationEngine,
};

mod navigation_controller;
use navigation_controller::NavigationController;
pub use navigation_controller::PageNavigationHistoryEntry;
pub(crate) use navigation_controller::{
    HistoryTraversalDestination, InitialDocument, InitialDocumentCreator, ResolvedHistoryTraversal,
};

mod document_host;
mod document_policy;
pub(in crate::conn::state) use document_policy::InheritedDocumentPolicy;
mod emulation_policy;
mod initial_document;
mod javascript_dialog;
pub(in crate::conn) use initial_document::InitialDocumentAdmission;
pub(in crate::conn::state) use initial_document::InitialDocumentBuildState;
pub(crate) use initial_document::{
    BuiltInitialDocument, InitialDocumentBuildKey, InitialDocumentPageBuildWaiter,
};
mod navigation_commit;
pub(in crate::conn) use navigation_commit::AdmittedDocumentMaterialization;
mod navigation_history;
mod navigation_interception;
pub(crate) use navigation_interception::{
    ClaimedNavigationRequest, InterceptedNavigationLoad, InterceptedNavigationResponse,
    NavigationInterceptionPermit, NavigationRequestInterception,
};
mod navigation_load;
pub(crate) use navigation_history::SameDocumentNavigationCommitted;
pub(in crate::conn) use navigation_load::{AdmittedNavigationLoad, PreparedNavigationResponse};
mod network_request_policy;
pub(crate) use navigation_commit::{
    CommittedDocumentLifecycle, DocumentNavigationDestination, PreparedDocumentNavigation,
    RetiringDocument,
};
mod page_surface;
mod resource_runtime;
mod session_storage;
#[cfg(test)]
mod tests;
mod window;
pub(in crate::conn) use document_host::DocumentHost;
pub(crate) use document_host::DocumentLifecycleEvent;
pub(crate) use emulation_policy::{EmulationPolicy, EmulationPolicyChange};
use javascript_dialog::JavaScriptDialogs;
pub(crate) use javascript_dialog::{
    JavaScriptDialogClosed, JavaScriptDialogError, JavaScriptDialogKey, JavaScriptDialogSnapshot,
};
pub(in crate::conn) use network_request_policy::NetworkRequestPolicy;
pub(in crate::conn) use network_request_policy::merge_extra_header_layers;
pub(crate) use page_surface::LIVE_DEVICE_METRICS_CLEAR_SCRIPT;
pub(in crate::conn) use page_surface::PageSurface;
pub(crate) use session_storage::SessionStorageNamespace;
pub(in crate::conn) use window::{Window, WindowOpener};
pub(crate) use window::{WindowSurface, WindowSurfaceState};

/// Stable Browser page ownership, independent of DevTools bindings.
///
/// Owned by the physical BrowserContext, privately embedded in the Protocol
/// migration residence until the typed API cutover (Commit 24b).
/// Declaration order cancels pending work and retires the Document before
/// releasing the engine and storage. This owner is deliberately not Clone.
#[derive(Debug)]
pub(in crate::conn) struct WebContents {
    id: WebContentsId,
    navigation: NavigationController,
    // Dismiss modal renderer work before Document/Page teardown.
    pub(in crate::conn) javascript_dialogs: JavaScriptDialogs,
    pub(in crate::conn) main_frame: MainFrameSlot,
    navigation_engine: Option<NavigationEngine>,
    pub(in crate::conn) session_storage: SessionStorageNamespace,
    pub(in crate::conn) window: Window,
    pub(in crate::conn) crashed: bool,
    pub(in crate::conn) emulation_policy: EmulationPolicy,
    pub(in crate::conn) network_request_policy: NetworkRequestPolicy,
    fetch_subresource_interception: (bool, Option<moli_core::page::SubresourceResourceType>),
    pub(in crate::conn) network_offline: bool,
    pub(in crate::conn) tls_verify_host_override: Option<bool>,
    pub(in crate::conn) bypass_content_security_policy: bool,
    pub(in crate::conn) browser_identity_override:
        Option<moli_browser_profile::BrowserIdentityProfile>,
    pub(in crate::conn) locale_override: Option<String>,
    pub(in crate::conn) timezone_override: Option<String>,
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
    #[cfg(test)]
    pub(in crate::conn::state) fn fetch_subresource_interception(
        &self,
    ) -> (bool, Option<moli_core::page::SubresourceResourceType>) {
        self.fetch_subresource_interception
    }

    pub(in crate::conn::state) fn start_fetch_interception_update(
        &mut self,
        enabled: bool,
        resource_type: Option<moli_core::page::SubresourceResourceType>,
    ) -> Result<Option<moli_core::page::PendingPageCommand>, String> {
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

    pub(in crate::conn::state) fn install_fetch_interception_policy(
        &mut self,
        enabled: bool,
        resource_type: Option<moli_core::page::SubresourceResourceType>,
    ) {
        self.fetch_subresource_interception = (enabled, resource_type);
    }

    /// Retire Browser authority synchronously, then close the renderer without
    /// retaining any Context/registry borrow across await.
    pub(in crate::conn) fn begin_close(mut self) -> ClosingWebContents {
        self.navigation.clear_document_navigation_state();
        let page = self.replace_document(None);
        ClosingWebContents {
            page,
            _contents: self,
        }
    }

    pub(in crate::conn) fn performance_metric_snapshot(
        &self,
    ) -> Option<moli_core::page::RendererPerformanceMetricSnapshot> {
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
    pub(in crate::conn) fn observe_renderer_page_state(
        &mut self,
        snapshot: &std::sync::Arc<moli_renderer_v8::RendererPageState>,
    ) -> bool {
        self.main_frame
            .current_document
            .as_mut()
            .is_some_and(|document| document.page.observe_renderer_page_state(snapshot))
    }

    pub(in crate::conn) fn apply_document_lifecycle(
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
                reason: moli_core::page::RendererLifecycleStartReason::ExplicitDocumentOpen
                    | moli_core::page::RendererLifecycleStartReason::JavascriptDocumentReplacement
            }
        ) {
            self.navigation.mark_initial_empty_document_exited();
        }
        Some(DocumentLifecycleEvent::new(document.id, event))
    }

    pub(in crate::conn) fn replace_document(&mut self, next: Option<DocumentHost>) -> Option<Page> {
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

    pub(in crate::conn) fn id(&self) -> WebContentsId {
        self.id
    }

    pub(in crate::conn) fn set_network_request_policy(&mut self, policy: NetworkRequestPolicy) {
        if let Some(engine) = self.navigation_engine.as_mut() {
            engine.set_cache_disabled(policy.cache_disabled);
        }
        self.network_request_policy = policy;
    }

    pub(in crate::conn) fn set_network_offline(&mut self, offline: bool) {
        self.network_offline = offline;
    }

    pub(in crate::conn) fn set_tls_verify_host_override(&mut self, enabled: Option<bool>) {
        self.tls_verify_host_override = enabled;
    }

    pub(in crate::conn) fn set_bypass_content_security_policy(&mut self, bypass: bool) {
        self.bypass_content_security_policy = bypass;
    }

    pub(in crate::conn) fn set_browser_identity_override(
        &mut self,
        identity: Option<moli_browser_profile::BrowserIdentityProfile>,
    ) {
        self.browser_identity_override = identity;
    }

    pub(in crate::conn) fn install_navigation_engine(&mut self, mut engine: NavigationEngine) {
        assert!(
            self.navigation_engine.is_none(),
            "WebContents must retain its first installed NavigationEngine"
        );
        engine.set_cache_disabled(self.network_request_policy.cache_disabled);
        engine.set_bypass_service_worker(self.network_request_policy.bypass_service_worker);
        self.navigation_engine = Some(engine);
    }

    pub(in crate::conn) fn set_locale_override(&mut self, locale: Option<String>) {
        self.locale_override = locale;
    }

    pub(in crate::conn) fn set_timezone_override(&mut self, timezone: Option<String>) {
        self.timezone_override = timezone;
    }
}

/// A move-owned Browser teardown participant, not a mutable WebContents handle.
/// Its Page retires before the engine and storage even if cleanup is cancelled.
pub(in crate::conn) struct ClosingWebContents {
    page: Option<Page>,
    _contents: WebContents,
}

impl ClosingWebContents {
    pub(in crate::conn) async fn close_async(mut self) {
        if let Some(page) = self.page.take() {
            let _ = page.close_async().await;
        }
    }
}

/// Stable main-frame slot; only the current Document is replaced on navigation.
#[derive(Debug)]
pub(in crate::conn) struct MainFrameSlot {
    id: MainFrameSlotId,
    pub(in crate::conn) current_document: Option<DocumentHost>,
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
    pub(in crate::conn) fn id(&self) -> MainFrameSlotId {
        self.id
    }

    fn replace_document(&mut self, next: Option<DocumentHost>) -> Option<Page> {
        std::mem::replace(&mut self.current_document, next).map(DocumentHost::retire)
    }
}
