//! Browser navigation state, private in the current residence until Commit 24b.
//! Protocol loader correlation and renderer output binding stay in TargetPageSlot.

use crate::browser::{DocumentId, NavigationId, WebContentsId};

mod history;
pub use history::PageNavigationHistoryEntry;
pub use history::{HistoryTraversalDestination, ResolvedHistoryTraversal};
use history::{NavigationHistoryState, PendingNavigationHistoryUpdate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialDocumentCreator {
    web_contents_id: WebContentsId,
    security_origin: String,
    secure_context_type: String,
}

impl InitialDocumentCreator {
    pub fn new(
        web_contents_id: WebContentsId,
        security_origin: String,
        secure_context_type: String,
    ) -> Self {
        Self {
            web_contents_id,
            security_origin,
            secure_context_type,
        }
    }

    pub fn web_contents_id(&self) -> WebContentsId {
        self.web_contents_id
    }
    pub fn security_origin(&self) -> &str {
        &self.security_origin
    }
    pub fn secure_context_type(&self) -> &str {
        &self.secure_context_type
    }
}

#[derive(Debug)]
enum InitialDocumentLifecycle {
    Unmaterialized,
    Building(super::InitialDocumentBuildState),
    Materialized,
    Exited,
}

/// Browser seed and lifecycle; no Target, loader or mirrored pending state.
#[derive(Debug)]
pub struct InitialDocument {
    initial_url: String,
    creator: Option<InitialDocumentCreator>,
    storage_key: Option<moli_storage_key::MoliStorageKey>,
    lifecycle: InitialDocumentLifecycle,
}

/// Owned diagnostic projection of an initial empty Document.
#[derive(Debug, Clone)]
pub struct InitialDocumentSnapshot {
    initial_url: String,
    creator: Option<InitialDocumentCreator>,
    storage_key: Option<moli_storage_key::MoliStorageKey>,
    materialized: bool,
    exited: bool,
}

impl InitialDocumentSnapshot {
    pub fn initial_url(&self) -> &str {
        &self.initial_url
    }

    pub fn creator(&self) -> Option<&InitialDocumentCreator> {
        self.creator.as_ref()
    }

    pub fn storage_key(&self) -> Option<&moli_storage_key::MoliStorageKey> {
        self.storage_key.as_ref()
    }

    pub fn materialized(&self) -> bool {
        self.materialized
    }

    pub fn exited(&self) -> bool {
        self.exited
    }

    pub fn is_on_initial_empty_document(&self) -> bool {
        !self.exited
    }
}

impl From<&InitialDocument> for InitialDocumentSnapshot {
    fn from(document: &InitialDocument) -> Self {
        Self {
            initial_url: document.initial_url.clone(),
            creator: document.creator.clone(),
            storage_key: document.storage_key.clone(),
            materialized: document.materialized(),
            exited: document.exited(),
        }
    }
}

impl InitialDocument {
    fn new(
        initial_url: String,
        creator: Option<InitialDocumentCreator>,
        storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) -> Self {
        Self {
            initial_url,
            creator,
            storage_key,
            lifecycle: InitialDocumentLifecycle::Unmaterialized,
        }
    }
    pub fn initial_url(&self) -> &str {
        &self.initial_url
    }
    pub fn creator(&self) -> Option<&InitialDocumentCreator> {
        self.creator.as_ref()
    }
    pub fn storage_key(&self) -> Option<&moli_storage_key::MoliStorageKey> {
        self.storage_key.as_ref()
    }
    pub fn materialized(&self) -> bool {
        matches!(self.lifecycle, InitialDocumentLifecycle::Materialized)
    }
    pub fn exited(&self) -> bool {
        matches!(self.lifecycle, InitialDocumentLifecycle::Exited)
    }
    pub fn is_on_initial_empty_document(&self) -> bool {
        !self.exited()
    }
    #[cfg(any(test, feature = "test-support"))]
    fn mark_materialized(&mut self) {
        if !self.exited() {
            self.lifecycle = InitialDocumentLifecycle::Materialized;
        }
    }
    fn mark_exited(&mut self) {
        self.lifecycle = InitialDocumentLifecycle::Exited;
    }
}

/// The browser-owned lifetime of one cross-Document navigation request.
///
/// The exact token remains here from navigation admission until the request
/// either commits or fails. Background navigation additionally keeps this
/// owner alive until its completion is drained. A background result can arrive
/// before Browser materialization/commit; neither transition alone retires the
/// cancellation authority of an operation that is still pending.
#[derive(Debug)]
struct PendingNavigationRequest {
    navigation_id: NavigationId,
    document_id: DocumentId,
    history_update: Option<PendingNavigationHistoryUpdate>,
    paused_interception: Option<super::navigation_interception::PausedNavigationInterception>,
    document_preparation: Option<(
        crate::browser::RendererPageResidenceIdentity,
        moli_fetch::FetchCancelHandle,
    )>,
    cancellation_handles: Vec<moli_fetch::FetchCancelHandle>,
    background_completion_pending: bool,
    committed: bool,
    native_initial_document: bool,
}

impl PendingNavigationRequest {
    fn new(
        navigation_id: NavigationId,
        history_update: Option<PendingNavigationHistoryUpdate>,
    ) -> Self {
        Self {
            navigation_id,
            document_id: DocumentId::allocate(),
            history_update,
            paused_interception: None,
            document_preparation: None,
            cancellation_handles: vec![moli_fetch::FetchCancelHandle::new()],
            background_completion_pending: false,
            committed: false,
            native_initial_document: false,
        }
    }

    fn matches(&self, token: &NavigationId) -> bool {
        self.navigation_id == *token
    }

    fn cancellation_handle(&self) -> moli_fetch::FetchCancelHandle {
        self.cancellation_handles
            .first()
            .expect("a pending navigation request must own cancellation authority")
            .clone()
    }

    fn arm_background_completion(
        &mut self,
        additional_cancellation: Option<moli_fetch::FetchCancelHandle>,
    ) {
        if let Some(cancellation) = additional_cancellation {
            self.cancellation_handles.push(cancellation);
        }
        self.background_completion_pending = true;
    }

    fn retire_without_cancellation(&mut self) {
        self.cancellation_handles.clear();
        self.document_preparation = None;
    }

    fn cancel(&self) {
        for cancellation in &self.cancellation_handles {
            cancellation.cancel();
        }
        if let Some((_, cancellation)) = &self.document_preparation {
            cancellation.cancel();
        }
    }
}

impl Drop for PendingNavigationRequest {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Debug, Default)]
pub struct NavigationController {
    pending_navigation_request: Option<PendingNavigationRequest>,
    failed_navigation: Option<(
        NavigationId,
        DocumentId,
        crate::browser::NavigationFailureReason,
    )>,
    committed_document_navigation: Option<NavigationId>,
    native_responses: Vec<crate::browser::NavigationResponseSnapshot>,
    history: NavigationHistoryState,
    initial_empty_document: Option<InitialDocument>,
}

impl NavigationController {
    pub(in crate::browser) fn attempt_snapshot(
        &self,
        web_contents: crate::browser::WebContentsHandle,
    ) -> Option<crate::browser::NavigationAttempt> {
        use crate::browser::{NavigationAttempt, NavigationRequest};
        if let Some((navigation, document)) = self.pending_document() {
            return Some(NavigationAttempt::Started(NavigationRequest {
                web_contents,
                navigation,
                document,
            }));
        }
        self.failed_navigation
            .map(|(navigation, document, reason)| NavigationAttempt::Failed {
                request: NavigationRequest {
                    web_contents,
                    navigation,
                    document,
                },
                reason,
            })
    }

    pub(in crate::browser) fn cancel_document_navigation(
        &mut self,
        navigation: &NavigationId,
        reason: crate::browser::NavigationFailureReason,
    ) -> bool {
        let Some((pending, document)) = self
            .pending_document()
            .filter(|(pending, _)| pending == navigation)
        else {
            return false;
        };
        self.failed_navigation = Some((pending, document, reason));
        self.pending_navigation_request = None;
        self.retain_native_responses();
        true
    }

    pub(in crate::browser) fn finish_navigation_as_download(&mut self, navigation: NavigationId) {
        let mut pending = self
            .pending_navigation_request
            .take()
            .expect("admitted download navigation");
        assert_eq!(pending.navigation_id, navigation);
        assert!(!pending.committed);
        // Cancel the document candidate, not the response stream moved to the
        // download manager. Dropping that stream now owns transport cancellation.
        pending.cancellation_handle().cancel();
        if let Some((_, cancellation)) = &pending.document_preparation {
            cancellation.cancel();
        }
        pending.retire_without_cancellation();
        self.failed_navigation = Some((
            navigation,
            pending.document_id,
            crate::browser::NavigationFailureReason::Download,
        ));
        for response in &mut self.native_responses {
            if response.request.navigation == navigation {
                response.body = Some(Err(moli_fetch::NET_ERR_ABORTED_ERROR_TEXT.into()));
            }
        }
        self.retain_native_responses();
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn has_paused_request_for_test(&self) -> bool {
        self.pending_navigation_request
            .as_ref()
            .and_then(|request| request.paused_interception.as_ref())
            .is_some_and(|paused| {
                matches!(
                    paused,
                    super::navigation_interception::PausedNavigationInterception::Request(_)
                )
            })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn has_paused_auth_for_test(&self) -> bool {
        self.pending_navigation_request
            .as_ref()
            .and_then(|request| request.paused_interception.as_ref())
            .is_some_and(|paused| {
                matches!(
                    paused,
                    super::navigation_interception::PausedNavigationInterception::Auth(_)
                )
            })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn paused_response_for_test(&self) -> Option<&super::PausedDocumentTransfer> {
        match self
            .pending_navigation_request
            .as_ref()?
            .paused_interception
            .as_ref()?
        {
            super::navigation_interception::PausedNavigationInterception::Response(paused) => {
                Some(&paused.transfer)
            }
            super::navigation_interception::PausedNavigationInterception::Request(_)
            | super::navigation_interception::PausedNavigationInterception::Auth(_)
            | super::navigation_interception::PausedNavigationInterception::Driver(_) => None,
        }
    }

    pub(super) fn pause_request(
        &mut self,
        web_contents: WebContentsId,
        navigation: NavigationId,
        navigation_request: super::navigation_interception::NavigationRequestInterception,
    ) -> Result<super::NavigationInterceptionPermit, String> {
        let pending = self
            .pending_navigation_request
            .as_mut()
            .filter(|pending| {
                pending.navigation_id == navigation
                    && !pending.committed
                    && !pending.cancellation_handle().is_cancelled()
            })
            .ok_or("stale request-stage navigation")?;
        if pending.paused_interception.is_some() {
            return Err("navigation already has a paused interception".to_owned());
        }
        let request = crate::browser::BrowserRequestId::allocate();
        let permit = super::NavigationInterceptionPermit {
            web_contents,
            navigation,
            document: pending.document_id,
            request,
        };
        pending.paused_interception = Some(
            super::navigation_interception::PausedNavigationInterception::Request(Box::new(
                super::navigation_interception::PausedNavigationRequest {
                    request,
                    navigation_request,
                },
            )),
        );
        Ok(permit)
    }

    pub(super) fn take_request(
        &mut self,
        permit: super::NavigationInterceptionPermit,
    ) -> Option<super::navigation_interception::ClaimedNavigationRequest> {
        let pending = self.pending_navigation_request.as_mut().filter(|pending| {
            pending.navigation_id == permit.navigation
                && pending.document_id == permit.document
                && !pending.committed
                && !pending.cancellation_handle().is_cancelled()
        })?;
        if let Some(super::navigation_interception::PausedNavigationInterception::Driver(paused)) =
            pending.paused_interception.as_mut()
        {
            return paused.take_request(permit);
        }
        let super::navigation_interception::PausedNavigationInterception::Request(paused) =
            pending.paused_interception.as_ref()?
        else {
            return None;
        };
        if paused.request != permit.request {
            return None;
        }
        match pending
            .paused_interception
            .take()
            .expect("validated paused navigation request")
        {
            super::navigation_interception::PausedNavigationInterception::Request(paused) => Some(
                super::navigation_interception::ClaimedNavigationRequest::new(
                    permit,
                    paused.navigation_request,
                ),
            ),
            super::navigation_interception::PausedNavigationInterception::Auth(_)
            | super::navigation_interception::PausedNavigationInterception::Response(_)
            | super::navigation_interception::PausedNavigationInterception::Driver(_) => {
                unreachable!("validated request interception")
            }
        }
    }

    pub(super) fn pause_auth_response(
        &mut self,
        response: super::InterceptedNavigationResponse<moli_fetch::RawResponse>,
    ) -> Result<super::NavigationInterceptionPermit, String> {
        let identity = response.identity();
        let pending = self
            .pending_navigation_request
            .as_mut()
            .filter(|pending| {
                pending.navigation_id == identity.navigation
                    && pending.document_id == identity.document
                    && !pending.committed
                    && !identity.is_cancelled()
            })
            .ok_or("stale navigation auth response")?;
        if pending.paused_interception.is_some() {
            return Err("navigation already has a paused interception".to_owned());
        }
        let request = crate::browser::BrowserRequestId::allocate();
        let permit = super::NavigationInterceptionPermit {
            web_contents: identity.web_contents,
            navigation: identity.navigation,
            document: identity.document,
            request,
        };
        pending.paused_interception = Some(
            super::navigation_interception::PausedNavigationInterception::Auth(Box::new(
                super::navigation_interception::PausedNavigationAuth { request, response },
            )),
        );
        Ok(permit)
    }

    pub(super) fn take_auth_response(
        &mut self,
        permit: super::NavigationInterceptionPermit,
    ) -> Option<super::InterceptedNavigationResponse<moli_fetch::RawResponse>> {
        let pending = self.pending_navigation_request.as_mut().filter(|pending| {
            pending.navigation_id == permit.navigation
                && pending.document_id == permit.document
                && !pending.committed
                && !pending.cancellation_handle().is_cancelled()
        })?;
        let super::navigation_interception::PausedNavigationInterception::Auth(paused) =
            pending.paused_interception.as_ref()?
        else {
            return None;
        };
        if paused.request != permit.request || paused.response.identity().is_cancelled() {
            return None;
        }
        Some(
            match pending
                .paused_interception
                .take()
                .expect("validated auth response")
            {
                super::navigation_interception::PausedNavigationInterception::Auth(paused) => {
                    paused.response
                }
                super::navigation_interception::PausedNavigationInterception::Request(_)
                | super::navigation_interception::PausedNavigationInterception::Response(_)
                | super::navigation_interception::PausedNavigationInterception::Driver(_) => {
                    unreachable!("validated auth interception")
                }
            },
        )
    }

    pub(super) fn pause_response(
        &mut self,
        web_contents: WebContentsId,
        navigation: NavigationId,
        transfer: super::PausedDocumentTransfer,
    ) -> Result<super::NavigationInterceptionPermit, String> {
        let pending = self
            .pending_navigation_request
            .as_mut()
            .filter(|pending| {
                pending.navigation_id == navigation
                    && !pending.committed
                    && !pending.cancellation_handle().is_cancelled()
            })
            .ok_or("stale response-stage navigation")?;
        if pending.paused_interception.is_some() {
            return Err("navigation already has a paused interception".to_owned());
        }
        let request = crate::browser::BrowserRequestId::allocate();
        let permit = super::NavigationInterceptionPermit {
            web_contents,
            navigation,
            document: pending.document_id,
            request,
        };
        pending.paused_interception = Some(
            super::navigation_interception::PausedNavigationInterception::Response(Box::new(
                super::navigation_interception::PausedNavigationResponse { request, transfer },
            )),
        );
        Ok(permit)
    }

    pub(super) fn take_response(
        &mut self,
        permit: super::NavigationInterceptionPermit,
    ) -> Option<super::PausedDocumentTransfer> {
        let pending = self.pending_navigation_request.as_mut().filter(|pending| {
            pending.navigation_id == permit.navigation
                && pending.document_id == permit.document
                && !pending.committed
                && !pending.cancellation_handle().is_cancelled()
        })?;
        if let Some(super::navigation_interception::PausedNavigationInterception::Driver(paused)) =
            pending.paused_interception.as_mut()
        {
            return paused.take_response(permit);
        }
        let super::navigation_interception::PausedNavigationInterception::Response(paused) =
            pending.paused_interception.as_ref()?
        else {
            return None;
        };
        if paused.request != permit.request {
            return None;
        }
        Some(
            match pending
                .paused_interception
                .take()
                .expect("validated paused navigation response")
            {
                super::navigation_interception::PausedNavigationInterception::Response(paused) => {
                    paused.transfer
                }
                super::navigation_interception::PausedNavigationInterception::Request(_)
                | super::navigation_interception::PausedNavigationInterception::Auth(_)
                | super::navigation_interception::PausedNavigationInterception::Driver(_) => {
                    unreachable!("validated response interception")
                }
            },
        )
    }

    pub(super) fn restore_response(
        &mut self,
        permit: super::NavigationInterceptionPermit,
        transfer: super::PausedDocumentTransfer,
    ) -> Result<(), Box<super::PausedDocumentTransfer>> {
        let Some(pending) = self.pending_navigation_request.as_mut().filter(|pending| {
            pending.navigation_id == permit.navigation
                && pending.document_id == permit.document
                && !pending.committed
                && !pending.cancellation_handle().is_cancelled()
        }) else {
            return Err(Box::new(transfer));
        };
        if let Some(super::navigation_interception::PausedNavigationInterception::Driver(paused)) =
            pending.paused_interception.as_mut()
        {
            return paused.restore_response(permit, transfer);
        }
        if pending.paused_interception.is_some() {
            return Err(Box::new(transfer));
        }
        pending.paused_interception = Some(
            super::navigation_interception::PausedNavigationInterception::Response(Box::new(
                super::navigation_interception::PausedNavigationResponse {
                    request: permit.request,
                    transfer,
                },
            )),
        );
        Ok(())
    }

    pub(super) fn resolve_history_traversal(
        &self,
        destination: HistoryTraversalDestination,
    ) -> Result<ResolvedHistoryTraversal, &'static str> {
        self.history.resolve_traversal(destination)
    }

    pub fn initial_document_build(&self) -> Option<&super::InitialDocumentBuildState> {
        match &self.initial_empty_document.as_ref()?.lifecycle {
            InitialDocumentLifecycle::Building(build) => Some(build),
            _ => None,
        }
    }

    pub fn admit_initial_document_build(
        &mut self,
        url: &str,
        build: super::InitialDocumentBuildState,
    ) {
        let initial = self
            .initial_empty_document
            .get_or_insert_with(|| InitialDocument::new(url.to_owned(), None, None));
        initial.lifecycle = InitialDocumentLifecycle::Building(build);
    }

    pub fn cancel_initial_document_build(&mut self) {
        if let Some(initial) = self.initial_empty_document.as_mut()
            && matches!(initial.lifecycle, InitialDocumentLifecycle::Building(_))
        {
            initial.lifecycle = InitialDocumentLifecycle::Unmaterialized;
        }
    }

    pub fn take_initial_document_build_for_commit(&mut self) -> super::InitialDocumentBuildState {
        let initial = self
            .initial_empty_document
            .as_mut()
            .expect("admitted initial document");
        let InitialDocumentLifecycle::Building(build) = std::mem::replace(
            &mut initial.lifecycle,
            InitialDocumentLifecycle::Materialized,
        ) else {
            unreachable!("validated initial document build");
        };
        build
    }

    pub fn pending_document(&self) -> Option<(NavigationId, DocumentId)> {
        self.pending_navigation_request
            .as_ref()
            .filter(|request| !request.committed)
            .map(|request| (request.navigation_id, request.document_id))
    }

    pub fn current_document_navigation(&self) -> Option<NavigationId> {
        self.pending_document()
            .map(|(navigation, _)| navigation)
            .or(self.committed_document_navigation)
    }

    pub fn committed_document_navigation(&self) -> Option<NavigationId> {
        self.committed_document_navigation
    }

    pub fn retains_navigation(&self, navigation: NavigationId) -> bool {
        self.pending_navigation_request
            .as_ref()
            .is_some_and(|request| request.matches(&navigation))
            || self.committed_document_navigation == Some(navigation)
    }

    fn retain_native_responses(&mut self) {
        // A download's terminal response remains observable with its latest
        // attempt. Starting another navigation replaces it: still at most two
        // records, with no download body duplicated in the Document cache.
        let pending = self
            .pending_document()
            .map(|(navigation, _)| navigation)
            .or_else(|| {
                self.failed_navigation.and_then(|(navigation, _, reason)| {
                    (reason == crate::browser::NavigationFailureReason::Download)
                        .then_some(navigation)
                })
            });
        let committed = self.committed_document_navigation;
        self.native_responses.retain(|response| {
            Some(response.request.navigation) == pending
                || Some(response.request.navigation) == committed
        });
    }

    pub(in crate::browser) fn response_snapshots(
        &self,
    ) -> Vec<crate::browser::NavigationResponseSnapshot> {
        self.native_responses.clone()
    }

    pub(in crate::browser) fn record_native_response(
        &mut self,
        response: crate::browser::NavigationResponseSnapshot,
    ) -> bool {
        if self.pending_document() != Some((response.request.navigation, response.request.document))
            || self
                .native_responses
                .iter()
                .any(|current| current.request == response.request)
        {
            return false;
        }
        self.native_responses.push(response);
        true
    }

    pub(in crate::browser) fn complete_native_response(
        &mut self,
        request: crate::browser::NavigationRequest,
        body: Result<crate::browser::CapturedBody, String>,
    ) -> bool {
        let Some(response) = self
            .native_responses
            .iter_mut()
            .find(|response| response.request == request && response.body.is_none())
        else {
            return false;
        };
        response.body = Some(body);
        true
    }

    pub(super) fn start_document_navigation(&mut self) -> NavigationId {
        self.cancel_initial_document_build();
        self.failed_navigation = None;
        let navigation = NavigationId::allocate();
        // The preflight intent moves into this request at Start. Supersession
        // and cancellation drop only that request's intent; a late completion
        // cannot clear or inherit a newer navigation's reload/traversal.
        self.pending_navigation_request = Some(PendingNavigationRequest::new(
            navigation,
            self.history.take_pending_update(),
        ));
        self.retain_native_responses();
        navigation
    }

    pub(super) fn commit_pending_document_navigation_if_matches(
        &mut self,
        navigation: &NavigationId,
    ) -> bool {
        let Some(request) = self
            .pending_navigation_request
            .as_mut()
            .filter(|request| request.matches(navigation) && !request.committed)
        else {
            return false;
        };
        self.committed_document_navigation = Some(*navigation);
        self.failed_navigation = None;
        request.committed = true;
        if !request.background_completion_pending {
            request.retire_without_cancellation();
            self.pending_navigation_request = None;
        }
        self.mark_initial_empty_document_exited();
        self.retain_native_responses();
        true
    }

    pub(super) fn clear_document_navigation_state(&mut self) {
        self.cancel_initial_document_build();
        if let Some((navigation, document)) = self.pending_document() {
            self.failed_navigation = Some((
                navigation,
                document,
                crate::browser::NavigationFailureReason::Canceled,
            ));
        }
        self.pending_navigation_request = None;
        self.committed_document_navigation = None;
        self.native_responses.clear();
        self.history.clear_pending_update();
    }

    pub fn initial_empty_document_pending_cross_document_navigation(&self) -> bool {
        self.is_on_initial_empty_document() == Some(true) && self.has_pending_document_navigation()
    }

    pub(super) fn clear_navigation_history(&mut self) {
        self.history.clear();
    }

    pub fn is_default(&self) -> bool {
        self.pending_navigation_request.is_none()
            && self.committed_document_navigation.is_none()
            && self.initial_empty_document.is_none()
            && self.history == NavigationHistoryState::default()
    }

    pub fn current_url(&self) -> Option<&str> {
        self.history.current_url()
    }

    pub fn document_navigation_cancellation_handle(
        &self,
        token: &NavigationId,
    ) -> Option<moli_fetch::FetchCancelHandle> {
        self.pending_navigation_request
            .as_ref()
            .filter(|request| request.matches(token) && !request.committed)
            .map(PendingNavigationRequest::cancellation_handle)
    }

    pub(super) fn accepts_interception_permit(
        &self,
        permit: super::NavigationInterceptionPermit,
    ) -> bool {
        self.pending_navigation_request
            .as_ref()
            .is_some_and(|pending| {
                pending.navigation_id == permit.navigation
                    && pending.document_id == permit.document
                    && !pending.committed
                    && !pending.cancellation_handle().is_cancelled()
            })
    }

    pub(super) fn admit_document_load(
        &mut self,
        navigation: NavigationId,
        renderer: crate::browser::RendererPageResidenceIdentity,
        cancellation: moli_fetch::FetchCancelHandle,
        request_cancellation: moli_fetch::FetchCancelHandle,
    ) -> bool {
        let Some(pending) = self.pending_navigation_request.as_mut().filter(|pending| {
            pending.matches(&navigation)
                && !pending.committed
                && !pending.cancellation_handle().is_cancelled()
        }) else {
            return false;
        };
        if let Some((_, previous)) = pending
            .document_preparation
            .replace((renderer, cancellation))
        {
            previous.cancel();
        }
        // Transport consumers can cancel a response (for example to replace it
        // with synthetic content), but cannot cancel the Browser navigation.
        // The navigation still revokes every admitted transport when retired.
        pending.cancellation_handles.push(request_cancellation);
        true
    }

    pub fn accepts_document_preparation(
        &self,
        navigation: NavigationId,
        renderer: crate::browser::RendererPageResidenceIdentity,
    ) -> bool {
        self.pending_navigation_request
            .as_ref()
            .is_some_and(|pending| {
                pending.matches(&navigation)
                    && !pending.committed
                    && !pending.cancellation_handle().is_cancelled()
                    && pending.document_preparation.as_ref().is_some_and(
                        |(current, cancellation)| {
                            *current == renderer && !cancellation.is_cancelled()
                        },
                    )
            })
    }

    pub(super) fn arm_background_navigation_completion(
        &mut self,
        token: &NavigationId,
        additional_cancellation: Option<moli_fetch::FetchCancelHandle>,
    ) -> bool {
        let Some(request) = self.pending_navigation_request.as_mut().filter(|request| {
            request.matches(token) && !request.committed && !request.background_completion_pending
        }) else {
            if let Some(cancellation) = additional_cancellation {
                cancellation.cancel();
            }
            return false;
        };
        request.arm_background_completion(additional_cancellation);
        true
    }

    pub(super) fn settle_background_navigation_completion(&mut self, token: &NavigationId) -> bool {
        let Some(request) = self
            .pending_navigation_request
            .as_mut()
            .filter(|request| request.matches(token) && request.background_completion_pending)
        else {
            return false;
        };
        request.background_completion_pending = false;
        if request.committed {
            request.retire_without_cancellation();
            self.pending_navigation_request = None;
        }
        true
    }

    pub fn has_inflight_background_navigation(&self) -> bool {
        self.pending_navigation_request
            .as_ref()
            .is_some_and(|request| request.background_completion_pending)
    }

    pub fn accepts_pending_document_navigation_event(&self, token: &NavigationId) -> bool {
        self.pending_navigation_request
            .as_ref()
            .is_some_and(|request| request.matches(token) && !request.committed)
    }

    pub fn accepts_document_body_completion_event(&self, token: &NavigationId) -> bool {
        match self.pending_navigation_request.as_ref() {
            Some(pending) => pending.matches(token),
            None => self.committed_document_navigation.as_ref() == Some(token),
        }
    }

    pub fn has_pending_document_navigation(&self) -> bool {
        self.pending_navigation_request
            .as_ref()
            .is_some_and(|request| !request.committed)
    }

    pub(super) fn begin_initial_empty_document(
        &mut self,
        initial_url: String,
        creator: Option<InitialDocumentCreator>,
        storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) {
        if !is_initial_empty_document_url(&initial_url) {
            self.initial_empty_document = None;
            return;
        }
        if self.history.is_empty() {
            let entry = PageNavigationHistoryEntry {
                id: self.history.allocate_entry_id(),
                url: initial_url.clone(),
                user_typed_url: initial_url.clone(),
                title: String::new(),
                transition_type: "auto_toplevel".to_owned(),
                document_sequence_number: None,
            };
            self.history.seed_entry(entry);
        }
        self.initial_empty_document = Some(InitialDocument::new(initial_url, creator, storage_key));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn mark_initial_empty_document_materialized(&mut self) {
        if let Some(state) = self.initial_empty_document.as_mut() {
            state.mark_materialized();
        }
    }

    pub(super) fn mark_initial_empty_document_exited(&mut self) {
        if let Some(state) = self.initial_empty_document.as_mut() {
            state.mark_exited();
        }
    }

    pub fn initial_empty_document_state(&self) -> Option<&InitialDocument> {
        self.initial_empty_document.as_ref()
    }

    pub fn initial_empty_document_url_if_current(&self) -> Option<&str> {
        self.initial_empty_document_state()
            .filter(|state| state.is_on_initial_empty_document())
            .map(InitialDocument::initial_url)
    }

    pub fn initial_empty_document_storage_key_if_current(
        &self,
    ) -> Option<&moli_storage_key::MoliStorageKey> {
        self.initial_empty_document_state()
            .filter(|state| state.is_on_initial_empty_document())
            .and_then(InitialDocument::storage_key)
    }

    pub fn is_on_initial_empty_document(&self) -> Option<bool> {
        self.initial_empty_document_state()
            .map(InitialDocument::is_on_initial_empty_document)
    }

    pub fn can_install_current_initial_empty_document_page(&self) -> bool {
        (!self.has_pending_document_navigation() || self.has_native_initial_document())
            && self
                .initial_empty_document_state()
                .is_none_or(InitialDocument::is_on_initial_empty_document)
    }

    pub(in crate::browser) fn has_native_initial_document(&self) -> bool {
        self.pending_navigation_request
            .as_ref()
            .is_some_and(|request| request.native_initial_document)
    }

    pub(in crate::browser) fn set_native_initial_document(
        &mut self,
        navigation: NavigationId,
        initial: bool,
    ) -> Result<(), String> {
        let request = self
            .pending_navigation_request
            .as_mut()
            .filter(|request| request.matches(&navigation) && !request.committed)
            .ok_or("native navigation is no longer pending")?;
        request.native_initial_document = initial;
        Ok(())
    }

    pub(in crate::browser) fn pause_driver(
        &mut self,
        web_contents: WebContentsId,
        navigation: NavigationId,
        stage: crate::browser::NavigationDecisionStage,
    ) -> Result<tokio::sync::oneshot::Receiver<crate::browser::NavigationDecision>, String> {
        self.install_driver_decision(web_contents, navigation, move |permit, sender| {
            crate::browser::navigation_decision::PendingNavigationDecision::new(
                permit, stage, sender,
            )
        })
    }

    pub(in crate::browser) fn pause_driver_response(
        &mut self,
        web_contents: WebContentsId,
        navigation: NavigationId,
        stage: crate::browser::NavigationDecisionStage,
        transfer: super::PausedDocumentTransfer,
    ) -> Result<tokio::sync::oneshot::Receiver<crate::browser::NavigationDecision>, String> {
        self.install_driver_decision(web_contents, navigation, move |permit, sender| {
            crate::browser::navigation_decision::PendingNavigationDecision::with_response(
                permit, stage, sender, transfer,
            )
        })
    }

    fn install_driver_decision(
        &mut self,
        web_contents: WebContentsId,
        navigation: NavigationId,
        build: impl FnOnce(
            super::NavigationInterceptionPermit,
            tokio::sync::oneshot::Sender<crate::browser::NavigationDecision>,
        ) -> Result<
            crate::browser::navigation_decision::PendingNavigationDecision,
            String,
        >,
    ) -> Result<tokio::sync::oneshot::Receiver<crate::browser::NavigationDecision>, String> {
        let pending = self
            .pending_navigation_request
            .as_mut()
            .filter(|pending| pending.matches(&navigation) && !pending.committed)
            .ok_or("native navigation is no longer pending")?;
        if pending.paused_interception.is_some() {
            return Err("navigation already has a paused decision".into());
        }
        let permit = super::NavigationInterceptionPermit {
            web_contents,
            navigation,
            document: pending.document_id,
            request: crate::browser::BrowserRequestId::allocate(),
        };
        let (completion, result) = tokio::sync::oneshot::channel();
        pending.paused_interception = Some(
            super::navigation_interception::PausedNavigationInterception::Driver(Box::new(build(
                permit, completion,
            )?)),
        );
        Ok(result)
    }

    pub(in crate::browser) fn driver_decision(
        &self,
    ) -> Option<crate::browser::NavigationDecisionSnapshot> {
        let paused = self
            .pending_navigation_request
            .as_ref()?
            .paused_interception
            .as_ref()?;
        let super::navigation_interception::PausedNavigationInterception::Driver(paused) = paused
        else {
            return None;
        };
        paused.snapshot()
    }

    pub(in crate::browser) fn resolve_driver_decision(
        &mut self,
        permit: super::NavigationInterceptionPermit,
        decision: crate::browser::NavigationDecision,
    ) -> bool {
        let Some(super::navigation_interception::PausedNavigationInterception::Driver(paused)) =
            self.pending_navigation_request
                .as_ref()
                .and_then(|request| request.paused_interception.as_ref())
        else {
            return false;
        };
        if !paused.accepts(permit, &decision) {
            return false;
        }
        let Some(super::navigation_interception::PausedNavigationInterception::Driver(paused)) =
            self.pending_navigation_request
                .as_mut()
                .unwrap()
                .paused_interception
                .take()
        else {
            unreachable!("validated native decision")
        };
        paused.resolve(decision)
    }

    pub(in crate::browser) fn finish_driver_decision(
        &mut self,
        permit: super::NavigationInterceptionPermit,
    ) -> bool {
        let Some(pending) = self.pending_navigation_request.as_mut().filter(|pending| {
            pending.navigation_id == permit.navigation
                && pending.document_id == permit.document
                && !pending.committed
                && !pending.cancellation_handle().is_cancelled()
        }) else {
            return false;
        };
        match pending.paused_interception.as_ref() {
            None => true,
            Some(super::navigation_interception::PausedNavigationInterception::Driver(paused))
                if paused.permit() == permit =>
            {
                pending.paused_interception.take();
                true
            }
            _ => false,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn has_materialized_current_initial_empty_document(&self) -> bool {
        self.initial_empty_document_state()
            .is_some_and(|state| state.is_on_initial_empty_document() && state.materialized())
    }

    fn navigation_history_entry_for_page_snapshot(
        &mut self,
        page_snapshot: (String, String),
    ) -> PageNavigationHistoryEntry {
        let (url, title) = page_snapshot;
        PageNavigationHistoryEntry {
            id: self.history.allocate_entry_id(),
            user_typed_url: url.clone(),
            url,
            title,
            transition_type: "typed".to_owned(),
            document_sequence_number: None,
        }
    }

    pub(super) fn seed_document_history(&mut self, page_snapshot: (String, String)) {
        if !self.history.is_empty() {
            return;
        }
        let entry = self.navigation_history_entry_for_page_snapshot(page_snapshot);
        self.history.seed_entry(entry);
    }

    pub(super) fn refresh_current_navigation_history_title(&mut self, title: String) -> bool {
        self.history.refresh_current_entry_title(title)
    }

    pub(super) fn current_history_title(&self) -> Option<&str> {
        self.history.current_title()
    }

    pub(super) fn mark_next_navigation_history_replace_current(&mut self) {
        self.history.mark_replace_current();
    }

    pub(super) fn mark_next_navigation_history_replace_initial_empty_document(&mut self) {
        self.history.mark_replace_initial_empty_document();
    }

    pub(super) fn mark_next_navigation_history_traverse_to_entry(&mut self, entry_id: i32) {
        self.history.mark_traverse_to_entry(entry_id);
    }

    pub fn navigation_history_entry_url(&self, entry_id: i32) -> Option<String> {
        self.history.entry_url(entry_id)
    }

    pub fn navigation_history_snapshot(&self) -> (usize, Vec<PageNavigationHistoryEntry>) {
        self.history.snapshot()
    }

    pub(super) fn reset_navigation_history(&mut self) -> bool {
        self.can_reset_navigation_history() && self.history.prune_all_but_current()
    }

    pub(super) fn can_reset_navigation_history(&self) -> bool {
        !matches!(
            self.pending_navigation_request
                .as_ref()
                .and_then(|request| request.history_update),
            Some(PendingNavigationHistoryUpdate::TraverseToEntry(_))
        ) && self.history.can_prune_all_but_current()
    }

    pub(super) fn record_loaded_page_navigation_history(
        &mut self,
        page_snapshot: (String, String),
    ) {
        let entry = self.navigation_history_entry_for_page_snapshot(page_snapshot);
        let update = self
            .pending_navigation_request
            .as_mut()
            .expect("admitted navigation owns the history commit")
            .history_update
            .take();
        self.history.record_loaded_entry(entry, update);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn record_navigation_history_for_test(&mut self, snapshot: (String, String)) {
        let entry = self.navigation_history_entry_for_page_snapshot(snapshot);
        let update = self.history.take_pending_update();
        self.history.record_loaded_entry(entry, update);
    }

    pub(super) fn record_same_document_navigation_history(
        &mut self,
        url: String,
        title: String,
        history_update: crate::page::SameDocumentHistoryUpdate,
    ) -> bool {
        self.history
            .record_same_document_update(url, title, history_update)
    }
}

impl super::WebContents {
    pub(in crate::browser) fn navigation_mut(&mut self) -> &mut NavigationController {
        &mut self.navigation
    }

    pub fn navigation(&self) -> &NavigationController {
        &self.navigation
    }

    pub fn start_document_navigation(&mut self) -> NavigationId {
        self.navigation.start_document_navigation()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn commit_pending_document_navigation_if_matches(&mut self, token: &NavigationId) -> bool {
        self.navigation
            .commit_pending_document_navigation_if_matches(token)
    }

    pub(in crate::browser) fn cancel_document_navigation(
        &mut self,
        navigation: &NavigationId,
        reason: crate::browser::NavigationFailureReason,
    ) -> bool {
        self.navigation
            .cancel_document_navigation(navigation, reason)
    }

    pub fn clear_document_navigation_state(&mut self) {
        self.navigation.clear_document_navigation_state()
    }

    pub fn arm_background_navigation_completion(
        &mut self,
        token: &NavigationId,
        additional_cancellation: Option<moli_fetch::FetchCancelHandle>,
    ) -> bool {
        self.navigation
            .arm_background_navigation_completion(token, additional_cancellation)
    }

    pub fn settle_background_navigation_completion(&mut self, token: &NavigationId) -> bool {
        self.navigation
            .settle_background_navigation_completion(token)
    }

    pub fn begin_initial_empty_document(
        &mut self,
        initial_url: String,
        creator: Option<InitialDocumentCreator>,
        storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) {
        self.navigation
            .begin_initial_empty_document(initial_url, creator, storage_key)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn mark_initial_empty_document_materialized(&mut self) {
        self.navigation.mark_initial_empty_document_materialized()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn mark_initial_empty_document_exited(&mut self) {
        self.navigation.mark_initial_empty_document_exited()
    }

    pub fn mark_next_navigation_history_replace_current(&mut self) {
        self.navigation
            .mark_next_navigation_history_replace_current()
    }

    pub fn mark_next_navigation_history_replace_initial_empty_document(&mut self) {
        self.navigation
            .mark_next_navigation_history_replace_initial_empty_document()
    }

    pub fn mark_next_navigation_history_traverse_to_entry(&mut self, entry_id: i32) {
        self.navigation
            .mark_next_navigation_history_traverse_to_entry(entry_id)
    }
}

fn is_initial_empty_document_url(raw_url: &str) -> bool {
    url::Url::parse(raw_url)
        .ok()
        .as_ref()
        .is_some_and(moli_url::is_about_blank)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_response_snapshots_retire_with_exact_pending_and_committed_navigations() {
        use crate::browser::{
            BrowserContextId, CapturedBody, NavigationRequest, NavigationResponseSnapshot,
            WebContentsHandle,
        };
        let contents =
            WebContentsHandle::new(BrowserContextId::allocate(), WebContentsId::allocate());
        let mut controller = NavigationController::default();
        let response = |controller: &NavigationController| {
            let (navigation, document) = controller.pending_document().unwrap();
            NavigationResponseSnapshot {
                request: NavigationRequest {
                    web_contents: contents,
                    navigation,
                    document,
                },
                response: moli_fetch::ResponseHead {
                    final_url: url::Url::parse("https://native.example/").unwrap(),
                    status: 200,
                    headers: Vec::new(),
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                },
                observations: Default::default(),
                body: None,
            }
        };
        let first = controller.start_document_navigation();
        let first_response = response(&controller);
        assert!(controller.record_native_response(first_response.clone()));
        assert!(controller.commit_pending_document_navigation_if_matches(&first));
        controller.start_document_navigation();
        let superseded = response(&controller);
        assert!(controller.record_native_response(superseded.clone()));
        assert_eq!(controller.response_snapshots().len(), 2);
        let replacement = controller.start_document_navigation();
        assert_eq!(controller.response_snapshots().len(), 1);
        assert!(!controller.complete_native_response(
            superseded.request,
            Ok(CapturedBody::from_string("stale".into()))
        ));
        assert!(controller.complete_native_response(
            first_response.request,
            Ok(CapturedBody::from_string("current".into()))
        ));
        assert!(
            !controller.complete_native_response(first_response.request, Err("duplicate".into()))
        );
        assert!(controller.record_native_response(response(&controller)));
        assert!(controller.commit_pending_document_navigation_if_matches(&replacement));
        assert_eq!(controller.response_snapshots().len(), 1);
        assert!(!controller.complete_native_response(first_response.request, Err("late".into())));
        controller.clear_document_navigation_state();
        assert!(controller.response_snapshots().is_empty());
    }
    use crate::page::SameDocumentHistoryUpdate;

    #[test]
    fn dropping_claimed_native_request_releases_its_exact_decision() {
        use crate::browser::{NavigationDecision, NavigationDecisionStage};
        let mut owner = NavigationController::default();
        let navigation = owner.start_document_navigation();
        let mut result = owner
            .pause_driver(
                WebContentsId::allocate(),
                navigation,
                NavigationDecisionStage::Request {
                    url: url::Url::parse("data:text/html,claim-drop").unwrap(),
                    method: "GET".into(),
                    headers: Vec::new(),
                    opening: std::sync::Weak::new(),
                },
            )
            .unwrap();
        let permit = owner.driver_decision().unwrap().permit;
        let claimed = owner.take_request(permit).unwrap();
        assert!(owner.take_request(permit).is_none());
        assert!(owner.driver_decision().is_none());
        drop(claimed);
        assert!(matches!(
            result.try_recv(),
            Ok(NavigationDecision::Continue)
        ));
        assert!(owner.finish_driver_decision(permit));
        assert!(!owner.resolve_driver_decision(permit, NavigationDecision::Cancel));
    }

    #[test]
    fn superseding_a_claimed_native_request_cancels_it_without_waiting_for_the_claim() {
        use crate::browser::NavigationDecisionStage;
        let mut owner = NavigationController::default();
        let navigation = owner.start_document_navigation();
        let mut result = owner
            .pause_driver(
                WebContentsId::allocate(),
                navigation,
                NavigationDecisionStage::Request {
                    url: url::Url::parse("data:text/html,obsolete-claim").unwrap(),
                    method: "GET".into(),
                    headers: Vec::new(),
                    opening: std::sync::Weak::new(),
                },
            )
            .unwrap();
        let permit = owner.driver_decision().unwrap().permit;
        let claimed = owner.take_request(permit).unwrap();
        let replacement = owner.start_document_navigation();
        assert!(matches!(
            result.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ));
        drop(claimed);
        assert!(!owner.finish_driver_decision(permit));
        assert_eq!(
            owner
                .pending_navigation_request
                .as_ref()
                .unwrap()
                .navigation_id,
            replacement
        );
    }

    #[test]
    fn navigation_history_seed_entry_preserves_pending_update() {
        let mut history = NavigationHistoryState::default();
        history.mark_replace_current();

        let seed_id = history.allocate_entry_id();
        history.seed_entry(PageNavigationHistoryEntry {
            id: seed_id,
            url: "https://example.test/seed".to_owned(),
            user_typed_url: "https://example.test/seed".to_owned(),
            title: "seed".to_owned(),
            transition_type: "typed".to_owned(),
            document_sequence_number: None,
        });

        let reloaded_id = history.allocate_entry_id();
        let update = history.take_pending_update();
        history.record_loaded_entry(
            PageNavigationHistoryEntry {
                id: reloaded_id,
                url: "https://example.test/reloaded".to_owned(),
                user_typed_url: "https://example.test/reloaded".to_owned(),
                title: "reloaded".to_owned(),
                transition_type: "typed".to_owned(),
                document_sequence_number: None,
            },
            update,
        );

        let (current_index, entries) = history.snapshot();
        assert_eq!(current_index, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, "https://example.test/reloaded");
        assert_eq!(entries[0].user_typed_url, "https://example.test/seed");
        assert_eq!(entries[0].transition_type, "reload");
    }

    #[test]
    fn initial_empty_document_seeds_browser_navigation_history_metadata() {
        let mut owner = NavigationController::default();

        owner.begin_initial_empty_document("about:blank".to_owned(), None, None);

        let (current_index, entries) = owner.navigation_history_snapshot();
        assert_eq!(current_index, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, "about:blank");
        assert_eq!(entries[0].user_typed_url, "about:blank");
        assert_eq!(entries[0].transition_type, "auto_toplevel");
    }

    #[test]
    fn direct_target_initial_url_replaces_empty_document_history_entry() {
        let mut owner = NavigationController::default();

        owner.begin_initial_empty_document("about:blank".to_owned(), None, None);
        owner.mark_next_navigation_history_replace_initial_empty_document();
        owner.start_document_navigation();
        owner.record_loaded_page_navigation_history((
            "https://example.test/direct".to_owned(),
            "direct".to_owned(),
        ));

        let (current_index, entries) = owner.navigation_history_snapshot();
        assert_eq!(current_index, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, "https://example.test/direct");
        assert_eq!(entries[0].user_typed_url, "https://example.test/direct");
        assert_eq!(entries[0].title, "direct");
        assert_eq!(entries[0].transition_type, "auto_toplevel");
    }

    #[test]
    fn navigation_history_prune_rejects_only_pending_existing_entry_traversal() {
        let mut history = NavigationHistoryState::default();
        let initial_id = history.allocate_entry_id();
        history.seed_entry(PageNavigationHistoryEntry {
            id: initial_id,
            url: "https://example.test/initial".to_owned(),
            user_typed_url: "https://example.test/initial".to_owned(),
            title: "initial".to_owned(),
            transition_type: "typed".to_owned(),
            document_sequence_number: None,
        });
        assert!(history.record_same_document_update(
            "https://example.test/pushed".to_owned(),
            "pushed".to_owned(),
            SameDocumentHistoryUpdate::Push,
        ));
        let pushed_id = history.snapshot().1[1].id;

        history.mark_replace_current();
        assert!(
            history.can_prune_all_but_current(),
            "a new pending reload/replace entry must survive pruning"
        );
        assert!(history.prune_all_but_current());
        let (current_index, entries) = history.snapshot();
        assert_eq!(current_index, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, pushed_id);

        history.mark_traverse_to_entry(pushed_id);
        assert!(
            !history.can_prune_all_but_current(),
            "pending traversal to an existing history index cannot be pruned"
        );
        assert!(!history.prune_all_but_current());
    }

    #[test]
    fn navigation_history_traversal_reuses_same_document_entries() {
        let mut history = NavigationHistoryState::default();
        let initial_id = history.allocate_entry_id();
        history.seed_entry(PageNavigationHistoryEntry {
            id: initial_id,
            url: "https://example.test/page".to_owned(),
            user_typed_url: "https://example.test/page".to_owned(),
            title: "page".to_owned(),
            transition_type: "typed".to_owned(),
            document_sequence_number: None,
        });
        assert!(history.record_same_document_update(
            "https://example.test/page?state=pushed".to_owned(),
            "page".to_owned(),
            SameDocumentHistoryUpdate::Push,
        ));

        let (_, entries) = history.snapshot();
        let pushed_id = entries[1].id;
        assert_eq!(
            entries[0].document_sequence_number, entries[1].document_sequence_number,
            "pushState entries must retain the current document sequence"
        );
        assert_eq!(entries[1].user_typed_url, "https://example.test/page");
        assert_eq!(entries[1].transition_type, "link");

        assert!(history.record_same_document_update(
            "https://example.test/page".to_owned(),
            "page".to_owned(),
            SameDocumentHistoryUpdate::Traverse { delta: -1 },
        ));
        let (current_index, entries) = history.snapshot();
        assert_eq!(current_index, 0);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, initial_id);
        assert_eq!(entries[1].id, pushed_id);

        assert!(history.record_same_document_update(
            "https://example.test/page?state=pushed".to_owned(),
            "page".to_owned(),
            SameDocumentHistoryUpdate::Traverse { delta: 1 },
        ));
        let (current_index, entries) = history.snapshot();
        assert_eq!(current_index, 1);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].id, pushed_id);
    }

    #[test]
    fn background_result_keeps_cancellation_until_browser_commit_or_retirement() {
        for commit in [false, true] {
            let mut controller = NavigationController::default();
            let navigation = controller.start_document_navigation();
            let cancellation = controller
                .document_navigation_cancellation_handle(&navigation)
                .unwrap();
            assert!(controller.arm_background_navigation_completion(&navigation, None));
            assert!(controller.settle_background_navigation_completion(&navigation));
            let admitted = controller
                .document_navigation_cancellation_handle(&navigation)
                .unwrap();
            assert!(!admitted.is_cancelled());
            assert!(controller.pending_document().is_some());
            if commit {
                assert!(controller.commit_pending_document_navigation_if_matches(&navigation));
            } else {
                assert!(controller.cancel_document_navigation(
                    &navigation,
                    crate::browser::NavigationFailureReason::Canceled
                ));
            }
            assert_eq!(admitted.is_cancelled(), !commit);
            assert_eq!(cancellation.is_cancelled(), !commit);
            assert!(controller.pending_document().is_none());
            assert!(!controller.has_inflight_background_navigation());
        }
    }

    #[test]
    fn browser_commit_keeps_background_transport_live_until_its_completion() {
        let mut controller = NavigationController::default();
        let navigation = controller.start_document_navigation();
        let cancellation = controller
            .document_navigation_cancellation_handle(&navigation)
            .unwrap();
        assert!(controller.arm_background_navigation_completion(&navigation, None));
        assert!(controller.commit_pending_document_navigation_if_matches(&navigation));
        assert!(!cancellation.is_cancelled());
        assert!(controller.has_inflight_background_navigation());
        assert!(controller.settle_background_navigation_completion(&navigation));
        assert!(!cancellation.is_cancelled());
        assert!(!controller.has_inflight_background_navigation());
        drop(controller);
        assert!(!cancellation.is_cancelled());
    }

    #[test]
    fn download_retirement_cancels_only_its_document_candidate() {
        let mut controller = NavigationController::default();
        let navigation = controller.start_document_navigation();
        let cancellation = controller
            .document_navigation_cancellation_handle(&navigation)
            .unwrap();
        let transport = moli_fetch::FetchCancelHandle::new();
        controller
            .pending_navigation_request
            .as_mut()
            .unwrap()
            .cancellation_handles
            .push(transport.clone());
        assert!(controller.arm_background_navigation_completion(&navigation, None));
        controller.finish_navigation_as_download(navigation);
        assert!(cancellation.is_cancelled());
        assert!(!transport.is_cancelled());
        assert!(controller.pending_document().is_none());
        assert!(!controller.has_inflight_background_navigation());
        let replacement = controller.start_document_navigation();
        let replacement_cancellation = controller
            .document_navigation_cancellation_handle(&replacement)
            .unwrap();
        assert!(!controller.cancel_document_navigation(
            &navigation,
            crate::browser::NavigationFailureReason::Canceled
        ));
        assert!(!replacement_cancellation.is_cancelled());
        drop(controller);
        assert!(replacement_cancellation.is_cancelled());
        assert!(!transport.is_cancelled());
    }
}
