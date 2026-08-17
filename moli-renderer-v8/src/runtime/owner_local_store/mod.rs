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
use super::owner::RendererCreateStreamingRawPageRequest;
use super::owner_deadline_index::OwnerDeadlineIndex;
use super::owner_local::RendererAttachedPage;
use super::owner_maintenance::{
    RendererOwnerMaintenanceTask, RendererPageOwnerMaintenanceResidence,
};
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

mod bound;
mod entry;
mod phase_one;
#[cfg(test)]
mod tests;

pub(in crate::runtime) use bound::*;
#[cfg(test)]
use entry::StandaloneNavigationFollowState;
pub(in crate::runtime) use entry::{
    LivePageEntry, PublishedReplacementDocument, RetiringPageEntry,
};
pub(in crate::runtime) use phase_one::{
    LivePagePendingNavigationPhaseOneAdvance, PendingPhaseOneEntryAdvance,
    advance_pending_phase_one_navigation_on_entry_via_local_task,
};

pub(super) type LivePageEntryCheckout =
    std::result::Result<LivePageEntry, LivePageEntryCheckoutError>;

pub(super) type RendererPageTurnCheckout = std::result::Result<
    (LivePageEntry, PageTurnTrigger, RendererPageScheduledTurn),
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

pub(super) enum LivePageEntryCheckoutError {
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
        entry: &mut LivePageEntry,
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

pub(super) struct RendererOwnerLocalContext {
    pub(super) owner_state: Arc<RendererOwnerState>,
    pub(super) render_runtime: crate::render_runtime::RenderRuntimeHandle,
    pub(super) local_host_id: RendererOwnerLocalHostId,
    #[cfg(debug_assertions)]
    pub(super) local_thread_id: ThreadId,
}

impl Clone for RendererOwnerLocalContext {
    fn clone(&self) -> Self {
        Self {
            owner_state: self.owner_state.clone(),
            render_runtime: self.render_runtime.clone(),
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
    turn_scheduler: PageTurnScheduler<LivePageEntry>,
    owner_maintenance: RendererPageOwnerMaintenanceResidence,
    task_sources: RendererPageOwnedTaskSources,
    lifecycle_gate: Option<LifecycleGate>,
    page_creation_navigation_failure_publisher: PageCreationNavigationFailurePublisher,
    script_environment_pin: RendererPageScriptEnvironmentPin,
}

impl RendererOwnerLocalPageSlot {
    fn new(
        owner_slot: RendererPageSlotHandle,
        entry: LivePageEntry,
        task_sources: RendererPageOwnedTaskSources,
        lifecycle_gate: Option<PageVmInitStage>,
        page_creation_navigation_failure_publisher: PageCreationNavigationFailurePublisher,
        script_environment_pin: RendererPageScriptEnvironmentPin,
    ) -> Self {
        Self {
            owner_slot,
            turn_scheduler: PageTurnScheduler::new(entry),
            owner_maintenance: RendererPageOwnerMaintenanceResidence::new(std::time::Instant::now()),
            task_sources,
            lifecycle_gate: lifecycle_gate.map(LifecycleGate::new),
            page_creation_navigation_failure_publisher,
            script_environment_pin,
        }
    }

    fn resident_entry(&self) -> Option<&LivePageEntry> {
        self.turn_scheduler.resident()
    }

    fn resident_entry_mut(&mut self) -> Option<&mut LivePageEntry> {
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
        let action_window_deadline = entry.page_vm().next_action_window_deadline();
        let internal_loading_deadline = task_sources
            .next_internal_loading_deadline(entry.page_vm().current_page_internal_loading_owner());
        earliest_deadline(
            earliest_deadline(timer_deadline, action_window_deadline),
            internal_loading_deadline,
        )
    }

    #[cfg(debug_assertions)]
    fn local_page_task_deadline(&self) -> Option<std::time::Instant> {
        let entry = self.turn_scheduler.resident()?;
        let timer_deadline = entry.next_javascript_timer_deadline();
        let action_window_deadline = entry.page_vm().next_action_window_deadline();
        let internal_loading_deadline = self
            .task_sources
            .local_internal_loading_deadline(entry.page_vm().current_page_internal_loading_owner());
        earliest_deadline(
            earliest_deadline(timer_deadline, action_window_deadline),
            internal_loading_deadline,
        )
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
        bound::reserve_renderer_document_isolate_on_bound_owner_local_store(
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
            bound::remove_reserved_renderer_document_isolate_on_bound_owner_local_store(
                self.token,
                self.reservation_id,
            );
            self.active.set(false);
        }
    }
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

    fn take_entry_for_command(&mut self, token: RendererPageToken) -> Result<LivePageEntry> {
        self.store.take_entry_for_command(token)
    }

    fn checkout_entry_for_owner_turn(&mut self, token: RendererPageToken) -> LivePageEntryCheckout {
        self.store.checkout_entry_for_owner_turn(token)
    }

    fn remove_page(&mut self, token: RendererPageToken) {
        self.store.remove_page(token)
    }

    fn restore_entry_after_command(&mut self, token: RendererPageToken, entry: LivePageEntry) {
        self.store.restore_entry_after_command(token, entry);
    }

    fn restore_retiring_entry_after_command(
        &mut self,
        token: RendererPageToken,
        entry: RetiringPageEntry,
    ) {
        self.store
            .restore_retiring_entry_after_command(token, entry);
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
        let observation = bound::reconcile_page_creation_lifecycle_observation(
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

    fn take_entry_for_command(&mut self, token: RendererPageToken) -> Result<LivePageEntry> {
        match self.checkout_entry_for_owner_turn(token) {
            Ok(entry) => Ok(entry),
            Err(LivePageEntryCheckoutError::Busy) => Err(anyhow!(
                "renderer local host page {} is already running an owner turn",
                token.page_id.as_u64()
            )),
            Err(LivePageEntryCheckoutError::Retired) => Err(anyhow!(
                "renderer local host page {} is retiring",
                token.page_id.as_u64()
            )),
            Err(LivePageEntryCheckoutError::Missing) => Err(anyhow!(
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
            .and_then(LivePageEntry::uncommitted_page_vm_creation_id)
    }

    fn checkout_entry_for_owner_turn(&mut self, token: RendererPageToken) -> LivePageEntryCheckout {
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
                RendererPageEntryCheckout::Busy => Err(LivePageEntryCheckoutError::Busy),
                RendererPageEntryCheckout::Retired => Err(LivePageEntryCheckoutError::Retired),
            },
            None => Err(LivePageEntryCheckoutError::Missing),
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
                        let scheduled_turn = bound::select_page_scheduler_turn(
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
        let page_turn_is_runnable = !bound::page_ready_descriptor_snapshot(entry, task_sources)
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
        let snapshot = bound::page_ready_descriptor_snapshot(entry, task_sources);
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
        let entry = LivePageEntry::new(slot.clone(), vm)?;
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
        let entry =
            LivePageEntry::new_with_pending_phase_one_navigation(slot.clone(), pending_navigation)?;
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
            Err(LivePageEntryCheckoutError::Busy) => {
                return RendererPageCreationCommit {
                    finalized: Err(anyhow!(
                        "renderer page {} remained checked out while finalizing page creation",
                        token.page_id.as_u64()
                    )),
                    renderer_output: None,
                };
            }
            Err(LivePageEntryCheckoutError::Retired | LivePageEntryCheckoutError::Missing) => {
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
        entry: LivePageEntry,
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
        mut entry: LivePageEntry,
        failure: PageCreationRetirement,
    ) -> RendererPageCreationResolution {
        let renderer_output = entry.page_vm_mut().settle_renderer_output_publication();
        self.restore_entry_after_command(token, entry);
        RendererPageCreationResolution::retiring(failure, renderer_output)
    }

    fn commit_page_creation_reply(
        &mut self,
        pending: RendererPendingPageCreation,
        mut entry: LivePageEntry,
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
            let devtools_target = entry.page_vm().devtools_target();
            let script_execution_control = entry.slot.script_execution_control();
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
                devtools_target,
                script_execution_control,
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
                devtools_target,
                script_execution_control,
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
                        devtools_target,
                        script_execution_control,
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
            vm.script_execution_control(),
        )
    }

    fn attach_page_entry_for_owner(
        &mut self,
        owner: &RendererOwnerLocalContext,
        slot: RendererPageSlotHandle,
        mut entry: LivePageEntry,
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
        let devtools_target = attached_entry.page_vm().devtools_target();
        devtools_target.pause_ref().configure_page_route(
            attached_entry
                .page_vm()
                .renderer_page_script_environment()
                .expect("an attached Page must own a renderer script environment")
                .output_journal(),
        );
        devtools_target
            .main_ref()
            .configure_owner_wake(owner.render_runtime.clone());
        devtools_target
            .io_ref()
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

    fn view_generation(entry: &LivePageEntry) -> u64 {
        entry.slot.entry().view_generation
    }

    fn prepare_next_view_generation(entry: &LivePageEntry) -> u64 {
        Self::view_generation(entry).saturating_add(1)
    }

    fn advance_command_epoch(entry: &LivePageEntry) -> u64 {
        entry.slot.entry().command_epoch().saturating_add(1)
    }

    fn current_view_for_testing_on_entry(entry: &LivePageEntry) -> Result<RendererPageView> {
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

    fn refresh_view_on_entry(entry: &LivePageEntry, view: RendererPageView) -> Result<()> {
        entry.slot.refresh_owned_view(view)
    }

    fn commit_next_page_state_on_entry(
        entry: &LivePageEntry,
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
        entry: &LivePageEntry,
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
        entry: &mut LivePageEntry,
    ) -> Result<Arc<RendererPageState>> {
        Self::commit_active_vm_page_state_on_entry_with_policy(
            entry,
            super::RendererPageStateCapturePolicy::FullReport,
        )
    }

    fn commit_active_vm_page_state_on_entry_with_policy(
        entry: &mut LivePageEntry,
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
        entry: &mut LivePageEntry,
    ) -> Result<Arc<RendererPageState>> {
        Self::commit_current_vm_page_state_on_entry_with_policy(
            entry,
            super::RendererPageStateCapturePolicy::FullReport,
        )
    }

    fn commit_current_vm_page_state_on_entry_with_policy(
        entry: &mut LivePageEntry,
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
        entry: &mut LivePageEntry,
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
        let _nested_main_page = super::nested_main::bind_active_nested_main_page(entry);
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
        entry: LivePageEntry,
    ) {
        self.restore_entry_after_command_inner(token, entry);
    }

    fn restore_retiring_entry_after_command(
        &mut self,
        token: RendererPageToken,
        entry: RetiringPageEntry,
    ) {
        let slot_is_retiring = self
            .page_hosts
            .get(&token.local_host_id)
            .and_then(|host| host.pages.get(&token.page_id))
            .is_none_or(|page_slot| page_slot.turn_scheduler.is_retiring());
        assert!(
            slot_is_retiring,
            "renderer page {} must be marked retiring before returning a RetiringPageEntry",
            token.page_id.as_u64()
        );
        self.restore_entry_after_command_inner(token, entry.entry);
    }

    fn restore_entry_after_command_inner(
        &mut self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
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
