use std::sync::Arc;

use super::{RendererDocumentLifecycleIdentity, RendererWindowDocumentSource};
use crate::SharedWebStorageStore;

/// Request context frozen in the initiating Document, before target selection
/// or an Inspector startup pause can change the source's URL or policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererNavigationInitiator {
    data: Arc<RendererNavigationInitiatorData>,
}

#[derive(Debug, Eq, PartialEq)]
struct RendererNavigationInitiatorData {
    url: url::Url,
    origin: String,
    referrer_policy: String,
}

impl RendererNavigationInitiator {
    pub(crate) fn new(
        url: url::Url,
        origin: moli_url::WebOrigin,
        referrer_policy: Option<String>,
    ) -> Self {
        Self {
            data: Arc::new(RendererNavigationInitiatorData {
                url,
                referrer_policy: if origin.is_opaque() {
                    "no-referrer".to_owned()
                } else {
                    referrer_policy
                        .unwrap_or_else(|| moli_fetch::DEFAULT_REFERRER_POLICY.to_owned())
                },
                origin: origin.ascii_serialization().to_owned(),
            }),
        }
    }

    pub fn url(&self) -> &url::Url {
        &self.data.url
    }

    pub fn origin(&self) -> moli_url::WebOrigin {
        moli_url::WebOrigin::from_serialized(&self.data.origin)
    }

    pub fn request_metadata(&self) -> moli_fetch::SubresourceRequestMetadata {
        moli_fetch::SubresourceRequestMetadata {
            referrer_policy: Some(self.data.referrer_policy.clone()),
            ..Default::default()
        }
    }

    pub fn document_referrer(&self, destination: &url::Url) -> String {
        moli_fetch::referrer_value(
            &self.data.url,
            destination,
            Some(&self.data.referrer_policy),
            None,
        )
        .unwrap_or_default()
    }

    pub(crate) fn outgoing_referrer(&self) -> String {
        if self.origin().is_opaque() {
            return String::new();
        }
        moli_fetch::referrer_value(&self.data.url, &self.data.url, Some("unsafe-url"), None)
            .unwrap_or_default()
    }
}

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
/// NewWindow requests an independent browser window when a target is created.
/// Reusing an existing context keeps its window and follows foreground policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererPopupDisposition {
    Foreground,
    Background,
    NewWindow,
}

/// A renderer-accepted request to create an auxiliary browsing context or
/// reuse a named related browsing context, including an ordinary root Page.
///
/// Special targets (`_self`, `_parent`, `_top`) are not valid values here:
/// they navigate an existing browsing context and use the corresponding
/// navigation authority instead. The shared name cell identifies the chosen
/// context independently of later changes to its name or the current session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererPendingPopupActivation {
    parts: RendererPopupActivationParts,
}

/// The complete accepted popup action, moved to the browser owner at once.
#[derive(Debug, Clone)]
pub struct RendererPopupActivationParts {
    pub source: RendererPopupActivationSource,
    pub disposition: RendererPopupDisposition,
    pub navigation_requested: bool,
    pub navigation_initiator: Option<RendererNavigationInitiator>,
    /// Origin equality with the selected Document at navigation acceptance.
    /// Unlike serialized origins, this preserves inherited opaque identity.
    pub same_origin_with_target: Option<bool>,
    pub popup_id: Option<u64>,
    pub browsing_context_name: Option<super::RendererBrowsingContextName>,
    pub auxiliary_window: Option<super::RendererAuxiliaryWindow>,
    pub document_response: Option<super::RendererAuxiliaryDocumentResponse>,
    pub initial_document_environment: Option<super::RendererCapturedDocumentEnvironment>,
    pub pending_auxiliary_page: Option<crate::runtime::RendererPendingAuxiliaryPage>,
    pub url: String,
    pub target_name: String,
    pub session_storage_store: Option<SharedWebStorageStore>,
    pub initial_empty_document_storage_key: Option<moli_storage_key::MoliStorageKey>,
}

impl RendererPendingPopupActivation {
    pub(crate) fn with_same_origin_target(mut self, same_origin: Option<bool>) -> Self {
        self.parts.same_origin_with_target = same_origin;
        self
    }

    pub(crate) fn with_navigation_initiator(
        mut self,
        initiator: RendererNavigationInitiator,
    ) -> Self {
        self.parts.navigation_initiator = Some(initiator);
        self
    }

    pub fn navigation_initiator(&self) -> Option<&RendererNavigationInitiator> {
        self.parts.navigation_initiator.as_ref()
    }

    pub(crate) fn with_initial_document_environment(
        mut self,
        environment: Option<super::RendererCapturedDocumentEnvironment>,
    ) -> Self {
        self.parts.initial_document_environment = environment;
        self
    }

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
            parts: RendererPopupActivationParts {
                source: RendererPopupActivationSource::Window {
                    root_document,
                    window,
                    exposes_opener,
                },
                disposition,
                navigation_requested: true,
                navigation_initiator: None,
                same_origin_with_target: None,
                popup_id,
                browsing_context_name: None,
                auxiliary_window: None,
                document_response: None,
                initial_document_environment: None,
                pending_auxiliary_page: None,
                url,
                target_name,
                session_storage_store: None,
                initial_empty_document_storage_key: None,
            },
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
            parts: RendererPopupActivationParts {
                source: RendererPopupActivationSource::BrowserContext,
                disposition,
                navigation_requested: true,
                navigation_initiator: None,
                same_origin_with_target: None,
                popup_id,
                browsing_context_name: None,
                auxiliary_window: None,
                document_response: None,
                initial_document_environment: None,
                pending_auxiliary_page: None,
                url,
                target_name,
                session_storage_store: None,
                initial_empty_document_storage_key: None,
            },
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
        self.parts.session_storage_store = session_storage_store;
        self.parts.initial_empty_document_storage_key = initial_empty_document_storage_key;
        self
    }

    pub(crate) fn with_browsing_context_name(
        mut self,
        name: super::RendererBrowsingContextName,
    ) -> Self {
        self.parts.browsing_context_name = Some(name);
        self
    }

    pub fn browsing_context_name(&self) -> Option<super::RendererBrowsingContextName> {
        self.parts.browsing_context_name.clone()
    }

    pub(crate) fn with_auxiliary_window(
        mut self,
        window: Option<super::RendererAuxiliaryWindow>,
    ) -> Self {
        self.parts.auxiliary_window = window;
        self
    }

    pub(crate) fn with_document_response(
        mut self,
        response: Option<super::RendererAuxiliaryDocumentResponse>,
    ) -> Self {
        self.parts.document_response = response;
        self
    }

    pub(crate) fn with_pending_auxiliary_page(
        mut self,
        page: Option<crate::runtime::RendererPendingAuxiliaryPage>,
    ) -> Self {
        self.parts.pending_auxiliary_page = page;
        self
    }

    pub fn pending_auxiliary_page(&self) -> Option<crate::runtime::RendererPendingAuxiliaryPage> {
        self.parts.pending_auxiliary_page.clone()
    }

    pub fn source(&self) -> &RendererPopupActivationSource {
        &self.parts.source
    }

    pub fn disposition(&self) -> RendererPopupDisposition {
        self.parts.disposition
    }

    pub(crate) fn with_navigation_requested(mut self, requested: bool) -> Self {
        self.parts.navigation_requested = requested;
        self
    }

    pub fn navigation_requested(&self) -> bool {
        self.parts.navigation_requested
    }

    pub fn popup_id(&self) -> Option<u64> {
        self.parts.popup_id
    }

    pub fn url(&self) -> &str {
        &self.parts.url
    }

    pub fn target_name(&self) -> &str {
        &self.parts.target_name
    }

    pub fn into_parts(self) -> RendererPopupActivationParts {
        self.parts
    }
}

impl PartialEq for RendererPopupActivationParts {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && self.navigation_requested == other.navigation_requested
            && self.navigation_initiator == other.navigation_initiator
            && self.same_origin_with_target == other.same_origin_with_target
            && self.disposition == other.disposition
            && self.popup_id == other.popup_id
            && self.browsing_context_name == other.browsing_context_name
            && self.auxiliary_window == other.auxiliary_window
            && self.document_response == other.document_response
            && self.initial_document_environment == other.initial_document_environment
            && self.pending_auxiliary_page == other.pending_auxiliary_page
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

impl Eq for RendererPopupActivationParts {}

fn is_special_browsing_context_target(target_name: &str) -> bool {
    target_name.eq_ignore_ascii_case("_self")
        || target_name.eq_ignore_ascii_case("_parent")
        || target_name.eq_ignore_ascii_case("_top")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_action_equality_distinguishes_navigation_from_lookup() {
        let action = RendererPendingPopupActivation::browser_context(
            None,
            "about:blank".to_owned(),
            "report".to_owned(),
            RendererPopupDisposition::Foreground,
        );
        assert_ne!(action.clone().with_navigation_requested(false), action);
    }
}
