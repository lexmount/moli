use std::sync::Arc;

use super::{
    RendererDocumentLifecycleIdentity, RendererTopLevelNavigationRequest,
    RendererWindowDocumentSource,
};
use crate::{
    SharedWebStorageStore,
    document_runtime::{DocumentPolicyContainer, DocumentSandboxPolicy},
    runtime::{
        PageId, RendererOutputResidenceIdentity, RendererOwnerLocalHostId,
        RendererPendingAuxiliaryPage, RendererScriptAgentAdmission,
        RendererServiceWorkerClientsOpenWindowContinuation,
    },
};

/// Renderer-owned frame policy frozen when a new auxiliary browsing context
/// passes creator-side sandbox admission.
///
/// Browser/protocol owners may retain and return this value with each new
/// Document build, but its sandbox representation stays private to the
/// renderer. Opener suppression changes the browsing-context group and DOM
/// opener surface; it does not discard this accepted frame policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererAuxiliaryBrowsingContextPolicy {
    sandbox: DocumentSandboxPolicy,
}

impl RendererAuxiliaryBrowsingContextPolicy {
    pub(crate) const fn from_sandbox(sandbox: DocumentSandboxPolicy) -> Self {
        Self { sandbox }
    }

    pub(crate) const fn sandbox(self) -> DocumentSandboxPolicy {
        self.sandbox
    }

    pub(crate) const fn forces_opaque_origin(self) -> bool {
        self.sandbox.forces_opaque_origin
    }

    pub(crate) fn sandbox_with_response_content_security_policies(
        self,
        policies: &[String],
    ) -> DocumentSandboxPolicy {
        self.sandbox.with_response_content_security_policy(
            DocumentSandboxPolicy::from_response_content_security_policies(policies),
        )
    }

    pub(crate) fn with_response_content_security_policies(self, policies: &[String]) -> Self {
        Self::from_sandbox(self.sandbox_with_response_content_security_policies(policies))
    }

    pub(crate) fn initial_document_policy_container(self) -> DocumentPolicyContainer {
        DocumentPolicyContainer {
            sandbox: self.sandbox,
            ..DocumentPolicyContainer::default()
        }
    }
}

/// Renderer-owned terminal response sanitation result for a top-level
/// Document navigation.
///
/// Chromium runs this check before enforcing COOP for the response. Keeping
/// the result typed lets the fetch/protocol owners stop transport and expose
/// diagnostics without either layer re-deriving sandbox or COOP semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererMainDocumentResponseBlock {
    CrossOriginOpenerPolicySandboxedNavigation,
}

impl RendererMainDocumentResponseBlock {
    pub fn sanitize(
        auxiliary_policy: Option<RendererAuxiliaryBrowsingContextPolicy>,
        bypass_content_security_policy: bool,
        response_url: &url::Url,
        response_headers: &[(String, String)],
    ) -> Option<Self> {
        let inherited_sandbox = auxiliary_policy
            .map(RendererAuxiliaryBrowsingContextPolicy::sandbox)
            .unwrap_or_default();
        let response_sandbox = if bypass_content_security_policy {
            DocumentSandboxPolicy::default()
        } else {
            DocumentPolicyContainer::from_navigation_response_headers(
                response_headers,
                response_url,
            )
            .sandbox
        };
        let pending_sandbox =
            inherited_sandbox.with_response_content_security_policy(response_sandbox);
        if pending_sandbox.sandboxes_navigation
            && crate::cross_origin_isolation::response_enforces_cross_origin_opener_policy(
                response_url,
                response_headers,
            )
        {
            Some(Self::CrossOriginOpenerPolicySandboxedNavigation)
        } else {
            None
        }
    }
}

/// Exact already-live renderer Page selected for a popup navigation.
///
/// Named browsing-context lookup is a renderer Page-group operation. Carrying
/// both residence coordinates prevents the protocol layer from repeating
/// that lookup through its eventually-consistent target-name projection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RendererResolvedPopupTarget {
    owner_local_host_id: RendererOwnerLocalHostId,
    page_id: PageId,
}

/// Renderer-decided browsing-context-group policy for one newly created
/// auxiliary Page.
///
/// This is decided beside named-target lookup. The protocol layer may adopt
/// the reserved Page and expose a DevTools target, but must not infer whether
/// that Page can participate in the creator's related-name lookup from a
/// later target-name projection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RendererPopupNewTargetDisposition {
    /// The new Page remains in the creator's related Page group.
    Related,
    /// Opener suppression created a fresh group without a browsing-context
    /// name (`""` or `_blank`).
    FreshUnnamed,
    /// Opener suppression created a fresh group whose first realm must retain
    /// the requested ordinary browsing-context name.
    FreshNamed,
}

/// Frozen user-activation result for one admitted new auxiliary context.
///
/// The generation identifies the exact transient grant observed by the
/// creation transaction. An admitted creation consumes that grant at most
/// once; an embedder-policy bypass has no generation and consumes nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPopupCreationUserActivation {
    transient_activation_generation: Option<u64>,
    consumed_transient_activation_generation: Option<u64>,
}

impl RendererPopupCreationUserActivation {
    pub(crate) fn new(
        transient_activation_generation: Option<u64>,
        consumed_transient_activation_generation: Option<u64>,
    ) -> Self {
        assert_eq!(
            transient_activation_generation, consumed_transient_activation_generation,
            "an admitted auxiliary creation must consume exactly the transient activation it observed"
        );
        Self {
            transient_activation_generation,
            consumed_transient_activation_generation,
        }
    }

    pub(crate) const fn user_gesture(self) -> bool {
        self.transient_activation_generation.is_some()
    }

    pub(crate) const fn consumed(self) -> bool {
        self.consumed_transient_activation_generation.is_some()
    }
}

impl RendererPopupNewTargetDisposition {
    pub const fn is_fresh(self) -> bool {
        matches!(self, Self::FreshUnnamed | Self::FreshNamed)
    }

    pub const fn carries_initial_name(self) -> bool {
        matches!(self, Self::FreshNamed)
    }
}

impl RendererResolvedPopupTarget {
    pub(crate) const fn from_wire_parts(owner_local_host_id: u64, page_id: u64) -> Option<Self> {
        let Some(owner_local_host_id) = RendererOwnerLocalHostId::from_wire(owner_local_host_id)
        else {
            return None;
        };
        let Some(page_id) = PageId::from_wire(page_id) else {
            return None;
        };
        Some(Self {
            owner_local_host_id,
            page_id,
        })
    }

    pub(crate) const fn from_residence(residence: RendererOutputResidenceIdentity) -> Option<Self> {
        match residence {
            RendererOutputResidenceIdentity::Page {
                owner_local_host_id,
                page_id,
            } => Some(Self {
                owner_local_host_id,
                page_id,
            }),
            RendererOutputResidenceIdentity::SharedWorker { .. }
            | RendererOutputResidenceIdentity::ServiceWorker { .. } => None,
        }
    }

    pub const fn owner_local_host_id(self) -> RendererOwnerLocalHostId {
        self.owner_local_host_id
    }

    pub const fn page_id(self) -> PageId {
        self.page_id
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
    /// URL observed by synchronous target selection and `Page.windowOpen`.
    ///
    /// This remains present when form target selection creates an auxiliary
    /// browsing context but a later source-Document policy check suppresses
    /// the destination navigation.
    requested_url: String,
    destination_request: Option<Box<RendererTopLevelNavigationRequest>>,
    /// Whether a newly created DevTools target reports `requested_url` even
    /// though the URL is renderer-owned and has no browser destination work.
    /// This is true for `javascript:` popup creation, but false for a
    /// creation-only form action whose destination was denied.
    reports_requested_url_without_destination: bool,
    target_name: String,
    /// Heap-owned so adding popup policy facts does not inflate every
    /// `RendererOwnerAction` variant and its async orchestration frames.
    referrers: Option<Box<RendererPopupNavigationReferrers>>,
    pending_auxiliary_page: Option<RendererPendingAuxiliaryPage>,
    resolved_target_page: Option<RendererResolvedPopupTarget>,
    new_target_disposition: Option<RendererPopupNewTargetDisposition>,
    /// Heap-owned to keep two generation identities out of every owner-action
    /// stack frame; only admitted new-context actions allocate it.
    creation_user_activation: Option<Box<RendererPopupCreationUserActivation>>,
    auxiliary_browsing_context_policy: Option<Box<RendererAuxiliaryBrowsingContextPolicy>>,
    /// Once-only worker Promise completion bound to this activation's exact
    /// Fresh Page reservation. Ordinary Window producers never carry it.
    service_worker_clients_open_window_continuation:
        Option<RendererServiceWorkerClientsOpenWindowContinuation>,
    session_storage_store: Option<SharedWebStorageStore>,
    initial_empty_document_storage_key: Option<moli_storage_key::MoliStorageKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RendererPopupNavigationReferrers {
    /// Frozen Referer value for the destination navigation.
    ///
    /// `Some("")` explicitly suppresses the header. `None` is reserved for
    /// legacy/browser-context producers that still rely on target-local
    /// inference; protocol code must otherwise use this creator-side result
    /// instead of deriving a referrer from the popup's initial about:blank.
    navigation: String,
    /// Frozen referrer for the auxiliary context's initial empty Document.
    ///
    /// This is the creator's full URL unless `noreferrer` applies, independent
    /// of HTTP header eligibility and the destination navigation policy.
    initial_document: String,
    /// Frozen script-visible referrer for the committed destination Document.
    ///
    /// This differs from `navigation_referrer` for non-HTTP destinations. In
    /// particular, a noopener initial `about:blank` keeps the creator's full
    /// URL even though no HTTP Referer header can be emitted.
    document: String,
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
            requested_url: url.clone(),
            destination_request: Some(Box::new(RendererTopLevelNavigationRequest::get(url))),
            reports_requested_url_without_destination: false,
            target_name,
            referrers: None,
            pending_auxiliary_page: None,
            resolved_target_page: None,
            new_target_disposition: None,
            creation_user_activation: None,
            auxiliary_browsing_context_policy: None,
            service_worker_clients_open_window_continuation: None,
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
            requested_url: url.clone(),
            destination_request: Some(Box::new(RendererTopLevelNavigationRequest::get(url))),
            reports_requested_url_without_destination: false,
            target_name,
            referrers: None,
            pending_auxiliary_page: None,
            resolved_target_page: None,
            new_target_disposition: None,
            creation_user_activation: None,
            auxiliary_browsing_context_policy: None,
            service_worker_clients_open_window_continuation: None,
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

    /// Replaces the default GET navigation with the exact request selected by
    /// the renderer producer.
    ///
    /// The URL must remain the one used for synchronous target creation and
    /// `Page.windowOpen`; only the request metadata is enriched. Keeping this
    /// request whole is required for auxiliary form POSTs.
    pub fn with_navigation_request(mut self, request: RendererTopLevelNavigationRequest) -> Self {
        assert_eq!(
            self.requested_url,
            request.url(),
            "popup target selection and navigation request must carry one URL"
        );
        self.destination_request = Some(Box::new(request));
        self
    }

    /// Retains the already-observed target-creation URL while suppressing the
    /// destination navigation.
    ///
    /// Blink can reach this state for direct `form.submit()`: target lookup
    /// (and therefore new auxiliary-context creation) precedes the late
    /// sandboxed-forms check. The initial empty Document is not a substitute
    /// navigation request and must not be represented as one.
    pub fn without_destination_navigation(mut self) -> Self {
        self.destination_request = None;
        self.reports_requested_url_without_destination = false;
        self
    }

    /// Keeps the renderer-owned requested URL as the new target's DevTools
    /// creation projection without turning it back into browser navigation.
    pub fn without_destination_navigation_with_requested_url_observation(mut self) -> Self {
        self.destination_request = None;
        self.reports_requested_url_without_destination = true;
        self
    }

    /// Binds the renderer-owned browsing-context and Page identities reserved
    /// when this action synchronously created a new auxiliary context.
    pub fn with_pending_auxiliary_page(
        mut self,
        pending_auxiliary_page: Option<RendererPendingAuxiliaryPage>,
    ) -> Self {
        assert!(
            pending_auxiliary_page.is_none() || self.resolved_target_page.is_none(),
            "a popup action cannot create and reuse a renderer Page"
        );
        self.pending_auxiliary_page = pending_auxiliary_page;
        self
    }

    /// Binds the exact already-live renderer Page selected by related-page
    /// named browsing-context lookup.
    pub fn with_resolved_target_page(
        mut self,
        resolved_target_page: RendererResolvedPopupTarget,
    ) -> Self {
        assert!(
            self.pending_auxiliary_page.is_none(),
            "a popup action cannot reuse and create a renderer Page"
        );
        self.resolved_target_page = Some(resolved_target_page);
        self
    }

    /// Records that the renderer completed target lookup and deliberately
    /// selected the attached new Page reservation.
    ///
    /// Unmigrated producers may still reserve a Page before the protocol's
    /// legacy name projection chooses between new and existing targets. They
    /// must not set this fact.
    pub fn with_new_target_disposition(
        mut self,
        disposition: RendererPopupNewTargetDisposition,
    ) -> Self {
        assert!(
            self.pending_auxiliary_page.is_some(),
            "a renderer-selected new popup target requires its Page reservation"
        );
        assert!(
            self.resolved_target_page.is_none(),
            "an existing popup target cannot also be selected as new"
        );
        assert!(
            !disposition.carries_initial_name()
                || (!self.target_name.is_empty()
                    && !is_special_browsing_context_target(&self.target_name)),
            "a fresh named popup requires an ordinary browsing-context name"
        );
        let admission = self
            .pending_auxiliary_page
            .expect("new popup target reservation")
            .page_reservation()
            .script_agent_admission();
        assert!(
            matches!(
                (disposition, admission),
                (
                    RendererPopupNewTargetDisposition::Related,
                    RendererScriptAgentAdmission::RelatedAuxiliaryPage { .. }
                ) | (
                    RendererPopupNewTargetDisposition::FreshUnnamed
                        | RendererPopupNewTargetDisposition::FreshNamed,
                    RendererScriptAgentAdmission::Fresh
                )
            ),
            "popup group disposition must match its Page reservation admission"
        );
        assert!(
            !disposition.is_fresh() || self.popup_id.is_none(),
            "a Fresh popup Page cannot retain an opener-local lightweight owner"
        );
        assert_eq!(
            disposition.is_fresh(),
            self.auxiliary_browsing_context_policy.is_some(),
            "Fresh popup admission must retain exactly one accepted auxiliary frame policy"
        );
        self.new_target_disposition = Some(disposition);
        self
    }

    /// Attaches the renderer-frozen sandbox policy for a newly admitted Fresh
    /// auxiliary Page. The protocol target may retain this opaque value, but
    /// only renderer Document construction interprets it.
    pub fn with_auxiliary_browsing_context_policy(
        mut self,
        policy: RendererAuxiliaryBrowsingContextPolicy,
    ) -> Self {
        assert!(
            self.pending_auxiliary_page.is_some(),
            "accepted auxiliary frame policy requires its Page reservation"
        );
        assert!(
            self.resolved_target_page.is_none(),
            "existing popup targets already own their frame policy"
        );
        self.auxiliary_browsing_context_policy = Some(Box::new(policy));
        self
    }

    /// Attaches the exact ServiceWorker `Clients.openWindow()` continuation
    /// after its Fresh Page identity has been reserved.
    pub fn with_service_worker_clients_open_window_continuation(
        mut self,
        continuation: RendererServiceWorkerClientsOpenWindowContinuation,
    ) -> Self {
        let pending = self
            .pending_auxiliary_page
            .expect("openWindow continuation requires its auxiliary Page reservation");
        assert_eq!(
            pending.page_reservation().page_id(),
            continuation.expected_page_id(),
            "openWindow continuation must bind the activation's exact Page"
        );
        assert!(
            self.resolved_target_page.is_none(),
            "openWindow continuation cannot complete from an existing target"
        );
        assert!(
            self.service_worker_clients_open_window_continuation
                .replace(continuation)
                .is_none(),
            "openWindow continuation must be attached once"
        );
        self
    }

    /// Attaches the renderer-owned activation decision made immediately before
    /// creating this new auxiliary browsing context.
    pub(crate) fn with_creation_user_activation(
        mut self,
        activation: RendererPopupCreationUserActivation,
    ) -> Self {
        assert!(
            self.resolved_target_page.is_none(),
            "existing popup targets must not carry a creation activation transaction"
        );
        self.creation_user_activation = Some(Box::new(activation));
        self
    }

    /// Attaches the creator-resolved network, initial-empty-Document, and
    /// destination-Document referrers for this exact activation.
    pub fn with_navigation_referrers(
        mut self,
        navigation_referrer: String,
        initial_document_referrer: String,
        document_referrer: String,
    ) -> Self {
        self.referrers = Some(Box::new(RendererPopupNavigationReferrers {
            navigation: navigation_referrer,
            initial_document: initial_document_referrer,
            document: document_referrer,
        }));
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
        &self.requested_url
    }

    pub fn has_destination_navigation(&self) -> bool {
        self.destination_request.is_some()
    }

    pub fn request_method(&self) -> Option<&str> {
        self.destination_request
            .as_deref()
            .map(RendererTopLevelNavigationRequest::request_method)
    }

    pub fn request_body(&self) -> Option<&[u8]> {
        self.destination_request
            .as_deref()
            .and_then(RendererTopLevelNavigationRequest::request_body)
    }

    pub fn request_headers(&self) -> Option<&[(String, String)]> {
        self.destination_request
            .as_deref()
            .map(RendererTopLevelNavigationRequest::request_headers)
    }

    pub fn browser_navigation_kind(&self) -> Option<moli_fetch::BrowserNavigationRequestKind> {
        self.destination_request
            .as_deref()
            .map(RendererTopLevelNavigationRequest::browser_navigation_kind)
    }

    /// Returns the exact causal source retained by the destination request.
    ///
    /// Browser-context producers deliberately return a source without a
    /// Window/Document owner. Keeping this accessor on the activation lets
    /// protocol and regression code verify that those producers did not get
    /// projected onto whichever root Window happened to host the handoff.
    pub fn navigation_source(&self) -> Option<&super::RendererTopLevelNavigationSource> {
        self.destination_request
            .as_deref()
            .and_then(RendererTopLevelNavigationRequest::source)
    }

    pub fn target_name(&self) -> &str {
        &self.target_name
    }

    pub fn navigation_referrer(&self) -> Option<&str> {
        self.referrers
            .as_deref()
            .map(|referrers| referrers.navigation.as_str())
    }

    pub fn initial_document_referrer(&self) -> Option<&str> {
        self.referrers
            .as_deref()
            .map(|referrers| referrers.initial_document.as_str())
    }

    pub fn document_referrer(&self) -> Option<&str> {
        self.referrers
            .as_deref()
            .map(|referrers| referrers.document.as_str())
    }

    pub fn pending_auxiliary_page(&self) -> Option<RendererPendingAuxiliaryPage> {
        self.pending_auxiliary_page
    }

    pub fn resolved_target_page(&self) -> Option<RendererResolvedPopupTarget> {
        self.resolved_target_page
    }

    pub fn new_target_disposition(&self) -> Option<RendererPopupNewTargetDisposition> {
        self.new_target_disposition
    }

    pub fn creation_had_transient_user_activation(&self) -> Option<bool> {
        self.creation_user_activation
            .as_deref()
            .copied()
            .map(RendererPopupCreationUserActivation::user_gesture)
    }

    pub fn creation_consumed_transient_user_activation(&self) -> Option<bool> {
        self.creation_user_activation
            .as_deref()
            .copied()
            .map(RendererPopupCreationUserActivation::consumed)
    }

    pub fn service_worker_clients_open_window_continuation(
        &self,
    ) -> Option<&RendererServiceWorkerClientsOpenWindowContinuation> {
        self.service_worker_clients_open_window_continuation
            .as_ref()
    }

    #[allow(clippy::type_complexity)]
    pub fn into_parts(
        self,
    ) -> (
        RendererPopupActivationSource,
        RendererPopupDisposition,
        Option<u64>,
        String,
        Option<RendererTopLevelNavigationRequest>,
        bool,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<RendererPendingAuxiliaryPage>,
        Option<RendererResolvedPopupTarget>,
        Option<RendererPopupNewTargetDisposition>,
        Option<RendererAuxiliaryBrowsingContextPolicy>,
        Option<RendererServiceWorkerClientsOpenWindowContinuation>,
        Option<SharedWebStorageStore>,
        Option<moli_storage_key::MoliStorageKey>,
    ) {
        let (navigation_referrer, initial_document_referrer, document_referrer) = self
            .referrers
            .map(|referrers| {
                (
                    Some(referrers.navigation),
                    Some(referrers.initial_document),
                    Some(referrers.document),
                )
            })
            .unwrap_or((None, None, None));
        (
            self.source,
            self.disposition,
            self.popup_id,
            self.requested_url,
            self.destination_request.map(|request| *request),
            self.reports_requested_url_without_destination,
            self.target_name,
            navigation_referrer,
            initial_document_referrer,
            document_referrer,
            self.pending_auxiliary_page,
            self.resolved_target_page,
            self.new_target_disposition,
            self.auxiliary_browsing_context_policy.map(|policy| *policy),
            self.service_worker_clients_open_window_continuation,
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
            && self.requested_url == other.requested_url
            && self.destination_request == other.destination_request
            && self.reports_requested_url_without_destination
                == other.reports_requested_url_without_destination
            && self.target_name == other.target_name
            && self.referrers == other.referrers
            && self.pending_auxiliary_page == other.pending_auxiliary_page
            && self.resolved_target_page == other.resolved_target_page
            && self.new_target_disposition == other.new_target_disposition
            && self.creation_user_activation == other.creation_user_activation
            && self.auxiliary_browsing_context_policy == other.auxiliary_browsing_context_policy
            && self.service_worker_clients_open_window_continuation
                == other.service_worker_clients_open_window_continuation
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coop_response_is_blocked_by_response_or_inherited_sandbox_before_commit() {
        let response_url = url::Url::parse("https://example.test/popup").expect("valid URL");
        let coop = (
            "Cross-Origin-Opener-Policy".to_owned(),
            "same-origin".to_owned(),
        );
        let response_sandbox = (
            "Content-Security-Policy".to_owned(),
            "sandbox allow-popups allow-scripts allow-same-origin".to_owned(),
        );
        assert_eq!(
            RendererMainDocumentResponseBlock::sanitize(
                None,
                false,
                &response_url,
                &[coop.clone(), response_sandbox],
            ),
            Some(RendererMainDocumentResponseBlock::CrossOriginOpenerPolicySandboxedNavigation)
        );

        let inherited =
            RendererAuxiliaryBrowsingContextPolicy::from_sandbox(DocumentSandboxPolicy {
                sandboxes_navigation: true,
                ..DocumentSandboxPolicy::default()
            });
        assert_eq!(
            RendererMainDocumentResponseBlock::sanitize(
                Some(inherited),
                false,
                &response_url,
                &[coop],
            ),
            Some(RendererMainDocumentResponseBlock::CrossOriginOpenerPolicySandboxedNavigation)
        );
    }

    #[test]
    fn report_only_coop_and_bypassed_response_csp_do_not_block() {
        let response_url = url::Url::parse("https://example.test/popup").expect("valid URL");
        let sandbox = (
            "Content-Security-Policy".to_owned(),
            "sandbox allow-popups allow-scripts".to_owned(),
        );
        assert_eq!(
            RendererMainDocumentResponseBlock::sanitize(
                None,
                false,
                &response_url,
                &[
                    (
                        "Cross-Origin-Opener-Policy-Report-Only".to_owned(),
                        "same-origin".to_owned(),
                    ),
                    sandbox.clone(),
                ],
            ),
            None
        );
        assert_eq!(
            RendererMainDocumentResponseBlock::sanitize(
                None,
                true,
                &response_url,
                &[
                    (
                        "Cross-Origin-Opener-Policy".to_owned(),
                        "same-origin".to_owned(),
                    ),
                    sandbox,
                ],
            ),
            None
        );
    }
}
