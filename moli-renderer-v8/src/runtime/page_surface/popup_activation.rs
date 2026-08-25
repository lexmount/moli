use std::sync::Arc;

use super::{RendererDocumentLifecycleIdentity, RendererWindowDocumentSource};
use crate::SharedWebStorageStore;

/// Exact renderer-side initiator of one auxiliary browsing-context action.
///
/// Window-originated actions retain the root lifecycle identity as causal
/// metadata plus the concrete source Window/Document. `exposes_opener`
/// records the already-decided `noopener`/`noreferrer` policy; protocol code
/// must not reconstruct it from a later target or DOM state.
///
/// Browser-context actions are produced by APIs such as
/// `Clients.openWindow()` and notification navigation. They intentionally have
/// no Window opener and must not be projected as if the current root frame had
/// initiated them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RendererPopupActivationSource {
    Window {
        root_document: RendererDocumentLifecycleIdentity,
        window: RendererWindowDocumentSource,
        exposes_opener: bool,
    },
    BrowserContext,
}

/// Browser-owner selection policy for an accepted auxiliary browsing context.
///
/// This records only whether the target should become the active target. It
/// deliberately does not distinguish tab and window chrome, which the
/// renderer target model does not expose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererPopupDisposition {
    Foreground,
    Background,
}

/// A renderer-accepted request to create or reuse an auxiliary browsing
/// context.
///
/// Special targets (`_self`, `_parent`, `_top`) are not valid values here:
/// they navigate an existing browsing context and use the corresponding
/// navigation authority instead. Keeping this carrier auxiliary-only prevents
/// protocol code from deciding the target from a later current session.
#[derive(Debug, Clone)]
pub struct RendererPendingPopupActivation {
    source: RendererPopupActivationSource,
    disposition: RendererPopupDisposition,
    popup_id: Option<u64>,
    url: String,
    target_name: String,
    session_storage_store: Option<SharedWebStorageStore>,
    initial_empty_document_storage_key: Option<moli_storage_key::MoliStorageKey>,
}

impl RendererPendingPopupActivation {
    pub fn window(
        root_document: RendererDocumentLifecycleIdentity,
        window: RendererWindowDocumentSource,
        exposes_opener: bool,
        popup_id: Option<u64>,
        url: String,
        target_name: String,
        disposition: RendererPopupDisposition,
    ) -> Self {
        assert!(
            !is_special_browsing_context_target(&target_name),
            "popup activation must not carry an existing-context special target"
        );
        Self {
            source: RendererPopupActivationSource::Window {
                root_document,
                window,
                exposes_opener,
            },
            disposition,
            popup_id,
            url,
            target_name,
            session_storage_store: None,
            initial_empty_document_storage_key: None,
        }
    }

    pub fn browser_context(
        popup_id: Option<u64>,
        url: String,
        target_name: String,
        disposition: RendererPopupDisposition,
    ) -> Self {
        assert!(
            !is_special_browsing_context_target(&target_name),
            "browser-context popup activation must not carry a special target"
        );
        Self {
            source: RendererPopupActivationSource::BrowserContext,
            disposition,
            popup_id,
            url,
            target_name,
            session_storage_store: None,
            initial_empty_document_storage_key: None,
        }
    }

    /// Attaches the state captured when the auxiliary browsing context was
    /// accepted in the renderer.
    ///
    /// The cloned session-storage namespace and initial about:blank storage
    /// key belong to this exact popup action. They must travel with the action
    /// rather than be reconstructed from whichever target is current when
    /// protocol output is emitted. `Page.windowOpen` is a separate concrete
    /// observation recorded beside this action at the renderer production
    /// boundary; it must not be hidden inside an after-response owner action.
    pub fn with_initial_auxiliary_state(
        mut self,
        session_storage_store: Option<SharedWebStorageStore>,
        initial_empty_document_storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) -> Self {
        self.session_storage_store = session_storage_store;
        self.initial_empty_document_storage_key = initial_empty_document_storage_key;
        self
    }

    pub fn source(&self) -> &RendererPopupActivationSource {
        &self.source
    }

    pub fn disposition(&self) -> RendererPopupDisposition {
        self.disposition
    }

    pub fn popup_id(&self) -> Option<u64> {
        self.popup_id
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn target_name(&self) -> &str {
        &self.target_name
    }

    #[allow(clippy::type_complexity)]
    pub fn into_parts(
        self,
    ) -> (
        RendererPopupActivationSource,
        RendererPopupDisposition,
        Option<u64>,
        String,
        String,
        Option<SharedWebStorageStore>,
        Option<moli_storage_key::MoliStorageKey>,
    ) {
        (
            self.source,
            self.disposition,
            self.popup_id,
            self.url,
            self.target_name,
            self.session_storage_store,
            self.initial_empty_document_storage_key,
        )
    }
}

impl PartialEq for RendererPendingPopupActivation {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && self.disposition == other.disposition
            && self.popup_id == other.popup_id
            && self.url == other.url
            && self.target_name == other.target_name
            && match (&self.session_storage_store, &other.session_storage_store) {
                (None, None) => true,
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                _ => false,
            }
            && self.initial_empty_document_storage_key == other.initial_empty_document_storage_key
    }
}

impl Eq for RendererPendingPopupActivation {}

fn is_special_browsing_context_target(target_name: &str) -> bool {
    target_name.eq_ignore_ascii_case("_self")
        || target_name.eq_ignore_ascii_case("_parent")
        || target_name.eq_ignore_ascii_case("_top")
}
