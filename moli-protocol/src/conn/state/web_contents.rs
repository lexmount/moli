use moli_core::{
    browser::{DocumentLifecycle, MainFrameSlotId, WebContentsId},
    page::{
        Page, RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
        RendererDocumentLifecycleSnapshot,
    },
    runtime::NavigationEngine,
};

use super::navigation_controller::NavigationController;

mod document_host;
mod emulation_policy;
mod javascript_dialog;
mod network_request_policy;
mod session_storage;
mod window;
pub(in crate::conn) use document_host::DocumentHost;
pub(crate) use emulation_policy::{EmulationPolicy, EmulationPolicyChange, EmulationPolicyDelta};
use javascript_dialog::JavaScriptDialogs;
pub(crate) use javascript_dialog::{
    JavaScriptDialogClosed, JavaScriptDialogError, JavaScriptDialogKey, JavaScriptDialogSnapshot,
};
pub(in crate::conn) use network_request_policy::NetworkRequestPolicy;
pub(crate) use session_storage::SessionStorageNamespace;
pub(in crate::conn) use window::{Window, WindowOpener};
pub(crate) use window::{WindowSurface, WindowSurfaceState};

/// Stable Browser page ownership, independent of DevTools bindings.
///
/// Embedded in the legacy residence until the typed API cutover (Commit 24b).
/// Declaration order cancels pending work and retires the Document before
/// releasing the engine and storage. This owner is deliberately not Clone.
#[derive(Debug)]
pub(in crate::conn) struct WebContents {
    id: WebContentsId,
    pub(in crate::conn) navigation: NavigationController,
    // Dismiss modal renderer work before Document/Page teardown.
    pub(in crate::conn) javascript_dialogs: JavaScriptDialogs,
    pub(in crate::conn) main_frame: MainFrameSlot,
    pub(in crate::conn) navigation_engine: Option<NavigationEngine>,
    pub(in crate::conn) session_storage: SessionStorageNamespace,
    pub(in crate::conn) window: Window,
    pub(in crate::conn) crashed: bool,
    pub(in crate::conn) emulation_policy: EmulationPolicy,
    pub(in crate::conn) network_request_policy: NetworkRequestPolicy,
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
    pub(in crate::conn) fn bind_document_lifecycle(
        &mut self,
        snapshot: RendererDocumentLifecycleSnapshot,
    ) -> bool {
        let Some(document) = self.main_frame.current_document.as_mut() else {
            return false;
        };
        let previous = document
            .lifecycle
            .snapshot()
            .map(|snapshot| (snapshot.frame, snapshot.document, snapshot.epoch));
        document.lifecycle = DocumentLifecycle::from_snapshot(snapshot);
        if previous != Some((snapshot.frame, snapshot.document, snapshot.epoch))
            || snapshot.terminated.is_some()
        {
            self.javascript_dialogs.clear();
        }
        true
    }

    pub(in crate::conn) fn observe_document_lifecycle(
        &mut self,
        event: RendererDocumentLifecycleEvent,
    ) -> bool {
        let Some(document) = self.main_frame.current_document.as_mut() else {
            return false;
        };
        let restarts = document
            .lifecycle
            .snapshot()
            .is_some_and(|snapshot| snapshot.epoch != event.epoch);
        if !document.lifecycle.observe(event) {
            return false;
        }
        if restarts
            || matches!(
                event.kind,
                RendererDocumentLifecycleEventKind::Terminated { .. }
            )
        {
            self.javascript_dialogs.clear();
        }
        true
    }

    pub(in crate::conn) fn replace_document(&mut self, next: Option<DocumentHost>) -> Option<Page> {
        self.javascript_dialogs.clear();
        self.main_frame.replace_document(next)
    }

    pub(in crate::conn) fn id(&self) -> WebContentsId {
        self.id
    }

    pub(in crate::conn) fn set_network_request_policy(&mut self, policy: NetworkRequestPolicy) {
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
