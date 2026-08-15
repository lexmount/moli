use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::ptr::NonNull;
use std::rc::Rc;
#[cfg(debug_assertions)]
use std::thread::ThreadId;

use anyhow::Context;

use super::access::run_named_owner_local_task;
use super::document_lifecycle_turn::DocumentLifecycleObserverOutcome;
use super::lifecycle_decision::PendingLifecycleDecision;
use super::navigation::{
    PageCreationNavigationFailureObserver, PageCreationNavigationFailurePublication,
    PageCreationNavigationFailurePublisher, PageCreationResolution, PageCreationRetirement,
    PageNavigationOwnerFailure, page_creation_navigation_failure_scope,
};
use super::owner::{
    PageCommandFirstDispatchLane, RenderRuntimePendingTurn, RendererCreateStreamingRawPageRequest,
};
use super::owner_deadline_index::OwnerDeadlineIndex;
use super::owner_local::RendererAttachedPage;
use super::owner_maintenance::{
    RendererOwnerMaintenanceTask, RendererPageOwnerMaintenanceResidence,
};
use super::page_command_residence::PageCommandFirstDispatchResidence;
use super::page_entry_residence::{RendererPageEntryCheckout, RendererPageEntryRestore};
use super::page_turn_scheduler::{
    DocumentLifecycleClassReadiness, PageTurnAdmission, PageTurnClass, PageTurnScheduler,
    PageTurnTrigger, ScheduledPageTurnCheckout,
};
use super::page_vm::{
    DocumentLifecycleTurnAction, DocumentLifecycleTurnOutcome, DocumentLifecycleTurnReadiness,
    PageVmRuntimeCommandLifecycleAdvance, PageVmRuntimeCommandOutputScopeId,
    renderer_document_lifecycle_milestone_for_stage,
};
use super::phase_one::{ParseTimePageVmCreationOutcome, PendingPhaseOneResumeOutcome};
use super::*;
use crate::page_task_queue::{
    PostParsePageOwnedWork, RendererPageOwnedTaskSources, RendererPageSchedulerTask,
};
use crate::script_vm::{
    DocumentInspectorBinding, PendingRuntimeEvaluateCall, RendererDocumentIsolateBootstrap,
    RendererDocumentIsolateHandle, RendererDocumentIsolateReservationAccounting,
    RendererPageScriptEnvironment,
};
use crate::{RendererNavigationReplyPolicy, RendererTopLevelNavigationDispatch};
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum StandaloneNavigationFollowState {
    #[default]
    Idle,
    Following {
        handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
    },
    FailedWithPendingNavigation {
        handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PublishedReplacementDocument {
    pub(super) navigation_handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
    pub(super) vm_creation_id: u64,
    pub(super) view_generation: u64,
}

impl StandaloneNavigationFollowState {
    fn claim(
        &mut self,
        current: Option<crate::page_task_queue::RendererTopLevelNavigationHandoff>,
        requested: Option<crate::page_task_queue::RendererTopLevelNavigationHandoff>,
    ) -> bool {
        if matches!(
            *self,
            Self::FailedWithPendingNavigation { handoff } if Some(handoff) != current
        ) {
            *self = Self::Idle;
        }
        let Some(current) = current else {
            return false;
        };
        if requested.is_some_and(|requested| requested != current) {
            return false;
        }
        if !matches!(*self, Self::Idle) {
            return false;
        }
        *self = Self::Following { handoff: current };
        true
    }

    fn settle(
        &mut self,
        current: Option<crate::page_task_queue::RendererTopLevelNavigationHandoff>,
        succeeded: bool,
    ) {
        *self = match *self {
            Self::Following { .. } if !succeeded => current
                .map(|handoff| Self::FailedWithPendingNavigation { handoff })
                .unwrap_or(Self::Idle),
            Self::Following { .. } => Self::Idle,
            Self::FailedWithPendingNavigation { handoff } if Some(handoff) != current => Self::Idle,
            state => state,
        };
    }
}

pub(super) struct RendererPageLocalEntry {
    pub(super) slot: RendererPageSlotHandle,
    top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
    standalone_navigation_follow: StandaloneNavigationFollowState,
    // Keep executable continuation state before `vm`: Rust drops fields in
    // declaration order, so an exceptional entry drop still releases any
    // ScriptVm-bound task before releasing the PageVm itself.
    pending_document_lifecycle_turn: Option<PendingDocumentLifecycleTurn>,
    post_response_document_lifecycle: Option<RendererDocumentLifecycleIdentity>,
    pub(super) vm: Option<PageVm>,
    pending_phase_one_navigation: Option<PageVmPendingPhaseOneNavigation>,
    last_published_replacement_document: Option<PublishedReplacementDocument>,
}

impl std::fmt::Debug for RendererPageLocalEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererPageLocalEntry")
            .field("slot", &self.slot)
            .field(
                "top_level_navigation_dispatch",
                &self.top_level_navigation_dispatch,
            )
            .field(
                "standalone_navigation_follow",
                &self.standalone_navigation_follow,
            )
            .field("vm", &self.vm)
            .field(
                "pending_document_lifecycle_turn",
                &self
                    .pending_document_lifecycle_turn
                    .as_ref()
                    .map(|pending| pending.document),
            )
            .field(
                "post_response_document_lifecycle",
                &self.post_response_document_lifecycle,
            )
            .field(
                "has_pending_phase_one_navigation",
                &self.pending_phase_one_navigation.is_some(),
            )
            .field(
                "last_published_replacement_document",
                &self.last_published_replacement_document,
            )
            .finish()
    }
}

pub(super) type RendererPageLocalEntryCheckout =
    std::result::Result<RendererPageLocalEntry, RendererPageLocalEntryCheckoutError>;

pub(super) type RendererPageTurnCheckout = std::result::Result<
    (
        RendererPageLocalEntry,
        PageTurnTrigger,
        RendererPageScheduledTurn,
    ),
    RendererPageTurnCheckoutError,
>;

#[derive(Debug)]
pub(super) enum RendererPageScheduledTurn {
    Ordinary(Box<RendererPageSchedulerTask>),
    DocumentLifecycle {
        displaced_ordinary: RendererDisplacedOrdinaryTurn,
    },
    /// The admitted notification was already spent before arbitration found a
    /// concrete task. It carries no execution authority and can be settled
    /// directly after restoring the Page entry.
    SpentWake,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RendererDisplacedOrdinaryTurn {
    None,
    ReconsiderWhenLifecycleBlocks,
}

impl RendererDisplacedOrdinaryTurn {
    const fn from_ready_source(has_ready_source: bool) -> Self {
        if has_ready_source {
            Self::ReconsiderWhenLifecycleBlocks
        } else {
            Self::None
        }
    }

    pub(super) const fn requires_reconsideration(self) -> bool {
        !matches!(self, Self::None)
    }
}

pub(super) enum RendererPageLocalEntryCheckoutError {
    Busy,
    Retired,
    Missing,
}

pub(super) enum RendererPageTurnCheckoutError {
    NotScheduled,
    Busy,
    Retired,
    Missing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RendererPageTurnAdmission {
    EnqueueOwnerTurn,
    AlreadyScheduled,
    Retired,
    MissingPage,
}

pub(super) struct RendererPendingPageCreation {
    pub(super) token: RendererPageToken,
    navigation_failure_observer: PageCreationNavigationFailureObserver,
    page_context_cancel_tx: RendererPageContextCancelSender,
    pending_download: Option<RendererPendingDownloadActivation>,
    lifecycle_decider: Option<PendingLifecycleDecision>,
}

pub(super) struct RendererPageCreationResolution {
    outcome: PageCreationResolution<RendererPendingPageCreation, RendererAttachedPage>,
    renderer_output: Option<RendererOutputPublication>,
    retire_page_after_publication: bool,
}

impl RendererPageCreationResolution {
    fn without_renderer_output(
        outcome: PageCreationResolution<RendererPendingPageCreation, RendererAttachedPage>,
    ) -> Self {
        Self {
            outcome,
            renderer_output: None,
            retire_page_after_publication: false,
        }
    }

    fn retiring(
        failure: PageCreationRetirement,
        renderer_output: Option<RendererOutputPublication>,
    ) -> Self {
        Self {
            outcome: PageCreationResolution::Retired { failure },
            renderer_output,
            retire_page_after_publication: true,
        }
    }

    pub(super) fn publish_then_resolve(
        self,
        publish: impl FnOnce(RendererOutputPublication),
    ) -> (
        PageCreationResolution<RendererPendingPageCreation, RendererAttachedPage>,
        bool,
    ) {
        if let Some(output) = self.renderer_output {
            publish(output);
        }
        (self.outcome, self.retire_page_after_publication)
    }
}

impl RendererPendingPageCreation {
    pub(super) fn with_pending_download(
        mut self,
        download: RendererPendingDownloadActivation,
    ) -> Self {
        self.pending_download = Some(download);
        self
    }

    pub(super) fn with_lifecycle_decider(
        mut self,
        target_stage: PageVmInitStage,
        decider: Option<RendererLifecycleDecider>,
    ) -> Self {
        self.lifecycle_decider =
            decider.map(|decider| PendingLifecycleDecision::new(target_stage, decider));
        self
    }

    pub(super) fn has_lifecycle_decider(&self) -> bool {
        self.lifecycle_decider.is_some()
    }

    pub(super) fn take_lifecycle_decider(
        &mut self,
    ) -> Option<(PageVmInitStage, RendererLifecycleDecider)> {
        self.lifecycle_decider
            .take()
            .map(PendingLifecycleDecision::into_parts)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LifecycleGateTurnPolicy {
    Normal,
    Drive { reconsider_displaced_ordinary: bool },
    Park,
}

#[derive(Debug)]
struct LifecycleGate {
    target_stage: PageVmInitStage,
    parked_admitted_wake: bool,
    reconsider_ordinary_on_next_turn: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ReleasedLifecycleGate {
    pub(super) target_stage: PageVmInitStage,
    pub(super) resume_parked_page_turn: bool,
}

impl LifecycleGate {
    fn new(target_stage: PageVmInitStage) -> Self {
        Self {
            target_stage,
            parked_admitted_wake: false,
            reconsider_ordinary_on_next_turn: false,
        }
    }

    fn turn_policy(
        &mut self,
        entry: &mut RendererPageLocalEntry,
        has_eligible_ordinary_source: bool,
    ) -> LifecycleGateTurnPolicy {
        if entry.page_vm().vm().has_pending_location_navigation() {
            self.reconsider_ordinary_on_next_turn = false;
            return LifecycleGateTurnPolicy::Normal;
        }
        match entry.page_vm().document_lifecycle_wait_outcome(
            renderer_document_lifecycle_milestone_for_stage(self.target_stage),
        ) {
            RendererDocumentLifecycleWaitOutcome::Reached(_)
            | RendererDocumentLifecycleWaitOutcome::Interrupted(_) => {
                self.parked_admitted_wake = true;
                LifecycleGateTurnPolicy::Park
            }
            RendererDocumentLifecycleWaitOutcome::Pending
                if matches!(self.target_stage, PageVmInitStage::Load) =>
            {
                self.reconsider_ordinary_on_next_turn = false;
                LifecycleGateTurnPolicy::Normal
            }
            RendererDocumentLifecycleWaitOutcome::Pending
                if entry.pending_document_lifecycle_identity().is_some() =>
            {
                let reconsider_displaced_ordinary =
                    self.reconsider_ordinary_on_next_turn && has_eligible_ordinary_source;
                self.reconsider_ordinary_on_next_turn = false;
                LifecycleGateTurnPolicy::Drive {
                    reconsider_displaced_ordinary,
                }
            }
            RendererDocumentLifecycleWaitOutcome::Pending => {
                self.reconsider_ordinary_on_next_turn = false;
                LifecycleGateTurnPolicy::Normal
            }
        }
    }

    fn settle_lifecycle_turn(&mut self, reconsider_displaced_ordinary: bool) {
        // A lifecycle turn that remains runnable owns the next bounded action:
        // in particular, the interactive transition and DOMContentLoaded stay
        // in the same lifecycle chain. Ordinary work is reconsidered only
        // after the lifecycle reports that it cannot currently progress.
        self.reconsider_ordinary_on_next_turn = reconsider_displaced_ordinary;
    }
}

pub(super) struct RendererFinalizedPageCreation {
    pub(super) attached_page: RendererAttachedPage,
    pub(super) resume_parked_page_turn: bool,
}

pub(super) struct RendererPageCreationCommit {
    finalized: Result<RendererFinalizedPageCreation>,
    renderer_output: Option<RendererOutputPublication>,
}

impl RendererPageCreationCommit {
    pub(super) fn publish_then_finalize(
        self,
        publish: impl FnOnce(RendererOutputPublication),
    ) -> Result<RendererFinalizedPageCreation> {
        if let Some(output) = self.renderer_output {
            publish(output);
        }
        self.finalized
    }
}

pub(super) type NavigationReplyPolicy = RendererNavigationReplyPolicy;

pub(super) enum LivePagePendingNavigationCompletion {
    Background,
    PublishedPageCreation {
        navigation_reply_policy: NavigationReplyPolicy,
    },
    CompletePageCreation {
        pending: RendererPendingPageCreation,
        navigation_reply_policy: NavigationReplyPolicy,
    },
    ReplyWithSnapshot {
        reply: Box<RendererPageReply>,
        capture_policy: super::RendererPageStateCapturePolicy,
    },
    ContinueNetworkIdle {
        deadline: std::time::Instant,
        loader: ResourceRequestClient,
    },
    ContinueDomStable {
        deadline: std::time::Instant,
        loader: ResourceRequestClient,
    },
    ContinueSubresourceResponse {
        criteria: SubresourceResponseWaitCriteria,
        deadline: std::time::Instant,
        loader: ResourceRequestClient,
        capture_policy: super::RendererPageStateCapturePolicy,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LivePageNavigationFailureRecipient {
    Initiator,
    PageCreationObserver,
    Background,
}

impl LivePagePendingNavigationCompletion {
    pub(super) fn continues_committed_document_parser_prefix(&self) -> bool {
        matches!(self, Self::PublishedPageCreation { .. })
    }

    pub(super) fn chain_limit_error_context(&self) -> &'static str {
        match self {
            Self::Background | Self::PublishedPageCreation { .. } => {
                "running background navigation"
            }
            Self::CompletePageCreation { .. } => "creating page",
            Self::ReplyWithSnapshot { .. } => "refreshing page",
            Self::ContinueNetworkIdle { .. } => "waiting for networkidle",
            Self::ContinueDomStable { .. } => "waiting for domstable",
            Self::ContinueSubresourceResponse { .. } => "waiting for subresource response",
        }
    }

    pub(super) fn retires_page_on_navigation_failure(&self) -> bool {
        matches!(self, Self::CompletePageCreation { .. })
    }

    pub(super) fn failure_recipient(&self) -> LivePageNavigationFailureRecipient {
        match self {
            Self::Background => LivePageNavigationFailureRecipient::PageCreationObserver,
            Self::PublishedPageCreation { .. } => LivePageNavigationFailureRecipient::Background,
            Self::CompletePageCreation { .. }
            | Self::ReplyWithSnapshot { .. }
            | Self::ContinueNetworkIdle { .. }
            | Self::ContinueDomStable { .. }
            | Self::ContinueSubresourceResponse { .. } => {
                LivePageNavigationFailureRecipient::Initiator
            }
        }
    }

    pub(super) fn returns_with_pending_location_navigation(&self) -> bool {
        match self {
            Self::PublishedPageCreation {
                navigation_reply_policy,
            }
            | Self::CompletePageCreation {
                navigation_reply_policy,
                ..
            } => navigation_reply_policy.returns_with_pending_navigation(),
            Self::Background
            | Self::ReplyWithSnapshot { .. }
            | Self::ContinueNetworkIdle { .. }
            | Self::ContinueDomStable { .. }
            | Self::ContinueSubresourceResponse { .. } => false,
        }
    }

    pub(super) fn detach_command_observer(self) -> (Self, bool) {
        match self {
            Self::CompletePageCreation { .. } => (self, false),
            Self::Background
            | Self::PublishedPageCreation { .. }
            | Self::ReplyWithSnapshot { .. }
            | Self::ContinueNetworkIdle { .. }
            | Self::ContinueDomStable { .. }
            | Self::ContinueSubresourceResponse { .. } => (Self::Background, true),
        }
    }
}

pub(super) enum LivePageNavigationFollowOutcome {
    Completed,
    PostParseLifecycle {
        target_stage: PageVmInitStage,
        outcome: DocumentLifecycleTurnOutcome,
    },
    Download(RendererPendingDownloadActivation),
    /// Navigation yielded during phase one. The caller must first restore the
    /// returned entry, then reconcile the resident continuation against its
    /// stable producer source.
    PendingPhaseOne {
        wake_token: RendererPageToken,
    },
    TriggeredNavigation {
        stage: PageVmInitStage,
    },
}

pub(super) struct LivePageNavigationFollowTurn {
    pub(super) outcome: LivePageNavigationFollowOutcome,
    pub(super) document_commit: Option<PublishedReplacementDocument>,
}

pub(super) enum LivePagePendingNavigationPhaseOneAdvance {
    /// The resumed parser yielded again. The caller restores the entry before
    /// deriving a one-shot admission wake from the stable Page slot.
    Pending {
        wake_token: RendererPageToken,
    },
    TriggeredNavigation {
        stage: PageVmInitStage,
    },
    PostParseLifecycle {
        target_stage: PageVmInitStage,
        outcome: DocumentLifecycleTurnOutcome,
    },
}

pub(super) struct RendererOwnerLocalContext {
    pub(super) owner_state: Arc<RendererOwnerState>,
    pub(super) local_host_id: RendererOwnerLocalHostId,
    #[cfg(debug_assertions)]
    pub(super) local_thread_id: ThreadId,
}

impl Clone for RendererOwnerLocalContext {
    fn clone(&self) -> Self {
        Self {
            owner_state: self.owner_state.clone(),
            local_host_id: self.local_host_id,
            #[cfg(debug_assertions)]
            local_thread_id: self.local_thread_id,
        }
    }
}

#[derive(Default)]
pub(super) struct RendererOwnerLocalStore {
    page_hosts: HashMap<RendererOwnerLocalHostId, RendererOwnerLocalPageHost>,
    prepared_documents: HashMap<RendererPageReservationToken, RendererPreparedDocumentResidence>,
    page_task_deadline_index: OwnerDeadlineIndex<RendererPageToken>,
    owner_maintenance_deadline_index: OwnerDeadlineIndex<RendererPageToken>,
    next_host_instance_key: usize,
    next_renderer_document_isolate_reservation_id: u64,
}

pub(super) struct RendererPreparedDocumentResidence {
    pub(super) request: RendererCreateStreamingRawPageRequest,
    pub(super) isolate_allocator: RendererDocumentIsolateAllocator,
    pub(super) isolate_bootstrap: RendererDocumentIsolateBootstrap,
    pub(super) isolate_reservation: RendererDocumentIsolateReservation,
}

pub(super) struct RendererOwnerLocalStoreSession<'a> {
    store: &'a mut RendererOwnerLocalStore,
}

thread_local! {
    static CURRENT_RENDER_RUNTIME_OWNER_LOCAL_STORE: RefCell<Option<NonNull<RendererOwnerLocalStore>>> =
        const { RefCell::new(None) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[doc(hidden)]
pub struct RendererPageToken {
    pub(super) local_host_id: RendererOwnerLocalHostId,
    #[cfg(debug_assertions)]
    pub(super) local_thread_id: ThreadId,
    pub(super) page_id: PageId,
}

impl RendererPageToken {
    pub(crate) const fn local_host_id(self) -> RendererOwnerLocalHostId {
        self.local_host_id
    }

    pub(crate) const fn page_id(self) -> PageId {
        self.page_id
    }
}

#[cfg(test)]
impl RendererPageToken {
    pub(crate) fn new_for_testing(page_id: PageId) -> Self {
        Self {
            local_host_id: RendererOwnerLocalHostId::new_for_testing(0),
            #[cfg(debug_assertions)]
            local_thread_id: std::thread::current().id(),
            page_id,
        }
    }
}

#[derive(Debug)]
struct RendererOwnerLocalPageHost {
    pages: HashMap<PageId, RendererOwnerLocalPageSlot>,
    reserved_renderer_document_isolates:
        HashMap<PageId, Vec<RendererDocumentIsolateReservationEntry>>,
    instance_key: usize,
}

#[derive(Debug)]
struct RendererPageScriptEnvironmentPin {
    environment: RendererPageScriptEnvironment,
}

impl RendererPageScriptEnvironmentPin {
    fn new(environment: RendererPageScriptEnvironment) -> Self {
        Self { environment }
    }

    fn identity_key(&self) -> usize {
        self.environment.isolate_identity_key()
    }

    fn clear_page_runtime_tasks(&self) {
        self.environment.clear_page_runtime_tasks();
    }
}

#[derive(Debug)]
struct RendererOwnerLocalPageSlot {
    owner_slot: RendererPageSlotHandle,
    turn_scheduler: PageTurnScheduler<RendererPageLocalEntry>,
    page_command_first_dispatch:
        PageCommandFirstDispatchResidence<PageCommandFirstDispatchLane, RenderRuntimePendingTurn>,
    owner_maintenance: RendererPageOwnerMaintenanceResidence,
    task_sources: RendererPageOwnedTaskSources,
    lifecycle_gate: Option<LifecycleGate>,
    page_creation_navigation_failure_publisher: PageCreationNavigationFailurePublisher,
    script_environment_pin: RendererPageScriptEnvironmentPin,
}

impl RendererOwnerLocalPageSlot {
    fn new(
        owner_slot: RendererPageSlotHandle,
        entry: RendererPageLocalEntry,
        task_sources: RendererPageOwnedTaskSources,
        lifecycle_gate: Option<PageVmInitStage>,
        page_creation_navigation_failure_publisher: PageCreationNavigationFailurePublisher,
        script_environment_pin: RendererPageScriptEnvironmentPin,
    ) -> Self {
        Self {
            owner_slot,
            turn_scheduler: PageTurnScheduler::new(entry),
            page_command_first_dispatch: PageCommandFirstDispatchResidence::default(),
            owner_maintenance: RendererPageOwnerMaintenanceResidence::new(std::time::Instant::now()),
            task_sources,
            lifecycle_gate: lifecycle_gate.map(LifecycleGate::new),
            page_creation_navigation_failure_publisher,
            script_environment_pin,
        }
    }

    fn resident_entry(&self) -> Option<&RendererPageLocalEntry> {
        self.turn_scheduler.resident()
    }

    fn resident_entry_mut(&mut self) -> Option<&mut RendererPageLocalEntry> {
        self.turn_scheduler.resident_mut()
    }

    fn indexed_owner_maintenance_deadline(&self) -> Option<std::time::Instant> {
        self.owner_maintenance.indexed_deadline()
    }

    fn next_page_task_deadline(&mut self) -> Option<std::time::Instant> {
        let Self {
            turn_scheduler,
            task_sources,
            ..
        } = self;
        let entry = turn_scheduler.resident()?;
        let timer_deadline = entry.next_javascript_timer_deadline();
        let internal_loading_deadline = task_sources
            .next_internal_loading_deadline(entry.page_vm().current_page_internal_loading_owner());
        earliest_deadline(timer_deadline, internal_loading_deadline)
    }

    #[cfg(debug_assertions)]
    fn local_page_task_deadline(&self) -> Option<std::time::Instant> {
        let entry = self.turn_scheduler.resident()?;
        let timer_deadline = entry.next_javascript_timer_deadline();
        let internal_loading_deadline = self
            .task_sources
            .local_internal_loading_deadline(entry.page_vm().current_page_internal_loading_owner());
        earliest_deadline(timer_deadline, internal_loading_deadline)
    }
}

fn earliest_deadline(
    left: Option<std::time::Instant>,
    right: Option<std::time::Instant>,
) -> Option<std::time::Instant> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(deadline), None) | (None, Some(deadline)) => Some(deadline),
        (None, None) => None,
    }
}

impl Drop for RendererOwnerLocalPageSlot {
    fn drop(&mut self) {
        // This is the stable Page lifetime boundary. A replacement PageVm
        // drops only Document-owned queues; retiring the slot also discards
        // V8 foreground work and typed Page source payloads.
        for waiting in self.page_command_first_dispatch.drain_waiting() {
            waiting.reject_page_command_admission(self.owner_slot.page_id());
        }
        self.task_sources.clear();
        self.script_environment_pin
            .environment
            .retire_output_stream();
        self.script_environment_pin.clear_page_runtime_tasks();
    }
}

#[derive(Debug)]
struct RendererDocumentIsolateReservationEntry {
    id: u64,
    handle: RendererDocumentIsolateHandle,
    /// The stream opens together with the isolate reservation, before that
    /// isolate is attached to a stable Page slot. Until attachment transfers
    /// lifetime ownership to `RendererPageScriptEnvironmentPin`, this entry
    /// must close the stream on every cancellation/failure path.
    output_journal: RendererTurnOutputJournal,
    /// Initial page creation owns the not-yet-attached consumer set here.
    /// A same-Page replacement reservation reuses the live slot's producer
    /// routes and therefore must not manufacture a second consumer set.
    initial_task_sources: Option<RendererPageOwnedTaskSources>,
    _accounting: RendererDocumentIsolateReservationAccounting,
}

#[derive(Clone)]
pub(crate) struct RendererDocumentIsolateAllocator {
    owner: RendererOwnerLocalContext,
    page_id: PageId,
}

#[derive(Clone)]
pub(crate) struct RendererDocumentIsolateReservation {
    inner: Rc<RendererDocumentIsolateReservationState>,
}

struct RendererDocumentIsolateReservationState {
    token: RendererPageToken,
    reservation_id: u64,
    active: std::cell::Cell<bool>,
}

impl RendererDocumentIsolateAllocator {
    pub(super) fn new(owner: RendererOwnerLocalContext, page_id: PageId) -> Self {
        Self { owner, page_id }
    }

    pub(crate) fn reserve_renderer_document_isolate(
        &self,
        page_runtime_task_source: crate::page_task_queue::PageRuntimeTaskSource,
    ) -> Result<(
        RendererDocumentIsolateBootstrap,
        RendererDocumentIsolateReservation,
    )> {
        reserve_renderer_document_isolate_on_bound_owner_local_store(
            &self.owner,
            self.page_id,
            page_runtime_task_source,
        )
    }
}

impl RendererDocumentIsolateReservation {
    fn token(&self) -> RendererPageToken {
        self.inner.token
    }

    fn reservation_id(&self) -> u64 {
        self.inner.reservation_id
    }

    fn disarm_for_attach(&self) {
        self.inner.active.set(false);
    }

    pub(crate) fn is_active(&self) -> bool {
        self.inner.active.get()
    }
}

impl std::fmt::Debug for RendererDocumentIsolateReservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererDocumentIsolateReservation")
            .field("token", &self.inner.token)
            .field("reservation_id", &self.inner.reservation_id)
            .field("active", &self.inner.active.get())
            .finish()
    }
}

impl Drop for RendererDocumentIsolateReservationState {
    fn drop(&mut self) {
        if self.active.get() && has_current_render_runtime_owner_local_store() {
            remove_reserved_renderer_document_isolate_on_bound_owner_local_store(
                self.token,
                self.reservation_id,
            );
            self.active.set(false);
        }
    }
}

impl RendererPageLocalEntry {
    fn new(slot: RendererPageSlotHandle, vm: PageVm) -> Result<Self> {
        Ok(Self {
            slot,
            top_level_navigation_dispatch:
                RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            standalone_navigation_follow: StandaloneNavigationFollowState::Idle,
            pending_document_lifecycle_turn: None,
            post_response_document_lifecycle: None,
            vm: Some(vm),
            pending_phase_one_navigation: None,
            last_published_replacement_document: None,
        })
    }

    fn new_with_pending_phase_one_navigation(
        slot: RendererPageSlotHandle,
        mut pending: PageVmPendingPhaseOneNavigation,
    ) -> Result<Self> {
        let mut entry = Self {
            slot,
            top_level_navigation_dispatch:
                RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            standalone_navigation_follow: StandaloneNavigationFollowState::Idle,
            pending_document_lifecycle_turn: None,
            post_response_document_lifecycle: None,
            vm: None,
            pending_phase_one_navigation: None,
            last_published_replacement_document: None,
        };
        entry.prepare_pending_phase_one_navigation_install(&mut pending)?;
        entry.pending_phase_one_navigation = Some(pending);
        Ok(entry)
    }

    pub(super) fn page_vm(&self) -> &PageVm {
        self.active_page_vm()
            .expect("resident renderer page entry must retain an active PageVm")
    }

    pub(super) fn set_top_level_navigation_dispatch(
        &mut self,
        dispatch: RendererTopLevelNavigationDispatch,
    ) {
        self.top_level_navigation_dispatch = dispatch;
    }

    pub(super) fn top_level_navigation_dispatch(&self) -> RendererTopLevelNavigationDispatch {
        self.top_level_navigation_dispatch
    }

    /// Claim the single standalone owner chain for the current pending
    /// location navigation. A failed chain remains suppressed while that same
    /// descriptor is pending, so a duplicate producer handoff cannot restart
    /// the chain with a fresh limit.
    pub(super) fn begin_standalone_navigation_follow(&mut self) -> bool {
        self.begin_standalone_navigation_follow_for_handoff(None)
    }

    /// Claim a producer handoff only while the same request still occupies
    /// the ScriptVm's unique pending navigation slot. A delayed wake for an
    /// overwritten request therefore cannot start the replacement request.
    pub(super) fn begin_standalone_navigation_follow_from_handoff(
        &mut self,
        handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
    ) -> bool {
        self.begin_standalone_navigation_follow_for_handoff(Some(handoff))
    }

    fn begin_standalone_navigation_follow_for_handoff(
        &mut self,
        requested: Option<crate::page_task_queue::RendererTopLevelNavigationHandoff>,
    ) -> bool {
        if !matches!(
            self.top_level_navigation_dispatch,
            RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter
        ) {
            return false;
        }
        let current = self
            .active_page_vm()
            .and_then(|page_vm| page_vm.vm().pending_location_navigation_handoff());
        self.standalone_navigation_follow.claim(current, requested)
    }

    pub(super) fn settle_standalone_navigation_follow(&mut self, succeeded: bool) {
        let current = self
            .active_page_vm()
            .and_then(|page_vm| page_vm.vm().pending_location_navigation_handoff());
        self.standalone_navigation_follow.settle(current, succeeded);
    }

    /// A replacement PageVm becomes the active owner-local runtime before its
    /// view is committed to the stable cross-thread Page slot.
    pub(super) fn has_uncommitted_page_vm(&self) -> bool {
        self.uncommitted_page_vm_creation_id().is_some()
    }

    pub(super) fn uncommitted_page_vm_creation_id(&self) -> Option<u64> {
        let stable_entry = self.slot.entry();
        if !stable_entry.is_active() {
            return None;
        }
        self.active_page_vm().and_then(|page_vm| {
            (stable_entry.vm_creation_id() != page_vm.creation_id).then_some(page_vm.creation_id)
        })
    }

    fn publish_replacement_document_commit(&mut self) -> Result<PublishedReplacementDocument> {
        let stable_before = self.slot.entry();
        ensure!(
            stable_before.is_active(),
            "replacement Document cannot commit into a retired Page slot"
        );
        let page_vm = self
            .active_page_vm()
            .ok_or_else(|| anyhow!("replacement Document commit lost its active PageVm"))?;
        let vm_creation_id = page_vm.creation_id;
        let navigation_handoff =
            page_vm
                .replacement_document_commit_handoff()
                .ok_or_else(|| {
                    anyhow!("replacement PageVm is missing its navigation commit identity")
                })?;
        ensure!(
            stable_before.vm_creation_id() != vm_creation_id,
            "replacement Document commit attempted to republish stable PageVm {vm_creation_id}"
        );
        ensure!(
            self.last_published_replacement_document
                .is_none_or(|published| {
                    published.vm_creation_id != vm_creation_id
                        && published.navigation_handoff != navigation_handoff
                }),
            "replacement Document commit attempted to reuse a published navigation or PageVm identity"
        );

        self.page_vm_mut()
            .settle_replacement_document_commit(navigation_handoff)?;
        RendererOwnerLocalStore::commit_active_vm_page_state_on_entry(self)?;
        let stable_after = self.slot.entry();
        ensure!(
            stable_after.vm_creation_id() == vm_creation_id,
            "replacement Document publication did not install its PageVm identity"
        );
        ensure!(
            stable_after.view_generation > stable_before.view_generation,
            "replacement Document publication did not advance the stable view generation"
        );
        let published = PublishedReplacementDocument {
            navigation_handoff,
            vm_creation_id,
            view_generation: stable_after.view_generation,
        };
        self.last_published_replacement_document = Some(published);

        // Pending phase one owns the committed replacement. The old PageVm
        // was already terminated before the response commit and can no longer
        // be a rollback candidate.
        if self.pending_phase_one_navigation.is_some() {
            self.vm = None;
        }
        Ok(published)
    }

    pub(super) fn active_page_vm(&self) -> Option<&PageVm> {
        self.pending_phase_one_navigation
            .as_ref()
            .map(PageVmPendingPhaseOneNavigation::page_vm)
            .or(self.vm.as_ref())
    }

    pub(super) fn page_vm_mut(&mut self) -> &mut PageVm {
        if let Some(pending) = self.pending_phase_one_navigation.as_mut() {
            return pending.page_vm_mut();
        }
        self.vm
            .as_mut()
            .expect("resident renderer page entry must retain an active PageVm")
    }

    pub(super) fn pending_phase_one_navigation_has_ready_streaming_input(&mut self) -> bool {
        self.pending_phase_one_navigation
            .as_mut()
            .is_some_and(PageVmPendingPhaseOneNavigation::has_ready_streaming_input)
    }

    fn page_vm_and_document_lifecycle_turn_mut(
        &mut self,
    ) -> (&mut PageVm, &mut Option<PendingDocumentLifecycleTurn>) {
        self.retire_stale_document_lifecycle_turn();
        let Self {
            vm,
            pending_document_lifecycle_turn,
            pending_phase_one_navigation,
            ..
        } = self;
        let page_vm = if let Some(pending) = pending_phase_one_navigation.as_mut() {
            pending.page_vm_mut()
        } else {
            vm.as_mut()
                .expect("resident renderer page entry must retain an active PageVm")
        };
        (page_vm, pending_document_lifecycle_turn)
    }

    fn retire_document_lifecycle_turn(&mut self) {
        self.pending_document_lifecycle_turn = None;
        self.post_response_document_lifecycle = None;
    }

    fn retire_stale_document_lifecycle_turn(&mut self) {
        let Some(pending_document) = self
            .pending_document_lifecycle_turn
            .as_ref()
            .map(|pending| pending.document)
        else {
            // A bounded lifecycle action may retire its resident before a
            // later operation in the same action fails. A post-response
            // continuation cannot survive without that exact resident.
            self.post_response_document_lifecycle = None;
            return;
        };
        let current_document = self.active_page_vm().map(|page_vm| {
            RendererDocumentLifecycleIdentity::from(page_vm.document_lifecycle.current_snapshot())
        });
        if current_document == Some(pending_document) {
            return;
        }
        tracing::debug!(
            ?pending_document,
            ?current_document,
            "retired stale lifecycle continuation at the stable page-residence boundary"
        );
        self.retire_document_lifecycle_turn();
    }

    pub(super) fn pending_document_lifecycle_identity(
        &mut self,
    ) -> Option<RendererDocumentLifecycleIdentity> {
        self.retire_stale_document_lifecycle_turn();
        self.pending_document_lifecycle_turn
            .as_ref()
            .map(|pending| pending.document)
    }

    pub(super) fn defer_document_lifecycle_until_response(
        &mut self,
        document: RendererDocumentLifecycleIdentity,
    ) -> Result<()> {
        anyhow::ensure!(
            self.pending_document_lifecycle_identity() == Some(document),
            "post-response lifecycle continuation does not match the resident Document"
        );
        self.post_response_document_lifecycle = Some(document);
        Ok(())
    }

    fn document_lifecycle_is_deferred_until_response(&mut self) -> bool {
        let pending_document = self.pending_document_lifecycle_identity();
        if self.post_response_document_lifecycle == pending_document {
            return pending_document.is_some();
        }
        if self.post_response_document_lifecycle.is_some() {
            self.post_response_document_lifecycle = None;
        }
        false
    }

    fn release_document_lifecycle_after_response(
        &mut self,
        document: RendererDocumentLifecycleIdentity,
    ) -> bool {
        if self.post_response_document_lifecycle != Some(document) {
            return false;
        }
        self.post_response_document_lifecycle = None;
        self.pending_document_lifecycle_identity() == Some(document)
    }

    pub(super) fn has_ready_main_parser_script_continuation(&mut self) -> bool {
        self.retire_stale_document_lifecycle_turn();
        let has_sealed_queue = self
            .pending_document_lifecycle_turn
            .as_ref()
            .is_some_and(|pending| pending.has_sealed_main_parser_script_queue);
        has_sealed_queue
            && self
                .page_vm_mut()
                .sealed_main_parser_script_continuation_is_ready()
    }

    pub(super) fn document_lifecycle_owner_turn_is_runnable(&mut self) -> bool {
        self.retire_stale_document_lifecycle_turn();
        self.pending_document_lifecycle_turn
            .as_ref()
            .is_some_and(|pending| pending.owner_turn_is_runnable)
    }

    fn observe_document_lifecycle(
        &mut self,
        document: RendererDocumentLifecycleIdentity,
        target_stage: PageVmInitStage,
    ) -> DocumentLifecycleObserverOutcome {
        self.retire_stale_document_lifecycle_turn();
        let page_vm = self.page_vm();
        let current_document = page_vm.document_lifecycle.identity();
        if current_document != document {
            return DocumentLifecycleObserverOutcome::DocumentReplaced {
                document: current_document,
            };
        }

        match page_vm.document_lifecycle_wait_outcome(
            renderer_document_lifecycle_milestone_for_stage(target_stage),
        ) {
            RendererDocumentLifecycleWaitOutcome::Reached(_) => {
                DocumentLifecycleObserverOutcome::Reached
            }
            RendererDocumentLifecycleWaitOutcome::Interrupted(termination) => {
                DocumentLifecycleObserverOutcome::Interrupted(termination)
            }
            RendererDocumentLifecycleWaitOutcome::Pending => {
                classify_pending_document_lifecycle_residence(
                    page_vm.vm().has_pending_location_navigation(),
                    self.pending_phase_one_navigation.is_some(),
                    self.pending_document_lifecycle_turn
                        .as_ref()
                        .is_some_and(|pending| pending.document == document),
                    page_vm.has_blocked_document_replacement_lifecycle_admission(document),
                )
            }
        }
    }

    fn prepare_pending_phase_one_navigation_install(
        &self,
        pending: &mut PageVmPendingPhaseOneNavigation,
    ) -> Result<RendererPageToken> {
        let validation = if self.pending_phase_one_navigation.is_some() {
            Err(anyhow!(
                "renderer page already owns a pending phase-one navigation"
            ))
        } else if self.vm.as_ref().is_some_and(PageVm::has_live_script_vm) {
            Err(anyhow!(
                "phase-one-blocked replacement must be installed after the old document context is detached"
            ))
        } else {
            Ok(())
        };
        if let Err(error) = validation {
            Self::reject_pending_phase_one_navigation_state(
                pending,
                format!("Cannot install navigation: {error}"),
            );
            return Err(error);
        }
        let Some(wake_token) = pending.owner_wake_token() else {
            let error = anyhow!("pending phase-one navigation requires an owner wake token");
            Self::reject_pending_phase_one_navigation_state(
                pending,
                format!("Cannot install navigation: {error}"),
            );
            return Err(error);
        };
        Ok(wake_token)
    }

    /// Install a newly created replacement Document while retaining the Page's
    /// stable typed resource source. The replacement PageVm must share the
    /// source carried by its Page runtime environment; no receiver transfer is
    /// part of Document installation.
    pub(super) fn install_new_pending_phase_one_navigation(
        &mut self,
        mut pending: PageVmPendingPhaseOneNavigation,
    ) -> Result<RendererPageToken> {
        let wake_token = self.prepare_pending_phase_one_navigation_install(&mut pending)?;
        pending.attach_committed_response();
        self.retire_document_lifecycle_turn();
        self.pending_phase_one_navigation = Some(pending);
        Ok(wake_token)
    }

    /// Re-park the same phase-one creation runtime after one bounded
    /// phase-one turn. Its typed resource source already lives in the stable
    /// Page runtime environment and must not be replaced.
    fn restore_pending_phase_one_navigation(
        &mut self,
        mut pending: PageVmPendingPhaseOneNavigation,
    ) -> Result<RendererPageToken> {
        let wake_token = self.prepare_pending_phase_one_navigation_install(&mut pending)?;
        self.pending_phase_one_navigation = Some(pending);
        Ok(wake_token)
    }

    fn install_resumed_phase_one_page_vm(&mut self, page_vm: PageVm) {
        debug_assert!(
            self.pending_phase_one_navigation.is_none(),
            "a resumed phase-one PageVm cannot coexist with its consumed residence"
        );
        self.retire_document_lifecycle_turn();
        self.vm = Some(page_vm);
    }

    fn reject_pending_phase_one_navigation_state(
        pending: &mut PageVmPendingPhaseOneNavigation,
        message: String,
    ) {
        let browser_context_runtime = pending
            .page_vm()
            .runtime_hooks
            .browser_context_runtime
            .clone();
        pending
            .metadata
            .reject(None, &browser_context_runtime, message);
        pending.page_vm_mut().close_for_context_teardown();
    }

    pub(super) fn take_pending_phase_one_navigation(
        &mut self,
    ) -> Result<PageVmPendingPhaseOneNavigation> {
        self.pending_phase_one_navigation
            .take()
            .ok_or_else(|| anyhow!("renderer page has no pending phase-one navigation to resume"))
    }

    pub(super) fn reject_pending_phase_one_navigation(&mut self, message: &str) {
        let Some(mut pending) = self.pending_phase_one_navigation.take() else {
            return;
        };
        Self::reject_pending_phase_one_navigation_state(&mut pending, message.to_owned());
    }

    fn close_for_context_teardown(&mut self) {
        self.retire_document_lifecycle_turn();
        self.reject_pending_phase_one_navigation(
            "Location navigation was cancelled because its page was retired.",
        );
        if let Some(vm) = self.vm.as_mut() {
            vm.close_for_context_teardown();
        }
    }

    fn next_javascript_timer_deadline(&self) -> Option<std::time::Instant> {
        self.page_vm().vm().next_timeout_deadline()
    }
}

fn classify_pending_document_lifecycle_residence(
    has_pending_location_navigation: bool,
    has_pending_phase_one_navigation: bool,
    has_exact_document_lifecycle_turn: bool,
    has_blocked_document_replacement_lifecycle_admission: bool,
) -> DocumentLifecycleObserverOutcome {
    if has_pending_location_navigation {
        DocumentLifecycleObserverOutcome::NavigationPending
    } else if has_pending_phase_one_navigation
        || has_exact_document_lifecycle_turn
        || has_blocked_document_replacement_lifecycle_admission
    {
        DocumentLifecycleObserverOutcome::Pending
    } else {
        DocumentLifecycleObserverOutcome::MissingResident
    }
}

pub(super) struct RenderRuntimeOwnerLocalStoreBinding;

fn with_bound_render_runtime_owner_local_store_session<R>(
    f: impl FnOnce(RendererOwnerLocalStoreSession<'_>) -> R,
) -> R {
    CURRENT_RENDER_RUNTIME_OWNER_LOCAL_STORE.with(|current_store| {
        let mut store = current_store
            .borrow()
            .expect("bound render-runtime owner-local store should exist on current thread");
        // Safety: the pointer is installed only for the lifetime of the
        // render-runtime owner loop on the current thread.
        unsafe {
            f(RendererOwnerLocalStoreSession {
                store: store.as_mut(),
            })
        }
    })
}

pub(super) fn bind_render_runtime_owner_local_store(
    store: &mut RendererOwnerLocalStore,
) -> RenderRuntimeOwnerLocalStoreBinding {
    CURRENT_RENDER_RUNTIME_OWNER_LOCAL_STORE.with(|current_store| {
        let mut current_store = current_store.borrow_mut();
        assert!(
            current_store.is_none(),
            "render-runtime owner-local store must not be rebound while already active"
        );
        *current_store = Some(NonNull::from(store));
    });
    RenderRuntimeOwnerLocalStoreBinding
}

pub(super) fn has_current_render_runtime_owner_local_store() -> bool {
    CURRENT_RENDER_RUNTIME_OWNER_LOCAL_STORE.with(|current_store| current_store.borrow().is_some())
}

pub(super) fn owner_local_store_session(
    store: &mut RendererOwnerLocalStore,
) -> RendererOwnerLocalStoreSession<'_> {
    RendererOwnerLocalStoreSession { store }
}

pub(super) fn install_page_vm_on_bound_owner_local_store(
    owner_local: &RendererOwnerLocalContext,
    requested_url: Url,
    navigation_initiator_url: Option<Url>,
    navigation_redirected: bool,
    navigation_redirect_count: usize,
    response_status: u16,
    response_headers: Vec<(String, String)>,
    vm: PageVm,
    pending_download: Option<RendererPendingDownloadActivation>,
    lifecycle_gate: Option<PageVmInitStage>,
) -> Result<RendererPendingPageCreation> {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.install_page_vm(
            owner_local,
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            vm,
            pending_download,
            lifecycle_gate,
        )
    })
}

pub(super) fn install_phase_one_blocked_page_on_bound_owner_local_store(
    owner_local: &RendererOwnerLocalContext,
    requested_url: Url,
    navigation_initiator_url: Option<Url>,
    navigation_redirected: bool,
    navigation_redirect_count: usize,
    response_status: u16,
    response_headers: Vec<(String, String)>,
    pending_navigation: PageVmPendingPhaseOneNavigation,
    lifecycle_gate: Option<PageVmInitStage>,
) -> Result<RendererPendingPageCreation> {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.install_phase_one_blocked_page_for_owner(
            owner_local,
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            pending_navigation,
            lifecycle_gate,
        )
    })
}

pub(super) fn finalize_pending_page_creation_on_bound_owner_local_store(
    pending: RendererPendingPageCreation,
) -> RendererPageCreationCommit {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.finalize_pending_page_creation(pending)
    })
}

pub(super) fn resolve_pending_page_creation_on_bound_owner_local_store(
    pending: RendererPendingPageCreation,
    document: RendererDocumentLifecycleIdentity,
    target_stage: PageVmInitStage,
    navigation_reply_policy: NavigationReplyPolicy,
) -> RendererPageCreationResolution {
    // This operation runs inside one owner-lane task and contains no await
    // boundary. Before the task starts the entry remains resident; once it is
    // checked out, observation and the resulting page-state refresh,
    // restoration, or retirement run to completion without exposing the entry
    // to the outer owner loop.
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.resolve_pending_page_creation(
            pending,
            document,
            target_stage,
            navigation_reply_policy,
        )
    })
}

fn reserve_renderer_document_isolate_on_bound_owner_local_store(
    owner_local: &RendererOwnerLocalContext,
    page_id: PageId,
    page_runtime_task_source: crate::page_task_queue::PageRuntimeTaskSource,
) -> Result<(
    RendererDocumentIsolateBootstrap,
    RendererDocumentIsolateReservation,
)> {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.reserve_renderer_document_isolate(owner_local, page_id, page_runtime_task_source)
    })
}

fn remove_reserved_renderer_document_isolate_on_bound_owner_local_store(
    token: RendererPageToken,
    reservation_id: u64,
) {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.remove_reserved_renderer_document_isolate(token, reservation_id)
    })
}

async fn run_on_bound_owner_local_store_local_task<R, F>(
    local_executor: JsLocalExecutor,
    future: F,
) -> Result<R>
where
    R: 'static,
    F: Future<Output = Result<R>> + 'static,
{
    run_named_owner_local_task(
        local_executor,
        "bound render-runtime owner-local local task was cancelled",
        future,
    )
    .await
}

pub(super) type EntryLocalTaskFuture<'a, R> = Pin<Box<dyn Future<Output = Result<R>> + 'a>>;

#[cfg(debug_assertions)]
const PANIC_WAIT_FOR_SELECTOR_FOR_TESTING: &str = "__moli_panic_wait_for_selector_for_testing__";
#[cfg(debug_assertions)]
const PANIC_WAIT_FOR_SCRIPT_TRUTHY_FOR_TESTING: &str =
    "__moli_panic_wait_for_script_truthy_for_testing__";

struct EntryLocalTaskGuard<E, R> {
    entry: Option<E>,
    reply_tx: Option<oneshot::Sender<(E, Result<R>)>>,
}

impl<E, R> EntryLocalTaskGuard<E, R> {
    fn new(entry: E, reply_tx: oneshot::Sender<(E, Result<R>)>) -> Self {
        Self {
            entry: Some(entry),
            reply_tx: Some(reply_tx),
        }
    }

    fn entry_mut(&mut self) -> &mut E {
        self.entry
            .as_mut()
            .expect("entry local task guard should retain its page entry")
    }

    fn complete(mut self, result: Result<R>) {
        let entry = self
            .entry
            .take()
            .expect("entry local task guard should retain its page entry on completion");
        let reply_tx = self
            .reply_tx
            .take()
            .expect("entry local task guard should retain its reply sender on completion");
        let _ = reply_tx.send((entry, result));
    }
}

impl<E, R> Drop for EntryLocalTaskGuard<E, R> {
    fn drop(&mut self) {
        if let (Some(entry), Some(reply_tx)) = (self.entry.take(), self.reply_tx.take()) {
            let _ = reply_tx.send((
                entry,
                Err(anyhow!(
                    "bound render-runtime owner-local local task panicked before restoring its page entry"
                )),
            ));
        }
    }
}

pub(super) async fn run_entry_on_bound_owner_local_store_local_task<R, F>(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
    operation: F,
) -> (RendererPageLocalEntry, Result<R>)
where
    R: 'static,
    F: for<'a> FnOnce(&'a mut RendererPageLocalEntry) -> EntryLocalTaskFuture<'a, R> + 'static,
{
    let (reply_tx, reply_rx) = oneshot::channel();
    // Construct the guard before spawning. If the local task is cancelled
    // before its first poll, dropping the future must still return the entry
    // and an inner error to the owner so command-specific cleanup can run.
    let guard = EntryLocalTaskGuard::new(entry, reply_tx);
    tokio::task::spawn_local(async move {
        let mut guard = guard;
        let result = local_executor
            .scope_on_current_thread(operation(guard.entry_mut()))
            .await;
        guard.complete(result);
    });
    reply_rx
        .await
        .expect("entry local task guard must return the page entry on task termination")
}

pub(super) fn take_entry_for_command_on_bound_owner_local_store(
    token: RendererPageToken,
) -> Result<RendererPageLocalEntry> {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.take_entry_for_command(token)
    })
}

pub(super) fn admit_page_command_first_dispatch_on_bound_owner_local_store(
    token: RendererPageToken,
    lane: PageCommandFirstDispatchLane,
    turn: RenderRuntimePendingTurn,
) -> Option<RenderRuntimePendingTurn> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session
            .store
            .admit_page_command_first_dispatch(token, lane, turn)
    })
}

pub(super) fn complete_page_command_first_dispatch_on_bound_owner_local_store(
    token: RendererPageToken,
    lane: &PageCommandFirstDispatchLane,
) -> Option<RenderRuntimePendingTurn> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session
            .store
            .complete_page_command_first_dispatch(token, lane)
    })
}

pub(super) fn checkout_entry_for_owner_turn_on_bound_owner_local_store(
    token: RendererPageToken,
) -> RendererPageLocalEntryCheckout {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.checkout_entry_for_owner_turn(token)
    })
}

pub(super) fn schedule_page_turn_on_bound_owner_local_store(
    token: RendererPageToken,
    trigger: PageTurnTrigger,
) -> RendererPageTurnAdmission {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.schedule_page_turn(token, trigger)
    })
}

pub(super) fn release_post_response_document_lifecycle_on_bound_owner_local_store(
    token: RendererPageToken,
    document: RendererDocumentLifecycleIdentity,
) -> bool {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session
            .store
            .release_post_response_document_lifecycle(token, document)
    })
}

pub(super) fn checkout_scheduled_page_turn_on_bound_owner_local_store(
    token: RendererPageToken,
) -> RendererPageTurnCheckout {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.checkout_scheduled_page_turn(token)
    })
}

pub(super) fn has_ready_page_networking_task_on_bound_owner_local_store(
    token: RendererPageToken,
    current_document: RendererDocumentToken,
) -> bool {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session
            .store
            .has_ready_page_networking_task(token, current_document)
    })
}

/// Reconcile one restored phase-one residence against its stable
/// producer sources.
///
/// The residence must already be visible in the Page slot before this function
/// is called. A producer may have published its payload and spent its
/// empty-to-nonempty wake before restoration completed; current source
/// readiness is therefore authoritative, while the stored suspension reason
/// only selects the exact source to inspect. No task is dequeued here.
pub(super) fn pending_phase_one_admission_after_restore_on_bound_owner_local_store(
    token: RendererPageToken,
) -> PhaseOneResidenceAdmission {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session
            .store
            .pending_phase_one_admission_after_restore(token)
    })
}

pub(super) fn page_turn_readiness_after_restore_on_bound_owner_local_store(
    token: RendererPageToken,
) -> Option<PageOwnerTurnReadiness> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.page_turn_readiness_after_restore(token)
    })
}

pub(super) fn renderer_output_fence_for_tail_on_bound_owner_local_store(
    token: RendererPageToken,
) -> Option<RendererOutputFence> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session
            .store
            .page_hosts
            .get(&token.local_host_id)
            .and_then(|host| host.pages.get(&token.page_id))
            .and_then(|slot| {
                let journal = slot.script_environment_pin.environment.output_journal();
                journal
                    .last_published_cursor()
                    .map(|cursor| journal.declare_fence(cursor))
            })
    })
}

pub(super) fn restore_entry_after_command_on_bound_owner_local_store(
    token: RendererPageToken,
    entry: RendererPageLocalEntry,
) {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.restore_entry_after_command(token, entry);
    });
}

pub(super) fn restore_entry_after_document_lifecycle_on_bound_owner_local_store(
    token: RendererPageToken,
    entry: RendererPageLocalEntry,
    reconsider_displaced_ordinary: bool,
) {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.restore_entry_after_command(token, entry);
        if let Some(gate) = session
            .store
            .page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
            .and_then(|slot| slot.lifecycle_gate.as_mut())
        {
            gate.settle_lifecycle_turn(reconsider_displaced_ordinary);
        }
    });
}

pub(super) fn release_lifecycle_gate_on_bound_owner_local_store(
    token: RendererPageToken,
) -> Result<ReleasedLifecycleGate> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.release_lifecycle_gate(token)
    })
}

pub(super) fn renderer_page_token_for_owner_context(
    owner: &RendererOwnerLocalContext,
    page_id: PageId,
) -> RendererPageToken {
    RendererPageToken {
        local_host_id: owner.local_host_id,
        #[cfg(debug_assertions)]
        local_thread_id: owner.local_thread_id,
        page_id,
    }
}

pub(super) async fn dispatch_async_command_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
    command: RendererPageCommand,
) -> (RendererPageLocalEntry, Result<RendererPageCommandDispatch>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(
            async move { RendererOwnerLocalStore::dispatch_async_on_entry(entry, command).await },
        )
    })
    .await
}

/// Command result plus any replacement lifecycle admission created while the
/// Page entry was checked out. The owner must restore stable residence before
/// publishing the admitted lifecycle turn.
pub(super) struct RendererPageCommandDispatch {
    pub(super) reply: RendererPageReply,
    pub(super) replacement_lifecycle: Option<DocumentLifecycleTurnOutcome>,
    pub(super) turn_records: Vec<PendingRendererOutputRecord>,
}

pub(super) async fn advance_runtime_command_lifecycle_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
    scope_id: PageVmRuntimeCommandOutputScopeId,
) -> (
    RendererPageLocalEntry,
    Result<PageVmRuntimeCommandLifecycleAdvance>,
) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            entry
                .page_vm_mut()
                .advance_pending_runtime_command_lifecycle_one_turn(scope_id)
                .await
        })
    })
    .await
}

pub(super) async fn begin_post_parse_lifecycle_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
    work: Vec<PostParsePageOwnedWork>,
    stage: PageVmInitStage,
    started: std::time::Instant,
) -> (RendererPageLocalEntry, Result<DocumentLifecycleTurnOutcome>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            let (page_vm, pending_document_lifecycle_turn) =
                entry.page_vm_and_document_lifecycle_turn_mut();
            page_vm
                .begin_post_parse_lifecycle_on_named_owner_lane(
                    pending_document_lifecycle_turn,
                    work,
                    stage,
                    started,
                )
                .await
        })
    })
    .await
}

pub(super) async fn commit_page_state_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
) -> (RendererPageLocalEntry, Result<Arc<RendererPageState>>) {
    commit_page_state_on_entry_via_local_task_with_policy(
        local_executor,
        entry,
        super::RendererPageStateCapturePolicy::FullReport,
    )
    .await
}

pub(super) async fn commit_page_state_on_entry_via_local_task_with_policy(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
    capture_policy: super::RendererPageStateCapturePolicy,
) -> (RendererPageLocalEntry, Result<Arc<RendererPageState>>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            RendererOwnerLocalStore::commit_current_vm_page_state_on_entry_with_policy(
                entry,
                capture_policy,
            )
            .map_err(|error| anyhow!("failed to refresh renderer owner page view: {error}"))
        })
    })
    .await
}

pub(super) async fn advance_network_idle_wait_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
    state: PageVmNetworkIdleWaitState,
    remaining: std::time::Duration,
) -> (RendererPageLocalEntry, Result<PageVmNetworkIdleWaitAdvance>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            entry
                .page_vm_mut()
                .advance_network_idle_wait_turn(state, remaining)
                .await
        })
    })
    .await
}

pub(super) async fn advance_dom_stable_wait_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
    state: PageVmDomStableWaitState,
    remaining: std::time::Duration,
) -> (RendererPageLocalEntry, Result<PageVmDomStableWaitAdvance>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            entry
                .page_vm_mut()
                .advance_dom_stable_wait_turn(state, remaining)
                .await
        })
    })
    .await
}

pub(super) async fn advance_selector_wait_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
    selector: String,
    remaining: std::time::Duration,
) -> (RendererPageLocalEntry, Result<PageVmCommandWaitAdvance>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            #[cfg(debug_assertions)]
            if selector == PANIC_WAIT_FOR_SELECTOR_FOR_TESTING {
                panic!("wait-for-selector local task panicked for testing")
            }
            entry
                .page_vm_mut()
                .advance_selector_wait_turn(&selector, remaining)
                .await
        })
    })
    .await
}

pub(super) async fn advance_script_truthy_wait_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
    expression: String,
    pending_call: Option<PendingRuntimeEvaluateCall>,
    remaining: std::time::Duration,
) -> (
    RendererPageLocalEntry,
    Result<PageVmScriptTruthyWaitAdvance>,
) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            #[cfg(debug_assertions)]
            if expression == PANIC_WAIT_FOR_SCRIPT_TRUTHY_FOR_TESTING {
                panic!("wait-for-script-truthy local task panicked for testing")
            }
            entry
                .page_vm_mut()
                .advance_script_truthy_wait_turn(&expression, pending_call, remaining)
                .await
        })
    })
    .await
}

pub(super) async fn advance_runtime_expression_await_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
    execution_context_id: Option<i64>,
    expression: String,
    pending_call: Option<PendingRuntimeEvaluateCall>,
    remaining: std::time::Duration,
) -> (
    RendererPageLocalEntry,
    Result<PageVmRuntimeExpressionAwaitAdvance>,
) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            entry
                .page_vm_mut()
                .advance_runtime_expression_await_turn(
                    execution_context_id,
                    &expression,
                    pending_call,
                    remaining,
                )
                .await
        })
    })
    .await
}

pub(super) async fn advance_subresource_response_wait_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
    criteria: SubresourceResponseWaitCriteria,
    remaining: std::time::Duration,
) -> (
    RendererPageLocalEntry,
    Result<PageVmSubresourceResponseWaitAdvance>,
) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            entry
                .page_vm_mut()
                .advance_subresource_response_wait_turn(&criteria, remaining)
                .await
        })
    })
    .await
}

pub(super) async fn follow_pending_location_navigation_one_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
    stage: PageVmInitStage,
) -> (RendererPageLocalEntry, Result<LivePageNavigationFollowTurn>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            let outcome = {
                let (page_vm, pending_document_lifecycle_turn) =
                    entry.page_vm_and_document_lifecycle_turn_mut();
                page_vm
                    .prepare_pending_location_navigation_document_commit_one_turn_async(
                        pending_document_lifecycle_turn,
                        stage,
                    )
                    .await
            };
            let outcome = match outcome? {
                PageVmFollowNavigationTurnOutcome::Completed => {
                    LivePageNavigationFollowOutcome::Completed
                }
                PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                    target_stage,
                    outcome,
                } => LivePageNavigationFollowOutcome::PostParseLifecycle {
                    target_stage,
                    outcome,
                },
                PageVmFollowNavigationTurnOutcome::Download(download) => {
                    LivePageNavigationFollowOutcome::Download(download)
                }
                PageVmFollowNavigationTurnOutcome::PendingPhaseOne(pending) => {
                    let wake_token = entry.install_new_pending_phase_one_navigation(pending)?;
                    LivePageNavigationFollowOutcome::PendingPhaseOne { wake_token }
                }
                PageVmFollowNavigationTurnOutcome::TriggeredNavigation { stage } => {
                    LivePageNavigationFollowOutcome::TriggeredNavigation { stage }
                }
            };
            let document_commit = entry
                .has_uncommitted_page_vm()
                .then(|| entry.publish_replacement_document_commit())
                .transpose()?;
            Ok(LivePageNavigationFollowTurn {
                outcome,
                document_commit,
            })
        })
    })
    .await
}

pub(super) async fn advance_pending_phase_one_navigation_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
) -> (
    RendererPageLocalEntry,
    Result<LivePagePendingNavigationPhaseOneAdvance>,
) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            let pending = entry.take_pending_phase_one_navigation()?;
            let (residence, mut metadata) = pending.into_parts();
            let browser_context_runtime = residence
                .page_vm()
                .runtime_hooks
                .browser_context_runtime
                .clone();
            let phase_one_outcome = match residence.resume().await {
                Ok(outcome) => outcome,
                Err(error) => {
                    metadata.reject(
                        None,
                        &browser_context_runtime,
                        format!("Cannot navigate to URL: {error}"),
                    );
                    return Err(error);
                }
            };
            let phase_one_outcome = match phase_one_outcome {
                PendingPhaseOneResumeOutcome::Progress(outcome) => outcome,
                PendingPhaseOneResumeOutcome::MainResourceLoadFailed { page_vm, error } => {
                    metadata.reject(
                        None,
                        &browser_context_runtime,
                        format!("Cannot navigate to URL: {error}"),
                    );
                    entry.install_resumed_phase_one_page_vm(page_vm);
                    return Err(error);
                }
            };
            match phase_one_outcome {
                ParseTimePageVmCreationOutcome::PendingPhaseOne(residence) => {
                    let pending = PageVmPendingPhaseOneNavigation::new(residence, metadata);
                    let wake_token = entry.restore_pending_phase_one_navigation(pending)?;
                    Ok(LivePagePendingNavigationPhaseOneAdvance::Pending { wake_token })
                }
                ParseTimePageVmCreationOutcome::TriggeredNavigation { mut page_vm, stage } => {
                    metadata.complete_service_worker_follow(&mut page_vm);
                    entry.install_resumed_phase_one_page_vm(page_vm);
                    Ok(LivePagePendingNavigationPhaseOneAdvance::TriggeredNavigation { stage })
                }
                ParseTimePageVmCreationOutcome::ContinuePhaseTwo {
                    mut page_vm,
                    page_tasks,
                    stage,
                    started,
                } => {
                    metadata.complete_service_worker_follow(&mut page_vm);
                    entry.install_resumed_phase_one_page_vm(page_vm);
                    let (page_vm, pending_document_lifecycle_turn) =
                        entry.page_vm_and_document_lifecycle_turn_mut();
                    let outcome = page_vm
                        .begin_post_parse_lifecycle_on_named_owner_lane(
                            pending_document_lifecycle_turn,
                            page_tasks,
                            stage,
                            started,
                        )
                        .await?;
                    Ok(
                        LivePagePendingNavigationPhaseOneAdvance::PostParseLifecycle {
                            target_stage: stage,
                            outcome,
                        },
                    )
                }
            }
        })
    })
    .await
}

pub(super) fn remove_page_on_bound_owner_local_store(token: RendererPageToken) {
    with_bound_render_runtime_owner_local_store_session(|mut session| session.remove_page(token))
}

pub(super) fn publish_page_navigation_failure_on_bound_owner_local_store(
    token: RendererPageToken,
    failure: PageNavigationOwnerFailure,
) -> Result<PageCreationNavigationFailurePublication> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session
            .store
            .publish_page_navigation_failure(token, failure)
    })
}

pub(super) async fn remove_page_on_bound_owner_local_store_via_local_task(
    local_executor: JsLocalExecutor,
    token: RendererPageToken,
) -> Result<()> {
    run_on_bound_owner_local_store_local_task(local_executor, async move {
        remove_page_on_bound_owner_local_store(token);
        Ok(())
    })
    .await
}

pub(super) fn next_page_task_deadline_on_bound_owner_local_store() -> Option<std::time::Instant> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.next_page_task_deadline()
    })
}

/// Snapshot Pages with any due owner-scheduled task from the derived deadline
/// index. Only resident entries are indexed because a checked-out PageVm may
/// change its timer heap or delayed typed-source state before restoration.
pub(super) fn snapshot_due_page_task_tokens_on_bound_owner_local_store(
    due_at_or_before: std::time::Instant,
) -> Vec<RendererPageToken> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session
            .store
            .snapshot_due_page_task_tokens(due_at_or_before)
    })
}

pub(super) fn next_owner_maintenance_deadline_on_bound_owner_local_store()
-> Option<std::time::Instant> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.next_owner_maintenance_deadline()
    })
}

pub(super) fn claim_due_owner_maintenance_task_on_bound_owner_local_store(
    now: std::time::Instant,
) -> Option<RendererOwnerMaintenanceTask> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.claim_due_owner_maintenance_task(now)
    })
}

pub(super) fn settle_owner_maintenance_task_on_bound_owner_local_store(
    task: RendererOwnerMaintenanceTask,
    now: std::time::Instant,
) -> Result<()> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.settle_owner_maintenance_task(task, now)
    })
}

struct PageReadyDescriptorSnapshot {
    eligible: Vec<crate::page_task_queue::RendererPageReadyDescriptor>,
    stable_source_was_ready: bool,
    due_timer_was_ready: bool,
}

impl PageReadyDescriptorSnapshot {
    const fn has_ready_ordinary_source(&self) -> bool {
        self.stable_source_was_ready || self.due_timer_was_ready
    }
}

fn page_ready_descriptor_snapshot(
    entry: &mut RendererPageLocalEntry,
    task_sources: &mut RendererPageOwnedTaskSources,
) -> PageReadyDescriptorSnapshot {
    let mut descriptors = task_sources.ready_descriptors();
    let stable_source_was_ready = !descriptors.is_empty();
    let mut due_timer_was_ready = false;
    if let Some(timer) = entry.page_vm().due_page_timer_ready_descriptor() {
        descriptors.push(timer);
        due_timer_was_ready = true;
    }
    descriptors.retain(|descriptor| {
        entry
            .page_vm_mut()
            .page_ready_descriptor_is_eligible(*descriptor)
    });
    PageReadyDescriptorSnapshot {
        eligible: descriptors,
        stable_source_was_ready,
        due_timer_was_ready,
    }
}

fn select_page_scheduler_turn(
    scheduler: &mut PageTurnScheduler<RendererPageLocalEntry>,
    entry: &mut RendererPageLocalEntry,
    task_sources: &mut RendererPageOwnedTaskSources,
    lifecycle_gate: &mut Option<LifecycleGate>,
    trigger: PageTurnTrigger,
) -> RendererPageScheduledTurn {
    let snapshot = page_ready_descriptor_snapshot(entry, task_sources);
    // A lifecycle action can change an already-queued source's eligibility.
    // Preserve queue readiness, not only the pre-action eligible set, so a
    // blocked/idle lifecycle result can request exactly one fresh arbitration
    // without re-reading mutable source state after execution.
    let lifecycle_is_deferred = entry.document_lifecycle_is_deferred_until_response();
    let has_pending_document_lifecycle_turn =
        !lifecycle_is_deferred && entry.pending_document_lifecycle_identity().is_some();
    let document_lifecycle_owner_turn_is_runnable =
        !lifecycle_is_deferred && entry.document_lifecycle_owner_turn_is_runnable();
    let has_ready_main_parser_script_continuation =
        !lifecycle_is_deferred && entry.has_ready_main_parser_script_continuation();
    let document_lifecycle = DocumentLifecycleClassReadiness::from_resident_state(
        has_pending_document_lifecycle_turn,
        document_lifecycle_owner_turn_is_runnable,
        has_ready_main_parser_script_continuation,
    );
    let gate_policy = lifecycle_gate
        .as_mut()
        .map(|gate| gate.turn_policy(entry, !snapshot.eligible.is_empty()))
        .unwrap_or(LifecycleGateTurnPolicy::Normal);
    let selected_class = match gate_policy {
        LifecycleGateTurnPolicy::Normal => {
            scheduler.select_turn_class(trigger, !snapshot.eligible.is_empty(), document_lifecycle)
        }
        LifecycleGateTurnPolicy::Drive {
            reconsider_displaced_ordinary,
        } => scheduler.select_lifecycle_turn(
            reconsider_displaced_ordinary,
            !snapshot.eligible.is_empty(),
            document_lifecycle,
        ),
        LifecycleGateTurnPolicy::Park => {
            return RendererPageScheduledTurn::SpentWake;
        }
    };
    match selected_class {
        Some(PageTurnClass::DocumentLifecycle) => RendererPageScheduledTurn::DocumentLifecycle {
            displaced_ordinary: RendererDisplacedOrdinaryTurn::from_ready_source(
                snapshot.has_ready_ordinary_source(),
            ),
        },
        Some(PageTurnClass::Ordinary) => {
            let selected = scheduler
                .select_ready_descriptor(snapshot.eligible)
                .expect("selected ordinary Page-turn class must retain an eligible descriptor");
            let task = task_sources.take_task(selected);
            RendererPageScheduledTurn::Ordinary(Box::new(task))
        }
        None => RendererPageScheduledTurn::SpentWake,
    }
}

/// Run one ordinary page-owner turn already selected by the Page scheduler.
/// Every ordinary task is a concrete typed source payload or a due timer. The
/// caller must restore the returned entry before scheduling any continuation.
pub(super) async fn advance_page_owner_one_turn_via_local_task(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
    task: RendererPageSchedulerTask,
    loader: ResourceRequestClient,
) -> (RendererPageLocalEntry, Result<()>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            let replacement_lifecycle_snapshot = entry
                .page_vm()
                .document_replacement_lifecycle_action_snapshot();
            // This scope belongs to the common Page owner, not the selected
            // task executor: command-local wait drivers reuse that executor
            // but retain their own typed navigation continuation.
            let _navigation_handoff_scope = entry
                .page_vm()
                .vm()
                .begin_ordinary_page_turn_navigation_handoff()?;
            let application = entry
                .page_vm_mut()
                .apply_selected_page_scheduler_task(task, &loader)
                .await;
            // Any JavaScript-capable Page action can synchronously replace the
            // main Document through document.open()/document.close(). Reconcile
            // the exact transition caused by this action at the common owner
            // boundary. Reconciliation must still run when the action reports
            // an error: JavaScript side effects are not rolled back by an
            // exception. A selected Page action cannot repair an older missed
            // transition.
            let reconciliation = {
                let (page_vm, pending_document_lifecycle_turn) =
                    entry.page_vm_and_document_lifecycle_turn_mut();
                page_vm
                    .reconcile_document_replacement_lifecycle_after_owner_action(
                        replacement_lifecycle_snapshot,
                        pending_document_lifecycle_turn,
                    )
                    .await
            };

            match (application, reconciliation) {
                (Ok(()), Ok(_)) => Ok(()),
                (Err(action_error), Ok(_)) => Err(action_error),
                (Ok(_), Err(reconciliation_error)) => Err(reconciliation_error),
                (Err(action_error), Err(reconciliation_error)) => Err(anyhow!(
                    "page action failed ({action_error:#}) and its Document replacement lifecycle reconciliation also failed ({reconciliation_error:#})"
                )),
            }
        })
    })
    .await
}

/// Execute at most one action from the exact-Document lifecycle resident.
/// A missing resident is an idle stale-wake outcome; this helper never binds
/// a page wake to whichever Document happens to be current.
pub(super) async fn advance_document_lifecycle_one_page_turn_via_local_task(
    local_executor: JsLocalExecutor,
    entry: RendererPageLocalEntry,
) -> (RendererPageLocalEntry, Result<DocumentLifecycleTurnOutcome>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            let Some(document) = entry.pending_document_lifecycle_identity() else {
                return Ok(DocumentLifecycleTurnOutcome::idle(
                    DocumentLifecycleTurnAction::None,
                ));
            };
            let (page_vm, pending_document_lifecycle_turn) =
                entry.page_vm_and_document_lifecycle_turn_mut();
            let outcome = page_vm
                .advance_post_parse_lifecycle_one_owner_turn(
                    pending_document_lifecycle_turn,
                    document,
                )
                .await?;
            if let Some(pending) = pending_document_lifecycle_turn.as_mut() {
                pending.owner_turn_is_runnable = matches!(
                    outcome.readiness,
                    DocumentLifecycleTurnReadiness::Runnable { .. }
                );
            }
            Ok(outcome)
        })
    })
    .await
}

pub(super) fn observe_document_lifecycle_on_entry(
    entry: &mut RendererPageLocalEntry,
    document: RendererDocumentLifecycleIdentity,
    target_stage: PageVmInitStage,
) -> DocumentLifecycleObserverOutcome {
    entry.observe_document_lifecycle(document, target_stage)
}

fn reconcile_page_creation_lifecycle_observation(
    observation: DocumentLifecycleObserverOutcome,
    has_pending_location_navigation: bool,
) -> DocumentLifecycleObserverOutcome {
    match observation {
        DocumentLifecycleObserverOutcome::Reached if has_pending_location_navigation => {
            DocumentLifecycleObserverOutcome::NavigationPending
        }
        observation => observation,
    }
}

pub(super) fn has_pending_document_lifecycle_turn_on_entry(
    entry: &mut RendererPageLocalEntry,
) -> bool {
    entry.pending_document_lifecycle_identity().is_some()
}

impl RendererOwnerLocalStoreSession<'_> {
    fn reserve_renderer_document_isolate(
        &mut self,
        owner: &RendererOwnerLocalContext,
        page_id: PageId,
        page_runtime_task_source: crate::page_task_queue::PageRuntimeTaskSource,
    ) -> Result<(
        RendererDocumentIsolateBootstrap,
        RendererDocumentIsolateReservation,
    )> {
        self.store.reserve_renderer_document_isolate_for_owner(
            owner,
            page_id,
            page_runtime_task_source,
        )
    }

    fn remove_reserved_renderer_document_isolate(
        &mut self,
        token: RendererPageToken,
        reservation_id: u64,
    ) {
        self.store
            .remove_reserved_renderer_document_isolate(token, reservation_id)
    }

    fn install_page_vm(
        &mut self,
        owner: &RendererOwnerLocalContext,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        vm: PageVm,
        pending_download: Option<RendererPendingDownloadActivation>,
        lifecycle_gate: Option<PageVmInitStage>,
    ) -> Result<RendererPendingPageCreation> {
        self.store.install_page_vm_for_owner(
            owner,
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            vm,
            pending_download,
            lifecycle_gate,
        )
    }

    fn install_phase_one_blocked_page_for_owner(
        &mut self,
        owner: &RendererOwnerLocalContext,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        pending_navigation: PageVmPendingPhaseOneNavigation,
        lifecycle_gate: Option<PageVmInitStage>,
    ) -> Result<RendererPendingPageCreation> {
        self.store.install_phase_one_blocked_page_for_owner(
            owner,
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            pending_navigation,
            lifecycle_gate,
        )
    }

    fn finalize_pending_page_creation(
        &mut self,
        pending: RendererPendingPageCreation,
    ) -> RendererPageCreationCommit {
        self.store.finalize_pending_page_creation(pending)
    }

    fn take_entry_for_command(
        &mut self,
        token: RendererPageToken,
    ) -> Result<RendererPageLocalEntry> {
        self.store.take_entry_for_command(token)
    }

    fn checkout_entry_for_owner_turn(
        &mut self,
        token: RendererPageToken,
    ) -> RendererPageLocalEntryCheckout {
        self.store.checkout_entry_for_owner_turn(token)
    }

    fn remove_page(&mut self, token: RendererPageToken) {
        self.store.remove_page(token)
    }

    fn restore_entry_after_command(
        &mut self,
        token: RendererPageToken,
        entry: RendererPageLocalEntry,
    ) {
        self.store.restore_entry_after_command(token, entry);
    }

    pub(super) fn current_page_state_for_testing(
        &mut self,
        token: RendererPageToken,
    ) -> Result<Arc<RendererPageState>> {
        self.store.current_page_state_for_testing(token)
    }

    pub(super) fn renderer_page_view_for_testing(
        &mut self,
        token: RendererPageToken,
    ) -> Result<RendererPageView> {
        self.store.renderer_page_view_for_testing(token)
    }

    pub(super) fn owner_slot_for_testing(
        &mut self,
        token: RendererPageToken,
    ) -> Result<RendererPageSlotHandle> {
        self.store.owner_slot_for_testing(token)
    }

    pub(super) fn host_instance_key_for_testing(
        &mut self,
        token: RendererPageToken,
    ) -> Result<usize> {
        self.store.host_instance_key_for_testing(token)
    }

    pub(super) fn host_unique_document_isolate_count_for_testing(
        &mut self,
        token: RendererPageToken,
    ) -> Result<usize> {
        self.store
            .host_unique_document_isolate_count_for_testing(token)
    }
}

impl RendererOwnerLocalStore {
    pub(super) fn store_prepared_document(
        &mut self,
        token: RendererPageReservationToken,
        residence: RendererPreparedDocumentResidence,
    ) -> Result<()> {
        match self.prepared_documents.entry(token) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(residence);
                Ok(())
            }
            std::collections::hash_map::Entry::Occupied(_) => Err(anyhow!(
                "renderer owner already tracks prepared document for page {}",
                token.page_id().as_u64()
            )),
        }
    }

    pub(super) fn take_prepared_document(
        &mut self,
        token: RendererPageReservationToken,
    ) -> Result<RendererPreparedDocumentResidence> {
        self.prepared_documents.remove(&token).ok_or_else(|| {
            anyhow!(
                "renderer owner no longer tracks prepared document for page {}",
                token.page_id().as_u64()
            )
        })
    }

    pub(super) fn update_prepared_document_commit_configuration(
        &mut self,
        token: RendererPageReservationToken,
        configuration: crate::runtime::RendererPreparedDocumentCommitConfiguration,
    ) -> Result<()> {
        let residence = self.prepared_documents.get_mut(&token).ok_or_else(|| {
            anyhow!(
                "renderer owner no longer tracks prepared document for page {}",
                token.page_id().as_u64()
            )
        })?;
        macro_rules! apply_configuration {
            ($request:expr) => {{
                let request = $request;
                request.document_start_scripts = configuration.document_start_scripts;
                request.runtime_bindings = configuration.runtime_bindings;
                request.runtime_inspector_session_restore_snapshots =
                    configuration.runtime_inspector_session_restore_snapshots;
                request.runtime_isolated_worlds = configuration.runtime_isolated_worlds;
                request.permission_overrides = configuration.permission_overrides;
                request.extra_http_headers = configuration.extra_http_headers;
                request.locale_override = configuration.locale_override;
                request.timezone_override = configuration.timezone_override;
                request.script_execution_disabled = configuration.script_execution_disabled;
                request.bypass_content_security_policy =
                    configuration.bypass_content_security_policy;
                request.cpu_throttling_rate = configuration.cpu_throttling_rate;
                request.emulated_media = configuration.emulated_media;
                request.idle_override = configuration.idle_override;
                request.viewport_surface = configuration.viewport_surface;
                request.network_offline = configuration.network_offline;
                request.blocked_url_patterns = configuration.blocked_url_patterns;
                request.fetch_subresource_interception_enabled =
                    configuration.fetch_subresource_interception_enabled;
                request.fetch_subresource_interception_resource_type =
                    configuration.fetch_subresource_interception_resource_type;
            }};
        }
        apply_configuration!(&mut residence.request);
        Ok(())
    }

    pub(super) fn cancel_prepared_document(&mut self, token: RendererPageReservationToken) {
        if let Some(residence) = self.prepared_documents.remove(&token) {
            self.drop_prepared_document_residence(residence);
        }
    }

    fn drop_prepared_document_residence(&mut self, residence: RendererPreparedDocumentResidence) {
        let reservation_token = residence.isolate_reservation.token();
        let reservation_id = residence.isolate_reservation.reservation_id();
        self.remove_reserved_renderer_document_isolate(reservation_token, reservation_id);
        residence.isolate_reservation.disarm_for_attach();
        drop(residence);
    }

    fn publish_page_navigation_failure(
        &mut self,
        token: RendererPageToken,
        failure: PageNavigationOwnerFailure,
    ) -> Result<PageCreationNavigationFailurePublication> {
        #[cfg(debug_assertions)]
        Self::ensure_token_thread(&token)?;
        let page_slot = self
            .page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
            .ok_or_else(|| {
                anyhow!(
                    "renderer local host no longer tracks page {}",
                    token.page_id.as_u64()
                )
            })?;
        Ok(page_slot
            .page_creation_navigation_failure_publisher
            .publish(failure))
    }

    fn resolve_pending_page_creation(
        &mut self,
        pending: RendererPendingPageCreation,
        document: RendererDocumentLifecycleIdentity,
        target_stage: PageVmInitStage,
        navigation_reply_policy: NavigationReplyPolicy,
    ) -> RendererPageCreationResolution {
        // Failure selection and lifecycle entry checkout are one owner-local
        // operation. The creation observer was registered before this Page
        // could run navigation work, so a concrete navigation terminal must
        // win before its generic lifecycle wait resumes.
        let token = pending.token;
        let mut entry = match self.take_entry_for_command(token) {
            Ok(entry) => entry,
            Err(error) => {
                return RendererPageCreationResolution::without_renderer_output(
                    PageCreationResolution::EntryUnavailable { error },
                );
            }
        };
        if let Some(failure) = pending.navigation_failure_observer.failure() {
            let renderer_output = entry.page_vm_mut().settle_renderer_output_publication();
            self.restore_entry_after_command(token, entry);
            return RendererPageCreationResolution::retiring(
                PageCreationRetirement::NavigationFailed(failure),
                renderer_output,
            );
        }
        let observation = observe_document_lifecycle_on_entry(&mut entry, document, target_stage);
        // A load handler can enqueue a same-Document navigation in the turn
        // that reaches the requested milestone. Page creation with
        // `FollowBeforeReply` must observe that navigation before publishing
        // the old Document; generic lifecycle observers still treat the
        // milestone itself as reached.
        let observation = reconcile_page_creation_lifecycle_observation(
            observation,
            entry.page_vm().vm().has_pending_location_navigation(),
        );
        match observation {
            DocumentLifecycleObserverOutcome::Reached => {
                self.commit_observed_page_creation(pending, entry)
            }
            DocumentLifecycleObserverOutcome::NavigationPending
                if navigation_reply_policy.returns_with_pending_navigation() =>
            {
                self.commit_observed_page_creation(pending, entry)
            }
            DocumentLifecycleObserverOutcome::Pending
            | DocumentLifecycleObserverOutcome::NavigationPending => {
                self.restore_entry_after_command(token, entry);
                RendererPageCreationResolution::without_renderer_output(
                    PageCreationResolution::Waiting { pending, document },
                )
            }
            DocumentLifecycleObserverOutcome::DocumentReplaced { document } => {
                self.restore_entry_after_command(token, entry);
                RendererPageCreationResolution::without_renderer_output(
                    PageCreationResolution::Waiting { pending, document },
                )
            }
            DocumentLifecycleObserverOutcome::Interrupted(termination) => self
                .retire_checked_out_page_creation(
                    token,
                    entry,
                    PageCreationRetirement::LifecycleInterrupted {
                        target_stage,
                        termination,
                    },
                ),
            DocumentLifecycleObserverOutcome::MissingResident => self
                .retire_checked_out_page_creation(
                    token,
                    entry,
                    PageCreationRetirement::MissingLifecycleResident {
                        target_stage,
                        document,
                    },
                ),
        }
    }

    fn host_for_id(
        &mut self,
        owner_key: RendererOwnerLocalHostId,
    ) -> &mut RendererOwnerLocalPageHost {
        if !self.page_hosts.contains_key(&owner_key) {
            let host = RendererOwnerLocalPageHost {
                pages: HashMap::new(),
                reserved_renderer_document_isolates: HashMap::new(),
                instance_key: self.next_host_instance_key,
            };
            self.next_host_instance_key = self.next_host_instance_key.saturating_add(1);
            self.page_hosts.insert(owner_key, host);
        }
        self.page_hosts
            .get_mut(&owner_key)
            .expect("renderer owner local runtime should retain host after insertion")
    }

    fn host_by_id_mut(
        &mut self,
        host_id: RendererOwnerLocalHostId,
    ) -> Result<&mut RendererOwnerLocalPageHost> {
        self.page_hosts.get_mut(&host_id).ok_or_else(|| {
            anyhow!(
                "renderer owner local runtime no longer tracks host {}",
                host_id.as_u64()
            )
        })
    }

    fn reserve_renderer_document_isolate_for_owner(
        &mut self,
        owner: &RendererOwnerLocalContext,
        page_id: PageId,
        page_runtime_task_source: crate::page_task_queue::PageRuntimeTaskSource,
    ) -> Result<(
        RendererDocumentIsolateBootstrap,
        RendererDocumentIsolateReservation,
    )> {
        debug_assert!(
            is_on_named_owner_execution_lane_for(&owner.owner_state.local_executor),
            "renderer document isolate reservation must execute on the matching named owner lane"
        );
        let token = renderer_page_token_for_owner_context(owner, page_id);
        #[cfg(debug_assertions)]
        Self::ensure_token_thread(&token)?;
        let existing_page_routes = self
            .page_hosts
            .get(&owner.local_host_id)
            .and_then(|host| host.pages.get(&page_id))
            .map(|page_slot| page_slot.task_sources.producer_routes());
        let (runtime_wake, owner_wake) = page_runtime_task_source
            .owner_attached_page_source_wakes()
            .ok_or_else(|| anyhow!("owner-reserved Page sources require a stable owner wake"))?;
        let (initial_task_sources, producer_routes) = match existing_page_routes {
            Some(producer_routes) => (None, producer_routes),
            None => {
                let (task_sources, producer_routes) =
                    RendererPageOwnedTaskSources::new(runtime_wake, owner_wake);
                (Some(task_sources), producer_routes)
            }
        };
        page_runtime_task_source.bind_page_task_producer_routes(producer_routes)?;
        let v8_foreground_task_sender = page_runtime_task_source
            .v8_foreground_task_sender()
            .ok_or_else(|| anyhow!("owner-reserved Page is missing its V8 foreground source"))?;
        let bootstrap =
            RendererDocumentIsolateHandle::new_owner_reserved_page(v8_foreground_task_sender)?;
        let host_handle = bootstrap.clone_renderer_document_isolate_handle_for_owner_retention();
        let reservation_id = self.next_renderer_document_isolate_reservation_id;
        self.next_renderer_document_isolate_reservation_id = self
            .next_renderer_document_isolate_reservation_id
            .saturating_add(1);
        let page_inspector =
            DocumentInspectorBinding::new(bootstrap.inspector_isolate_backend_handle());
        let output_stream = RendererOutputStreamIdentity::new_page(
            owner.local_host_id,
            page_id,
            page_inspector.agent_token(),
        );
        let output_journal = match owner
            .owner_state
            .browser_context_runtime
            .renderer_output_transport_sender()
        {
            Some(transport) => {
                RendererTurnOutputJournal::new_with_transport(output_stream, transport)
            }
            None => RendererTurnOutputJournal::new(output_stream),
        };
        let page_inspector = page_inspector.with_output_journal(output_journal.clone());
        let page_script_environment = RendererPageScriptEnvironment::new(
            page_id.as_u64(),
            host_handle.clone(),
            page_runtime_task_source,
            output_journal.clone(),
        );
        let host = self.host_for_id(owner.local_host_id);
        host.reserved_renderer_document_isolates
            .entry(page_id)
            .or_default()
            .push(RendererDocumentIsolateReservationEntry {
                id: reservation_id,
                handle: host_handle,
                output_journal,
                initial_task_sources,
                _accounting: RendererDocumentIsolateReservationAccounting::new(),
            });
        Ok((
            bootstrap
                .with_page_inspector(page_inspector)
                .with_renderer_page_script_environment(page_script_environment),
            RendererDocumentIsolateReservation {
                inner: Rc::new(RendererDocumentIsolateReservationState {
                    token,
                    reservation_id,
                    active: std::cell::Cell::new(true),
                }),
            },
        ))
    }

    fn host_has_no_page_state(host: &RendererOwnerLocalPageHost) -> bool {
        host.pages.is_empty() && host.reserved_renderer_document_isolates.is_empty()
    }

    fn remove_reserved_renderer_document_isolate(
        &mut self,
        token: RendererPageToken,
        reservation_id: u64,
    ) {
        #[cfg(debug_assertions)]
        {
            debug_assert!(
                Self::ensure_token_thread(&token).is_ok(),
                "renderer document isolate reservation for page {} dropped on a different thread than its owner-local host",
                token.page_id.as_u64()
            );
            if Self::ensure_token_thread(&token).is_err() {
                return;
            }
        }
        let mut removed_reservation = None;
        let should_remove_host = if let Ok(host) = self.host_by_id_mut(token.local_host_id) {
            if let Some(entries) = host
                .reserved_renderer_document_isolates
                .get_mut(&token.page_id)
            {
                if let Some(index) = entries.iter().position(|entry| entry.id == reservation_id) {
                    removed_reservation = Some(entries.swap_remove(index));
                }
                if entries.is_empty() {
                    host.reserved_renderer_document_isolates
                        .remove(&token.page_id);
                }
            }
            Self::host_has_no_page_state(host)
        } else {
            false
        };
        if should_remove_host {
            let removed = self.page_hosts.remove(&token.local_host_id);
            debug_assert!(
                removed
                    .as_ref()
                    .is_none_or(|host| { Self::host_has_no_page_state(host) }),
                "renderer owner local runtime removed non-empty host {}",
                token.local_host_id.as_u64()
            );
        }
        if let Some(reservation) = removed_reservation {
            Self::retire_unattached_renderer_document_isolates([reservation]);
        }
    }

    fn retire_unattached_renderer_document_isolates(
        reservations: impl IntoIterator<Item = RendererDocumentIsolateReservationEntry>,
    ) {
        for reservation in reservations {
            reservation
                .output_journal
                .retire(RendererOutputStreamCloseReason::ResidenceRetired);
        }
    }

    fn take_entry_for_command(
        &mut self,
        token: RendererPageToken,
    ) -> Result<RendererPageLocalEntry> {
        match self.checkout_entry_for_owner_turn(token) {
            Ok(entry) => Ok(entry),
            Err(RendererPageLocalEntryCheckoutError::Busy) => Err(anyhow!(
                "renderer local host page {} is already running an owner turn",
                token.page_id.as_u64()
            )),
            Err(RendererPageLocalEntryCheckoutError::Retired) => Err(anyhow!(
                "renderer local host page {} is retiring",
                token.page_id.as_u64()
            )),
            Err(RendererPageLocalEntryCheckoutError::Missing) => Err(anyhow!(
                "renderer local host no longer tracks page {}",
                token.page_id.as_u64()
            )),
        }
    }

    pub(super) fn page_uncommitted_vm_creation_id(&self, token: RendererPageToken) -> Option<u64> {
        self.page_hosts
            .get(&token.local_host_id)
            .and_then(|host| host.pages.get(&token.page_id))
            .and_then(RendererOwnerLocalPageSlot::resident_entry)
            .and_then(RendererPageLocalEntry::uncommitted_page_vm_creation_id)
    }

    fn admit_page_command_first_dispatch(
        &mut self,
        token: RendererPageToken,
        lane: PageCommandFirstDispatchLane,
        turn: RenderRuntimePendingTurn,
    ) -> Option<RenderRuntimePendingTurn> {
        let Some(page_slot) = self
            .page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
        else {
            // Let the command run once so the ordinary stale-Page error path
            // remains responsible for its protocol reply.
            return Some(turn);
        };
        page_slot.page_command_first_dispatch.admit(lane, turn)
    }

    fn complete_page_command_first_dispatch(
        &mut self,
        token: RendererPageToken,
        lane: &PageCommandFirstDispatchLane,
    ) -> Option<RenderRuntimePendingTurn> {
        self.page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
            .and_then(|slot| slot.page_command_first_dispatch.complete(lane))
    }

    fn checkout_entry_for_owner_turn(
        &mut self,
        token: RendererPageToken,
    ) -> RendererPageLocalEntryCheckout {
        #[cfg(debug_assertions)]
        assert_eq!(
            token.local_thread_id,
            std::thread::current().id(),
            "renderer page entry checkout must run on its owner thread"
        );
        let checkout = match self
            .page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
        {
            Some(page_slot) => match page_slot.turn_scheduler.checkout() {
                RendererPageEntryCheckout::Entry(entry) => Ok(entry),
                RendererPageEntryCheckout::Busy => Err(RendererPageLocalEntryCheckoutError::Busy),
                RendererPageEntryCheckout::Retired => {
                    Err(RendererPageLocalEntryCheckoutError::Retired)
                }
            },
            None => Err(RendererPageLocalEntryCheckoutError::Missing),
        };
        if checkout.is_ok() {
            self.page_task_deadline_index.remove(token);
        }
        #[cfg(debug_assertions)]
        self.debug_assert_page_task_deadline_index_consistent_for_token(token);
        checkout
    }

    fn schedule_page_turn(
        &mut self,
        token: RendererPageToken,
        trigger: PageTurnTrigger,
    ) -> RendererPageTurnAdmission {
        #[cfg(debug_assertions)]
        assert_eq!(
            token.local_thread_id,
            std::thread::current().id(),
            "renderer page wake must be scheduled on its owner thread"
        );
        let Some(page_slot) = self
            .page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
        else {
            return RendererPageTurnAdmission::MissingPage;
        };
        let admission = page_slot.turn_scheduler.admit_turn(trigger);
        match admission {
            PageTurnAdmission::EnqueueOwnerTurn => RendererPageTurnAdmission::EnqueueOwnerTurn,
            PageTurnAdmission::AlreadyScheduled => RendererPageTurnAdmission::AlreadyScheduled,
            PageTurnAdmission::Retired => RendererPageTurnAdmission::Retired,
        }
    }

    fn release_post_response_document_lifecycle(
        &mut self,
        token: RendererPageToken,
        document: RendererDocumentLifecycleIdentity,
    ) -> bool {
        #[cfg(debug_assertions)]
        assert_eq!(
            token.local_thread_id,
            std::thread::current().id(),
            "post-response continuation must be released on its owner thread"
        );
        self.page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
            .and_then(RendererOwnerLocalPageSlot::resident_entry_mut)
            .is_some_and(|entry| entry.release_document_lifecycle_after_response(document))
    }

    fn release_lifecycle_gate(
        &mut self,
        token: RendererPageToken,
    ) -> Result<ReleasedLifecycleGate> {
        #[cfg(debug_assertions)]
        Self::ensure_token_thread(&token)?;
        let gate = self
            .page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
            .and_then(|slot| slot.lifecycle_gate.take())
            .with_context(|| {
                anyhow!(
                    "renderer page {} has no page-creation lifecycle-target gate",
                    token.page_id.as_u64()
                )
            })?;
        Ok(ReleasedLifecycleGate {
            target_stage: gate.target_stage,
            resume_parked_page_turn: gate.parked_admitted_wake,
        })
    }

    fn checkout_scheduled_page_turn(
        &mut self,
        token: RendererPageToken,
    ) -> RendererPageTurnCheckout {
        #[cfg(debug_assertions)]
        assert_eq!(
            token.local_thread_id,
            std::thread::current().id(),
            "renderer page turn must run on its owner thread"
        );
        let checkout = match self
            .page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
        {
            Some(page_slot) => {
                let RendererOwnerLocalPageSlot {
                    turn_scheduler,
                    task_sources,
                    lifecycle_gate,
                    ..
                } = page_slot;
                match turn_scheduler.checkout_scheduled_turn() {
                    ScheduledPageTurnCheckout::Turn { mut entry, trigger } => {
                        let scheduled_turn = select_page_scheduler_turn(
                            turn_scheduler,
                            &mut entry,
                            task_sources,
                            lifecycle_gate,
                            trigger,
                        );
                        Ok((entry, trigger, scheduled_turn))
                    }
                    ScheduledPageTurnCheckout::NotScheduled => {
                        Err(RendererPageTurnCheckoutError::NotScheduled)
                    }
                    ScheduledPageTurnCheckout::Busy => Err(RendererPageTurnCheckoutError::Busy),
                    ScheduledPageTurnCheckout::Retired => {
                        Err(RendererPageTurnCheckoutError::Retired)
                    }
                }
            }
            None => Err(RendererPageTurnCheckoutError::Missing),
        };
        if checkout.is_ok() {
            self.page_task_deadline_index.remove(token);
        }
        #[cfg(debug_assertions)]
        self.debug_assert_page_task_deadline_index_consistent_for_token(token);
        checkout
    }

    fn has_ready_page_networking_task(
        &mut self,
        token: RendererPageToken,
        current_document: RendererDocumentToken,
    ) -> bool {
        #[cfg(debug_assertions)]
        assert_eq!(
            token.local_thread_id,
            std::thread::current().id(),
            "Page networking readiness must be queried on its owner thread"
        );
        let Some(host) = self.page_hosts.get_mut(&token.local_host_id) else {
            return false;
        };
        if let Some(page_slot) = host.pages.get_mut(&token.page_id) {
            return page_slot
                .task_sources
                .has_ready_networking_task_for(current_document);
        }
        host.reserved_renderer_document_isolates
            .get_mut(&token.page_id)
            .is_some_and(|reservations| {
                reservations.iter_mut().any(|reservation| {
                    reservation
                        .initial_task_sources
                        .as_mut()
                        .is_some_and(|sources| {
                            sources.has_ready_networking_task_for(current_document)
                        })
                })
            })
    }

    fn pending_phase_one_admission_after_restore(
        &mut self,
        token: RendererPageToken,
    ) -> PhaseOneResidenceAdmission {
        #[cfg(debug_assertions)]
        assert_eq!(
            token.local_thread_id,
            std::thread::current().id(),
            "phase-one admission must be reconciled on its owner thread"
        );
        let page_slot = self
            .page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
            .expect("phase-one admission requires a live Page slot");
        let RendererOwnerLocalPageSlot {
            turn_scheduler,
            task_sources,
            ..
        } = page_slot;
        let entry = turn_scheduler
            .resident_mut()
            .expect("phase-one admission requires a restored Page entry");
        let restore_requirement = {
            let pending = entry
                .pending_phase_one_navigation
                .as_ref()
                .expect("phase-one admission requires a resident continuation");
            pending.phase_one_restore_requirement()
        };
        let streaming_input_ready = entry.pending_phase_one_navigation_has_ready_streaming_input();
        let page_turn_is_runnable = !page_ready_descriptor_snapshot(entry, task_sources)
            .eligible
            .is_empty();

        PhaseOneResidenceAdmission::after_stable_restore(
            restore_requirement,
            page_turn_is_runnable,
            streaming_input_ready,
        )
    }

    fn page_turn_readiness_after_restore(
        &mut self,
        token: RendererPageToken,
    ) -> Option<PageOwnerTurnReadiness> {
        #[cfg(debug_assertions)]
        assert_eq!(
            token.local_thread_id,
            std::thread::current().id(),
            "Page turn readiness must be settled on its owner thread"
        );
        let page_slot = self
            .page_hosts
            .get_mut(&token.local_host_id)?
            .pages
            .get_mut(&token.page_id)?;
        if page_slot.turn_scheduler.is_retiring() {
            return None;
        }
        let RendererOwnerLocalPageSlot {
            turn_scheduler,
            task_sources,
            ..
        } = page_slot;
        let entry = turn_scheduler.resident_mut()?;
        let snapshot = page_ready_descriptor_snapshot(entry, task_sources);
        if !snapshot.eligible.is_empty() {
            return Some(PageOwnerTurnReadiness::Runnable);
        }
        if snapshot.stable_source_was_ready {
            return Some(PageOwnerTurnReadiness::Blocked {
                reason: PageOwnerBlockedReason::NoEligibleSource,
                deadline: entry.page_vm().vm().next_timeout_deadline(),
            });
        }
        Some(PageOwnerTurnReadiness::Idle)
    }

    fn next_page_task_deadline(&self) -> Option<std::time::Instant> {
        #[cfg(debug_assertions)]
        self.debug_assert_page_task_deadline_index_consistent();
        self.page_task_deadline_index.next_deadline()
    }

    fn snapshot_due_page_task_tokens(
        &self,
        due_at_or_before: std::time::Instant,
    ) -> Vec<RendererPageToken> {
        #[cfg(debug_assertions)]
        self.debug_assert_page_task_deadline_index_consistent();
        self.page_task_deadline_index
            .snapshot_due_tokens(due_at_or_before)
    }

    fn next_owner_maintenance_deadline(&self) -> Option<std::time::Instant> {
        #[cfg(debug_assertions)]
        self.debug_assert_owner_maintenance_deadline_index_consistent();
        self.owner_maintenance_deadline_index.next_deadline()
    }

    fn claim_due_owner_maintenance_task(
        &mut self,
        now: std::time::Instant,
    ) -> Option<RendererOwnerMaintenanceTask> {
        #[cfg(debug_assertions)]
        self.debug_assert_owner_maintenance_deadline_index_consistent();
        let token = self
            .owner_maintenance_deadline_index
            .snapshot_due_tokens(now)
            .into_iter()
            .next()?;
        self.owner_maintenance_deadline_index.remove(token);
        let task = self
            .page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
            .and_then(|page_slot| page_slot.owner_maintenance.claim_if_due(token, now))
            .expect("due owner-maintenance index entry must retain a claimable Page residence");
        #[cfg(debug_assertions)]
        self.debug_assert_owner_maintenance_deadline_index_consistent_for_token(token);
        Some(task)
    }

    fn settle_owner_maintenance_task(
        &mut self,
        task: RendererOwnerMaintenanceTask,
        now: std::time::Instant,
    ) -> Result<()> {
        let token = task.token();
        let page_slot = self
            .page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
            .ok_or_else(|| {
                anyhow!(
                    "checked-out owner-maintenance Page {} lost its stable slot before settlement",
                    token.page_id.as_u64()
                )
            })?;
        page_slot.owner_maintenance.settle(task, now)?;
        self.reindex_owner_maintenance_for_token(token);
        Ok(())
    }

    fn install_page_vm_for_owner(
        &mut self,
        owner: &RendererOwnerLocalContext,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        mut vm: PageVm,
        pending_download: Option<RendererPendingDownloadActivation>,
        lifecycle_gate: Option<PageVmInitStage>,
    ) -> Result<RendererPendingPageCreation> {
        debug_assert!(
            is_on_named_owner_execution_lane_for(&owner.owner_state.local_executor),
            "page installation must execute on the matching named owner lane"
        );
        let state_capture = vm.capture_page_state_on_named_owner_lane()?;
        let page_state = RendererPageState::from_vm_state_capture(
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            state_capture,
        );
        let slot = Self::create_initial_slot_for_vm(owner, &vm, page_state);
        let page_context_cancel_tx = slot.page_context_cancel_sender();
        let entry = RendererPageLocalEntry::new(slot.clone(), vm)?;
        let (navigation_failure_publisher, navigation_failure_observer) =
            page_creation_navigation_failure_scope();
        let token = self.attach_page_entry_for_owner(
            owner,
            slot,
            entry,
            lifecycle_gate,
            navigation_failure_publisher,
        )?;
        Ok(self.prepare_pending_page_creation(
            token,
            navigation_failure_observer,
            page_context_cancel_tx,
            pending_download,
        ))
    }

    fn install_phase_one_blocked_page_for_owner(
        &mut self,
        owner: &RendererOwnerLocalContext,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        mut pending_navigation: PageVmPendingPhaseOneNavigation,
        lifecycle_gate: Option<PageVmInitStage>,
    ) -> Result<RendererPendingPageCreation> {
        debug_assert!(
            is_on_named_owner_execution_lane_for(&owner.owner_state.local_executor),
            "phase-one-blocked page installation must execute on the matching named owner lane"
        );
        let page_vm = pending_navigation.page_vm_mut();
        let state_capture = page_vm.capture_page_state_on_named_owner_lane()?;
        let page_state = RendererPageState::from_vm_state_capture(
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            state_capture,
        );
        let slot = Self::create_initial_slot_for_vm(owner, page_vm, page_state);
        let page_context_cancel_tx = slot.page_context_cancel_sender();
        let entry = RendererPageLocalEntry::new_with_pending_phase_one_navigation(
            slot.clone(),
            pending_navigation,
        )?;
        let (navigation_failure_publisher, navigation_failure_observer) =
            page_creation_navigation_failure_scope();
        let token = self.attach_page_entry_for_owner(
            owner,
            slot,
            entry,
            lifecycle_gate,
            navigation_failure_publisher,
        )?;
        Ok(self.prepare_pending_page_creation(
            token,
            navigation_failure_observer,
            page_context_cancel_tx,
            None,
        ))
    }

    fn prepare_pending_page_creation(
        &mut self,
        token: RendererPageToken,
        navigation_failure_observer: PageCreationNavigationFailureObserver,
        page_context_cancel_tx: RendererPageContextCancelSender,
        pending_download: Option<RendererPendingDownloadActivation>,
    ) -> RendererPendingPageCreation {
        self.reindex_page_task_for_token(token);
        self.reindex_owner_maintenance_for_token(token);
        RendererPendingPageCreation {
            token,
            navigation_failure_observer,
            page_context_cancel_tx,
            pending_download,
            lifecycle_decider: None,
        }
    }

    fn finalize_pending_page_creation(
        &mut self,
        pending: RendererPendingPageCreation,
    ) -> RendererPageCreationCommit {
        let token = pending.token;
        let entry = match self.checkout_entry_for_owner_turn(token) {
            Ok(entry) => entry,
            Err(RendererPageLocalEntryCheckoutError::Busy) => {
                return RendererPageCreationCommit {
                    finalized: Err(anyhow!(
                        "renderer page {} remained checked out while finalizing page creation",
                        token.page_id.as_u64()
                    )),
                    renderer_output: None,
                };
            }
            Err(
                RendererPageLocalEntryCheckoutError::Retired
                | RendererPageLocalEntryCheckoutError::Missing,
            ) => {
                return RendererPageCreationCommit {
                    finalized: Err(anyhow!(
                        "renderer page {} was retired before page creation completed",
                        token.page_id.as_u64()
                    )),
                    renderer_output: None,
                };
            }
        };
        self.commit_page_creation_reply(pending, entry)
    }

    fn commit_observed_page_creation(
        &mut self,
        pending: RendererPendingPageCreation,
        entry: RendererPageLocalEntry,
    ) -> RendererPageCreationResolution {
        if pending.has_lifecycle_decider() {
            self.restore_entry_after_command(pending.token, entry);
            return RendererPageCreationResolution::without_renderer_output(
                PageCreationResolution::LifecycleDecisionRequired { pending },
            );
        }
        let commit = self.commit_page_creation_reply(pending, entry);
        match commit.finalized {
            Ok(finalized) => RendererPageCreationResolution {
                outcome: PageCreationResolution::Finalized {
                    attached: finalized.attached_page,
                    resume_parked_page_turn: finalized.resume_parked_page_turn,
                },
                renderer_output: commit.renderer_output,
                retire_page_after_publication: false,
            },
            Err(error) => RendererPageCreationResolution::retiring(
                PageCreationRetirement::PageStateFailed(error),
                commit.renderer_output,
            ),
        }
    }

    fn retire_checked_out_page_creation(
        &mut self,
        token: RendererPageToken,
        mut entry: RendererPageLocalEntry,
        failure: PageCreationRetirement,
    ) -> RendererPageCreationResolution {
        let renderer_output = entry.page_vm_mut().settle_renderer_output_publication();
        self.restore_entry_after_command(token, entry);
        RendererPageCreationResolution::retiring(failure, renderer_output)
    }

    fn commit_page_creation_reply(
        &mut self,
        pending: RendererPendingPageCreation,
        mut entry: RendererPageLocalEntry,
    ) -> RendererPageCreationCommit {
        let RendererPendingPageCreation {
            token,
            navigation_failure_observer: _,
            page_context_cancel_tx,
            pending_download,
            lifecycle_decider,
        } = pending;
        debug_assert_eq!(entry.slot.page_id(), token.page_id);
        let result = (|| -> Result<_> {
            ensure!(
                lifecycle_decider.is_none(),
                "renderer page {} tried to reply before its lifecycle decider ran",
                token.page_id.as_u64()
            );
            if matches!(
                entry.top_level_navigation_dispatch(),
                RendererTopLevelNavigationDispatch::DelegateToBrowser
            ) {
                entry
                    .page_vm_mut()
                    .vm_mut()
                    .publish_pending_non_javascript_location_navigation()?;
            }
            let javascript_dialog_broker = entry.page_vm().javascript_dialog_broker();
            let inspector_pause_bridge = entry.page_vm().inspector_pause_bridge();
            let inspector_io_ingress = entry.page_vm().inspector_io_ingress();
            let page_state = Self::commit_current_vm_page_state_on_entry(&mut entry)?;
            let initial_runtime_realms = entry.page_vm_mut().vm_mut().runtime_realm_inventory();
            let devtools_agent_token = entry.page_vm().devtools_agent_token();
            let creation_diagnostics = RendererPageCreationDiagnostics {
                initial_runtime_realms,
                renderer_output_predecessor: None,
            };
            let creation_artifacts = entry.page_vm_mut().take_page_creation_artifacts();
            Ok((
                javascript_dialog_broker,
                inspector_pause_bridge,
                inspector_io_ingress,
                page_state,
                devtools_agent_token,
                creation_diagnostics,
                creation_artifacts,
            ))
        })();
        let lifecycle_gate = self
            .page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
            .and_then(|page_slot| page_slot.lifecycle_gate.take());
        let resume_parked_page_turn = lifecycle_gate.is_some_and(|gate| gate.parked_admitted_wake);
        let renderer_output = entry.page_vm_mut().settle_renderer_output_publication();
        // Page creation can span several admitted owner turns. Earlier turns
        // may already have settled Runtime/lifecycle observations before the
        // final lifecycle target is reached, so the commit fence is the
        // complete stream tail rather than only the optional batch settled by
        // this final action.
        let renderer_output_fence = entry
            .page_vm()
            .renderer_output_tail_cursor()
            .map(|cursor| entry.page_vm().declare_renderer_output_fence(cursor));
        self.restore_entry_after_command(token, entry);
        let finalized = result.map(
            |(
                javascript_dialog_broker,
                inspector_pause_bridge,
                inspector_io_ingress,
                page_state,
                devtools_agent_token,
                mut creation_diagnostics,
                creation_artifacts,
            )| {
                creation_diagnostics.renderer_output_predecessor = renderer_output_fence;
                RendererFinalizedPageCreation {
                    attached_page: RendererAttachedPage {
                        token,
                        devtools_agent_token,
                        page_context_cancel_tx,
                        javascript_dialog_broker,
                        inspector_pause_bridge,
                        inspector_io_ingress,
                        page_state,
                        creation_diagnostics,
                        creation_artifacts,
                        pending_download,
                        committed_document_post_response_continuation: None,
                    },
                    resume_parked_page_turn,
                }
            },
        );
        RendererPageCreationCommit {
            finalized,
            renderer_output,
        }
    }

    fn remove_page(&mut self, token: RendererPageToken) {
        #[cfg(debug_assertions)]
        {
            debug_assert!(
                Self::ensure_token_thread(&token).is_ok(),
                "renderer page handle for page {} dropped on a different thread than its owner-local host",
                token.page_id.as_u64()
            );
            if Self::ensure_token_thread(&token).is_err() {
                return;
            }
        }
        self.page_task_deadline_index.remove(token);
        self.owner_maintenance_deadline_index.remove(token);
        let should_remove_host = if let Ok(host) = self.host_by_id_mut(token.local_host_id) {
            let removed_reserved_isolates = host
                .reserved_renderer_document_isolates
                .remove(&token.page_id);
            let resident_entry = host.pages.get_mut(&token.page_id).and_then(|page_slot| {
                page_slot.owner_slot.remove_from_owner();
                page_slot.owner_maintenance.retire();
                page_slot.turn_scheduler.request_retirement()
            });
            if let Some(mut entry) = resident_entry {
                let removed_page_slot = host
                    .pages
                    .remove(&token.page_id)
                    .expect("resident retiring page should retain its owner-local slot");
                entry.close_for_context_teardown();
                // PageVm owns document contexts and isolate-local inspector
                // bindings. It must finish teardown before the stable slot can
                // release the page script environment.
                drop(entry);
                drop(removed_page_slot);
            }
            if let Some(reservations) = removed_reserved_isolates {
                Self::retire_unattached_renderer_document_isolates(reservations);
            }
            Self::host_has_no_page_state(host)
        } else {
            false
        };
        if should_remove_host {
            let removed = self.page_hosts.remove(&token.local_host_id);
            debug_assert!(
                removed
                    .as_ref()
                    .is_none_or(|host| { Self::host_has_no_page_state(host) }),
                "renderer owner local runtime removed non-empty host {}",
                token.local_host_id.as_u64()
            );
        }
        #[cfg(debug_assertions)]
        {
            self.debug_assert_page_task_deadline_index_consistent_for_token(token);
            self.debug_assert_owner_maintenance_deadline_index_consistent_for_token(token);
        }
    }

    fn reindex_page_task_for_token(&mut self, token: RendererPageToken) {
        self.page_task_deadline_index.remove(token);
        let deadline = self
            .page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
            .and_then(RendererOwnerLocalPageSlot::next_page_task_deadline);
        let Some(deadline) = deadline else {
            #[cfg(debug_assertions)]
            self.debug_assert_page_task_deadline_index_consistent_for_token(token);
            return;
        };
        self.page_task_deadline_index.insert(token, deadline);
        #[cfg(debug_assertions)]
        self.debug_assert_page_task_deadline_index_consistent_for_token(token);
    }

    fn reindex_owner_maintenance_for_token(&mut self, token: RendererPageToken) {
        self.owner_maintenance_deadline_index.remove(token);
        let Some(deadline) = self
            .page_hosts
            .get(&token.local_host_id)
            .and_then(|host| host.pages.get(&token.page_id))
            .and_then(RendererOwnerLocalPageSlot::indexed_owner_maintenance_deadline)
        else {
            #[cfg(debug_assertions)]
            self.debug_assert_owner_maintenance_deadline_index_consistent_for_token(token);
            return;
        };
        self.owner_maintenance_deadline_index
            .insert(token, deadline);
        #[cfg(debug_assertions)]
        self.debug_assert_owner_maintenance_deadline_index_consistent_for_token(token);
    }

    #[cfg(debug_assertions)]
    fn debug_assert_page_task_deadline_index_consistent_for_token(&self, token: RendererPageToken) {
        let resident_deadline = self
            .page_hosts
            .get(&token.local_host_id)
            .and_then(|host| host.pages.get(&token.page_id))
            .and_then(RendererOwnerLocalPageSlot::local_page_task_deadline);
        debug_assert_eq!(
            resident_deadline,
            self.page_task_deadline_index.deadline_for(token),
            "resident Page task deadline and owner deadline index diverged for page {}",
            token.page_id.as_u64()
        );
    }

    #[cfg(debug_assertions)]
    fn debug_assert_page_task_deadline_index_consistent(&self) {
        for token in self.page_task_deadline_index.indexed_tokens() {
            self.debug_assert_page_task_deadline_index_consistent_for_token(token);
        }
        for (local_host_id, host) in &self.page_hosts {
            for page_id in host.pages.keys() {
                self.debug_assert_page_task_deadline_index_consistent_for_token(
                    RendererPageToken {
                        local_host_id: *local_host_id,
                        local_thread_id: std::thread::current().id(),
                        page_id: *page_id,
                    },
                );
            }
        }
    }

    #[cfg(debug_assertions)]
    fn debug_assert_owner_maintenance_deadline_index_consistent_for_token(
        &self,
        token: RendererPageToken,
    ) {
        let resident_deadline = self
            .page_hosts
            .get(&token.local_host_id)
            .and_then(|host| host.pages.get(&token.page_id))
            .and_then(RendererOwnerLocalPageSlot::indexed_owner_maintenance_deadline);
        debug_assert_eq!(
            resident_deadline,
            self.owner_maintenance_deadline_index.deadline_for(token),
            "Page owner-maintenance residence and deadline index diverged for page {}",
            token.page_id.as_u64()
        );
    }

    #[cfg(debug_assertions)]
    fn debug_assert_owner_maintenance_deadline_index_consistent(&self) {
        for token in self.owner_maintenance_deadline_index.indexed_tokens() {
            self.debug_assert_owner_maintenance_deadline_index_consistent_for_token(token);
        }
        for (local_host_id, host) in &self.page_hosts {
            for page_id in host.pages.keys() {
                self.debug_assert_owner_maintenance_deadline_index_consistent_for_token(
                    RendererPageToken {
                        local_host_id: *local_host_id,
                        #[cfg(debug_assertions)]
                        local_thread_id: std::thread::current().id(),
                        page_id: *page_id,
                    },
                );
            }
        }
    }

    fn current_page_state_for_testing(
        &mut self,
        token: RendererPageToken,
    ) -> Result<Arc<RendererPageState>> {
        #[cfg(debug_assertions)]
        Self::ensure_token_thread(&token)?;
        let host = self.page_hosts.get(&token.local_host_id).ok_or_else(|| {
            anyhow!(
                "renderer owner local runtime no longer tracks host {}",
                token.local_host_id.as_u64()
            )
        })?;
        Self::current_page_state_for_testing_on_host(host, token.page_id)
    }

    fn renderer_page_view_for_testing(
        &mut self,
        token: RendererPageToken,
    ) -> Result<RendererPageView> {
        #[cfg(debug_assertions)]
        Self::ensure_token_thread(&token)?;
        let host = self.page_hosts.get(&token.local_host_id).ok_or_else(|| {
            anyhow!(
                "renderer owner local runtime no longer tracks host {}",
                token.local_host_id.as_u64()
            )
        })?;
        Self::renderer_page_view_for_testing_on_host(host, token.page_id)
    }

    fn owner_slot_for_testing(
        &mut self,
        token: RendererPageToken,
    ) -> Result<RendererPageSlotHandle> {
        #[cfg(debug_assertions)]
        Self::ensure_token_thread(&token)?;
        let host = self.page_hosts.get(&token.local_host_id).ok_or_else(|| {
            anyhow!(
                "renderer owner local runtime no longer tracks host {}",
                token.local_host_id.as_u64()
            )
        })?;
        Self::owner_slot_for_testing_on_host(host, token.page_id)
    }

    fn host_instance_key_for_testing(&mut self, token: RendererPageToken) -> Result<usize> {
        #[cfg(debug_assertions)]
        Self::ensure_token_thread(&token)?;
        let host = self.page_hosts.get(&token.local_host_id).ok_or_else(|| {
            anyhow!(
                "renderer owner local runtime no longer tracks host {}",
                token.local_host_id.as_u64()
            )
        })?;
        Ok(host.instance_key)
    }

    fn host_unique_document_isolate_count_for_testing(
        &mut self,
        token: RendererPageToken,
    ) -> Result<usize> {
        #[cfg(debug_assertions)]
        Self::ensure_token_thread(&token)?;
        let host = self.page_hosts.get(&token.local_host_id).ok_or_else(|| {
            anyhow!(
                "renderer owner local runtime no longer tracks host {}",
                token.local_host_id.as_u64()
            )
        })?;
        // Count unique holder identities rather than assuming one pin per
        // page. This catches accidental isolate reuse as well as missing pins.
        let mut isolate_keys = HashSet::new();
        for page_slot in host.pages.values() {
            isolate_keys.insert(page_slot.script_environment_pin.identity_key());
        }
        Ok(isolate_keys.len())
    }

    #[cfg(debug_assertions)]
    fn ensure_token_thread(token: &RendererPageToken) -> Result<()> {
        let current_thread_id = std::thread::current().id();
        ensure!(
            current_thread_id == token.local_thread_id,
            "renderer page token for page {} was used on a different thread than its owner-local host",
            token.page_id.as_u64()
        );
        Ok(())
    }

    fn create_initial_slot_for_vm(
        owner: &RendererOwnerLocalContext,
        vm: &PageVm,
        page_state: Arc<RendererPageState>,
    ) -> RendererPageSlotHandle {
        debug_assert!(
            is_on_named_owner_execution_lane_for(&owner.owner_state.local_executor),
            "initial page-slot registration must execute on the matching named owner lane"
        );
        let page_id = vm.page_id;
        RendererPageSlotHandle::new(
            Arc::downgrade(&owner.owner_state),
            RendererPageEntry::active(page_id, vm.creation_id, 0, 0, page_state),
            vm.vm().page_context_cancel_sender(),
        )
    }

    fn attach_page_entry_for_owner(
        &mut self,
        owner: &RendererOwnerLocalContext,
        slot: RendererPageSlotHandle,
        mut entry: RendererPageLocalEntry,
        lifecycle_gate: Option<PageVmInitStage>,
        page_creation_navigation_failure_publisher: PageCreationNavigationFailurePublisher,
    ) -> Result<RendererPageToken> {
        let page_id = slot.page_id();
        let Some(reservation) = entry
            .page_vm_mut()
            .take_renderer_document_isolate_reservation_for_attach()
        else {
            entry.close_for_context_teardown();
            return Err(anyhow!(
                "renderer owner local host cannot attach page {} without a renderer document isolate reservation",
                page_id.as_u64()
            ));
        };
        let reservation_token = reservation.token();
        let reservation_id = reservation.reservation_id();
        let preflight = (|| {
            ensure!(
                reservation_token.local_host_id == owner.local_host_id
                    && reservation_token.page_id == page_id,
                "renderer owner local host received renderer document isolate reservation for a different page"
            );
            #[cfg(debug_assertions)]
            ensure!(
                reservation_token.local_thread_id == owner.local_thread_id,
                "renderer owner local host received renderer document isolate reservation for a different owner-local thread"
            );
            ensure!(
                !owner.owner_state.page_table.contains_page(page_id),
                "renderer owner already has a stable slot for new page {}",
                page_id.as_u64()
            );
            let host = self.page_hosts.get(&owner.local_host_id).ok_or_else(|| {
                anyhow!(
                    "renderer owner local runtime no longer tracks host {}",
                    owner.local_host_id.as_u64()
                )
            })?;
            ensure!(
                !host.pages.contains_key(&page_id),
                "renderer owner local host already has a stable slot for new page {}",
                page_id.as_u64()
            );
            let page_script_environment = entry
                .page_vm()
                .renderer_page_script_environment()
                .ok_or_else(|| {
                    anyhow!(
                        "renderer owner local host cannot attach page {} without a page script environment",
                        page_id.as_u64()
                    )
                })?;
            let reserved_isolate =
                Self::reserved_renderer_document_isolate_on_host(host, page_id, reservation_id)?;
            ensure!(
                reserved_isolate.handle.identity_key()
                    == page_script_environment.isolate_identity_key(),
                "renderer page script environment does not match its isolate reservation"
            );
            let task_sources = reserved_isolate
                .initial_task_sources
                .as_ref()
                .ok_or_else(|| {
                    anyhow!(
                        "initial renderer page reservation {} unexpectedly reused live Page sources",
                        reservation_id
                    )
                })?;
            ensure!(
                page_script_environment
                    .page_runtime_task_source()
                    .page_task_producer_routes_match(task_sources),
                "renderer page script environment producer routes do not match its stable Page sources"
            );
            Ok(page_script_environment)
        })();
        let page_script_environment = match preflight {
            Ok(page_script_environment) => page_script_environment,
            Err(error) => {
                self.remove_reserved_renderer_document_isolate(reservation_token, reservation_id);
                reservation.disarm_for_attach();
                entry.close_for_context_teardown();
                return Err(error);
            }
        };
        let script_environment_pin = RendererPageScriptEnvironmentPin::new(page_script_environment);
        let host = self
            .page_hosts
            .get_mut(&owner.local_host_id)
            .expect("renderer page attach preflight must retain its owner-local host");
        let reserved_isolate =
            Self::take_reserved_renderer_document_isolate_on_host(host, page_id, reservation_id)
                .expect("renderer page attach preflight must retain its isolate reservation");
        debug_assert_eq!(
            reserved_isolate.handle.identity_key(),
            script_environment_pin.identity_key(),
            "renderer page attach commit must preserve the preflighted isolate identity"
        );
        let RendererDocumentIsolateReservationEntry {
            initial_task_sources,
            handle: _,
            output_journal: _,
            id: _,
            _accounting: _,
        } = reserved_isolate;
        let task_sources = initial_task_sources
            .expect("initial renderer page attach must own its reserved Page task sources");
        reservation.disarm_for_attach();
        let previous_page = host.pages.insert(
            page_id,
            RendererOwnerLocalPageSlot::new(
                slot.clone(),
                entry,
                task_sources,
                lifecycle_gate,
                page_creation_navigation_failure_publisher,
                script_environment_pin,
            ),
        );
        debug_assert!(
            previous_page.is_none(),
            "renderer owner local host should not replace page entry {}",
            page_id.as_u64()
        );
        if let Err(error) = owner.owner_state.page_table.insert_new_slot(page_id, slot) {
            let rejected_page = host
                .pages
                .remove(&page_id)
                .expect("terminal Page attach must remove its owner-local slot");
            drop(rejected_page);
            return Err(error);
        }
        let attached_page = host
            .pages
            .get_mut(&page_id)
            .expect("new renderer page must retain its stable slot after attach commit");
        let has_page_task_source = attached_page.task_sources.has_resident_task();
        let attached_entry = attached_page
            .resident_entry_mut()
            .expect("new renderer page must be resident after attach commit");
        attached_entry
            .page_vm()
            .inspector_pause_bridge()
            .configure_page_route(
                attached_entry
                    .page_vm()
                    .renderer_page_script_environment()
                    .expect("an attached Page must own a renderer script environment")
                    .output_journal(),
            );
        attached_entry
            .page_vm()
            .inspector_io_ingress()
            .configure_owner_wake(owner.owner_state.inspector_io_wake_tx.clone());
        let _ = attached_entry
            .page_vm_mut()
            .replay_pending_owner_wakes_after_attach(has_page_task_source);
        Ok(RendererPageToken {
            local_host_id: owner.local_host_id,
            #[cfg(debug_assertions)]
            local_thread_id: owner.local_thread_id,
            page_id,
        })
    }

    fn reserved_renderer_document_isolate_on_host(
        host: &RendererOwnerLocalPageHost,
        page_id: PageId,
        reservation_id: u64,
    ) -> Result<&RendererDocumentIsolateReservationEntry> {
        host.reserved_renderer_document_isolates
            .get(&page_id)
            .and_then(|entries| entries.iter().find(|entry| entry.id == reservation_id))
            .ok_or_else(|| {
                anyhow!(
                    "renderer owner local host is missing renderer document isolate reservation {} for page {}",
                    reservation_id,
                    page_id.as_u64()
                )
            })
    }

    fn take_reserved_renderer_document_isolate_on_host(
        host: &mut RendererOwnerLocalPageHost,
        page_id: PageId,
        reservation_id: u64,
    ) -> Result<RendererDocumentIsolateReservationEntry> {
        let entries = host
            .reserved_renderer_document_isolates
            .get_mut(&page_id)
            .ok_or_else(|| {
                anyhow!(
                    "renderer owner local host is missing reserved renderer document isolate for page {}",
                    page_id.as_u64()
                )
            })?;
        let index = entries
            .iter()
            .position(|entry| entry.id == reservation_id)
            .ok_or_else(|| {
                anyhow!(
                    "renderer owner local host is missing renderer document isolate reservation {} for page {}",
                    reservation_id,
                    page_id.as_u64()
                )
            })?;
        let entry = entries.swap_remove(index);
        if entries.is_empty() {
            host.reserved_renderer_document_isolates.remove(&page_id);
        }
        Ok(entry)
    }

    fn consume_entry_renderer_document_isolate_reservation_on_host(
        host: &mut RendererOwnerLocalPageHost,
        token: RendererPageToken,
        vm: &mut PageVm,
    ) -> Result<Option<RendererPageScriptEnvironmentPin>> {
        let Some(reservation) = vm.take_renderer_document_isolate_reservation_for_attach() else {
            return Ok(None);
        };
        let reservation_token = reservation.token();
        let reservation_id = reservation.reservation_id();
        ensure!(
            reservation_token.local_host_id == token.local_host_id
                && reservation_token.page_id == token.page_id,
            "renderer owner local host received renderer document isolate reservation for a different restored page"
        );
        #[cfg(debug_assertions)]
        ensure!(
            reservation_token.local_thread_id == token.local_thread_id,
            "renderer owner local host received renderer document isolate reservation for a different restored owner-local thread"
        );
        let page_script_environment = vm
            .renderer_page_script_environment()
            .ok_or_else(|| anyhow!("restored page is missing its page script environment"))?;
        let reserved_isolate =
            Self::reserved_renderer_document_isolate_on_host(host, token.page_id, reservation_id)?;
        ensure!(
            reserved_isolate.handle.identity_key()
                == page_script_environment.isolate_identity_key(),
            "restored page script environment does not match its isolate reservation"
        );
        ensure!(
            reserved_isolate.initial_task_sources.is_none(),
            "replacement renderer isolate reservation unexpectedly owns a second Page consumer set"
        );
        let page_task_sources = &host
            .pages
            .get(&token.page_id)
            .ok_or_else(|| anyhow!("restored page is missing its stable Page slot"))?
            .task_sources;
        ensure!(
            page_script_environment
                .page_runtime_task_source()
                .page_task_producer_routes_match(page_task_sources),
            "restored page producer routes do not match its stable Page sources"
        );
        let reserved_isolate = Self::take_reserved_renderer_document_isolate_on_host(
            host,
            token.page_id,
            reservation_id,
        )
        .expect("restored page preflight must retain its isolate reservation");
        let RendererDocumentIsolateReservationEntry {
            initial_task_sources,
            handle: _,
            output_journal: _,
            id: _,
            _accounting: _,
        } = reserved_isolate;
        debug_assert!(initial_task_sources.is_none());
        reservation.disarm_for_attach();
        Ok(Some(RendererPageScriptEnvironmentPin::new(
            page_script_environment,
        )))
    }

    fn view_generation(entry: &RendererPageLocalEntry) -> u64 {
        entry.slot.entry().view_generation
    }

    fn prepare_next_view_generation(entry: &RendererPageLocalEntry) -> u64 {
        Self::view_generation(entry).saturating_add(1)
    }

    fn advance_command_epoch(entry: &RendererPageLocalEntry) -> u64 {
        entry.slot.entry().command_epoch().saturating_add(1)
    }

    fn current_view_for_testing_on_entry(
        entry: &RendererPageLocalEntry,
    ) -> Result<RendererPageView> {
        let page_vm = entry.page_vm();
        let stable_entry = entry.slot.entry();
        debug_assert_eq!(entry.slot.page_id().as_u64(), page_vm.page_id.as_u64());
        ensure!(
            stable_entry.vm_creation_id() == page_vm.creation_id,
            "renderer testing view observed active PageVm {} behind stable PageVm {}",
            page_vm.creation_id,
            stable_entry.vm_creation_id()
        );
        Ok(RendererPageView {
            page_id: entry.slot.page_id(),
            vm_creation_id: stable_entry.vm_creation_id(),
            view_generation: stable_entry.view_generation,
            page_state: entry.slot.active_page_state()?,
        })
    }

    fn refresh_view_on_entry(entry: &RendererPageLocalEntry, view: RendererPageView) -> Result<()> {
        entry.slot.refresh_owned_view(view)
    }

    fn commit_next_page_state_on_entry(
        entry: &RendererPageLocalEntry,
        vm_creation_id: u64,
        page_state: Arc<RendererPageState>,
    ) -> Result<()> {
        Self::refresh_view_on_entry(
            entry,
            RendererPageView {
                page_id: entry.slot.page_id(),
                vm_creation_id,
                view_generation: Self::prepare_next_view_generation(entry),
                page_state,
            },
        )
    }

    fn commit_vm_state_capture_as_page_state_on_entry(
        entry: &RendererPageLocalEntry,
        state_capture: PageVmStateCapture,
    ) -> Result<()> {
        let current_page_state = entry.slot.active_page_state()?;
        let page_state = RendererPageState::from_vm_state_capture(
            current_page_state.requested_url.clone(),
            current_page_state.navigation_initiator_url.clone(),
            current_page_state.navigation_redirected,
            current_page_state.navigation_redirect_count,
            current_page_state.status,
            current_page_state.headers.clone(),
            state_capture,
        );
        Self::commit_next_page_state_on_entry(entry, entry.page_vm().creation_id, page_state)
    }

    fn commit_active_vm_page_state_on_entry(
        entry: &mut RendererPageLocalEntry,
    ) -> Result<Arc<RendererPageState>> {
        Self::commit_active_vm_page_state_on_entry_with_policy(
            entry,
            super::RendererPageStateCapturePolicy::FullReport,
        )
    }

    fn commit_active_vm_page_state_on_entry_with_policy(
        entry: &mut RendererPageLocalEntry,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> Result<Arc<RendererPageState>> {
        debug_assert_eq!(
            entry.slot.page_id().as_u64(),
            entry.page_vm().page_id.as_u64()
        );
        let state_capture = entry
            .page_vm_mut()
            .capture_page_state_on_named_owner_lane_with_policy(capture_policy)?;
        Self::commit_vm_state_capture_as_page_state_on_entry(entry, state_capture)?;
        entry.slot.active_page_state()
    }

    fn commit_current_vm_page_state_on_entry(
        entry: &mut RendererPageLocalEntry,
    ) -> Result<Arc<RendererPageState>> {
        Self::commit_current_vm_page_state_on_entry_with_policy(
            entry,
            super::RendererPageStateCapturePolicy::FullReport,
        )
    }

    fn commit_current_vm_page_state_on_entry_with_policy(
        entry: &mut RendererPageLocalEntry,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> Result<Arc<RendererPageState>> {
        let stable_vm_creation_id = entry.slot.entry().vm_creation_id();
        let active_vm_creation_id = entry.page_vm().creation_id;
        ensure!(
            stable_vm_creation_id == active_vm_creation_id,
            "cross-Document PageVm publication requires the typed replacement commit boundary (stable {stable_vm_creation_id}, active {active_vm_creation_id})"
        );
        Self::commit_active_vm_page_state_on_entry_with_policy(entry, capture_policy)
    }

    async fn dispatch_async_on_entry(
        entry: &mut RendererPageLocalEntry,
        command: RendererPageCommand,
    ) -> Result<RendererPageCommandDispatch> {
        let directly_delegates_location_navigation = matches!(
            &command,
            RendererPageCommand::EvaluateExpression { .. }
                | RendererPageCommand::EvaluateExpressionInExecutionContext { .. }
        );
        // Every bounded Page command owns the concrete records produced while
        // it executes. Keeping this universal avoids a fragile allowlist where
        // a newly added mutating command (for example history traversal)
        // silently writes into the asynchronous Page journal and lets its
        // response overtake the resulting protocol fact.
        let command_turn_output_scope = entry.page_vm_mut().begin_command_turn_output_scope()?;
        let replacement_lifecycle_snapshot = entry
            .page_vm()
            .document_replacement_lifecycle_action_snapshot();
        let command_epoch = Self::advance_command_epoch(entry);
        let slot = entry.slot.clone();
        let reply = slot
            .dispatch_async_owned(command_epoch, entry.page_vm_mut(), command)
            .await;
        let replacement_lifecycle = {
            let (page_vm, pending_document_lifecycle_turn) =
                entry.page_vm_and_document_lifecycle_turn_mut();
            page_vm
                .reconcile_document_replacement_lifecycle_after_owner_action(
                    replacement_lifecycle_snapshot,
                    pending_document_lifecycle_turn,
                )
                .await
        };
        let runtime_command_completion = if replacement_lifecycle.is_ok()
            && entry.page_vm().has_pending_runtime_command_lifecycle()
        {
            // Chromium replies to Runtime.evaluate/callFunctionOn at the
            // command boundary. A synchronous document.open/write/close may
            // admit later parser/module/DCL/load turns, but those are
            // page-source work and must not hold the protocol response.
            entry
                .page_vm_mut()
                .complete_pending_runtime_command_lifecycle()
        } else {
            Ok(())
        };
        let input_triggered_top_level_navigation = reply.as_ref().is_ok_and(|reply| {
            matches!(
                reply,
                RendererPageReply::InputDispatchOutcome(outcome)
                    if outcome.triggered_top_level_navigation
            )
        });
        let should_delegate_location_navigation = replacement_lifecycle.is_ok()
            && runtime_command_completion.is_ok()
            && reply.is_ok()
            && matches!(
                entry.top_level_navigation_dispatch(),
                RendererTopLevelNavigationDispatch::DelegateToBrowser
            )
            && (directly_delegates_location_navigation
                || input_triggered_top_level_navigation
                || entry
                    .page_vm()
                    .vm()
                    .pending_location_navigation_runtime_command_cause()
                    .is_some());
        let location_navigation_publication = if should_delegate_location_navigation {
            entry
                .page_vm_mut()
                .vm_mut()
                .publish_pending_non_javascript_location_navigation()
                .map(|_| ())
        } else {
            Ok(())
        };
        let turn_records = entry
            .page_vm_mut()
            .finish_command_turn_output_scope(command_turn_output_scope);
        let replacement_lifecycle = replacement_lifecycle?;
        runtime_command_completion?;
        location_navigation_publication?;
        Ok(RendererPageCommandDispatch {
            reply: reply?,
            replacement_lifecycle,
            turn_records,
        })
    }

    fn current_page_state_for_testing_on_host(
        host: &RendererOwnerLocalPageHost,
        page_id: PageId,
    ) -> Result<Arc<RendererPageState>> {
        let page_slot = host.pages.get(&page_id).ok_or_else(|| {
            anyhow!(
                "renderer local host no longer tracks page {}",
                page_id.as_u64()
            )
        })?;
        let entry = page_slot.resident_entry().ok_or_else(|| {
            anyhow!(
                "renderer local host page {} is not resident",
                page_id.as_u64()
            )
        })?;
        (|| {
            Self::refresh_view_on_entry(entry, Self::current_view_for_testing_on_entry(entry)?)?;
            entry.slot.active_page_state()
        })()
        .map_err(|error| anyhow!("failed to refresh renderer owner page view: {error}"))
    }

    fn renderer_page_view_for_testing_on_host(
        host: &RendererOwnerLocalPageHost,
        page_id: PageId,
    ) -> Result<RendererPageView> {
        let page_slot = host.pages.get(&page_id).ok_or_else(|| {
            anyhow!(
                "renderer local host no longer tracks page {}",
                page_id.as_u64()
            )
        })?;
        let entry = page_slot.resident_entry().ok_or_else(|| {
            anyhow!(
                "renderer local host page {} is not resident",
                page_id.as_u64()
            )
        })?;
        Self::current_view_for_testing_on_entry(entry)
    }

    fn owner_slot_for_testing_on_host(
        host: &RendererOwnerLocalPageHost,
        page_id: PageId,
    ) -> Result<RendererPageSlotHandle> {
        let page_slot = host.pages.get(&page_id).ok_or_else(|| {
            anyhow!(
                "renderer local host no longer tracks page {}",
                page_id.as_u64()
            )
        })?;
        Ok(page_slot.owner_slot.clone())
    }
}

impl RendererOwnerLocalStore {
    pub(super) fn restore_entry_after_command(
        &mut self,
        token: RendererPageToken,
        mut entry: RendererPageLocalEntry,
    ) {
        // Every bounded owner operation returns through this residence
        // boundary. Retire an old-Document continuation here even when the
        // operation itself did not need mutable access to lifecycle state.
        entry.retire_stale_document_lifecycle_turn();
        let mut restored = false;
        let should_remove_host = if let Some(host) = self.page_hosts.get_mut(&token.local_host_id) {
            let retiring = host
                .pages
                .get(&token.page_id)
                .is_none_or(|page_slot| page_slot.turn_scheduler.is_retiring());
            if retiring {
                entry.close_for_context_teardown();
                entry.slot.remove_from_owner();
                drop(entry);
                host.pages.remove(&token.page_id);
                if let Some(reservations) = host
                    .reserved_renderer_document_isolates
                    .remove(&token.page_id)
                {
                    Self::retire_unattached_renderer_document_isolates(reservations);
                }
            } else {
                let replacement_attachment =
                    Self::consume_entry_renderer_document_isolate_reservation_on_host(
                        host,
                        token,
                        entry.page_vm_mut(),
                    )
                    .expect(
                        "restored renderer page entry should consume its renderer document isolate reservation",
                    );
                let page_slot = host
                    .pages
                    .get_mut(&token.page_id)
                    .expect("restored renderer page entry should retain its stable page slot");
                if let Some(replacement_environment_pin) = replacement_attachment {
                    let previous_environment_pin = std::mem::replace(
                        &mut page_slot.script_environment_pin,
                        replacement_environment_pin,
                    );
                    // A restored entry may represent a same-PageId
                    // cross-document navigation. Release the old host clone
                    // only after the replacement environment is installed.
                    drop(previous_environment_pin);
                }
                debug_assert!(
                    entry
                        .page_vm()
                        .page_task_producer_routes_match(&page_slot.task_sources),
                    "restored PageVm must retain the stable Page producer routes"
                );
                match page_slot.turn_scheduler.restore(entry) {
                    RendererPageEntryRestore::Restored => restored = true,
                    RendererPageEntryRestore::Retire(mut entry) => {
                        entry.close_for_context_teardown();
                        entry.slot.remove_from_owner();
                        drop(entry);
                        host.pages.remove(&token.page_id);
                    }
                    RendererPageEntryRestore::Duplicate(mut duplicate) => {
                        duplicate.close_for_context_teardown();
                        duplicate.slot.remove_from_owner();
                        panic!(
                            "renderer page {} was restored while its stable slot was already resident",
                            token.page_id.as_u64()
                        );
                    }
                }
            }
            Self::host_has_no_page_state(host)
        } else {
            entry.close_for_context_teardown();
            entry.slot.remove_from_owner();
            false
        };
        if restored {
            self.reindex_page_task_for_token(token);
        }
        if should_remove_host {
            let removed = self.page_hosts.remove(&token.local_host_id);
            debug_assert!(
                removed
                    .as_ref()
                    .is_none_or(RendererOwnerLocalStore::host_has_no_page_state),
                "renderer owner local runtime removed non-empty host {} after page turn restore",
                token.local_host_id.as_u64()
            );
        }
    }
}

impl Drop for RendererOwnerLocalStore {
    fn drop(&mut self) {
        let prepared_documents = std::mem::take(&mut self.prepared_documents);
        for (_, residence) in prepared_documents {
            self.drop_prepared_document_residence(residence);
        }
        for (_, mut host) in self.page_hosts.drain() {
            for (_, mut page_slot) in host.pages.drain() {
                page_slot.owner_slot.remove_from_owner();
                if let Some(mut entry) = page_slot.turn_scheduler.request_retirement() {
                    entry.close_for_context_teardown();
                }
            }
            for (_, reservations) in host.reserved_renderer_document_isolates.drain() {
                Self::retire_unattached_renderer_document_isolates(reservations);
            }
        }
    }
}

impl Drop for RenderRuntimeOwnerLocalStoreBinding {
    fn drop(&mut self) {
        CURRENT_RENDER_RUNTIME_OWNER_LOCAL_STORE.with(|current_store| {
            // Cleanup must remain idempotent and non-asserting: this guard can
            // run during panic unwinding, where a second panic would abort the
            // process instead of preserving the original failure.
            current_store.borrow_mut().take();
        });
    }
}

#[cfg(test)]
mod entry_local_task_guard_tests {
    use super::*;

    #[test]
    fn owner_local_store_binding_rejects_rebinding_without_replacing_the_active_store() {
        let mut active_store = RendererOwnerLocalStore::default();
        let mut rejected_store = RendererOwnerLocalStore::default();
        let binding = bind_render_runtime_owner_local_store(&mut active_store);

        let rebind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _rejected_binding = bind_render_runtime_owner_local_store(&mut rejected_store);
        }));

        assert!(rebind.is_err(), "a nested owner-local binding must fail");
        assert!(
            has_current_render_runtime_owner_local_store(),
            "rejecting a nested binding must preserve the active store"
        );
        drop(binding);
        assert!(
            !has_current_render_runtime_owner_local_store(),
            "dropping the active binding must clear the thread-local store"
        );
    }

    #[test]
    fn owner_local_store_binding_drop_does_not_double_panic_during_unwind() {
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut store = RendererOwnerLocalStore::default();
            let _binding = bind_render_runtime_owner_local_store(&mut store);
            CURRENT_RENDER_RUNTIME_OWNER_LOCAL_STORE.with(|current_store| {
                assert!(
                    current_store.borrow_mut().take().is_some(),
                    "the test must clear an active binding before unwinding"
                );
            });
            panic!("primary owner-loop failure");
        }));

        assert!(unwind.is_err(), "the primary panic must remain catchable");
        assert!(
            !has_current_render_runtime_owner_local_store(),
            "unwinding must leave no thread-local owner store binding"
        );
    }

    #[test]
    fn guard_returns_entry_when_task_future_is_dropped_before_first_poll() {
        let (reply_tx, mut reply_rx) = oneshot::channel();
        let guard = EntryLocalTaskGuard::<_, ()>::new(42_u8, reply_tx);
        let never_polled_task = async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        };

        drop(never_polled_task);

        let (entry, result) = reply_rx
            .try_recv()
            .expect("dropping the unpolled task should return its guarded entry");
        assert_eq!(entry, 42);
        assert!(
            result
                .expect_err("an unpolled task must not report successful completion")
                .to_string()
                .contains("before restoring its page entry")
        );
    }
}

#[cfg(test)]
mod navigation_dispatch_tests {
    use super::*;

    #[test]
    fn runnable_page_creation_lifecycle_clears_displaced_ordinary_grant() {
        let mut gate = LifecycleGate::new(PageVmInitStage::DomContentLoaded);

        gate.settle_lifecycle_turn(true);
        assert!(gate.reconsider_ordinary_on_next_turn);

        gate.settle_lifecycle_turn(false);
        assert!(!gate.reconsider_ordinary_on_next_turn);
    }

    fn entry_without_active_page_vm(
        standalone_navigation_follow: StandaloneNavigationFollowState,
    ) -> RendererPageLocalEntry {
        let page_id = PageId::new_for_testing(1);
        let (page_context_cancel_tx, _page_context_cancel_rx) =
            renderer_page_context_cancel_channel();
        let slot = RendererPageSlotHandle::new(
            std::sync::Weak::new(),
            RendererPageEntry::removed(page_id),
            page_context_cancel_tx,
        );
        RendererPageLocalEntry {
            slot,
            top_level_navigation_dispatch:
                RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            standalone_navigation_follow,
            pending_document_lifecycle_turn: None,
            post_response_document_lifecycle: None,
            vm: None,
            pending_phase_one_navigation: None,
            last_published_replacement_document: None,
        }
    }

    #[test]
    fn navigation_settlement_tolerates_phase_one_entry_without_active_vm() {
        let handoff = crate::page_task_queue::RendererTopLevelNavigationHandoff::new(1);
        for succeeded in [false, true] {
            for state in [
                StandaloneNavigationFollowState::Idle,
                StandaloneNavigationFollowState::Following { handoff },
                StandaloneNavigationFollowState::FailedWithPendingNavigation { handoff },
            ] {
                let mut entry = entry_without_active_page_vm(state);

                entry.settle_standalone_navigation_follow(succeeded);

                assert_eq!(
                    entry.standalone_navigation_follow,
                    StandaloneNavigationFollowState::Idle,
                    "an empty phase-one shell must settle {state:?} to Idle after succeeded={succeeded}"
                );
            }
        }
    }

    #[test]
    fn navigation_handoff_claim_rejects_stale_and_duplicate_requests() {
        let first = crate::page_task_queue::RendererTopLevelNavigationHandoff::new(1);
        let second = crate::page_task_queue::RendererTopLevelNavigationHandoff::new(2);
        let mut state = StandaloneNavigationFollowState::Idle;

        assert!(!state.claim(Some(second), Some(first)));
        assert_eq!(state, StandaloneNavigationFollowState::Idle);
        assert!(state.claim(Some(second), Some(second)));
        assert_eq!(
            state,
            StandaloneNavigationFollowState::Following { handoff: second }
        );
        assert!(!state.claim(Some(second), Some(second)));
    }

    #[test]
    fn failed_navigation_suppresses_only_the_same_request_identity() {
        let first = crate::page_task_queue::RendererTopLevelNavigationHandoff::new(1);
        let second = crate::page_task_queue::RendererTopLevelNavigationHandoff::new(2);
        let mut state = StandaloneNavigationFollowState::Following { handoff: first };

        state.settle(Some(first), false);
        assert_eq!(
            state,
            StandaloneNavigationFollowState::FailedWithPendingNavigation { handoff: first }
        );
        assert!(!state.claim(Some(first), Some(first)));
        assert!(state.claim(Some(second), Some(second)));
        assert_eq!(
            state,
            StandaloneNavigationFollowState::Following { handoff: second }
        );
    }

    #[test]
    fn pending_document_lifecycle_classifies_each_stable_residence() {
        let cases = [
            (
                (true, false, false, false),
                DocumentLifecycleObserverOutcome::NavigationPending,
            ),
            (
                (false, true, false, false),
                DocumentLifecycleObserverOutcome::Pending,
            ),
            (
                (false, false, true, false),
                DocumentLifecycleObserverOutcome::Pending,
            ),
            (
                (false, false, false, true),
                DocumentLifecycleObserverOutcome::Pending,
            ),
            (
                (false, false, false, false),
                DocumentLifecycleObserverOutcome::MissingResident,
            ),
        ];

        for ((location, phase_one, lifecycle_turn, replacement), expected) in cases {
            assert_eq!(
                classify_pending_document_lifecycle_residence(
                    location,
                    phase_one,
                    lifecycle_turn,
                    replacement,
                ),
                expected
            );
        }
    }

    #[test]
    fn page_creation_prioritizes_navigation_enqueued_by_reached_milestone() {
        assert_eq!(
            reconcile_page_creation_lifecycle_observation(
                DocumentLifecycleObserverOutcome::Reached,
                true,
            ),
            DocumentLifecycleObserverOutcome::NavigationPending
        );
    }

    #[test]
    fn page_creation_does_not_hide_an_interrupted_lifecycle() {
        let termination = RendererLifecycleTerminationStamp {
            sequence: 7,
            timestamp_micros: 11,
            reason: RendererDocumentTerminationReason::Detached,
        };

        assert_eq!(
            reconcile_page_creation_lifecycle_observation(
                DocumentLifecycleObserverOutcome::Interrupted(termination),
                true,
            ),
            DocumentLifecycleObserverOutcome::Interrupted(termination)
        );
    }

    #[test]
    fn published_page_creation_discards_reply_policy_when_observer_detaches() {
        let completion = LivePagePendingNavigationCompletion::PublishedPageCreation {
            navigation_reply_policy: NavigationReplyPolicy::ReturnWithPendingNavigation,
        };

        let (completion, detached) = completion.detach_command_observer();

        assert!(detached);
        assert!(matches!(
            completion,
            LivePagePendingNavigationCompletion::Background
        ));
    }

    #[test]
    fn detached_navigation_failure_routes_to_pending_page_creation() {
        assert_eq!(
            LivePagePendingNavigationCompletion::Background.failure_recipient(),
            LivePageNavigationFailureRecipient::PageCreationObserver
        );
    }

    #[test]
    fn already_published_page_creation_reports_later_navigation_failure_as_background() {
        let completion = LivePagePendingNavigationCompletion::PublishedPageCreation {
            navigation_reply_policy: NavigationReplyPolicy::FollowBeforeReply,
        };

        assert_eq!(
            completion.failure_recipient(),
            LivePageNavigationFailureRecipient::Background
        );
    }

    #[test]
    fn command_owned_navigation_failure_returns_to_its_initiator() {
        let completion = LivePagePendingNavigationCompletion::ReplyWithSnapshot {
            reply: Box::new(RendererPageReply::Unit),
            capture_policy: super::RendererPageStateCapturePolicy::FullReport,
        };

        assert_eq!(
            completion.failure_recipient(),
            LivePageNavigationFailureRecipient::Initiator
        );
    }
}
