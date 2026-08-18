use std::hash::{Hash, Hasher};

use moli_core::page::{
    Page, RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
    RendererDocumentLifecycleIdentity, RendererDocumentLifecycleMilestone,
    RendererDocumentLifecycleSnapshot, RendererDocumentLifecycleWaitOutcome,
    RendererDocumentLifecycleWaiter, RendererDocumentToken, RendererFrameToken,
    RendererLifecycleEpoch, RendererLifecycleEventStamp, RendererLifecycleStartReason,
    RendererLifecycleTerminationStamp, RendererPageCreationArtifacts,
};
use tokio::sync::watch;

use super::document_lifecycle_observer::{
    RendererDocumentLifecycleObservation, RendererDocumentLifecycleObservationPublisher,
    RendererDocumentLifecycleObserver,
};
use super::page_residence_token::{TargetPageResidencePublisher, TargetPageResidenceToken};
use super::{NavigationRequestId, RendererPageResidenceIdentity, TargetPageAttachmentId};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TargetPageAbsenceReason {
    #[default]
    NoTarget,
    InitialDocumentPageBuildPending,
    InitialDocumentPageBuildInProgress,
    NavigationFailed,
    TargetClosed,
    TargetCrashed,
    #[cfg(test)]
    TestFixture,
}

impl TargetPageAbsenceReason {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::NoTarget => "no-target",
            Self::InitialDocumentPageBuildPending => "initial-document-page-build-pending",
            Self::InitialDocumentPageBuildInProgress => "initial-document-page-build-in-progress",
            Self::NavigationFailed => "navigation-failed",
            Self::TargetClosed => "target-closed",
            Self::TargetCrashed => "target-crashed",
            #[cfg(test)]
            Self::TestFixture => "test-fixture",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DocumentNavigationToken {
    pub(crate) target_id: String,
    pub(crate) loader_id: String,
    pub(crate) request_id: NavigationRequestId,
}

impl PartialEq for DocumentNavigationToken {
    fn eq(&self, other: &Self) -> bool {
        self.request_id == other.request_id
            && self.target_id == other.target_id
            && self.loader_id == other.loader_id
    }
}

impl Eq for DocumentNavigationToken {}

impl Hash for DocumentNavigationToken {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.request_id.hash(state);
        self.target_id.hash(state);
        self.loader_id.hash(state);
    }
}

/// The target-owned lifetime of one cross-Document navigation request.
///
/// The exact token remains here from navigation admission until the request
/// either commits or fails. Background navigation additionally keeps this
/// owner alive until its lifecycle completion is drained; transport
/// cancellation is therefore retired by the same exact-token transition as
/// the protocol gate instead of by a scheduler-side mirror.
#[derive(Debug)]
pub(crate) struct PendingNavigationRequest {
    token: DocumentNavigationToken,
    page_attachment_id: TargetPageAttachmentId,
    cancellation_handles: Vec<moli_fetch::FetchCancelHandle>,
    background_completion_pending: bool,
    committed: bool,
}

impl PendingNavigationRequest {
    fn new(token: DocumentNavigationToken) -> Self {
        Self {
            token,
            page_attachment_id: TargetPageAttachmentId::allocate(),
            cancellation_handles: vec![moli_fetch::FetchCancelHandle::new()],
            background_completion_pending: false,
            committed: false,
        }
    }

    fn matches(&self, token: &DocumentNavigationToken) -> bool {
        self.token == *token
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

    fn settle_background_completion(&mut self) {
        self.background_completion_pending = false;
        self.cancellation_handles.clear();
    }

    fn retire_without_cancellation(&mut self) {
        self.cancellation_handles.clear();
    }

    fn cancel(&self) {
        for cancellation in &self.cancellation_handles {
            cancellation.cancel();
        }
    }
}

impl Drop for PendingNavigationRequest {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommittedRendererDocumentBinding {
    pub(crate) renderer_frame: RendererFrameToken,
    pub(crate) renderer_document: RendererDocumentToken,
    pub(crate) renderer_epoch: RendererLifecycleEpoch,
    pub(crate) navigation: Option<DocumentNavigationToken>,
    pub(crate) frame_id: String,
    pub(crate) loader_id: String,
    pub(crate) page_attachment_id: TargetPageAttachmentId,
    pub(crate) document_open_replacement_epoch: Option<RendererLifecycleEpoch>,
}

impl CommittedRendererDocumentBinding {
    pub(crate) fn renderer_document_identity(&self) -> RendererDocumentLifecycleIdentity {
        RendererDocumentLifecycleIdentity {
            frame: self.renderer_frame,
            document: self.renderer_document,
            epoch: self.renderer_epoch,
        }
    }
}

#[derive(Debug, Default)]
struct RendererDocumentLifecycleProtocolState {
    binding: Option<CommittedRendererDocumentBinding>,
    authoritative: RendererDocumentLifecycleProtocolCursor,
    visible: RendererDocumentLifecycleProtocolCursor,
    load_visibility: RendererDocumentLoadVisibility,
}

#[derive(Clone, Copy, Debug, Default)]
struct RendererDocumentLifecycleProtocolCursor {
    snapshot: Option<RendererDocumentLifecycleSnapshot>,
    last_sequence: Option<u64>,
}

impl RendererDocumentLifecycleProtocolCursor {
    fn from_snapshot(snapshot: RendererDocumentLifecycleSnapshot) -> Self {
        Self {
            snapshot: Some(snapshot),
            last_sequence: None,
        }
    }

    fn observe(&mut self, event: RendererDocumentLifecycleEvent) {
        debug_assert!(
            self.last_sequence
                .is_none_or(|sequence| event.sequence > sequence),
            "renderer lifecycle protocol cursors must advance monotonically"
        );
        self.last_sequence = Some(event.sequence);
        let Some(snapshot) = self.snapshot.as_mut() else {
            return;
        };
        apply_renderer_document_lifecycle_event_to_snapshot(snapshot, event);
    }
}

fn apply_renderer_document_lifecycle_event_to_snapshot(
    snapshot: &mut RendererDocumentLifecycleSnapshot,
    event: RendererDocumentLifecycleEvent,
) {
    match event.kind {
        RendererDocumentLifecycleEventKind::Started { .. } => {
            snapshot.frame = event.frame;
            snapshot.document = event.document;
            snapshot.epoch = event.epoch;
            snapshot.started = RendererLifecycleEventStamp {
                sequence: event.sequence,
                timestamp_micros: event.timestamp_micros,
            };
            snapshot.dom_content_loaded = None;
            snapshot.load = None;
            snapshot.terminated = None;
        }
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded,
        ) => {
            snapshot.dom_content_loaded = Some(RendererLifecycleEventStamp {
                sequence: event.sequence,
                timestamp_micros: event.timestamp_micros,
            });
        }
        RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load) => {
            snapshot.load = Some(RendererLifecycleEventStamp {
                sequence: event.sequence,
                timestamp_micros: event.timestamp_micros,
            });
        }
        RendererDocumentLifecycleEventKind::Terminated { reason, .. } => {
            snapshot.terminated = Some(RendererLifecycleTerminationStamp {
                sequence: event.sequence,
                timestamp_micros: event.timestamp_micros,
                reason,
            });
        }
    }
}

#[derive(Debug, Default)]
struct RendererDocumentLoadVisibility {
    barrier_loader_id: Option<String>,
    deferred_tail: Vec<RendererDocumentLifecycleEvent>,
}

#[derive(Debug)]
struct RegisteredRendererDocumentLifecycleWaiter {
    id: RendererDocumentLifecycleWaiterId,
    renderer_document: RendererDocumentToken,
    renderer_epoch: RendererLifecycleEpoch,
    frame_id: String,
    loader_id: String,
    waiter: RendererDocumentLifecycleWaiter,
    observer_publisher: Option<RendererDocumentLifecycleObservationPublisher>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RendererDocumentLifecycleWaiterId(u64);

impl RendererDocumentLifecycleWaiterId {
    #[cfg(test)]
    pub(crate) const fn new_for_test(id: u64) -> Self {
        Self(id)
    }

    fn allocate_next(&mut self) -> Self {
        self.0 = self
            .0
            .checked_add(1)
            .expect("renderer Document lifecycle waiter id overflow");
        *self
    }
}

fn lifecycle_observation_from_wait_outcome(
    outcome: RendererDocumentLifecycleWaitOutcome,
) -> RendererDocumentLifecycleObservation {
    match outcome {
        RendererDocumentLifecycleWaitOutcome::Pending => {
            RendererDocumentLifecycleObservation::Pending
        }
        RendererDocumentLifecycleWaitOutcome::Reached(_) => {
            RendererDocumentLifecycleObservation::Reached
        }
        RendererDocumentLifecycleWaitOutcome::Interrupted(_) => {
            RendererDocumentLifecycleObservation::Interrupted
        }
    }
}

#[derive(Debug)]
struct RootPostLoadObservation {
    binding: CommittedRendererDocumentBinding,
    frame_stopped_loading_pending: bool,
    network_idle_pending: bool,
}

pub type IsolatedWorldDefinition = moli_core::page::RuntimeIsolatedWorldDefinition;
pub type RuntimeBindingDefinition = moli_core::page::RuntimeBindingRegistration;
pub type DocumentStartScript = moli_core::page::DocumentStartScript;

#[derive(Debug, Clone)]
pub(crate) struct InitialDocumentPageBuildWaiter {
    receiver: watch::Receiver<Option<Result<(), String>>>,
}

impl InitialDocumentPageBuildWaiter {
    pub(crate) async fn wait(mut self) -> Result<(), String> {
        loop {
            if let Some(result) = self.receiver.borrow().clone() {
                return result;
            }
            self.receiver
                .changed()
                .await
                .map_err(|_| "InitialDocumentPageBuildCancelled".to_owned())?;
        }
    }
}

/// Exact renderer Page that is allowed to publish while its protocol target
/// has not installed the resulting [`Page`] yet.
///
/// Initial construction and cross-document navigation have different
/// retirement authorities, so the binding records which transition owns it.
/// A later navigation can never inherit an earlier Page reservation merely
/// because both builds used the same target/session.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingRendererPageBinding {
    PageBuild {
        renderer_page: RendererPageResidenceIdentity,
        page_attachment_id: TargetPageAttachmentId,
    },
    InitialDocumentBuild {
        renderer_page: RendererPageResidenceIdentity,
        page_attachment_id: TargetPageAttachmentId,
    },
    DocumentNavigation {
        navigation: DocumentNavigationToken,
        renderer_page: RendererPageResidenceIdentity,
        page_attachment_id: TargetPageAttachmentId,
    },
}

impl PendingRendererPageBinding {
    fn renderer_page(&self) -> RendererPageResidenceIdentity {
        match self {
            Self::PageBuild { renderer_page, .. }
            | Self::InitialDocumentBuild { renderer_page, .. }
            | Self::DocumentNavigation { renderer_page, .. } => *renderer_page,
        }
    }

    fn page_attachment_id(&self) -> TargetPageAttachmentId {
        match self {
            Self::PageBuild {
                page_attachment_id, ..
            }
            | Self::InitialDocumentBuild {
                page_attachment_id, ..
            }
            | Self::DocumentNavigation {
                page_attachment_id, ..
            } => *page_attachment_id,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct TargetPageSlot {
    loaded_page: Option<Page>,
    loaded_page_absence_reason: TargetPageAbsenceReason,
    page_attachment_id: Option<TargetPageAttachmentId>,
    page_residence_publisher: Option<TargetPageResidencePublisher>,
    pending_navigation_request: Option<PendingNavigationRequest>,
    committed_document_navigation: Option<DocumentNavigationToken>,
    renderer_document_lifecycle: RendererDocumentLifecycleProtocolState,
    next_renderer_document_lifecycle_waiter_id: RendererDocumentLifecycleWaiterId,
    renderer_document_lifecycle_waiters: Vec<RegisteredRendererDocumentLifecycleWaiter>,
    root_post_load_observation: Option<RootPostLoadObservation>,
    initial_document_page_build_completion: Option<watch::Sender<Option<Result<(), String>>>>,
    pending_renderer_page: Option<PendingRendererPageBinding>,
}

impl TargetPageSlot {
    pub(crate) fn empty_for_initial_document_page_build() -> Self {
        Self {
            loaded_page: None,
            loaded_page_absence_reason: TargetPageAbsenceReason::InitialDocumentPageBuildPending,
            ..Default::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test_fixture() -> Self {
        Self {
            loaded_page: None,
            loaded_page_absence_reason: TargetPageAbsenceReason::TestFixture,
            ..Default::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_loaded_page_for_test(loaded_page: Page) -> Self {
        Self {
            loaded_page: Some(loaded_page),
            page_attachment_id: Some(TargetPageAttachmentId::allocate()),
            ..Default::default()
        }
    }

    pub(crate) fn loaded_page(&self) -> Option<&Page> {
        self.loaded_page.as_ref()
    }

    pub(crate) fn loaded_page_mut(&mut self) -> Option<&mut Page> {
        self.loaded_page.as_mut()
    }

    pub(crate) fn has_loaded_page(&self) -> bool {
        self.loaded_page.is_some()
    }

    pub(crate) fn loaded_page_absence_reason(&self) -> Option<TargetPageAbsenceReason> {
        self.loaded_page
            .is_none()
            .then_some(self.loaded_page_absence_reason)
    }

    pub(crate) fn mark_loaded_page_absent(&mut self, reason: TargetPageAbsenceReason) {
        if self.loaded_page.is_none() {
            if self.loaded_page_absence_reason
                == TargetPageAbsenceReason::InitialDocumentPageBuildInProgress
                && reason != TargetPageAbsenceReason::InitialDocumentPageBuildInProgress
            {
                self.fail_initial_document_page_build("InitialDocumentPageBuildCancelled".into());
            }
            self.loaded_page_absence_reason = reason;
        }
    }

    pub(crate) fn start_initial_document_page_build(&mut self) {
        if self.loaded_page.is_none() {
            self.loaded_page_absence_reason =
                TargetPageAbsenceReason::InitialDocumentPageBuildInProgress;
        }
        self.pending_renderer_page = None;
        let (sender, _receiver) = watch::channel(None);
        self.initial_document_page_build_completion = Some(sender);
    }

    pub(crate) fn bind_initial_document_page_build_renderer_page(
        &mut self,
        renderer_page: RendererPageResidenceIdentity,
    ) -> bool {
        if self.loaded_page.is_some()
            || self.loaded_page_absence_reason
                != TargetPageAbsenceReason::InitialDocumentPageBuildInProgress
            || self.initial_document_page_build_completion.is_none()
            || self.pending_renderer_page.is_some()
        {
            return false;
        }
        self.pending_renderer_page = Some(PendingRendererPageBinding::InitialDocumentBuild {
            renderer_page,
            page_attachment_id: TargetPageAttachmentId::allocate(),
        });
        true
    }

    pub(crate) fn initial_document_page_build_waiter(
        &self,
    ) -> Option<InitialDocumentPageBuildWaiter> {
        self.initial_document_page_build_completion
            .as_ref()
            .map(|sender| InitialDocumentPageBuildWaiter {
                receiver: sender.subscribe(),
            })
    }

    pub(crate) fn complete_initial_document_page_build(&mut self) {
        if matches!(
            self.pending_renderer_page.as_ref(),
            Some(PendingRendererPageBinding::InitialDocumentBuild { .. })
        ) {
            self.pending_renderer_page = None;
        }
        if let Some(sender) = self.initial_document_page_build_completion.take() {
            let _ = sender.send(Some(Ok(())));
        }
    }

    pub(crate) fn fail_initial_document_page_build(&mut self, message: String) {
        if matches!(
            self.pending_renderer_page.as_ref(),
            Some(PendingRendererPageBinding::InitialDocumentBuild { .. })
        ) {
            self.pending_renderer_page = None;
        }
        if let Some(sender) = self.initial_document_page_build_completion.take() {
            let _ = sender.send(Some(Err(message)));
        }
    }

    pub(crate) fn replace_loaded_page_with_reason(
        &mut self,
        page: Option<Page>,
        absence_reason: TargetPageAbsenceReason,
    ) -> Option<Page> {
        let next_page_attachment_id = page.as_ref().map(|page| {
            let renderer_page = RendererPageResidenceIdentity::from_page(page);
            match self.pending_renderer_page.as_ref() {
                Some(binding) => {
                    assert_eq!(
                        binding.renderer_page(),
                        renderer_page,
                        "installed Page must match its explicit renderer Page reservation"
                    );
                    binding.page_attachment_id()
                }
                None => TargetPageAttachmentId::allocate(),
            }
        });
        if self.loaded_page.is_some() || page.is_some() {
            self.finish_renderer_document_lifecycle_observers(
                RendererDocumentLifecycleObservation::Superseded,
            );
        }
        if page.is_some() {
            self.complete_initial_document_page_build();
            self.loaded_page_absence_reason = TargetPageAbsenceReason::NoTarget;
        } else {
            if self.loaded_page_absence_reason
                == TargetPageAbsenceReason::InitialDocumentPageBuildInProgress
            {
                self.fail_initial_document_page_build("InitialDocumentPageBuildCancelled".into());
            }
            self.loaded_page_absence_reason = absence_reason;
        }
        self.page_attachment_id = next_page_attachment_id;
        self.pending_renderer_page = None;
        let previous = std::mem::replace(&mut self.loaded_page, page);
        self.supersede_page_residence();
        previous
    }

    pub(crate) fn replace_loaded_page(&mut self, page: Option<Page>) -> Option<Page> {
        let Some(page) = page else {
            panic!(
                "replace_loaded_page(None) is not a valid production transition; use clear_loaded_page_with_reason"
            );
        };
        self.replace_loaded_page_with_reason(Some(page), TargetPageAbsenceReason::NoTarget)
    }

    pub(crate) fn page_attachment_id(&self) -> Option<TargetPageAttachmentId> {
        self.page_attachment_id
    }

    pub(crate) fn pending_page_attachment_id(&self) -> Option<TargetPageAttachmentId> {
        self.pending_renderer_page
            .as_ref()
            .map(PendingRendererPageBinding::page_attachment_id)
            .or_else(|| {
                self.pending_navigation_request
                    .as_ref()
                    .filter(|request| !request.committed)
                    .map(|request| request.page_attachment_id)
            })
    }

    pub(crate) fn reserve_renderer_page_attachment(
        &mut self,
        renderer_page: RendererPageResidenceIdentity,
    ) -> TargetPageAttachmentId {
        if let Some(binding) = self.pending_renderer_page.as_ref() {
            if binding.renderer_page() == renderer_page {
                return binding.page_attachment_id();
            }
            // A newly reserved renderer Page supersedes an earlier build that
            // never reached installation. Its old output-owner binding remains
            // harmless because this slot no longer routes that renderer Page.
            self.pending_renderer_page = None;
        }

        if let Some(request) = self
            .pending_navigation_request
            .as_ref()
            .filter(|request| !request.committed)
        {
            let page_attachment_id = request.page_attachment_id;
            self.pending_renderer_page = Some(PendingRendererPageBinding::DocumentNavigation {
                navigation: request.token.clone(),
                renderer_page,
                page_attachment_id,
            });
            return page_attachment_id;
        }

        let page_attachment_id = TargetPageAttachmentId::allocate();
        self.pending_renderer_page = Some(PendingRendererPageBinding::PageBuild {
            renderer_page,
            page_attachment_id,
        });
        page_attachment_id
    }

    pub(crate) fn page_residence_token(&mut self) -> Option<TargetPageResidenceToken> {
        let attachment_id = self.page_attachment_id()?;
        let publisher = self
            .page_residence_publisher
            .get_or_insert_with(|| TargetPageResidencePublisher::new(attachment_id));
        Some(publisher.token())
    }

    fn supersede_page_residence(&mut self) {
        if let Some(publisher) = self.page_residence_publisher.take() {
            publisher.supersede();
        }
    }

    #[cfg(test)]
    pub(crate) fn set_page_attachment_id_for_test(&mut self, raw: u64) -> TargetPageAttachmentId {
        let attachment_id = TargetPageAttachmentId::from_raw_for_test(raw);
        self.install_page_attachment_id_for_test(attachment_id);
        attachment_id
    }

    #[cfg(test)]
    pub(crate) fn replace_page_attachment_id_for_test(&mut self) -> TargetPageAttachmentId {
        let mut attachment_id = TargetPageAttachmentId::allocate();
        while self.page_attachment_id == Some(attachment_id) {
            attachment_id = TargetPageAttachmentId::allocate();
        }
        self.install_page_attachment_id_for_test(attachment_id);
        attachment_id
    }

    #[cfg(test)]
    pub(crate) fn install_page_attachment_id_for_test(
        &mut self,
        attachment_id: TargetPageAttachmentId,
    ) {
        let attachment_changed = self.page_attachment_id != Some(attachment_id);
        self.page_attachment_id = Some(attachment_id);
        if attachment_changed {
            self.finish_renderer_document_lifecycle_observers(
                RendererDocumentLifecycleObservation::Superseded,
            );
            self.supersede_page_residence();
        }
    }

    pub(crate) fn start_document_navigation(
        &mut self,
        target_id: String,
        loader_id: String,
    ) -> DocumentNavigationToken {
        self.finish_renderer_document_lifecycle_observers(
            RendererDocumentLifecycleObservation::Superseded,
        );
        let token = DocumentNavigationToken {
            target_id,
            loader_id,
            request_id: NavigationRequestId::allocate(),
        };
        self.pending_renderer_page = None;
        self.pending_navigation_request = Some(PendingNavigationRequest::new(token.clone()));
        token
    }

    pub(crate) fn document_navigation_cancellation_handle(
        &self,
        token: &DocumentNavigationToken,
    ) -> Option<moli_fetch::FetchCancelHandle> {
        self.pending_navigation_request
            .as_ref()
            .filter(|request| request.matches(token) && !request.committed)
            .map(PendingNavigationRequest::cancellation_handle)
    }

    pub(crate) fn arm_background_navigation_completion(
        &mut self,
        token: &DocumentNavigationToken,
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

    pub(crate) fn settle_background_navigation_completion(
        &mut self,
        token: &DocumentNavigationToken,
    ) -> bool {
        let Some(request) = self
            .pending_navigation_request
            .as_mut()
            .filter(|request| request.matches(token) && request.background_completion_pending)
        else {
            return false;
        };
        request.settle_background_completion();
        if request.committed {
            self.pending_navigation_request = None;
        }
        true
    }

    pub(crate) fn has_inflight_background_navigation(&self) -> bool {
        self.pending_navigation_request
            .as_ref()
            .is_some_and(|request| request.background_completion_pending)
    }

    pub(crate) fn bind_pending_document_navigation_renderer_page(
        &mut self,
        token: &DocumentNavigationToken,
        renderer_page: RendererPageResidenceIdentity,
    ) -> bool {
        if !self
            .pending_navigation_request
            .as_ref()
            .is_some_and(|request| request.matches(token) && !request.committed)
        {
            return false;
        }
        if let Some(binding) = self.pending_renderer_page.as_ref() {
            return matches!(
                binding,
                PendingRendererPageBinding::DocumentNavigation {
                    navigation,
                    renderer_page: bound_renderer_page,
                    page_attachment_id,
                } if navigation == token
                    && *bound_renderer_page == renderer_page
                    && Some(*page_attachment_id)
                        == self
                            .pending_navigation_request
                            .as_ref()
                            .map(|request| request.page_attachment_id)
            );
        }
        self.pending_renderer_page = Some(PendingRendererPageBinding::DocumentNavigation {
            navigation: token.clone(),
            renderer_page,
            page_attachment_id: self
                .pending_navigation_request
                .as_ref()
                .expect("validated pending navigation request must remain installed")
                .page_attachment_id,
        });
        true
    }

    pub(crate) fn routes_renderer_page(
        &self,
        renderer_page: RendererPageResidenceIdentity,
    ) -> bool {
        self.loaded_page
            .as_ref()
            .is_some_and(|page| RendererPageResidenceIdentity::from_page(page) == renderer_page)
            || self
                .pending_renderer_page
                .as_ref()
                .is_some_and(|binding| binding.renderer_page() == renderer_page)
    }

    pub(crate) fn accepts_pending_document_navigation_event(
        &self,
        token: &DocumentNavigationToken,
    ) -> bool {
        self.pending_navigation_request
            .as_ref()
            .is_some_and(|request| request.matches(token) && !request.committed)
    }

    pub(crate) fn accepts_document_body_completion_event(
        &self,
        token: &DocumentNavigationToken,
    ) -> bool {
        match self.pending_navigation_request.as_ref() {
            Some(pending) => pending.matches(token),
            None => self.committed_document_navigation.as_ref() == Some(token),
        }
    }

    pub(crate) fn has_pending_document_navigation(&self) -> bool {
        self.pending_navigation_request
            .as_ref()
            .is_some_and(|request| !request.committed)
    }

    pub(crate) fn current_document_loader_id(&self) -> Option<&str> {
        self.pending_navigation_request
            .as_ref()
            .filter(|request| !request.committed)
            .map(|request| &request.token)
            .or(self.committed_document_navigation.as_ref())
            .map(|navigation| navigation.loader_id.as_str())
    }

    pub(crate) fn committed_document_loader_id(&self) -> Option<&str> {
        self.committed_document_navigation
            .as_ref()
            .map(|navigation| navigation.loader_id.as_str())
    }

    pub(crate) fn commit_pending_document_navigation_if_matches(
        &mut self,
        token: &DocumentNavigationToken,
    ) -> bool {
        let Some(request) = self
            .pending_navigation_request
            .as_mut()
            .filter(|request| request.matches(token) && !request.committed)
        else {
            return false;
        };
        self.committed_document_navigation = Some(token.clone());
        request.committed = true;
        if !request.background_completion_pending {
            request.retire_without_cancellation();
            self.pending_navigation_request = None;
        }
        true
    }

    pub(crate) fn clear_pending_document_navigation_if_loader_matches(
        &mut self,
        loader_id: &str,
    ) -> bool {
        if self
            .pending_navigation_request
            .as_ref()
            .is_some_and(|pending| !pending.committed && pending.token.loader_id == loader_id)
        {
            if matches!(
                self.pending_renderer_page.as_ref(),
                Some(PendingRendererPageBinding::DocumentNavigation {
                    navigation,
                    ..
                }) if navigation.loader_id == loader_id
            ) {
                self.pending_renderer_page = None;
            }
            self.pending_navigation_request = None;
            return true;
        }
        false
    }

    pub(crate) fn clear_document_navigation_state(&mut self) {
        self.finish_renderer_document_lifecycle_observers(
            RendererDocumentLifecycleObservation::Unavailable,
        );
        self.pending_navigation_request = None;
        self.committed_document_navigation = None;
        self.pending_renderer_page = None;
        self.renderer_document_lifecycle = RendererDocumentLifecycleProtocolState::default();
        self.root_post_load_observation = None;
    }

    pub(crate) fn bind_renderer_document_lifecycle(
        &mut self,
        artifacts: RendererPageCreationArtifacts,
        navigation: Option<DocumentNavigationToken>,
        frame_id: String,
        loader_id: String,
    ) -> Vec<RendererDocumentLifecycleEvent> {
        let RendererPageCreationArtifacts {
            active_document,
            active_epoch,
            lifecycle_snapshot,
            initial_lifecycle_events,
        } = artifacts;
        if lifecycle_snapshot.document != active_document
            || lifecycle_snapshot.epoch != active_epoch
        {
            tracing::warn!(
                ?active_document,
                ?active_epoch,
                snapshot_document = ?lifecycle_snapshot.document,
                snapshot_epoch = ?lifecycle_snapshot.epoch,
                "rejecting inconsistent renderer page creation lifecycle artifacts"
            );
            return Vec::new();
        }
        let Some(page_attachment_id) = self.page_attachment_id() else {
            tracing::debug!(
                ?active_document,
                ?active_epoch,
                "dropping renderer lifecycle artifacts without a current Page attachment"
            );
            return Vec::new();
        };
        let initial_snapshot = initial_lifecycle_events
            .iter()
            .find(|event| {
                event.frame == lifecycle_snapshot.frame
                    && event.document == active_document
                    && matches!(
                        event.kind,
                        RendererDocumentLifecycleEventKind::Started { .. }
                    )
            })
            .map(|event| RendererDocumentLifecycleSnapshot {
                frame: event.frame,
                document: event.document,
                epoch: event.epoch,
                started: RendererLifecycleEventStamp {
                    sequence: event.sequence,
                    timestamp_micros: event.timestamp_micros,
                },
                dom_content_loaded: None,
                load: None,
                terminated: None,
            })
            .unwrap_or(lifecycle_snapshot);
        let binding = CommittedRendererDocumentBinding {
            renderer_frame: lifecycle_snapshot.frame,
            renderer_document: active_document,
            renderer_epoch: initial_snapshot.epoch,
            navigation,
            frame_id,
            loader_id,
            page_attachment_id,
            document_open_replacement_epoch: None,
        };
        if self.renderer_document_lifecycle.binding.as_ref() != Some(&binding) {
            self.finish_renderer_document_lifecycle_observers(
                RendererDocumentLifecycleObservation::Superseded,
            );
        }
        tracing::trace!(
            target: "moli_renderer_document_lifecycle",
            renderer_document = ?active_document,
            renderer_lifecycle_epoch = active_epoch.0,
            frame_id = binding.frame_id,
            loader_id = binding.loader_id,
            page_attachment_id = binding.page_attachment_id.get(),
            "bound renderer document lifecycle to committed protocol document"
        );
        let lifecycle_cursor =
            RendererDocumentLifecycleProtocolCursor::from_snapshot(initial_snapshot);
        self.renderer_document_lifecycle = RendererDocumentLifecycleProtocolState {
            binding: Some(binding),
            authoritative: lifecycle_cursor,
            visible: lifecycle_cursor,
            load_visibility: RendererDocumentLoadVisibility::default(),
        };
        self.root_post_load_observation = None;
        self.ingest_renderer_document_lifecycle_events(initial_lifecycle_events)
    }

    pub(crate) fn begin_renderer_document_load_visibility_barrier(
        &mut self,
        loader_id: &str,
    ) -> bool {
        let binding_matches = self
            .renderer_document_lifecycle
            .binding
            .as_ref()
            .is_some_and(|binding| binding.loader_id == loader_id);
        if !binding_matches {
            return false;
        }
        if let Some(active_loader_id) = self
            .renderer_document_lifecycle
            .load_visibility
            .barrier_loader_id
            .as_deref()
        {
            return active_loader_id == loader_id;
        }
        debug_assert!(
            self.renderer_document_lifecycle
                .load_visibility
                .deferred_tail
                .is_empty(),
            "a new load visibility barrier must not inherit deferred events"
        );
        self.renderer_document_lifecycle
            .load_visibility
            .barrier_loader_id = Some(loader_id.to_owned());
        true
    }

    pub(crate) fn release_renderer_document_load_visibility_barrier(
        &mut self,
        loader_id: &str,
    ) -> Option<Vec<RendererDocumentLifecycleEvent>> {
        if self
            .renderer_document_lifecycle
            .load_visibility
            .barrier_loader_id
            .as_deref()
            != Some(loader_id)
        {
            return None;
        }
        self.renderer_document_lifecycle
            .load_visibility
            .barrier_loader_id = None;
        let deferred_tail = std::mem::take(
            &mut self
                .renderer_document_lifecycle
                .load_visibility
                .deferred_tail,
        );
        for event in &deferred_tail {
            self.renderer_document_lifecycle.visible.observe(*event);
        }
        Some(deferred_tail)
    }

    pub(crate) fn cancel_renderer_document_load_visibility_barrier(
        &mut self,
        loader_id: &str,
    ) -> bool {
        if self
            .renderer_document_lifecycle
            .load_visibility
            .barrier_loader_id
            .as_deref()
            != Some(loader_id)
        {
            return false;
        }
        self.renderer_document_lifecycle
            .load_visibility
            .barrier_loader_id = None;
        self.renderer_document_lifecycle
            .load_visibility
            .deferred_tail
            .clear();
        true
    }

    #[cfg(test)]
    fn renderer_document_load_visibility_barrier_active(&self) -> bool {
        self.renderer_document_lifecycle
            .load_visibility
            .barrier_loader_id
            .is_some()
    }

    pub(crate) fn ingest_renderer_document_lifecycle_events(
        &mut self,
        events: Vec<RendererDocumentLifecycleEvent>,
    ) -> Vec<RendererDocumentLifecycleEvent> {
        let binding_is_current = self
            .renderer_document_lifecycle
            .binding
            .as_ref()
            .is_some_and(|binding| {
                Some(binding.page_attachment_id) == self.page_attachment_id()
                    && binding.navigation.as_ref().is_none_or(|navigation| {
                        self.committed_document_navigation.as_ref() == Some(navigation)
                    })
            });
        if !binding_is_current {
            if !events.is_empty() {
                tracing::debug!(
                    event_count = events.len(),
                    "dropping renderer lifecycle events for stale protocol binding"
                );
            }
            return Vec::new();
        }
        let mut accepted = Vec::new();
        for event in events {
            let load_visibility_barrier_active = self
                .renderer_document_lifecycle
                .load_visibility
                .barrier_loader_id
                .is_some();
            let load_visibility_tail_started = !self
                .renderer_document_lifecycle
                .load_visibility
                .deferred_tail
                .is_empty();
            let defer_load_visibility = load_visibility_barrier_active
                && (load_visibility_tail_started
                    || matches!(
                        event.kind,
                        RendererDocumentLifecycleEventKind::Milestone(
                            RendererDocumentLifecycleMilestone::Load
                        )
                    ));
            let Some(binding) = self.renderer_document_lifecycle.binding.as_ref().cloned() else {
                tracing::debug!(
                    sequence = event.sequence,
                    "dropping renderer lifecycle event without committed binding"
                );
                continue;
            };
            if event.frame != binding.renderer_frame || event.document != binding.renderer_document
            {
                tracing::debug!(
                    sequence = event.sequence,
                    event_document = ?event.document,
                    bound_document = ?binding.renderer_document,
                    "dropping stale renderer lifecycle event for another document"
                );
                continue;
            }
            if self
                .renderer_document_lifecycle
                .authoritative
                .last_sequence
                .is_some_and(|sequence| event.sequence <= sequence)
            {
                tracing::debug!(
                    sequence = event.sequence,
                    "dropping duplicate or reordered renderer lifecycle event"
                );
                continue;
            }
            let restarts_same_document = event.epoch != binding.renderer_epoch
                && matches!(
                    event.kind,
                    RendererDocumentLifecycleEventKind::Started { .. }
                )
                && event.epoch.0 > binding.renderer_epoch.0
                && self
                    .renderer_document_lifecycle
                    .authoritative
                    .snapshot
                    .is_some_and(|snapshot| snapshot.terminated.is_some());
            if event.epoch != binding.renderer_epoch && !restarts_same_document {
                tracing::debug!(
                    sequence = event.sequence,
                    event_epoch = event.epoch.0,
                    bound_epoch = binding.renderer_epoch.0,
                    "dropping stale renderer lifecycle event for another epoch"
                );
                continue;
            }
            if restarts_same_document {
                self.finish_renderer_document_lifecycle_observers(
                    RendererDocumentLifecycleObservation::Superseded,
                );
                self.renderer_document_lifecycle
                    .binding
                    .as_mut()
                    .expect("validated lifecycle binding")
                    .renderer_epoch = event.epoch;
            }
            if let RendererDocumentLifecycleEventKind::Started { reason } = event.kind {
                self.renderer_document_lifecycle
                    .binding
                    .as_mut()
                    .expect("validated lifecycle binding")
                    .document_open_replacement_epoch = matches!(
                    reason,
                    RendererLifecycleStartReason::ExplicitDocumentOpen
                        | RendererLifecycleStartReason::JavascriptDocumentReplacement
                )
                .then_some(event.epoch);
            }
            for registration in &mut self.renderer_document_lifecycle_waiters {
                registration.waiter.observe(event);
                let observation =
                    lifecycle_observation_from_wait_outcome(registration.waiter.outcome());
                if observation.is_terminal()
                    && let Some(publisher) = registration.observer_publisher.as_ref()
                {
                    publisher.publish(observation);
                }
            }
            self.renderer_document_lifecycle_waiters
                .retain(|registration| {
                    registration
                        .observer_publisher
                        .as_ref()
                        .is_none_or(|publisher| {
                            publisher.has_observer()
                                && !lifecycle_observation_from_wait_outcome(
                                    registration.waiter.outcome(),
                                )
                                .is_terminal()
                        })
                });
            self.renderer_document_lifecycle
                .authoritative
                .observe(event);
            if defer_load_visibility {
                self.renderer_document_lifecycle
                    .load_visibility
                    .deferred_tail
                    .push(event);
            } else {
                self.renderer_document_lifecycle.visible.observe(event);
                accepted.push(event);
            }
        }
        accepted
    }

    pub(crate) fn renderer_document_lifecycle_binding(
        &self,
    ) -> Option<&CommittedRendererDocumentBinding> {
        self.renderer_document_lifecycle
            .binding
            .as_ref()
            .filter(|binding| {
                Some(binding.page_attachment_id) == self.page_attachment_id()
                    && binding.navigation.as_ref().is_none_or(|navigation| {
                        self.committed_document_navigation.as_ref() == Some(navigation)
                    })
            })
    }

    #[cfg(test)]
    pub(crate) fn renderer_document_lifecycle_authoritative_snapshot(
        &self,
    ) -> Option<RendererDocumentLifecycleSnapshot> {
        self.renderer_document_lifecycle.authoritative.snapshot
    }

    pub(crate) fn renderer_document_lifecycle_visible_snapshot(
        &self,
    ) -> Option<RendererDocumentLifecycleSnapshot> {
        self.renderer_document_lifecycle.visible.snapshot
    }

    pub(crate) fn register_renderer_document_lifecycle_waiter(
        &mut self,
        milestone: RendererDocumentLifecycleMilestone,
        expected_loader_id: &str,
    ) -> Option<(
        RendererDocumentLifecycleWaiterId,
        CommittedRendererDocumentBinding,
    )> {
        let binding = self.renderer_document_lifecycle.binding.clone()?;
        if binding.loader_id != expected_loader_id {
            return None;
        }
        let snapshot = self.renderer_document_lifecycle.authoritative.snapshot?;
        let id = self
            .next_renderer_document_lifecycle_waiter_id
            .allocate_next();
        self.renderer_document_lifecycle_waiters
            .push(RegisteredRendererDocumentLifecycleWaiter {
                id,
                renderer_document: binding.renderer_document,
                renderer_epoch: binding.renderer_epoch,
                frame_id: binding.frame_id.clone(),
                loader_id: binding.loader_id.clone(),
                waiter: RendererDocumentLifecycleWaiter::from_snapshot(snapshot, milestone),
                observer_publisher: None,
            });
        Some((id, binding))
    }

    pub(crate) fn register_exact_renderer_document_lifecycle_observer(
        &mut self,
        expected_binding: &CommittedRendererDocumentBinding,
        milestone: RendererDocumentLifecycleMilestone,
    ) -> RendererDocumentLifecycleObserver {
        let Some(binding) = self.renderer_document_lifecycle.binding.as_ref() else {
            return RendererDocumentLifecycleObserver::resolved(
                RendererDocumentLifecycleObservation::Unavailable,
            );
        };
        if binding != expected_binding {
            return RendererDocumentLifecycleObserver::resolved(
                RendererDocumentLifecycleObservation::Superseded,
            );
        }
        let Some(snapshot) = self.renderer_document_lifecycle.authoritative.snapshot else {
            return RendererDocumentLifecycleObserver::resolved(
                RendererDocumentLifecycleObservation::Unavailable,
            );
        };
        let waiter = RendererDocumentLifecycleWaiter::from_snapshot(snapshot, milestone);
        let observation = lifecycle_observation_from_wait_outcome(waiter.outcome());
        let (publisher, observer) = RendererDocumentLifecycleObserver::channel(observation);
        if observation == RendererDocumentLifecycleObservation::Pending {
            let id = self
                .next_renderer_document_lifecycle_waiter_id
                .allocate_next();
            self.renderer_document_lifecycle_waiters
                .retain(|registration| {
                    registration
                        .observer_publisher
                        .as_ref()
                        .is_none_or(RendererDocumentLifecycleObservationPublisher::has_observer)
                });
            self.renderer_document_lifecycle_waiters.push(
                RegisteredRendererDocumentLifecycleWaiter {
                    id,
                    renderer_document: binding.renderer_document,
                    renderer_epoch: binding.renderer_epoch,
                    frame_id: binding.frame_id.clone(),
                    loader_id: binding.loader_id.clone(),
                    waiter,
                    observer_publisher: Some(publisher),
                },
            );
        }
        observer
    }

    fn finish_renderer_document_lifecycle_observers(
        &mut self,
        observation: RendererDocumentLifecycleObservation,
    ) {
        assert!(
            observation.is_terminal(),
            "retiring lifecycle waiters requires a terminal observation"
        );
        self.renderer_document_lifecycle_waiters
            .retain(|registration| {
                let Some(publisher) = registration.observer_publisher.as_ref() else {
                    // Polling DevTools wait keys own their explicit release
                    // protocol. Preserve their reached/interrupted result
                    // across a successor binding until that consumer reads
                    // and releases the exact registration.
                    return true;
                };
                publisher.publish(observation);
                false
            });
    }

    pub(crate) fn renderer_document_lifecycle_waiter_outcome(
        &self,
        id: RendererDocumentLifecycleWaiterId,
        renderer_document: RendererDocumentToken,
        renderer_epoch: RendererLifecycleEpoch,
        frame_id: &str,
        loader_id: &str,
    ) -> Option<RendererDocumentLifecycleWaitOutcome> {
        self.renderer_document_lifecycle_waiters
            .iter()
            .find(|registration| {
                registration.id == id
                    && registration.renderer_document == renderer_document
                    && registration.renderer_epoch == renderer_epoch
                    && registration.frame_id == frame_id
                    && registration.loader_id == loader_id
            })
            .map(|registration| registration.waiter.outcome())
    }

    pub(crate) fn release_renderer_document_lifecycle_waiter(
        &mut self,
        id: RendererDocumentLifecycleWaiterId,
        renderer_document: RendererDocumentToken,
        renderer_epoch: RendererLifecycleEpoch,
        frame_id: &str,
        loader_id: &str,
    ) -> bool {
        let previous_len = self.renderer_document_lifecycle_waiters.len();
        self.renderer_document_lifecycle_waiters
            .retain(|registration| {
                registration.id != id
                    || registration.renderer_document != renderer_document
                    || registration.renderer_epoch != renderer_epoch
                    || registration.frame_id != frame_id
                    || registration.loader_id != loader_id
            });
        self.renderer_document_lifecycle_waiters.len() != previous_len
    }

    pub(crate) fn arm_root_post_load_observation(&mut self, loader_id: &str) -> bool {
        let Some(binding) = self
            .renderer_document_lifecycle
            .binding
            .as_ref()
            .filter(|binding| {
                binding.loader_id == loader_id
                    && Some(binding.page_attachment_id) == self.page_attachment_id()
            })
            .cloned()
        else {
            return false;
        };
        let snapshot_reached_load = self
            .renderer_document_lifecycle
            .authoritative
            .snapshot
            .is_some_and(|snapshot| {
                snapshot.document == binding.renderer_document
                    && snapshot.epoch == binding.renderer_epoch
                    && snapshot.load.is_some()
                    && snapshot.terminated.is_none()
            });
        if !snapshot_reached_load {
            return false;
        }
        if self
            .root_post_load_observation
            .as_ref()
            .is_some_and(|observation| observation.binding == binding)
        {
            return false;
        }
        self.root_post_load_observation = Some(RootPostLoadObservation {
            binding,
            frame_stopped_loading_pending: true,
            network_idle_pending: true,
        });
        true
    }

    pub(crate) fn take_root_frame_stopped_loading_binding(
        &mut self,
    ) -> Option<CommittedRendererDocumentBinding> {
        if !self.root_post_load_binding_is_current() {
            self.root_post_load_observation = None;
            return None;
        }
        let observation = self.root_post_load_observation.as_mut()?;
        if !observation.frame_stopped_loading_pending {
            return None;
        }
        observation.frame_stopped_loading_pending = false;
        Some(observation.binding.clone())
    }

    pub(crate) fn take_root_network_idle_binding(
        &mut self,
    ) -> Option<CommittedRendererDocumentBinding> {
        if self.has_pending_document_navigation() {
            return None;
        }
        if !self.root_post_load_binding_is_current() {
            self.root_post_load_observation = None;
            return None;
        }
        if !self.root_network_idle_snapshot_is_eligible() {
            if let Some(observation) = self.root_post_load_observation.as_mut() {
                observation.network_idle_pending = false;
            }
            return None;
        }
        let observation = self.root_post_load_observation.as_mut()?;
        if !observation.network_idle_pending {
            return None;
        }
        observation.network_idle_pending = false;
        Some(observation.binding.clone())
    }

    fn root_post_load_binding_is_current(&self) -> bool {
        let Some(observation) = self.root_post_load_observation.as_ref() else {
            return false;
        };
        self.renderer_document_lifecycle.binding.as_ref() == Some(&observation.binding)
    }

    fn root_network_idle_snapshot_is_eligible(&self) -> bool {
        let Some(observation) = self.root_post_load_observation.as_ref() else {
            return false;
        };
        self.renderer_document_lifecycle
            .authoritative
            .snapshot
            .is_some_and(|snapshot| {
                snapshot.document == observation.binding.renderer_document
                    && snapshot.epoch == observation.binding.renderer_epoch
                    && snapshot.load.is_some()
                    && snapshot.terminated.is_none()
            })
    }
}

#[cfg(test)]
mod page_residence_tests {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };

    use super::*;
    use crate::conn::TargetPageResidenceObservation;

    #[test]
    fn empty_slot_never_exposes_a_page_attachment() {
        let mut slot = TargetPageSlot::default();

        assert_eq!(slot.page_attachment_id(), None);
        assert!(
            slot.replace_loaded_page_with_reason(None, TargetPageAbsenceReason::TestFixture)
                .is_none()
        );
        assert_eq!(slot.page_attachment_id(), None);
    }

    #[test]
    fn attachment_token_terminates_on_attachment_replacement() {
        let mut slot = TargetPageSlot::default();
        slot.set_page_attachment_id_for_test(91);
        let token = slot
            .page_residence_token()
            .expect("the installed attachment should expose its lifetime token");

        let mut wait = Box::pin(token.wait());
        let mut context = Context::from_waker(Waker::noop());
        assert!(
            matches!(wait.as_mut().poll(&mut context), Poll::Pending),
            "a live attachment token must remain pending"
        );

        slot.set_page_attachment_id_for_test(92);

        assert!(matches!(
            wait.as_mut().poll(&mut context),
            Poll::Ready(TargetPageResidenceObservation::Superseded)
        ));
    }
}

#[cfg(test)]
mod pending_renderer_page_tests {
    use super::*;

    fn renderer_page(owner: u64, page: u64) -> RendererPageResidenceIdentity {
        RendererPageResidenceIdentity::new(
            moli_core::RendererOwnerLocalHostId::new_for_testing(owner),
            moli_core::PageId::new_for_testing(page),
        )
    }

    #[test]
    fn initial_build_binding_is_exact_and_retires_with_build() {
        let mut slot = TargetPageSlot::empty_for_initial_document_page_build();
        slot.start_initial_document_page_build();
        let expected = renderer_page(7, 11);
        let peer = renderer_page(7, 12);

        assert!(slot.bind_initial_document_page_build_renderer_page(expected));
        assert!(slot.routes_renderer_page(expected));
        assert!(!slot.routes_renderer_page(peer));
        assert!(
            !slot.bind_initial_document_page_build_renderer_page(peer),
            "an initial build owns exactly one renderer Page reservation"
        );

        slot.complete_initial_document_page_build();
        assert!(!slot.routes_renderer_page(expected));
    }

    #[test]
    fn navigation_reservation_preallocates_one_exact_page_attachment() {
        let mut slot = TargetPageSlot::default();
        let current_attachment = slot.set_page_attachment_id_for_test(19);
        let navigation =
            slot.start_document_navigation("TID-pending-page".to_owned(), "LOADER-next".to_owned());
        let reserved_attachment = slot
            .pending_page_attachment_id()
            .expect("navigation should reserve its future Page attachment");
        let reserved_page = renderer_page(8, 20);

        assert_ne!(reserved_attachment, current_attachment);
        assert_eq!(
            slot.reserve_renderer_page_attachment(reserved_page),
            reserved_attachment
        );
        assert_eq!(
            slot.reserve_renderer_page_attachment(reserved_page),
            reserved_attachment,
            "revisiting the same renderer Page reservation must be idempotent"
        );
        assert!(
            slot.bind_pending_document_navigation_renderer_page(&navigation, reserved_page),
            "the navigation binding should accept its already-reserved renderer Page"
        );
    }

    #[test]
    fn navigation_binding_cannot_follow_a_superseding_navigation() {
        let mut slot = TargetPageSlot::default();
        let first = slot
            .start_document_navigation("TID-pending-page".to_owned(), "LOADER-first".to_owned());
        let first_page = renderer_page(8, 21);
        assert!(slot.bind_pending_document_navigation_renderer_page(&first, first_page));
        assert!(slot.routes_renderer_page(first_page));
        assert!(
            !slot.bind_pending_document_navigation_renderer_page(&first, renderer_page(8, 23),),
            "one navigation request cannot replace its bound renderer Page"
        );

        let second = slot
            .start_document_navigation("TID-pending-page".to_owned(), "LOADER-second".to_owned());
        assert!(
            !slot.routes_renderer_page(first_page),
            "a new navigation must retire the prior pending renderer Page route"
        );
        assert!(
            !slot.bind_pending_document_navigation_renderer_page(&first, first_page),
            "a superseded navigation cannot reinstall its renderer Page route"
        );

        let second_page = renderer_page(8, 22);
        assert!(slot.bind_pending_document_navigation_renderer_page(&second, second_page));
        assert!(slot.clear_pending_document_navigation_if_loader_matches("LOADER-second"));
        assert!(!slot.routes_renderer_page(second_page));
    }
}

#[cfg(test)]
mod renderer_document_lifecycle_tests {
    use super::*;
    use moli_core::page::{
        RendererDocumentLifecycleEventKind, RendererDocumentTerminationReason,
        RendererLifecycleStartReason,
    };

    fn event(
        document: RendererDocumentToken,
        epoch: RendererLifecycleEpoch,
        sequence: u64,
        kind: RendererDocumentLifecycleEventKind,
    ) -> RendererDocumentLifecycleEvent {
        RendererDocumentLifecycleEvent {
            frame: RendererFrameToken {
                page_id: document.page_id,
            },
            document,
            epoch,
            sequence,
            timestamp_micros: sequence * 10,
            kind,
        }
    }

    fn page_slot_with_attachment() -> TargetPageSlot {
        TargetPageSlot {
            page_attachment_id: Some(TargetPageAttachmentId::allocate()),
            ..Default::default()
        }
    }

    #[test]
    fn lifecycle_binding_requires_and_tracks_the_current_page_attachment() {
        let page_id = moli_core::PageId::new_for_testing(8);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let epoch = RendererLifecycleEpoch(1);
        let started = event(
            document,
            epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let artifacts = RendererPageCreationArtifacts {
            active_document: document,
            active_epoch: epoch,
            lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                frame: started.frame,
                document,
                epoch,
                started: RendererLifecycleEventStamp {
                    sequence: 1,
                    timestamp_micros: 10,
                },
                dom_content_loaded: None,
                load: None,
                terminated: None,
            },
            initial_lifecycle_events: vec![started],
        };

        let mut slot = TargetPageSlot::default();
        assert!(
            slot.bind_renderer_document_lifecycle(
                artifacts.clone(),
                None,
                "FRAME-8".to_owned(),
                "LOADER-8".to_owned(),
            )
            .is_empty()
        );
        assert!(slot.renderer_document_lifecycle_binding().is_none());

        slot.set_page_attachment_id_for_test(8);
        assert_eq!(
            slot.bind_renderer_document_lifecycle(
                artifacts,
                None,
                "FRAME-8".to_owned(),
                "LOADER-8".to_owned(),
            ),
            vec![started]
        );
        assert!(slot.renderer_document_lifecycle_binding().is_some());

        slot.page_attachment_id = None;
        assert!(
            slot.renderer_document_lifecycle_binding().is_none(),
            "a binding from a removed Page attachment must never remain current"
        );
    }

    #[test]
    fn binding_accepts_current_identity_and_rejects_stale_document() {
        let page_id = moli_core::PageId::new_for_testing(9);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let epoch = RendererLifecycleEpoch(1);
        let started = event(
            document,
            epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let dcl = event(
            document,
            epoch,
            2,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        let mut slot = page_slot_with_attachment();
        slot.set_page_attachment_id_for_test(4);
        let navigation =
            slot.start_document_navigation("FRAME-9".to_owned(), "LOADER-9".to_owned());
        assert!(slot.commit_pending_document_navigation_if_matches(&navigation));
        let accepted = slot.bind_renderer_document_lifecycle(
            RendererPageCreationArtifacts {
                active_document: document,
                active_epoch: epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: started.frame,
                    document,
                    epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 1,
                        timestamp_micros: 10,
                    },
                    dom_content_loaded: Some(RendererLifecycleEventStamp {
                        sequence: 2,
                        timestamp_micros: 20,
                    }),
                    load: None,
                    terminated: None,
                },
                initial_lifecycle_events: vec![started, dcl],
            },
            Some(navigation),
            "FRAME-9".to_owned(),
            "LOADER-9".to_owned(),
        );
        assert_eq!(accepted, vec![started, dcl]);
        assert!(slot.begin_renderer_document_load_visibility_barrier("LOADER-9"));
        assert!(slot.renderer_document_load_visibility_barrier_active());
        assert!(
            slot.release_renderer_document_load_visibility_barrier("LOADER-stale")
                .is_none()
        );
        assert!(slot.renderer_document_load_visibility_barrier_active());
        assert_eq!(
            slot.release_renderer_document_load_visibility_barrier("LOADER-9"),
            Some(Vec::new())
        );
        assert!(!slot.renderer_document_load_visibility_barrier_active());
        assert_eq!(
            slot.renderer_document_lifecycle_binding()
                .unwrap()
                .page_attachment_id
                .get(),
            4
        );

        let stale = event(
            document.successor_for_testing(),
            epoch,
            3,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        assert!(
            slot.ingest_renderer_document_lifecycle_events(vec![stale])
                .is_empty()
        );
        assert!(
            slot.renderer_document_lifecycle_authoritative_snapshot()
                .unwrap()
                .load
                .is_none()
        );
    }

    #[test]
    fn load_visibility_barrier_exposes_dcl_and_defers_only_load_delivery() {
        let page_id = moli_core::PageId::new_for_testing(10);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let epoch = RendererLifecycleEpoch(1);
        let started = event(
            document,
            epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let dcl = event(
            document,
            epoch,
            2,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        let load = event(
            document,
            epoch,
            3,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        let terminated = event(
            document,
            epoch,
            4,
            RendererDocumentLifecycleEventKind::Terminated {
                last_reached: Some(RendererDocumentLifecycleMilestone::Load),
                reason: RendererDocumentTerminationReason::Stopped,
            },
        );
        let mut slot = page_slot_with_attachment();
        slot.set_page_attachment_id_for_test(5);
        let navigation =
            slot.start_document_navigation("FRAME-10".to_owned(), "LOADER-10".to_owned());
        assert!(slot.commit_pending_document_navigation_if_matches(&navigation));
        assert_eq!(
            slot.bind_renderer_document_lifecycle(
                RendererPageCreationArtifacts {
                    active_document: document,
                    active_epoch: epoch,
                    lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                        frame: started.frame,
                        document,
                        epoch,
                        started: RendererLifecycleEventStamp {
                            sequence: 1,
                            timestamp_micros: 10,
                        },
                        dom_content_loaded: None,
                        load: None,
                        terminated: None,
                    },
                    initial_lifecycle_events: vec![started],
                },
                Some(navigation),
                "FRAME-10".to_owned(),
                "LOADER-10".to_owned(),
            ),
            vec![started]
        );
        assert!(slot.begin_renderer_document_load_visibility_barrier("LOADER-10"));
        let (load_waiter_id, load_waiter_binding) = slot
            .register_renderer_document_lifecycle_waiter(
                RendererDocumentLifecycleMilestone::Load,
                "LOADER-10",
            )
            .expect("load waiter should bind to the authoritative document state");

        assert_eq!(
            slot.ingest_renderer_document_lifecycle_events(vec![dcl, load, terminated]),
            vec![dcl],
            "DOMContentLoaded remains visible while the ordered tail from load is gated"
        );
        assert_eq!(
            slot.renderer_document_lifecycle_authoritative_snapshot()
                .and_then(|snapshot| snapshot.load),
            Some(RendererLifecycleEventStamp {
                sequence: 3,
                timestamp_micros: 30,
            }),
            "load readiness is authoritative even while its protocol event is hidden"
        );
        assert_eq!(
            slot.renderer_document_lifecycle_waiter_outcome(
                load_waiter_id,
                load_waiter_binding.renderer_document,
                load_waiter_binding.renderer_epoch,
                &load_waiter_binding.frame_id,
                &load_waiter_binding.loader_id,
            ),
            Some(RendererDocumentLifecycleWaitOutcome::Reached(
                RendererLifecycleEventStamp {
                    sequence: 3,
                    timestamp_micros: 30,
                }
            )),
            "navigation waiters observe authoritative load readiness"
        );
        let visible_before_release = slot
            .renderer_document_lifecycle_visible_snapshot()
            .expect("visible lifecycle cursor");
        assert_eq!(
            visible_before_release.dom_content_loaded,
            Some(RendererLifecycleEventStamp {
                sequence: 2,
                timestamp_micros: 20,
            })
        );
        assert_eq!(visible_before_release.load, None);
        assert_eq!(visible_before_release.terminated, None);
        assert_eq!(
            slot.release_renderer_document_load_visibility_barrier("LOADER-10"),
            Some(vec![load, terminated]),
            "events after load must not overtake the delayed load milestone"
        );
        let visible_after_release = slot
            .renderer_document_lifecycle_visible_snapshot()
            .expect("released visible lifecycle cursor");
        assert_eq!(
            visible_after_release.load,
            Some(RendererLifecycleEventStamp {
                sequence: 3,
                timestamp_micros: 30,
            })
        );
        assert_eq!(
            visible_after_release.terminated,
            Some(RendererLifecycleTerminationStamp {
                sequence: 4,
                timestamp_micros: 40,
                reason: RendererDocumentTerminationReason::Stopped,
            })
        );
    }

    #[test]
    fn cancelling_load_visibility_barrier_discards_tail_without_revealing_it() {
        let page_id = moli_core::PageId::new_for_testing(16);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let epoch = RendererLifecycleEpoch(1);
        let started = event(
            document,
            epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let load = event(
            document,
            epoch,
            2,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        let mut slot = page_slot_with_attachment();
        slot.bind_renderer_document_lifecycle(
            RendererPageCreationArtifacts {
                active_document: document,
                active_epoch: epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: started.frame,
                    document,
                    epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 1,
                        timestamp_micros: 10,
                    },
                    dom_content_loaded: None,
                    load: None,
                    terminated: None,
                },
                initial_lifecycle_events: vec![started],
            },
            None,
            "FRAME-16".to_owned(),
            "LOADER-16".to_owned(),
        );
        assert!(slot.begin_renderer_document_load_visibility_barrier("LOADER-16"));
        assert!(
            slot.ingest_renderer_document_lifecycle_events(vec![load])
                .is_empty()
        );
        assert!(
            slot.renderer_document_lifecycle_authoritative_snapshot()
                .is_some_and(|snapshot| snapshot.load.is_some())
        );
        assert!(slot.cancel_renderer_document_load_visibility_barrier("LOADER-16"));
        assert!(!slot.renderer_document_load_visibility_barrier_active());
        assert!(
            slot.renderer_document_lifecycle_visible_snapshot()
                .is_some_and(|snapshot| snapshot.load.is_none()),
            "discarding a stale output tail must not make it replayable"
        );
        assert!(
            slot.release_renderer_document_load_visibility_barrier("LOADER-16")
                .is_none()
        );
        assert!(!slot.cancel_renderer_document_load_visibility_barrier("LOADER-16"));
    }

    #[test]
    fn load_visibility_barrier_keeps_later_epoch_behind_deferred_load_tail() {
        let page_id = moli_core::PageId::new_for_testing(11);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let first_epoch = RendererLifecycleEpoch(1);
        let second_epoch = RendererLifecycleEpoch(2);
        let started = event(
            document,
            first_epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let dcl = event(
            document,
            first_epoch,
            2,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        let load = event(
            document,
            first_epoch,
            3,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        let terminated = event(
            document,
            first_epoch,
            4,
            RendererDocumentLifecycleEventKind::Terminated {
                last_reached: Some(RendererDocumentLifecycleMilestone::Load),
                reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
            },
        );
        let restarted = event(
            document,
            second_epoch,
            5,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
            },
        );
        let restarted_dcl = event(
            document,
            second_epoch,
            6,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        let mut slot = page_slot_with_attachment();
        slot.set_page_attachment_id_for_test(6);
        let navigation =
            slot.start_document_navigation("FRAME-11".to_owned(), "LOADER-11".to_owned());
        assert!(slot.commit_pending_document_navigation_if_matches(&navigation));
        assert_eq!(
            slot.bind_renderer_document_lifecycle(
                RendererPageCreationArtifacts {
                    active_document: document,
                    active_epoch: first_epoch,
                    lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                        frame: started.frame,
                        document,
                        epoch: first_epoch,
                        started: RendererLifecycleEventStamp {
                            sequence: 1,
                            timestamp_micros: 10,
                        },
                        dom_content_loaded: None,
                        load: None,
                        terminated: None,
                    },
                    initial_lifecycle_events: vec![started, dcl],
                },
                Some(navigation),
                "FRAME-11".to_owned(),
                "LOADER-11".to_owned(),
            ),
            vec![started, dcl]
        );
        assert!(slot.begin_renderer_document_load_visibility_barrier("LOADER-11"));
        assert!(
            slot.ingest_renderer_document_lifecycle_events(vec![
                load,
                terminated,
                restarted,
                restarted_dcl,
            ])
            .is_empty(),
            "nothing after the hidden load may overtake its visibility boundary"
        );

        let authoritative = slot
            .renderer_document_lifecycle_authoritative_snapshot()
            .expect("authoritative restarted lifecycle");
        assert_eq!(authoritative.epoch, second_epoch);
        assert_eq!(
            authoritative.dom_content_loaded,
            Some(RendererLifecycleEventStamp {
                sequence: 6,
                timestamp_micros: 60,
            })
        );
        let visible = slot
            .renderer_document_lifecycle_visible_snapshot()
            .expect("visible lifecycle before release");
        assert_eq!(visible.epoch, first_epoch);
        assert_eq!(visible.dom_content_loaded.unwrap().sequence, 2);
        assert_eq!(visible.load, None);

        assert_eq!(
            slot.release_renderer_document_load_visibility_barrier("LOADER-11"),
            Some(vec![load, terminated, restarted, restarted_dcl])
        );
        let visible = slot
            .renderer_document_lifecycle_visible_snapshot()
            .expect("visible lifecycle after release");
        assert_eq!(visible.epoch, second_epoch);
        assert_eq!(
            visible.dom_content_loaded,
            Some(RendererLifecycleEventStamp {
                sequence: 6,
                timestamp_micros: 60,
            })
        );
        assert_eq!(visible.load, None);
        assert_eq!(visible.terminated, None);
    }

    #[test]
    fn same_document_restart_advances_epoch_without_rebinding_loader() {
        let page_id = moli_core::PageId::new_for_testing(10);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let first_epoch = RendererLifecycleEpoch(1);
        let started = event(
            document,
            first_epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let mut slot = page_slot_with_attachment();
        slot.bind_renderer_document_lifecycle(
            RendererPageCreationArtifacts {
                active_document: document,
                active_epoch: first_epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: started.frame,
                    document,
                    epoch: first_epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 1,
                        timestamp_micros: 10,
                    },
                    dom_content_loaded: None,
                    load: None,
                    terminated: None,
                },
                initial_lifecycle_events: vec![started],
            },
            None,
            "FRAME-10".to_owned(),
            "LOADER-10".to_owned(),
        );
        let terminated = event(
            document,
            first_epoch,
            2,
            RendererDocumentLifecycleEventKind::Terminated {
                last_reached: None,
                reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
            },
        );
        let second_epoch = RendererLifecycleEpoch(2);
        let restarted = event(
            document,
            second_epoch,
            3,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
            },
        );
        let dcl = event(
            document,
            second_epoch,
            4,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        assert_eq!(
            slot.ingest_renderer_document_lifecycle_events(vec![terminated, restarted, dcl]),
            vec![terminated, restarted, dcl]
        );
        assert_eq!(
            slot.renderer_document_lifecycle_binding()
                .unwrap()
                .renderer_epoch,
            second_epoch
        );
        assert_eq!(
            slot.renderer_document_lifecycle_authoritative_snapshot()
                .unwrap()
                .epoch,
            second_epoch
        );
    }

    #[test]
    fn creation_handoff_preserves_completed_epochs_before_the_active_epoch() {
        let page_id = moli_core::PageId::new_for_testing(11);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let first_epoch = RendererLifecycleEpoch(1);
        let second_epoch = RendererLifecycleEpoch(2);
        let first_started = event(
            document,
            first_epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let first_dcl = event(
            document,
            first_epoch,
            2,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        let first_terminated = event(
            document,
            first_epoch,
            3,
            RendererDocumentLifecycleEventKind::Terminated {
                last_reached: Some(RendererDocumentLifecycleMilestone::DomContentLoaded),
                reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
            },
        );
        let second_started = event(
            document,
            second_epoch,
            4,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
            },
        );
        let second_dcl = event(
            document,
            second_epoch,
            5,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        let initial_events = vec![
            first_started,
            first_dcl,
            first_terminated,
            second_started,
            second_dcl,
        ];

        let mut slot = page_slot_with_attachment();
        let accepted = slot.bind_renderer_document_lifecycle(
            RendererPageCreationArtifacts {
                active_document: document,
                active_epoch: second_epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: second_started.frame,
                    document,
                    epoch: second_epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 4,
                        timestamp_micros: 40,
                    },
                    dom_content_loaded: Some(RendererLifecycleEventStamp {
                        sequence: 5,
                        timestamp_micros: 50,
                    }),
                    load: None,
                    terminated: None,
                },
                initial_lifecycle_events: initial_events.clone(),
            },
            None,
            "FRAME-11".to_owned(),
            "LOADER-11".to_owned(),
        );

        assert_eq!(accepted, initial_events);
        let snapshot = slot
            .renderer_document_lifecycle_authoritative_snapshot()
            .expect("active lifecycle snapshot");
        assert_eq!(snapshot.epoch, second_epoch);
        assert_eq!(snapshot.dom_content_loaded.unwrap().sequence, 5);
    }

    #[test]
    fn successor_document_binding_discards_deferred_tail_but_preserves_reached_waiter() {
        let page_id = moli_core::PageId::new_for_testing(14);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let epoch = RendererLifecycleEpoch(1);
        let started = event(
            document,
            epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let dcl = event(
            document,
            epoch,
            2,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        let mut slot = page_slot_with_attachment();
        slot.bind_renderer_document_lifecycle(
            RendererPageCreationArtifacts {
                active_document: document,
                active_epoch: epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: started.frame,
                    document,
                    epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 1,
                        timestamp_micros: 10,
                    },
                    dom_content_loaded: Some(RendererLifecycleEventStamp {
                        sequence: 2,
                        timestamp_micros: 20,
                    }),
                    load: None,
                    terminated: None,
                },
                initial_lifecycle_events: vec![started, dcl],
            },
            None,
            "FRAME-14".to_owned(),
            "LOADER-14".to_owned(),
        );
        assert!(
            slot.register_renderer_document_lifecycle_waiter(
                RendererDocumentLifecycleMilestone::Load,
                "LOADER-previous",
            )
            .is_none(),
            "a fast-ack navigation must not register against the previous loader"
        );
        let (waiter_id, binding) = slot
            .register_renderer_document_lifecycle_waiter(
                RendererDocumentLifecycleMilestone::Load,
                "LOADER-14",
            )
            .expect("source document load waiter");
        let load = event(
            document,
            epoch,
            3,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        assert!(slot.begin_renderer_document_load_visibility_barrier("LOADER-14"));
        assert_eq!(
            slot.ingest_renderer_document_lifecycle_events(vec![load]),
            Vec::new()
        );
        assert!(
            slot.renderer_document_lifecycle_authoritative_snapshot()
                .is_some_and(|snapshot| snapshot.load.is_some())
        );
        assert!(
            slot.renderer_document_lifecycle_visible_snapshot()
                .is_some_and(|snapshot| snapshot.load.is_none())
        );

        let successor = RendererDocumentToken::new_for_testing(page_id, 2);
        let successor_epoch = RendererLifecycleEpoch(2);
        let successor_started = event(
            successor,
            successor_epoch,
            4,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::CrossDocumentCommit,
            },
        );
        slot.bind_renderer_document_lifecycle(
            RendererPageCreationArtifacts {
                active_document: successor,
                active_epoch: successor_epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: successor_started.frame,
                    document: successor,
                    epoch: successor_epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 4,
                        timestamp_micros: 40,
                    },
                    dom_content_loaded: None,
                    load: None,
                    terminated: None,
                },
                initial_lifecycle_events: vec![successor_started],
            },
            None,
            "FRAME-14".to_owned(),
            "LOADER-15".to_owned(),
        );

        assert!(!slot.renderer_document_load_visibility_barrier_active());
        assert!(
            slot.release_renderer_document_load_visibility_barrier("LOADER-14")
                .is_none(),
            "a successor binding must discard the previous document's deferred tail"
        );
        assert!(
            slot.renderer_document_lifecycle_visible_snapshot()
                .is_some_and(|snapshot| snapshot.document == successor && snapshot.load.is_none())
        );

        assert!(matches!(
            slot.renderer_document_lifecycle_waiter_outcome(
                waiter_id,
                binding.renderer_document,
                binding.renderer_epoch,
                &binding.frame_id,
                &binding.loader_id,
            ),
            Some(RendererDocumentLifecycleWaitOutcome::Reached(stamp)) if stamp.sequence == 3
        ));
        assert!(slot.release_renderer_document_lifecycle_waiter(
            waiter_id,
            binding.renderer_document,
            binding.renderer_epoch,
            &binding.frame_id,
            &binding.loader_id,
        ));
    }

    #[test]
    fn post_load_observers_are_armed_once_and_bound_to_the_loaded_document() {
        let page_id = moli_core::PageId::new_for_testing(12);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let epoch = RendererLifecycleEpoch(1);
        let started = event(
            document,
            epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let load = event(
            document,
            epoch,
            2,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        let mut slot = page_slot_with_attachment();
        slot.bind_renderer_document_lifecycle(
            RendererPageCreationArtifacts {
                active_document: document,
                active_epoch: epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: started.frame,
                    document,
                    epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 1,
                        timestamp_micros: 10,
                    },
                    dom_content_loaded: None,
                    load: Some(RendererLifecycleEventStamp {
                        sequence: 2,
                        timestamp_micros: 20,
                    }),
                    terminated: None,
                },
                initial_lifecycle_events: vec![started, load],
            },
            None,
            "FRAME-12".to_owned(),
            "LOADER-12".to_owned(),
        );

        assert!(slot.arm_root_post_load_observation("LOADER-12"));
        assert!(!slot.arm_root_post_load_observation("LOADER-12"));
        slot.start_document_navigation("FRAME-12".to_owned(), "LOADER-13".to_owned());
        assert!(slot.take_root_network_idle_binding().is_none());
        assert_eq!(
            slot.take_root_frame_stopped_loading_binding()
                .expect("frame-stop observation")
                .loader_id,
            "LOADER-12"
        );
        assert!(slot.take_root_frame_stopped_loading_binding().is_none());
        assert!(slot.clear_pending_document_navigation_if_loader_matches("LOADER-13"));
        assert_eq!(
            slot.take_root_network_idle_binding()
                .expect("network-idle observation after provisional navigation failure")
                .frame_id,
            "FRAME-12"
        );
        assert!(slot.take_root_network_idle_binding().is_none());
    }
}
