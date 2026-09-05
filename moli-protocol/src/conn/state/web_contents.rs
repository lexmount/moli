use moli_core::{
    browser::{MainFrameSlotId, WebContentsId},
    page::Page,
    runtime::NavigationEngine,
};

use super::navigation_controller::NavigationController;

mod document_host;
mod emulation_policy;
mod network_request_policy;
mod session_storage;
mod window;
pub(in crate::conn) use document_host::DocumentHost;
pub(crate) use emulation_policy::{EmulationPolicy, EmulationPolicyChange, EmulationPolicyDelta};
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
    pub(in crate::conn) main_frame: MainFrameSlot,
    pub(in crate::conn) navigation_engine: Option<NavigationEngine>,
    pub(in crate::conn) session_storage: SessionStorageNamespace,
    pub(in crate::conn) window: Window,
    pub(in crate::conn) crashed: bool,
    pub(in crate::conn) emulation_policy: EmulationPolicy,
    pub(in crate::conn) network_request_policy: NetworkRequestPolicy,
    pub(in crate::conn) network_offline: bool,
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
            main_frame: MainFrameSlot::default(),
            navigation_engine: None,
            session_storage: SessionStorageNamespace::default(),
            window: Window::default(),
            crashed: false,
            emulation_policy: EmulationPolicy::default(),
            network_request_policy: NetworkRequestPolicy::default(),
            network_offline: false,
            browser_identity_override: None,
            locale_override: None,
            timezone_override: None,
        }
    }
}

impl WebContents {
    pub(in crate::conn) fn id(&self) -> WebContentsId {
        self.id
    }

    pub(in crate::conn) fn set_network_request_policy(&mut self, policy: NetworkRequestPolicy) {
        self.network_request_policy = policy;
    }

    pub(in crate::conn) fn set_network_offline(&mut self, offline: bool) {
        self.network_offline = offline;
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

    pub(in crate::conn) fn replace_document(&mut self, next: Option<DocumentHost>) -> Option<Page> {
        std::mem::replace(&mut self.current_document, next).map(DocumentHost::retire)
    }
}
