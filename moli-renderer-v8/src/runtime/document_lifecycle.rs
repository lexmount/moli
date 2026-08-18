use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use super::PageId;
use tracing::trace;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RendererFrameToken {
    pub page_id: PageId,
}

impl RendererFrameToken {
    pub(crate) fn root(page_id: PageId) -> Self {
        Self { page_id }
    }
}

/// Identifies one cross-document root lifecycle within a Page.
///
/// `document.open()` keeps this token and advances [`RendererLifecycleEpoch`];
/// a cross-document commit allocates a new opaque lifecycle Document id.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RendererDocumentToken {
    pub page_id: PageId,
    lifecycle_document_id: RendererLifecycleDocumentId,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct RendererLifecycleDocumentId(u64);

impl RendererLifecycleDocumentId {
    const INITIAL: Self = Self(1);

    const fn successor(self) -> Self {
        match self.0.checked_add(1) {
            Some(value) => Self(value),
            None => panic!("renderer lifecycle Document id overflow"),
        }
    }
}

impl RendererDocumentToken {
    const fn initial(page_id: PageId) -> Self {
        Self {
            page_id,
            lifecycle_document_id: RendererLifecycleDocumentId::INITIAL,
        }
    }

    const fn with_id(page_id: PageId, lifecycle_document_id: RendererLifecycleDocumentId) -> Self {
        Self {
            page_id,
            lifecycle_document_id,
        }
    }

    #[doc(hidden)]
    pub const fn new_for_testing(page_id: PageId, lifecycle_document_id: u64) -> Self {
        Self::with_id(page_id, RendererLifecycleDocumentId(lifecycle_document_id))
    }

    #[doc(hidden)]
    pub const fn successor_for_testing(self) -> Self {
        Self::with_id(self.page_id, self.lifecycle_document_id.successor())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RendererLifecycleEpoch(pub u64);

impl RendererLifecycleEpoch {
    const fn successor(self) -> Self {
        match self.0.checked_add(1) {
            Some(value) => Self(value),
            None => panic!("renderer lifecycle epoch overflow"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererLifecycleStartReason {
    InitialDocument,
    CrossDocumentCommit,
    ExplicitDocumentOpen,
    JavascriptDocumentReplacement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererDocumentLifecycleMilestone {
    DomContentLoaded,
    Load,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererDocumentTerminationReason {
    SupersededByCrossDocumentNavigation,
    RestartedByDocumentOpen,
    ReplacedByJavascriptResult,
    MainResourceLoadFailed,
    Stopped,
    Detached,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererDocumentLifecycleEventKind {
    Started {
        reason: RendererLifecycleStartReason,
    },
    Milestone(RendererDocumentLifecycleMilestone),
    Terminated {
        last_reached: Option<RendererDocumentLifecycleMilestone>,
        reason: RendererDocumentTerminationReason,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererDocumentLifecycleEvent {
    pub frame: RendererFrameToken,
    pub document: RendererDocumentToken,
    pub epoch: RendererLifecycleEpoch,
    pub sequence: u64,
    pub timestamp_micros: u64,
    pub kind: RendererDocumentLifecycleEventKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererLifecycleEventStamp {
    pub sequence: u64,
    pub timestamp_micros: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererLifecycleTerminationStamp {
    pub sequence: u64,
    pub timestamp_micros: u64,
    pub reason: RendererDocumentTerminationReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererDocumentLifecycleSnapshot {
    pub frame: RendererFrameToken,
    pub document: RendererDocumentToken,
    pub epoch: RendererLifecycleEpoch,
    pub started: RendererLifecycleEventStamp,
    pub dom_content_loaded: Option<RendererLifecycleEventStamp>,
    pub load: Option<RendererLifecycleEventStamp>,
    pub terminated: Option<RendererLifecycleTerminationStamp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererPageCreationArtifacts {
    pub active_document: RendererDocumentToken,
    pub active_epoch: RendererLifecycleEpoch,
    pub lifecycle_snapshot: RendererDocumentLifecycleSnapshot,
    pub initial_lifecycle_events: Vec<RendererDocumentLifecycleEvent>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererDocumentLifecycleWaitOutcome {
    Pending,
    Reached(RendererLifecycleEventStamp),
    Interrupted(RendererLifecycleTerminationStamp),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererDocumentLifecycleWaiter {
    frame: RendererFrameToken,
    document: RendererDocumentToken,
    epoch: RendererLifecycleEpoch,
    milestone: RendererDocumentLifecycleMilestone,
    outcome: RendererDocumentLifecycleWaitOutcome,
}

impl RendererDocumentLifecycleWaiter {
    pub fn from_snapshot(
        snapshot: RendererDocumentLifecycleSnapshot,
        milestone: RendererDocumentLifecycleMilestone,
    ) -> Self {
        let reached = match milestone {
            RendererDocumentLifecycleMilestone::DomContentLoaded => snapshot.dom_content_loaded,
            RendererDocumentLifecycleMilestone::Load => snapshot.load,
        };
        let outcome = reached
            .map(RendererDocumentLifecycleWaitOutcome::Reached)
            .or_else(|| {
                snapshot
                    .terminated
                    .map(RendererDocumentLifecycleWaitOutcome::Interrupted)
            })
            .unwrap_or(RendererDocumentLifecycleWaitOutcome::Pending);
        Self {
            frame: snapshot.frame,
            document: snapshot.document,
            epoch: snapshot.epoch,
            milestone,
            outcome,
        }
    }

    pub fn observe(&mut self, event: RendererDocumentLifecycleEvent) {
        if !matches!(self.outcome, RendererDocumentLifecycleWaitOutcome::Pending)
            || event.frame != self.frame
            || event.document != self.document
            || event.epoch != self.epoch
        {
            return;
        }
        match event.kind {
            RendererDocumentLifecycleEventKind::Milestone(milestone)
                if milestone == self.milestone =>
            {
                self.outcome =
                    RendererDocumentLifecycleWaitOutcome::Reached(RendererLifecycleEventStamp {
                        sequence: event.sequence,
                        timestamp_micros: event.timestamp_micros,
                    });
            }
            RendererDocumentLifecycleEventKind::Terminated { reason, .. } => {
                self.outcome = RendererDocumentLifecycleWaitOutcome::Interrupted(
                    RendererLifecycleTerminationStamp {
                        sequence: event.sequence,
                        timestamp_micros: event.timestamp_micros,
                        reason,
                    },
                );
            }
            _ => {}
        }
    }

    pub fn outcome(self) -> RendererDocumentLifecycleWaitOutcome {
        self.outcome
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererDocumentLifecycleIdentity {
    pub frame: RendererFrameToken,
    pub document: RendererDocumentToken,
    pub epoch: RendererLifecycleEpoch,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererDocumentLifecycleDriveAdmissionId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererDocumentLifecycleDriveAdmission {
    pub(crate) id: RendererDocumentLifecycleDriveAdmissionId,
    pub(crate) from: RendererDocumentLifecycleIdentity,
    pub(crate) to: RendererDocumentLifecycleIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RendererDocumentLifecycleDriveAdmissionState {
    Pending(RendererDocumentLifecycleDriveAdmission),
    Active(RendererDocumentLifecycleDriveAdmission),
}

impl From<RendererDocumentLifecycleSnapshot> for RendererDocumentLifecycleIdentity {
    fn from(snapshot: RendererDocumentLifecycleSnapshot) -> Self {
        Self {
            frame: snapshot.frame,
            document: snapshot.document,
            epoch: snapshot.epoch,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MilestoneDispatch {
    identity: RendererDocumentLifecycleIdentity,
    milestone: RendererDocumentLifecycleMilestone,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DeferredLifecycleRestart {
    start_reason: RendererLifecycleStartReason,
    termination_reason: RendererDocumentTerminationReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererDocumentLifecycleTransition {
    DispatchStarted,
    DispatchCancelled,
    Recorded(u64),
    Deferred,
    Duplicate,
    RejectedStaleIdentity,
    RejectedTerminated,
    RejectedOutOfOrder,
    RejectedDispatchMismatch,
}

#[derive(Debug)]
pub(crate) struct RendererDocumentLifecycleJournal {
    frame: RendererFrameToken,
    current_snapshot: RendererDocumentLifecycleSnapshot,
    next_document_id: RendererLifecycleDocumentId,
    next_sequence: u64,
    initial_handoff_complete: bool,
    initial_events: VecDeque<RendererDocumentLifecycleEvent>,
    live_events: VecDeque<RendererDocumentLifecycleEvent>,
    active_dispatch: Option<MilestoneDispatch>,
    pending_completion: Option<MilestoneDispatch>,
    deferred_termination: Option<RendererDocumentTerminationReason>,
    deferred_restarts: VecDeque<DeferredLifecycleRestart>,
    command_turn_output: Option<super::RendererCommandTurnOutputRecorder>,
    output_journal: Option<super::RendererTurnOutputJournal>,
    next_document_open_start_reason: RendererLifecycleStartReason,
    pending_document_open_error: Option<RendererDocumentLifecycleTransition>,
    document_replacement_drive_admission: Option<RendererDocumentLifecycleDriveAdmissionState>,
}

#[derive(Clone, Debug)]
pub(crate) struct RendererDocumentLifecycleJournalHandle(
    Rc<RefCell<RendererDocumentLifecycleJournal>>,
);

impl RendererDocumentLifecycleJournalHandle {
    pub(crate) fn new_initial(page_id: PageId) -> Self {
        Self(Rc::new(RefCell::new(
            RendererDocumentLifecycleJournal::new_initial_at(page_id, monotonic_timestamp_micros()),
        )))
    }

    pub(crate) fn identity(&self) -> RendererDocumentLifecycleIdentity {
        self.0.borrow().current_snapshot.into()
    }

    pub(crate) fn bind_output_journal(&self, output_journal: super::RendererTurnOutputJournal) {
        let mut journal = self.0.borrow_mut();
        if let Some(existing) = &journal.output_journal {
            assert_eq!(
                existing.stream(),
                output_journal.stream(),
                "one lifecycle journal cannot change renderer output streams"
            );
            return;
        }
        journal.output_journal = Some(output_journal);
    }

    pub(crate) fn current_snapshot(&self) -> RendererDocumentLifecycleSnapshot {
        self.0.borrow().current_snapshot
    }

    pub(crate) fn pending_document_replacement_drive_admission(
        &self,
    ) -> Option<RendererDocumentLifecycleDriveAdmission> {
        match self.0.borrow().document_replacement_drive_admission {
            Some(RendererDocumentLifecycleDriveAdmissionState::Pending(admission)) => {
                Some(admission)
            }
            Some(RendererDocumentLifecycleDriveAdmissionState::Active(_)) | None => None,
        }
    }

    pub(crate) fn activate_document_replacement_drive_admission(
        &self,
        expected: RendererDocumentLifecycleDriveAdmission,
    ) -> bool {
        let mut journal = self.0.borrow_mut();
        if journal.document_replacement_drive_admission
            != Some(RendererDocumentLifecycleDriveAdmissionState::Pending(
                expected,
            ))
            || journal.identity() != expected.to
        {
            return false;
        }
        journal.document_replacement_drive_admission = Some(
            RendererDocumentLifecycleDriveAdmissionState::Active(expected),
        );
        true
    }

    pub(crate) fn active_document_replacement_drive_identity(
        &self,
    ) -> Option<RendererDocumentLifecycleIdentity> {
        match self.0.borrow().document_replacement_drive_admission {
            Some(RendererDocumentLifecycleDriveAdmissionState::Active(admission)) => {
                Some(admission.to)
            }
            Some(RendererDocumentLifecycleDriveAdmissionState::Pending(_)) | None => None,
        }
    }

    pub(crate) fn take_page_creation_artifacts(&self) -> RendererPageCreationArtifacts {
        self.0.borrow_mut().take_page_creation_artifacts()
    }

    #[cfg(test)]
    pub(crate) fn drain_live_events(&self) -> Vec<RendererDocumentLifecycleEvent> {
        self.0.borrow_mut().drain_live_events()
    }

    pub(crate) fn begin_milestone_dispatch(
        &self,
        identity: RendererDocumentLifecycleIdentity,
        milestone: RendererDocumentLifecycleMilestone,
    ) -> RendererDocumentLifecycleTransition {
        self.0
            .borrow_mut()
            .begin_milestone_dispatch(identity, milestone)
    }

    pub(crate) fn complete_milestone_dispatch(
        &self,
        identity: RendererDocumentLifecycleIdentity,
        milestone: RendererDocumentLifecycleMilestone,
    ) -> RendererDocumentLifecycleTransition {
        self.0.borrow_mut().complete_milestone_dispatch_at(
            identity,
            milestone,
            monotonic_timestamp_micros(),
        )
    }

    pub(crate) fn defer_milestone_completion(
        &self,
        identity: RendererDocumentLifecycleIdentity,
        milestone: RendererDocumentLifecycleMilestone,
    ) -> RendererDocumentLifecycleTransition {
        self.0.borrow_mut().defer_milestone_completion_at(
            identity,
            milestone,
            monotonic_timestamp_micros(),
        )
    }

    pub(crate) fn complete_pending_milestone(
        &self,
        identity: RendererDocumentLifecycleIdentity,
        milestone: RendererDocumentLifecycleMilestone,
    ) -> RendererDocumentLifecycleTransition {
        self.0.borrow_mut().complete_pending_milestone_at(
            identity,
            milestone,
            monotonic_timestamp_micros(),
        )
    }

    pub(crate) fn cancel_milestone_dispatch(
        &self,
        identity: RendererDocumentLifecycleIdentity,
        milestone: RendererDocumentLifecycleMilestone,
    ) -> RendererDocumentLifecycleTransition {
        self.0
            .borrow_mut()
            .cancel_milestone_dispatch(identity, milestone)
    }

    pub(crate) fn request_termination(
        &self,
        identity: RendererDocumentLifecycleIdentity,
        reason: RendererDocumentTerminationReason,
    ) -> RendererDocumentLifecycleTransition {
        self.0
            .borrow_mut()
            .request_termination_at(identity, reason, monotonic_timestamp_micros())
    }

    pub(crate) fn start_cross_document(
        &self,
    ) -> Result<RendererDocumentLifecycleIdentity, RendererDocumentLifecycleTransition> {
        self.0.borrow_mut().start_cross_document_at(
            RendererLifecycleStartReason::CrossDocumentCommit,
            monotonic_timestamp_micros(),
        )
    }

    pub(crate) fn begin_command_turn_output(
        &self,
        recorder: super::RendererCommandTurnOutputRecorder,
    ) -> anyhow::Result<()> {
        let mut journal = self.0.borrow_mut();
        anyhow::ensure!(
            journal.command_turn_output.is_none(),
            "renderer command-turn output scopes cannot overlap"
        );
        journal.command_turn_output = Some(recorder);
        Ok(())
    }

    pub(crate) fn end_command_turn_output(
        &self,
        recorder: &super::RendererCommandTurnOutputRecorder,
    ) {
        let mut journal = self.0.borrow_mut();
        if journal
            .command_turn_output
            .as_ref()
            .is_some_and(|active| active.records_into_same_sink(recorder))
        {
            journal.command_turn_output = None;
        }
    }

    pub(crate) fn set_next_document_open_start_reason(&self, reason: RendererLifecycleStartReason) {
        self.0.borrow_mut().next_document_open_start_reason = reason;
    }

    pub(crate) fn did_open_document(&self) {
        let mut journal = self.0.borrow_mut();
        let start_reason = std::mem::replace(
            &mut journal.next_document_open_start_reason,
            RendererLifecycleStartReason::ExplicitDocumentOpen,
        );
        let termination_reason = match start_reason {
            RendererLifecycleStartReason::JavascriptDocumentReplacement => {
                RendererDocumentTerminationReason::ReplacedByJavascriptResult
            }
            RendererLifecycleStartReason::ExplicitDocumentOpen => {
                RendererDocumentTerminationReason::RestartedByDocumentOpen
            }
            RendererLifecycleStartReason::InitialDocument
            | RendererLifecycleStartReason::CrossDocumentCommit => {
                journal.pending_document_open_error =
                    Some(RendererDocumentLifecycleTransition::RejectedOutOfOrder);
                return;
            }
        };
        match journal.request_restart_current_document_at(
            start_reason,
            termination_reason,
            monotonic_timestamp_micros(),
        ) {
            RendererDocumentLifecycleTransition::Recorded(_)
            | RendererDocumentLifecycleTransition::Deferred => {}
            transition => journal.pending_document_open_error = Some(transition),
        }
    }

    pub(crate) fn take_pending_document_open_error(
        &self,
    ) -> Option<RendererDocumentLifecycleTransition> {
        self.0.borrow_mut().pending_document_open_error.take()
    }
}

impl RendererDocumentLifecycleJournal {
    fn new_initial_at(page_id: PageId, timestamp_micros: u64) -> Self {
        let frame = RendererFrameToken::root(page_id);
        let document = RendererDocumentToken::initial(page_id);
        let epoch = RendererLifecycleEpoch(1);
        let started = RendererLifecycleEventStamp {
            sequence: 1,
            timestamp_micros,
        };
        let event = RendererDocumentLifecycleEvent {
            frame,
            document,
            epoch,
            sequence: started.sequence,
            timestamp_micros: started.timestamp_micros,
            kind: RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        };
        trace_lifecycle_transition(&event);
        Self {
            frame,
            current_snapshot: RendererDocumentLifecycleSnapshot {
                frame,
                document,
                epoch,
                started,
                dom_content_loaded: None,
                load: None,
                terminated: None,
            },
            next_document_id: RendererLifecycleDocumentId::INITIAL.successor(),
            next_sequence: 2,
            initial_handoff_complete: false,
            initial_events: VecDeque::from([event]),
            live_events: VecDeque::new(),
            active_dispatch: None,
            pending_completion: None,
            deferred_termination: None,
            deferred_restarts: VecDeque::new(),
            command_turn_output: None,
            output_journal: None,
            next_document_open_start_reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
            pending_document_open_error: None,
            document_replacement_drive_admission: None,
        }
    }

    fn identity(&self) -> RendererDocumentLifecycleIdentity {
        self.current_snapshot.into()
    }

    fn identity_matches(&self, identity: RendererDocumentLifecycleIdentity) -> bool {
        self.identity() == identity
    }

    fn take_page_creation_artifacts(&mut self) -> RendererPageCreationArtifacts {
        self.initial_handoff_complete = true;
        RendererPageCreationArtifacts {
            active_document: self.current_snapshot.document,
            active_epoch: self.current_snapshot.epoch,
            lifecycle_snapshot: self.current_snapshot,
            initial_lifecycle_events: self.initial_events.drain(..).collect(),
        }
    }

    #[cfg(test)]
    fn drain_live_events(&mut self) -> Vec<RendererDocumentLifecycleEvent> {
        self.live_events.drain(..).collect()
    }

    fn begin_milestone_dispatch(
        &mut self,
        identity: RendererDocumentLifecycleIdentity,
        milestone: RendererDocumentLifecycleMilestone,
    ) -> RendererDocumentLifecycleTransition {
        if !self.identity_matches(identity) {
            return RendererDocumentLifecycleTransition::RejectedStaleIdentity;
        }
        if self.current_snapshot.terminated.is_some() {
            return RendererDocumentLifecycleTransition::RejectedTerminated;
        }
        if self.active_dispatch.is_some() || self.pending_completion.is_some() {
            return RendererDocumentLifecycleTransition::RejectedDispatchMismatch;
        }
        match milestone {
            RendererDocumentLifecycleMilestone::DomContentLoaded
                if self.current_snapshot.dom_content_loaded.is_some() =>
            {
                return RendererDocumentLifecycleTransition::Duplicate;
            }
            RendererDocumentLifecycleMilestone::Load if self.current_snapshot.load.is_some() => {
                return RendererDocumentLifecycleTransition::Duplicate;
            }
            RendererDocumentLifecycleMilestone::Load
                if self.current_snapshot.dom_content_loaded.is_none() =>
            {
                return RendererDocumentLifecycleTransition::RejectedOutOfOrder;
            }
            _ => {}
        }
        self.active_dispatch = Some(MilestoneDispatch {
            identity,
            milestone,
        });
        RendererDocumentLifecycleTransition::DispatchStarted
    }

    fn complete_milestone_dispatch_at(
        &mut self,
        identity: RendererDocumentLifecycleIdentity,
        milestone: RendererDocumentLifecycleMilestone,
        timestamp_micros: u64,
    ) -> RendererDocumentLifecycleTransition {
        if self.active_dispatch
            != Some(MilestoneDispatch {
                identity,
                milestone,
            })
        {
            return RendererDocumentLifecycleTransition::RejectedDispatchMismatch;
        }
        self.active_dispatch = None;
        self.record_milestone_at(identity, milestone, timestamp_micros)
    }

    fn defer_milestone_completion_at(
        &mut self,
        identity: RendererDocumentLifecycleIdentity,
        milestone: RendererDocumentLifecycleMilestone,
        timestamp_micros: u64,
    ) -> RendererDocumentLifecycleTransition {
        let dispatch = MilestoneDispatch {
            identity,
            milestone,
        };
        if self.active_dispatch != Some(dispatch) || self.pending_completion.is_some() {
            return RendererDocumentLifecycleTransition::RejectedDispatchMismatch;
        }
        if self.deferred_termination.is_some() {
            return self.complete_milestone_dispatch_at(identity, milestone, timestamp_micros);
        }
        self.active_dispatch = None;
        self.pending_completion = Some(dispatch);
        RendererDocumentLifecycleTransition::Deferred
    }

    fn complete_pending_milestone_at(
        &mut self,
        identity: RendererDocumentLifecycleIdentity,
        milestone: RendererDocumentLifecycleMilestone,
        timestamp_micros: u64,
    ) -> RendererDocumentLifecycleTransition {
        if self.pending_completion
            != Some(MilestoneDispatch {
                identity,
                milestone,
            })
        {
            return RendererDocumentLifecycleTransition::RejectedDispatchMismatch;
        }
        self.pending_completion = None;
        self.record_milestone_at(identity, milestone, timestamp_micros)
    }

    fn record_milestone_at(
        &mut self,
        identity: RendererDocumentLifecycleIdentity,
        milestone: RendererDocumentLifecycleMilestone,
        timestamp_micros: u64,
    ) -> RendererDocumentLifecycleTransition {
        if !self.identity_matches(identity) {
            self.deferred_termination = None;
            return RendererDocumentLifecycleTransition::RejectedStaleIdentity;
        }
        if self.current_snapshot.terminated.is_some() {
            self.deferred_termination = None;
            return RendererDocumentLifecycleTransition::RejectedTerminated;
        }

        let sequence = self.next_sequence();
        let stamp = RendererLifecycleEventStamp {
            sequence,
            timestamp_micros,
        };
        match milestone {
            RendererDocumentLifecycleMilestone::DomContentLoaded => {
                if self
                    .current_snapshot
                    .dom_content_loaded
                    .replace(stamp)
                    .is_some()
                {
                    return RendererDocumentLifecycleTransition::Duplicate;
                }
            }
            RendererDocumentLifecycleMilestone::Load => {
                if self.current_snapshot.dom_content_loaded.is_none() {
                    return RendererDocumentLifecycleTransition::RejectedOutOfOrder;
                }
                if self.current_snapshot.load.replace(stamp).is_some() {
                    return RendererDocumentLifecycleTransition::Duplicate;
                }
            }
        }
        self.push_event(RendererDocumentLifecycleEvent {
            frame: identity.frame,
            document: identity.document,
            epoch: identity.epoch,
            sequence,
            timestamp_micros,
            kind: RendererDocumentLifecycleEventKind::Milestone(milestone),
        });

        if milestone == RendererDocumentLifecycleMilestone::Load
            && matches!(
                self.document_replacement_drive_admission,
                Some(RendererDocumentLifecycleDriveAdmissionState::Active(admission))
                    if admission.to == identity
            )
        {
            self.document_replacement_drive_admission = None;
        }

        let _ = self.finish_deferred_termination_or_restart(identity, timestamp_micros);
        RendererDocumentLifecycleTransition::Recorded(sequence)
    }

    fn cancel_milestone_dispatch(
        &mut self,
        identity: RendererDocumentLifecycleIdentity,
        milestone: RendererDocumentLifecycleMilestone,
    ) -> RendererDocumentLifecycleTransition {
        if self.active_dispatch
            != Some(MilestoneDispatch {
                identity,
                milestone,
            })
        {
            return RendererDocumentLifecycleTransition::RejectedDispatchMismatch;
        }
        self.active_dispatch = None;
        if self.deferred_termination.is_some() {
            return self
                .finish_deferred_termination_or_restart(identity, monotonic_timestamp_micros())
                .unwrap_or(RendererDocumentLifecycleTransition::DispatchCancelled);
        }
        RendererDocumentLifecycleTransition::DispatchCancelled
    }

    fn finish_deferred_termination_or_restart(
        &mut self,
        identity: RendererDocumentLifecycleIdentity,
        timestamp_micros: u64,
    ) -> Option<RendererDocumentLifecycleTransition> {
        let reason = self.deferred_termination.take()?;
        let termination = self.record_termination(identity, reason, timestamp_micros);
        if !matches!(
            termination,
            RendererDocumentLifecycleTransition::Recorded(_)
                | RendererDocumentLifecycleTransition::Duplicate
        ) {
            self.pending_document_open_error = Some(termination);
            self.deferred_restarts.clear();
            return Some(termination);
        }
        let mut restarts = std::mem::take(&mut self.deferred_restarts);
        let Some(restart) = restarts.pop_front() else {
            return Some(termination);
        };
        debug_assert_eq!(restart.termination_reason, reason);
        for restart in std::iter::once(restart).chain(restarts) {
            let from = self.identity();
            if self.current_snapshot.terminated.is_none() {
                let termination =
                    self.record_termination(from, restart.termination_reason, timestamp_micros);
                if !matches!(
                    termination,
                    RendererDocumentLifecycleTransition::Recorded(_)
                        | RendererDocumentLifecycleTransition::Duplicate
                ) {
                    self.pending_document_open_error = Some(termination);
                    break;
                }
            }
            let epoch = self.current_snapshot.epoch.successor();
            match self.start_lifecycle(
                self.current_snapshot.document,
                epoch,
                restart.start_reason,
                timestamp_micros,
            ) {
                Ok(to) => self.record_document_replacement_drive_admission(from, to),
                Err(transition) => {
                    self.pending_document_open_error = Some(transition);
                    break;
                }
            }
        }
        Some(termination)
    }

    fn record_document_replacement_drive_admission(
        &mut self,
        from: RendererDocumentLifecycleIdentity,
        to: RendererDocumentLifecycleIdentity,
    ) {
        let from = match self.document_replacement_drive_admission {
            Some(RendererDocumentLifecycleDriveAdmissionState::Pending(previous))
                if previous.to == from =>
            {
                previous.from
            }
            Some(RendererDocumentLifecycleDriveAdmissionState::Active(_)) | None => from,
            Some(RendererDocumentLifecycleDriveAdmissionState::Pending(previous)) => {
                debug_assert_eq!(
                    previous.to, from,
                    "consecutive document.open admissions must form one exact identity chain"
                );
                from
            }
        };
        self.document_replacement_drive_admission =
            Some(RendererDocumentLifecycleDriveAdmissionState::Pending(
                RendererDocumentLifecycleDriveAdmission {
                    id: RendererDocumentLifecycleDriveAdmissionId(
                        self.current_snapshot.started.sequence,
                    ),
                    from,
                    to,
                },
            ));
    }

    fn request_termination_at(
        &mut self,
        identity: RendererDocumentLifecycleIdentity,
        reason: RendererDocumentTerminationReason,
        timestamp_micros: u64,
    ) -> RendererDocumentLifecycleTransition {
        if !self.identity_matches(identity) {
            return RendererDocumentLifecycleTransition::RejectedStaleIdentity;
        }
        if self.current_snapshot.terminated.is_some() {
            return RendererDocumentLifecycleTransition::Duplicate;
        }
        if self
            .active_dispatch
            .is_some_and(|dispatch| dispatch.identity == identity)
        {
            self.deferred_termination.get_or_insert(reason);
            return RendererDocumentLifecycleTransition::Deferred;
        }
        self.record_termination(identity, reason, timestamp_micros)
    }

    fn record_termination(
        &mut self,
        identity: RendererDocumentLifecycleIdentity,
        reason: RendererDocumentTerminationReason,
        timestamp_micros: u64,
    ) -> RendererDocumentLifecycleTransition {
        if !self.identity_matches(identity) {
            return RendererDocumentLifecycleTransition::RejectedStaleIdentity;
        }
        if self.current_snapshot.terminated.is_some() {
            return RendererDocumentLifecycleTransition::Duplicate;
        }
        if self
            .pending_completion
            .is_some_and(|completion| completion.identity == identity)
        {
            self.pending_completion = None;
        }
        let last_reached = if self.current_snapshot.load.is_some() {
            Some(RendererDocumentLifecycleMilestone::Load)
        } else if self.current_snapshot.dom_content_loaded.is_some() {
            Some(RendererDocumentLifecycleMilestone::DomContentLoaded)
        } else {
            None
        };
        let sequence = self.next_sequence();
        self.current_snapshot.terminated = Some(RendererLifecycleTerminationStamp {
            sequence,
            timestamp_micros,
            reason,
        });
        self.push_event(RendererDocumentLifecycleEvent {
            frame: identity.frame,
            document: identity.document,
            epoch: identity.epoch,
            sequence,
            timestamp_micros,
            kind: RendererDocumentLifecycleEventKind::Terminated {
                last_reached,
                reason,
            },
        });
        if !matches!(
            reason,
            RendererDocumentTerminationReason::RestartedByDocumentOpen
                | RendererDocumentTerminationReason::ReplacedByJavascriptResult
        ) && matches!(
            self.document_replacement_drive_admission,
            Some(RendererDocumentLifecycleDriveAdmissionState::Pending(admission)
                | RendererDocumentLifecycleDriveAdmissionState::Active(admission))
                if admission.to == identity
        ) {
            self.document_replacement_drive_admission = None;
        }
        RendererDocumentLifecycleTransition::Recorded(sequence)
    }

    fn start_cross_document_at(
        &mut self,
        reason: RendererLifecycleStartReason,
        timestamp_micros: u64,
    ) -> Result<RendererDocumentLifecycleIdentity, RendererDocumentLifecycleTransition> {
        if self.active_dispatch.is_some() || self.pending_completion.is_some() {
            return Err(RendererDocumentLifecycleTransition::RejectedDispatchMismatch);
        }
        if self.current_snapshot.terminated.is_none() {
            return Err(RendererDocumentLifecycleTransition::RejectedOutOfOrder);
        }
        let document_id = self.next_document_id;
        self.next_document_id = document_id.successor();
        let document = RendererDocumentToken::with_id(self.frame.page_id, document_id);
        self.start_lifecycle(
            document,
            RendererLifecycleEpoch(1),
            reason,
            timestamp_micros,
        )
    }

    fn restart_current_document_at(
        &mut self,
        start_reason: RendererLifecycleStartReason,
        termination_reason: RendererDocumentTerminationReason,
        timestamp_micros: u64,
    ) -> Result<RendererDocumentLifecycleIdentity, RendererDocumentLifecycleTransition> {
        let from = self.identity();
        match self.request_termination_at(from, termination_reason, timestamp_micros) {
            RendererDocumentLifecycleTransition::Recorded(_)
            | RendererDocumentLifecycleTransition::Duplicate => {}
            transition => return Err(transition),
        }
        let epoch = self.current_snapshot.epoch.successor();
        let to = self.start_lifecycle(
            self.current_snapshot.document,
            epoch,
            start_reason,
            timestamp_micros,
        )?;
        self.record_document_replacement_drive_admission(from, to);
        Ok(to)
    }

    fn request_restart_current_document_at(
        &mut self,
        start_reason: RendererLifecycleStartReason,
        termination_reason: RendererDocumentTerminationReason,
        timestamp_micros: u64,
    ) -> RendererDocumentLifecycleTransition {
        if self.active_dispatch.is_some() {
            if self
                .deferred_termination
                .is_some_and(|reason| reason != termination_reason)
            {
                return RendererDocumentLifecycleTransition::RejectedOutOfOrder;
            }
            if self.deferred_restarts.is_empty() {
                let identity = self.identity();
                let transition =
                    self.request_termination_at(identity, termination_reason, timestamp_micros);
                if transition != RendererDocumentLifecycleTransition::Deferred {
                    return transition;
                }
            }
            self.deferred_restarts.push_back(DeferredLifecycleRestart {
                start_reason,
                termination_reason,
            });
            return RendererDocumentLifecycleTransition::Deferred;
        }
        match self.restart_current_document_at(start_reason, termination_reason, timestamp_micros) {
            Ok(_) => RendererDocumentLifecycleTransition::Recorded(
                self.current_snapshot.started.sequence,
            ),
            Err(transition) => transition,
        }
    }

    fn start_lifecycle(
        &mut self,
        document: RendererDocumentToken,
        epoch: RendererLifecycleEpoch,
        reason: RendererLifecycleStartReason,
        timestamp_micros: u64,
    ) -> Result<RendererDocumentLifecycleIdentity, RendererDocumentLifecycleTransition> {
        if self.active_dispatch.is_some()
            || self.pending_completion.is_some()
            || self.deferred_termination.is_some()
        {
            return Err(RendererDocumentLifecycleTransition::RejectedDispatchMismatch);
        }
        let sequence = self.next_sequence();
        let started = RendererLifecycleEventStamp {
            sequence,
            timestamp_micros,
        };
        self.current_snapshot = RendererDocumentLifecycleSnapshot {
            frame: self.frame,
            document,
            epoch,
            started,
            dom_content_loaded: None,
            load: None,
            terminated: None,
        };
        self.push_event(RendererDocumentLifecycleEvent {
            frame: self.frame,
            document,
            epoch,
            sequence,
            timestamp_micros,
            kind: RendererDocumentLifecycleEventKind::Started { reason },
        });
        Ok(self.identity())
    }

    fn next_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("renderer lifecycle event sequence overflow");
        sequence
    }

    fn push_event(&mut self, event: RendererDocumentLifecycleEvent) {
        trace_lifecycle_transition(&event);
        if self.initial_handoff_complete {
            if let Some(recorder) = &self.command_turn_output {
                recorder.push_document_lifecycle_event(event);
                return;
            }
            if let Some(output_journal) = &self.output_journal {
                output_journal.append(super::PendingRendererOutputRecord::observation(
                    None,
                    super::RendererProtocolObservation::DocumentLifecycle(event),
                ));
                return;
            }
            self.live_events.push_back(event);
        } else {
            self.initial_events.push_back(event);
        }
    }
}

fn monotonic_timestamp_micros() -> u64 {
    moli_time::monotonic_timestamp_micros()
}

fn trace_lifecycle_transition(event: &RendererDocumentLifecycleEvent) {
    trace!(
        target: "moli_renderer_document_lifecycle",
        page_id = event.document.page_id.as_u64(),
        lifecycle_document_id = event.document.lifecycle_document_id.0,
        lifecycle_epoch = event.epoch.0,
        sequence = event.sequence,
        timestamp_micros = event.timestamp_micros,
        kind = ?event.kind,
        "renderer document lifecycle transition"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal() -> RendererDocumentLifecycleJournal {
        RendererDocumentLifecycleJournal::new_initial_at(PageId::new_for_testing(7), 10)
    }

    #[test]
    #[should_panic(expected = "renderer lifecycle Document id overflow")]
    fn lifecycle_document_id_allocator_rejects_overflow() {
        let _ = RendererLifecycleDocumentId(u64::MAX).successor();
    }

    #[test]
    #[should_panic(expected = "renderer lifecycle epoch overflow")]
    fn lifecycle_epoch_allocator_rejects_overflow() {
        let _ = RendererLifecycleEpoch(u64::MAX).successor();
    }

    #[test]
    #[should_panic(expected = "renderer lifecycle event sequence overflow")]
    fn lifecycle_event_sequence_allocator_rejects_overflow() {
        let mut journal = journal();
        journal.next_sequence = u64::MAX;
        let _ = journal.next_sequence();
    }

    fn finish(
        journal: &mut RendererDocumentLifecycleJournal,
        milestone: RendererDocumentLifecycleMilestone,
        timestamp_micros: u64,
    ) {
        let identity = journal.identity();
        assert_eq!(
            journal.begin_milestone_dispatch(identity, milestone),
            RendererDocumentLifecycleTransition::DispatchStarted
        );
        assert!(matches!(
            journal.complete_milestone_dispatch_at(identity, milestone, timestamp_micros),
            RendererDocumentLifecycleTransition::Recorded(_)
        ));
    }

    #[test]
    fn document_open_drive_admission_is_exact_and_retires_at_load() {
        let mut journal = journal();
        let from = journal.identity();
        assert!(matches!(
            journal.request_restart_current_document_at(
                RendererLifecycleStartReason::ExplicitDocumentOpen,
                RendererDocumentTerminationReason::RestartedByDocumentOpen,
                15,
            ),
            RendererDocumentLifecycleTransition::Recorded(_)
        ));
        let to = journal.identity();
        let admission = match journal.document_replacement_drive_admission {
            Some(RendererDocumentLifecycleDriveAdmissionState::Pending(admission)) => admission,
            state => panic!("document.open should publish one pending admission, got {state:?}"),
        };
        assert_eq!(admission.from, from);
        assert_eq!(admission.to, to);
        assert_eq!(admission.id.0, journal.current_snapshot.started.sequence);

        journal.document_replacement_drive_admission = Some(
            RendererDocumentLifecycleDriveAdmissionState::Active(admission),
        );
        finish(
            &mut journal,
            RendererDocumentLifecycleMilestone::DomContentLoaded,
            20,
        );
        assert!(journal.document_replacement_drive_admission.is_some());
        finish(&mut journal, RendererDocumentLifecycleMilestone::Load, 30);
        assert_eq!(journal.document_replacement_drive_admission, None);
    }

    #[test]
    fn consecutive_unsettled_document_open_restarts_coalesce_exact_identity_chain() {
        let mut journal = journal();
        let original = journal.identity();
        assert!(matches!(
            journal.request_restart_current_document_at(
                RendererLifecycleStartReason::ExplicitDocumentOpen,
                RendererDocumentTerminationReason::RestartedByDocumentOpen,
                15,
            ),
            RendererDocumentLifecycleTransition::Recorded(_)
        ));
        let intermediate = journal.identity();
        assert!(matches!(
            journal.request_restart_current_document_at(
                RendererLifecycleStartReason::ExplicitDocumentOpen,
                RendererDocumentTerminationReason::RestartedByDocumentOpen,
                20,
            ),
            RendererDocumentLifecycleTransition::Recorded(_)
        ));
        let current = journal.identity();
        let admission = match journal.document_replacement_drive_admission {
            Some(RendererDocumentLifecycleDriveAdmissionState::Pending(admission)) => admission,
            state => panic!("consecutive document.open should retain one admission, got {state:?}"),
        };
        assert_eq!(admission.from, original);
        assert_eq!(admission.to, current);
        assert_ne!(intermediate, current);
        assert_eq!(admission.id.0, journal.current_snapshot.started.sequence);
    }

    #[test]
    fn initial_handoff_preserves_renderer_stamps_and_order() {
        let mut journal = journal();
        finish(
            &mut journal,
            RendererDocumentLifecycleMilestone::DomContentLoaded,
            20,
        );
        finish(&mut journal, RendererDocumentLifecycleMilestone::Load, 30);

        let artifacts = journal.take_page_creation_artifacts();
        assert_eq!(
            artifacts
                .initial_lifecycle_events
                .iter()
                .map(|event| (event.sequence, event.timestamp_micros, event.kind))
                .collect::<Vec<_>>(),
            vec![
                (
                    1,
                    10,
                    RendererDocumentLifecycleEventKind::Started {
                        reason: RendererLifecycleStartReason::InitialDocument,
                    },
                ),
                (
                    2,
                    20,
                    RendererDocumentLifecycleEventKind::Milestone(
                        RendererDocumentLifecycleMilestone::DomContentLoaded,
                    ),
                ),
                (
                    3,
                    30,
                    RendererDocumentLifecycleEventKind::Milestone(
                        RendererDocumentLifecycleMilestone::Load,
                    ),
                ),
            ]
        );
        assert_eq!(
            artifacts.lifecycle_snapshot.load,
            Some(RendererLifecycleEventStamp {
                sequence: 3,
                timestamp_micros: 30,
            })
        );
    }

    #[test]
    fn load_milestone_can_wait_for_descendant_completion_after_callback_dispatch() {
        let mut journal = journal();
        finish(
            &mut journal,
            RendererDocumentLifecycleMilestone::DomContentLoaded,
            20,
        );
        let identity = journal.identity();
        assert_eq!(
            journal.begin_milestone_dispatch(identity, RendererDocumentLifecycleMilestone::Load,),
            RendererDocumentLifecycleTransition::DispatchStarted
        );
        assert_eq!(
            journal.defer_milestone_completion_at(
                identity,
                RendererDocumentLifecycleMilestone::Load,
                30,
            ),
            RendererDocumentLifecycleTransition::Deferred
        );
        assert!(journal.active_dispatch.is_none());
        assert_eq!(
            journal.pending_completion,
            Some(MilestoneDispatch {
                identity,
                milestone: RendererDocumentLifecycleMilestone::Load,
            })
        );
        assert!(journal.current_snapshot.load.is_none());

        assert_eq!(
            journal.complete_pending_milestone_at(
                identity,
                RendererDocumentLifecycleMilestone::Load,
                40,
            ),
            RendererDocumentLifecycleTransition::Recorded(3)
        );
        assert_eq!(
            journal.current_snapshot.load,
            Some(RendererLifecycleEventStamp {
                sequence: 3,
                timestamp_micros: 40,
            })
        );
    }

    #[test]
    fn navigation_cancels_pending_load_completion_without_waiting_for_descendant() {
        let mut journal = journal();
        finish(
            &mut journal,
            RendererDocumentLifecycleMilestone::DomContentLoaded,
            20,
        );
        let identity = journal.identity();
        assert_eq!(
            journal.begin_milestone_dispatch(identity, RendererDocumentLifecycleMilestone::Load,),
            RendererDocumentLifecycleTransition::DispatchStarted
        );
        assert_eq!(
            journal.defer_milestone_completion_at(
                identity,
                RendererDocumentLifecycleMilestone::Load,
                30,
            ),
            RendererDocumentLifecycleTransition::Deferred
        );

        assert_eq!(
            journal.request_termination_at(
                identity,
                RendererDocumentTerminationReason::SupersededByCrossDocumentNavigation,
                35,
            ),
            RendererDocumentLifecycleTransition::Recorded(3)
        );
        assert!(journal.pending_completion.is_none());
        assert!(journal.current_snapshot.load.is_none());
        assert_eq!(
            journal.current_snapshot.terminated,
            Some(RendererLifecycleTerminationStamp {
                sequence: 3,
                timestamp_micros: 35,
                reason: RendererDocumentTerminationReason::SupersededByCrossDocumentNavigation,
            })
        );
    }

    #[test]
    fn parser_navigation_before_dcl_terminates_without_milestone() {
        let mut journal = journal();
        let identity = journal.identity();
        assert_eq!(
            journal.request_termination_at(
                identity,
                RendererDocumentTerminationReason::SupersededByCrossDocumentNavigation,
                15,
            ),
            RendererDocumentLifecycleTransition::Recorded(2)
        );

        assert_eq!(
            journal
                .take_page_creation_artifacts()
                .initial_lifecycle_events
                .into_iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![
                RendererDocumentLifecycleEventKind::Started {
                    reason: RendererLifecycleStartReason::InitialDocument,
                },
                RendererDocumentLifecycleEventKind::Terminated {
                    last_reached: None,
                    reason: RendererDocumentTerminationReason::SupersededByCrossDocumentNavigation,
                },
            ]
        );
    }

    #[test]
    fn dcl_handler_navigation_records_milestone_before_termination() {
        let mut journal = journal();
        let identity = journal.identity();
        assert_eq!(
            journal.begin_milestone_dispatch(
                identity,
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
            RendererDocumentLifecycleTransition::DispatchStarted
        );
        assert_eq!(
            journal.request_termination_at(
                identity,
                RendererDocumentTerminationReason::SupersededByCrossDocumentNavigation,
                15,
            ),
            RendererDocumentLifecycleTransition::Deferred
        );
        assert_eq!(
            journal.complete_milestone_dispatch_at(
                identity,
                RendererDocumentLifecycleMilestone::DomContentLoaded,
                20,
            ),
            RendererDocumentLifecycleTransition::Recorded(2)
        );

        assert_eq!(
            journal
                .take_page_creation_artifacts()
                .initial_lifecycle_events
                .into_iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![
                RendererDocumentLifecycleEventKind::Started {
                    reason: RendererLifecycleStartReason::InitialDocument,
                },
                RendererDocumentLifecycleEventKind::Milestone(
                    RendererDocumentLifecycleMilestone::DomContentLoaded,
                ),
                RendererDocumentLifecycleEventKind::Terminated {
                    last_reached: Some(RendererDocumentLifecycleMilestone::DomContentLoaded),
                    reason: RendererDocumentTerminationReason::SupersededByCrossDocumentNavigation,
                },
            ]
        );
    }

    #[test]
    fn document_open_during_milestone_dispatch_restarts_after_handler_completion() {
        let mut journal = journal();
        let initial = journal.identity();
        assert_eq!(
            journal.begin_milestone_dispatch(
                initial,
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
            RendererDocumentLifecycleTransition::DispatchStarted
        );
        assert_eq!(
            journal.request_restart_current_document_at(
                RendererLifecycleStartReason::ExplicitDocumentOpen,
                RendererDocumentTerminationReason::RestartedByDocumentOpen,
                15,
            ),
            RendererDocumentLifecycleTransition::Deferred
        );
        assert_eq!(
            journal.complete_milestone_dispatch_at(
                initial,
                RendererDocumentLifecycleMilestone::DomContentLoaded,
                20,
            ),
            RendererDocumentLifecycleTransition::Recorded(2)
        );

        let restarted = journal.identity();
        assert_eq!(restarted.document, initial.document);
        assert_eq!(restarted.epoch.0, initial.epoch.0 + 1);
        assert!(journal.pending_document_open_error.is_none());
        let kinds = journal
            .take_page_creation_artifacts()
            .initial_lifecycle_events
            .into_iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>();
        assert!(matches!(
            kinds.as_slice(),
            [
                RendererDocumentLifecycleEventKind::Started {
                    reason: RendererLifecycleStartReason::InitialDocument,
                },
                RendererDocumentLifecycleEventKind::Milestone(
                    RendererDocumentLifecycleMilestone::DomContentLoaded,
                ),
                RendererDocumentLifecycleEventKind::Terminated {
                    reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
                    ..
                },
                RendererDocumentLifecycleEventKind::Started {
                    reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
                },
            ]
        ));
    }

    #[test]
    fn stale_completion_cannot_mutate_replacement_document() {
        let mut journal = journal();
        let stale_identity = journal.identity();
        assert_eq!(
            journal.request_termination_at(
                stale_identity,
                RendererDocumentTerminationReason::SupersededByCrossDocumentNavigation,
                15,
            ),
            RendererDocumentLifecycleTransition::Recorded(2)
        );
        let replacement_identity = journal
            .start_cross_document_at(RendererLifecycleStartReason::CrossDocumentCommit, 20)
            .expect("terminated document should permit replacement");
        assert_ne!(stale_identity.document, replacement_identity.document);
        assert_eq!(
            journal.begin_milestone_dispatch(
                stale_identity,
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
            RendererDocumentLifecycleTransition::RejectedStaleIdentity
        );
        assert!(journal.current_snapshot.dom_content_loaded.is_none());
    }

    #[test]
    fn document_restart_keeps_token_and_increments_epoch() {
        let mut journal = journal();
        let first = journal.identity();
        let second = journal
            .restart_current_document_at(
                RendererLifecycleStartReason::ExplicitDocumentOpen,
                RendererDocumentTerminationReason::RestartedByDocumentOpen,
                20,
            )
            .expect("idle current document should restart");

        assert_eq!(first.document, second.document);
        assert_eq!(second.epoch, RendererLifecycleEpoch(first.epoch.0 + 1));
        assert_eq!(journal.current_snapshot.started.sequence, 3);
    }

    #[test]
    fn live_events_are_separate_from_creation_handoff() {
        let mut journal = journal();
        let artifacts = journal.take_page_creation_artifacts();
        assert_eq!(artifacts.initial_lifecycle_events.len(), 1);
        finish(
            &mut journal,
            RendererDocumentLifecycleMilestone::DomContentLoaded,
            20,
        );
        assert_eq!(journal.live_events.len(), 1);
        assert_eq!(journal.drain_live_events()[0].sequence, 2);
        assert!(journal.drain_live_events().is_empty());
    }

    #[test]
    fn waiter_uses_snapshot_then_rejects_stale_events() {
        let mut journal = journal();
        let snapshot = journal.current_snapshot;
        let mut waiter = RendererDocumentLifecycleWaiter::from_snapshot(
            snapshot,
            RendererDocumentLifecycleMilestone::DomContentLoaded,
        );
        assert_eq!(
            waiter.outcome(),
            RendererDocumentLifecycleWaitOutcome::Pending
        );

        let stale = RendererDocumentLifecycleEvent {
            document: snapshot.document.successor_for_testing(),
            kind: RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
            frame: snapshot.frame,
            epoch: snapshot.epoch,
            sequence: 2,
            timestamp_micros: 20,
        };
        waiter.observe(stale);
        assert_eq!(
            waiter.outcome(),
            RendererDocumentLifecycleWaitOutcome::Pending
        );

        finish(
            &mut journal,
            RendererDocumentLifecycleMilestone::DomContentLoaded,
            30,
        );
        waiter.observe(journal.initial_events.back().copied().unwrap());
        assert_eq!(
            waiter.outcome(),
            RendererDocumentLifecycleWaitOutcome::Reached(RendererLifecycleEventStamp {
                sequence: 2,
                timestamp_micros: 30,
            })
        );
    }
}
