use super::access::run_named_owner_local_task;
use super::document_lifecycle_turn::DocumentLifecycleObserverOutcome;
use super::navigation::{
    PageCreationNavigationFailurePublication, PageCreationResolution, PageNavigationOwnerFailure,
};
use super::owner_local::RendererAttachedPage;
use super::owner_local_store::{
    LivePageEntry, LivePageEntryCheckoutError, LivePageNavigationFailureRecipient,
    LivePageNavigationFollowOutcome, LivePageNavigationFollowTurn,
    LivePagePendingNavigationCompletion, LivePagePendingNavigationPhaseOneAdvance,
    NavigationReplyPolicy, PendingPhaseOneEntryAdvance, RendererDisplacedOrdinaryTurn,
    RendererDocumentIsolateAllocator, RendererDocumentIsolateReservation,
    RendererOwnerLocalContext, RendererOwnerLocalStore, RendererPageCommandDispatch,
    RendererPageCreationResolution, RendererPageScheduledTurn, RendererPageToken,
    RendererPageTurnAdmission, RendererPageTurnCheckoutError, RendererPendingPageCreation,
    RendererPreparedDocumentResidence, RetiringPageEntry,
    advance_document_lifecycle_one_page_turn_via_local_task,
    advance_dom_stable_wait_turn_on_entry_via_local_task,
    advance_network_idle_wait_turn_on_entry_via_local_task,
    advance_page_owner_one_turn_via_local_task,
    advance_pending_phase_one_navigation_on_entry_via_local_task,
    advance_runtime_command_lifecycle_on_entry_via_local_task,
    advance_runtime_expression_await_turn_on_entry_via_local_task,
    advance_script_truthy_wait_turn_on_entry_via_local_task,
    advance_selector_wait_turn_on_entry_via_local_task,
    advance_subresource_response_wait_turn_on_entry_via_local_task,
    begin_post_parse_lifecycle_on_entry_via_local_task, bind_render_runtime_owner_local_store,
    checkout_entry_for_owner_turn_on_bound_owner_local_store,
    checkout_scheduled_page_turn_on_bound_owner_local_store,
    claim_due_owner_maintenance_task_on_bound_owner_local_store,
    commit_page_state_on_entry_via_local_task,
    commit_page_state_on_entry_via_local_task_with_policy,
    dispatch_async_command_on_entry_via_local_task,
    finalize_pending_page_creation_on_bound_owner_local_store,
    follow_pending_location_navigation_one_turn_on_entry_via_local_task,
    has_pending_document_lifecycle_turn_on_entry, install_page_vm_on_bound_owner_local_store,
    install_phase_one_blocked_page_on_bound_owner_local_store,
    next_owner_maintenance_deadline_on_bound_owner_local_store,
    next_page_task_deadline_on_bound_owner_local_store, observe_document_lifecycle_on_entry,
    owner_local_store_session, page_turn_readiness_after_restore_on_bound_owner_local_store,
    pending_phase_one_admission_after_restore_on_bound_owner_local_store,
    publish_page_navigation_failure_on_bound_owner_local_store,
    release_lifecycle_gate_on_bound_owner_local_store,
    release_post_response_document_lifecycle_on_bound_owner_local_store,
    remove_page_on_bound_owner_local_store, remove_page_on_bound_owner_local_store_via_local_task,
    renderer_output_fence_for_tail_on_bound_owner_local_store,
    renderer_page_token_for_owner_context,
    resolve_pending_page_creation_on_bound_owner_local_store,
    restore_entry_after_command_on_bound_owner_local_store,
    restore_entry_after_document_lifecycle_on_bound_owner_local_store,
    restore_retiring_entry_after_command_on_bound_owner_local_store,
    schedule_page_turn_on_bound_owner_local_store,
    settle_owner_maintenance_task_on_bound_owner_local_store,
    snapshot_due_page_task_tokens_on_bound_owner_local_store,
    take_entry_for_command_on_bound_owner_local_store,
};
use super::owner_maintenance::{
    RendererOwnerMaintenanceTask, execute_owner_maintenance_task_on_local_lane,
};
use super::page_turn_scheduler::{PageOwnerNextTurn, PageTurnTrigger};
use super::page_vm::{
    DocumentLifecycleTurnAction, DocumentLifecycleTurnOutcome, DocumentLifecycleTurnReadiness,
    PageVmRuntimeCommandLifecycleAdvance, PageVmRuntimeCommandOutputScopeId,
};
use super::phase_one::{
    ConcurrentParseTimeRuntime, ExternalRawDocumentBodyStream, ParseTimePageVmCreationOutcome,
    StreamingHtmlPageCreationResult, StreamingNavigationPageCreationResult,
    response_headers_indicate_download,
};
use super::*;
use crate::RendererTopLevelNavigationDispatch;
use crate::devtools::ingress::{
    io::RendererInspectorIoOwnerWake,
    main::{RendererInspectorMainFirstDispatchGuard, RendererInspectorMainOwnerWake},
};
use crate::document_runtime::{
    response_content_security_policies_from_headers,
    response_content_security_report_only_policies_from_headers,
};
use crate::page_task_queue::{
    PostParsePageOwnedWork, RendererOwnerWake, RendererOwnerWakeSender, RendererOwnerWakeSource,
    RendererTopLevelNavigationHandoff,
};
use crate::referrer_policy::response_referrer_policy_from_headers;
use crate::render_runtime::{RenderRuntimeEnvelope, RenderRuntimeHandle, RenderRuntimeOwner};
use crate::script_vm::{
    PendingRuntimeEvaluateCall, RendererDocumentIsolateBootstrap, dispatch_inspector_io_owner_wake,
    dispatch_inspector_main_owner_wake,
};
use crate::service_worker_runtime::{
    ServiceWorkerRuntimeOwnerWake, service_worker_owner_wake_channel,
};
use crate::shared_worker_runtime::{
    SharedWorkerRuntimeOwnerWake, shared_worker_owner_wake_channel,
};
use moli_page_types::LayoutPolicy;
use std::collections::VecDeque;
use tokio::sync::{mpsc, oneshot};

mod lifecycle_decision;

use self::lifecycle_decision::PendingLifecycleNavigation;

#[derive(Debug, Clone)]
pub struct RendererPreparedDocumentCommitConfiguration {
    pub document_start_scripts: Vec<DocumentStartScript>,
    pub runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    pub runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
    pub runtime_isolated_worlds: Vec<crate::protocol_types::RuntimeIsolatedWorldDefinition>,
    pub permission_overrides: Vec<crate::protocol_types::PermissionOverrideRegistration>,
    pub extra_http_headers: Vec<(String, String)>,
    pub locale_override: Option<String>,
    pub timezone_override: Option<String>,
    pub script_execution_disabled: bool,
    pub bypass_content_security_policy: bool,
    pub cpu_throttling_rate: f64,
    pub emulated_media: crate::protocol_types::EmulatedMediaOverrides,
    pub idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    pub viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    pub network_offline: bool,
    pub blocked_url_patterns: Vec<String>,
    pub fetch_subresource_interception_enabled: bool,
    pub fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
}

#[derive(Debug)]
pub struct RendererCreateHtmlPageRequest {
    /// Exact owner-local Page identity reserved before this request is queued.
    ///
    /// Parser/resource output produced while the Page is still being built
    /// must route through this identity rather than the currently installed
    /// protocol target.
    pub page_reservation: RendererPageReservationToken,
    pub root_frame_id: Option<String>,
    pub main_document_commit: Option<RendererMainDocumentCommit>,
    pub top_level_storage_key: Option<moli_storage_key::MoliStorageKey>,
    pub requested_url: Url,
    pub navigation_initiator_url: Option<Url>,
    pub navigation_redirected: bool,
    pub navigation_redirect_count: usize,
    pub response_status: u16,
    pub response_headers: Vec<(String, String)>,
    pub loader: ResourceRequestClient,
    pub web_storage: crate::RendererWebStorageHandles,
    pub final_url: Url,
    pub html: String,
    pub document_start_scripts: Vec<DocumentStartScript>,
    pub runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    pub runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
    pub runtime_isolated_worlds: Vec<crate::protocol_types::RuntimeIsolatedWorldDefinition>,
    pub permission_overrides: Vec<crate::protocol_types::PermissionOverrideRegistration>,
    pub extra_http_headers: Vec<(String, String)>,
    pub locale_override: Option<String>,
    pub timezone_override: Option<String>,
    pub script_execution_disabled: bool,
    pub bypass_content_security_policy: bool,
    pub cpu_throttling_rate: f64,
    pub network_offline: bool,
    pub blocked_url_patterns: Vec<String>,
    pub indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    pub storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    pub emulated_media: crate::protocol_types::EmulatedMediaOverrides,
    pub idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    pub viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    pub fetch_subresource_interception_enabled: bool,
    pub fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
    pub layout_policy: LayoutPolicy,
    pub wpt_extensions_enabled: bool,
    pub stage: PageVmInitStage,
    pub reply_boundary: crate::RendererReplyBoundary,
    pub lifecycle_decider: Option<RendererLifecycleDecider>,
    /// Decides whether a non-`javascript:` location request remains inside
    /// the standalone adapter or becomes a browser-owner output action.
    ///
    /// This belongs to Page creation, not protocol capture: every later
    /// command and lifecycle turn must observe the same navigation owner.
    pub top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
    pub reserved_service_worker_client: Option<RendererReservedServiceWorkerClient>,
}

pub struct RendererCreateStreamingRawPageRequest {
    pub root_frame_id: Option<String>,
    pub main_document_commit: Option<RendererMainDocumentCommit>,
    pub requested_url: Url,
    pub final_url: Url,
    pub navigation_initiator_url: Option<Url>,
    pub navigation_redirected: bool,
    pub navigation_redirect_count: usize,
    pub response_status: u16,
    pub response_headers: Vec<(String, String)>,
    pub loader: ResourceRequestClient,
    pub web_storage: crate::RendererWebStorageHandles,
    pub raw_body: ExternalRawDocumentBodyStream,
    pub document_start_scripts: Vec<DocumentStartScript>,
    pub runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    pub runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
    pub runtime_isolated_worlds: Vec<crate::protocol_types::RuntimeIsolatedWorldDefinition>,
    pub permission_overrides: Vec<crate::protocol_types::PermissionOverrideRegistration>,
    pub extra_http_headers: Vec<(String, String)>,
    pub locale_override: Option<String>,
    pub timezone_override: Option<String>,
    pub script_execution_disabled: bool,
    pub bypass_content_security_policy: bool,
    pub cpu_throttling_rate: f64,
    pub network_offline: bool,
    pub blocked_url_patterns: Vec<String>,
    pub indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    pub storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    pub emulated_media: crate::protocol_types::EmulatedMediaOverrides,
    pub idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    pub viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    pub fetch_subresource_interception_enabled: bool,
    pub fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
    pub layout_policy: LayoutPolicy,
    pub wpt_extensions_enabled: bool,
    pub stage: PageVmInitStage,
    pub reply_boundary: crate::RendererReplyBoundary,
    pub lifecycle_decider: Option<RendererLifecycleDecider>,
    pub(super) top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
    pub(super) navigation_reply_policy: NavigationReplyPolicy,
    pub reserved_service_worker_client: Option<RendererReservedServiceWorkerClient>,
}

pub enum RendererOwnerCommand {
    CreateHtmlPage(RendererCreateHtmlPageRequest),
    PrepareStreamingRawDocument {
        token: RendererPageReservationToken,
        request: RendererCreateStreamingRawPageRequest,
    },
    UpdatePreparedRendererDocumentCommitConfiguration {
        token: RendererPageReservationToken,
        configuration: RendererPreparedDocumentCommitConfiguration,
    },
    CommitPreparedRendererDocument {
        permit: RendererDocumentCommitPermit,
    },
    CancelPreparedRendererDocument {
        token: RendererPageReservationToken,
    },
    RunAsyncPageCommand {
        token: RendererPageToken,
        command: RendererPageCommand,
    },
    RunProtocolPageCommand {
        token: RendererPageToken,
        command: RendererPageCommand,
    },
    /// Renderer-side cleanup after the browser/protocol owner has already
    /// disconnected a DevTools session and closed both of its ingress lanes.
    /// This is lifecycle work, not another frontend Inspector command.
    FinalizeRuntimeInspectorSessionDetach {
        token: RendererPageToken,
        inspector_session_id: Option<String>,
        pause_guard: RendererRuntimeInspectorSessionDetachGuard,
    },
    WaitForNetworkIdle {
        token: RendererPageToken,
        timeout_ms: u64,
        loader: ResourceRequestClient,
    },
    WaitForDomStable {
        token: RendererPageToken,
        timeout_ms: u64,
        loader: ResourceRequestClient,
    },
    RemovePage {
        token: RendererPageToken,
    },
    TestingCurrentPageState {
        token: RendererPageToken,
    },
    TestingRendererPageView {
        token: RendererPageToken,
    },
    TestingOwnerSlot {
        token: RendererPageToken,
    },
    TestingHostInstanceKey {
        token: RendererPageToken,
    },
    TestingHostUniqueDocumentIsolateCount {
        token: RendererPageToken,
    },
    #[cfg(test)]
    TestingDeferredPageVmDropPendingCount,
}

pub enum RendererOwnerReply {
    PageCreated(Box<RendererAttachedPage>),
    PreparedRendererDocumentStored {
        renderer_devtools_agent_token: RendererDevToolsAgentToken,
    },
    PreparedRendererDocumentCommitConfigurationUpdated,
    PreparedRendererDocumentCanceled,
    AsyncPageCommandRan(Box<RendererCommandTurnOutput>),
    RuntimeInspectorSessionDetachFinalized(bool),
    PageRemoved,
    TestingCurrentPageState(Arc<RendererPageState>),
    TestingRendererPageView(RendererPageView),
    TestingOwnerSlot(RendererPageSlotHandle),
    TestingHostInstanceKey(usize),
    TestingHostUniqueDocumentIsolateCount(usize),
    #[cfg(test)]
    TestingDeferredPageVmDropPendingCount(usize),
}

enum RenderRuntimeDispatchOutcome {
    Reply(Box<Result<RendererOwnerReply>>),
    InspectorMainCommandClaimed {
        reply_tx: oneshot::Sender<Result<RendererOwnerReply>>,
        turn: Box<RenderRuntimeTurn>,
        command_admission_output_predecessor: Option<RendererOutputFence>,
    },
    PageCreatedAndContinueNavigation {
        page: Box<RendererAttachedPage>,
        continuation: RenderRuntimePageCreationContinuation,
    },
    BackgroundComplete(Result<()>),
    PageCreationNavigationFailurePublished {
        token: RendererPageToken,
        failure: PageNavigationOwnerFailure,
    },
    ContinueNextTurn(Box<RenderRuntimeTurn>),
    ContinueAfterPageWakeOrDeadline {
        turn: Box<RenderRuntimeTurn>,
        wake_token: RendererPageToken,
        ready_at: Instant,
    },
    ContinueAfterPageWake {
        turn: Box<RenderRuntimeTurn>,
        wake_token: RendererPageToken,
    },
    ContinueCommittedDocumentParserAfterPageWake {
        turn: Box<RenderRuntimeTurn>,
        wake_token: RendererPageToken,
    },
}

enum RenderRuntimePageCreationContinuation {
    NextTurn(Box<RenderRuntimeTurn>),
    AfterCommittedDocumentResponse {
        turn: Box<RenderRuntimeTurn>,
        wake_token: RendererPageToken,
    },
}

impl RenderRuntimePageCreationContinuation {
    fn next_turn(turn: RenderRuntimeTurn) -> Self {
        Self::NextTurn(Box::new(turn))
    }

    fn into_turn(self) -> RenderRuntimeTurn {
        match self {
            Self::NextTurn(turn) | Self::AfterCommittedDocumentResponse { turn, .. } => *turn,
        }
    }

    fn turn(&self) -> &RenderRuntimeTurn {
        match self {
            Self::NextTurn(turn) | Self::AfterCommittedDocumentResponse { turn, .. } => turn,
        }
    }

    fn requires_committed_document_response_release(&self) -> bool {
        matches!(self, Self::AfterCommittedDocumentResponse { .. })
    }
}

enum LivePageNavigationFailureDisposition {
    ReturnToInitiator(anyhow::Error),
    PublishToPageCreation(PageNavigationOwnerFailure),
    ReportBackground(PageNavigationOwnerFailure),
}

impl std::fmt::Display for LivePageNavigationFailureDisposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReturnToInitiator(error) => std::fmt::Display::fmt(error, f),
            Self::PublishToPageCreation(failure) | Self::ReportBackground(failure) => {
                std::fmt::Display::fmt(failure, f)
            }
        }
    }
}

pub(super) struct RenderRuntimePendingTurn {
    /// `None` means the pending turn is detached/background work (originated
    /// from an idle-tick) and no caller is waiting on a reply.
    reply_tx: Option<oneshot::Sender<Result<RendererOwnerReply>>>,
    turn: RenderRuntimeTurn,
    allow_command_overtake: bool,
    command_admission_output_predecessor: Option<RendererOutputFence>,
}

impl RenderRuntimePendingTurn {
    const fn is_page_owner_turn(&self) -> bool {
        matches!(&self.turn, RenderRuntimeTurn::RunPageTurn { .. })
    }

    const fn page_owner_token(&self) -> Option<RendererPageToken> {
        match &self.turn {
            RenderRuntimeTurn::RunPageTurn { token } => Some(*token),
            _ => None,
        }
    }

    const fn is_owner_maintenance_turn(&self) -> bool {
        matches!(&self.turn, RenderRuntimeTurn::RunOwnerMaintenance { .. })
    }
}

#[derive(Default)]
struct RenderRuntimePendingTurnQueue {
    turns: VecDeque<RenderRuntimePendingTurn>,
    page_owner_turn_count: usize,
    owner_maintenance_turn_count: usize,
}

impl RenderRuntimePendingTurnQueue {
    fn push_back(&mut self, turn: RenderRuntimePendingTurn) {
        self.page_owner_turn_count += usize::from(turn.is_page_owner_turn());
        self.owner_maintenance_turn_count += usize::from(turn.is_owner_maintenance_turn());
        self.turns.push_back(turn);
    }

    fn push_front(&mut self, turn: RenderRuntimePendingTurn) {
        self.page_owner_turn_count += usize::from(turn.is_page_owner_turn());
        self.owner_maintenance_turn_count += usize::from(turn.is_owner_maintenance_turn());
        self.turns.push_front(turn);
    }

    fn pop_front(&mut self) -> Option<RenderRuntimePendingTurn> {
        let turn = self.turns.pop_front()?;
        self.page_owner_turn_count -= usize::from(turn.is_page_owner_turn());
        self.owner_maintenance_turn_count -= usize::from(turn.is_owner_maintenance_turn());
        Some(turn)
    }

    const fn has_page_owner_turn(&self) -> bool {
        self.page_owner_turn_count != 0
    }

    const fn has_owner_maintenance_turn(&self) -> bool {
        self.owner_maintenance_turn_count != 0
    }
}

struct RenderRuntimeParkedTurn {
    reply_tx: Option<oneshot::Sender<Result<RendererOwnerReply>>>,
    turn: RenderRuntimeTurn,
    wake_token: RendererPageToken,
    ready_at: Option<Instant>,
    condition: RenderRuntimeParkCondition,
    command_admission_output_predecessor: Option<RendererOutputFence>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RenderRuntimeParkCondition {
    PageActivity,
    CommittedDocumentParserContinuation { parser_unblocked: bool },
    ReplacementDocumentViewSettlement { expected_vm_creation_id: u64 },
}

impl RenderRuntimeParkCondition {
    const fn admits_page_activity(self) -> bool {
        matches!(
            self,
            Self::PageActivity
                | Self::CommittedDocumentParserContinuation {
                    parser_unblocked: true
                }
        )
    }

    const fn blocks_page_activity_until_parser_unblocked(self) -> bool {
        matches!(
            self,
            Self::CommittedDocumentParserContinuation {
                parser_unblocked: false
            }
        )
    }

    const fn allows_command_overtake(self) -> bool {
        !matches!(self, Self::CommittedDocumentParserContinuation { .. })
    }

    const fn is_unblocked_committed_document_parser_continuation(self) -> bool {
        matches!(
            self,
            Self::CommittedDocumentParserContinuation {
                parser_unblocked: true
            }
        )
    }

    fn unblock_committed_document_parser(&mut self) -> bool {
        let Self::CommittedDocumentParserContinuation { parser_unblocked } = self else {
            return false;
        };
        if *parser_unblocked {
            return false;
        }
        *parser_unblocked = true;
        true
    }

    const fn admits_replacement_view_settlement(self, vm_creation_id: u64) -> bool {
        matches!(
            self,
            Self::ReplacementDocumentViewSettlement {
                expected_vm_creation_id
            } if expected_vm_creation_id == vm_creation_id
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PageTurnAdmissionPreference {
    ProducerWake,
    Deadline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReadyPageTurnAdmission {
    Admitted,
    NoneReady,
    WakeChannelClosed,
}

const LIVE_PAGE_COMMAND_WAIT_TURN_SLICE: std::time::Duration =
    std::time::Duration::from_millis(100);
const LIVE_PAGE_RUNTIME_EXPRESSION_AWAIT_TIMEOUT_MS: u64 = 30_000;

fn checked_live_page_wait_deadline(timeout_ms: u64, operation: &str) -> Result<Instant> {
    let timeout = std::time::Duration::from_millis(timeout_ms);
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| anyhow!("{operation} timeout is too large"))
}

enum RenderRuntimeTurn {
    FinishHtmlCreatePage {
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        page_vm: Box<PageVm>,
        page_tasks: Vec<PostParsePageOwnedWork>,
        stage: PageVmInitStage,
        started: Instant,
        reply_boundary: crate::RendererReplyBoundary,
        lifecycle_decider: Option<RendererLifecycleDecider>,
        top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
        navigation_reply_policy: NavigationReplyPolicy,
    },
    ContinueAttachedPageCreationLifecycle {
        pending: RendererPendingPageCreation,
        document: RendererDocumentLifecycleIdentity,
        target_stage: PageVmInitStage,
        navigation_reply_policy: NavigationReplyPolicy,
    },
    DrainSharedWorkerServiceLane,
    DrainServiceWorkerServiceLane,
    RunPageTurn {
        token: RendererPageToken,
    },
    /// Browser housekeeping for one stable Page slot. This turn is owner-local
    /// and V8-thread-affine, but it is not admitted through the HTML Page task
    /// scheduler and does not create a microtask checkpoint.
    RunOwnerMaintenance {
        task: RendererOwnerMaintenanceTask,
    },
    /// One task posted by the Main DevTools receiver. The command remains
    /// unclaimed until this turn actually runs, so a nested debugger loop can
    /// pump the same receiver while an earlier Page turn is paused in V8.
    RunInspectorMainReceiver {
        wake: RendererInspectorMainOwnerWake,
    },
    /// An owner-claimed Main command carrying its ingress permit until the
    /// concrete Page agent first-dispatch boundary.
    RunDevToolsMainCommand {
        token: RendererPageToken,
        command: RendererPageCommand,
        first_dispatch: RendererInspectorMainFirstDispatchGuard,
        capture_policy: super::RendererPageStateCapturePolicy,
    },
    RunLivePageCommand {
        token: RendererPageToken,
        command: RendererPageCommand,
        capture_policy: super::RendererPageStateCapturePolicy,
    },
    ContinueLivePageRuntimeCommandLifecycle {
        token: RendererPageToken,
        scope_id: PageVmRuntimeCommandOutputScopeId,
        reply: Box<RendererPageReply>,
        should_follow_pending_navigation: bool,
        turn_records: Vec<PendingRendererOutputRecord>,
        capture_policy: super::RendererPageStateCapturePolicy,
    },
    ResumeLivePageDocumentLifecycleAfterReply {
        token: RendererPageToken,
        document: RendererDocumentLifecycleIdentity,
    },
    WaitLivePageNetworkIdle {
        token: RendererPageToken,
        state: PageVmNetworkIdleWaitState,
        deadline: Instant,
        loader: ResourceRequestClient,
    },
    WaitLivePageDomStable {
        token: RendererPageToken,
        state: PageVmDomStableWaitState,
        deadline: Instant,
        loader: ResourceRequestClient,
    },
    WaitLifecycleNavigation(PendingLifecycleNavigation),
    WaitLivePageSelector {
        token: RendererPageToken,
        selector: String,
        deadline: Instant,
        loader: ResourceRequestClient,
        capture_policy: super::RendererPageStateCapturePolicy,
    },
    WaitLivePageScriptTruthy {
        token: RendererPageToken,
        expression: String,
        pending_call: Option<PendingRuntimeEvaluateCall>,
        deadline: Instant,
        loader: ResourceRequestClient,
        capture_policy: super::RendererPageStateCapturePolicy,
    },
    WaitLivePageRuntimeExpressionAwait {
        token: RendererPageToken,
        execution_context_id: Option<i64>,
        expression: String,
        pending_call: Option<PendingRuntimeEvaluateCall>,
        deadline: Instant,
        follow_pending_navigation: bool,
        capture_policy: super::RendererPageStateCapturePolicy,
    },
    WaitLivePageSubresourceResponse {
        token: RendererPageToken,
        criteria: SubresourceResponseWaitCriteria,
        deadline: Instant,
        loader: ResourceRequestClient,
        capture_policy: super::RendererPageStateCapturePolicy,
    },
    WaitLivePageChildFrameLifecycle {
        token: RendererPageToken,
        deadline: Instant,
        capture_policy: super::RendererPageStateCapturePolicy,
    },
    ClaimLivePageTopLevelNavigationHandoff {
        token: RendererPageToken,
        handoff: RendererTopLevelNavigationHandoff,
    },
    FollowLivePagePendingLocationNavigation {
        token: RendererPageToken,
        stage: PageVmInitStage,
        follow_count: usize,
        completion: LivePagePendingNavigationCompletion,
    },
    ContinueLivePagePendingLocationNavigationPhaseOne {
        token: RendererPageToken,
        follow_count: usize,
        completion: LivePagePendingNavigationCompletion,
    },
    ContinueLivePageNavigationPostParseLifecycle {
        token: RendererPageToken,
        document: RendererDocumentLifecycleIdentity,
        target_stage: PageVmInitStage,
        follow_count: usize,
        completion: LivePagePendingNavigationCompletion,
    },
}

impl RenderRuntimeTurn {
    fn page_turn_should_yield_to_ready_command(&self) -> bool {
        !matches!(
            self,
            Self::DrainSharedWorkerServiceLane | Self::DrainServiceWorkerServiceLane
        )
    }

    /// Return the Page whose committed view this host-facing command needs.
    ///
    /// A same-Page cross-document navigation installs its replacement PageVm
    /// before publishing the matching `RendererPageState`. Commands must wait
    /// for that publication instead of running against the replacement VM and
    /// then trying to commit from the stale previous view.
    fn committed_page_view_command_token(&self) -> Option<RendererPageToken> {
        match self {
            Self::RunDevToolsMainCommand { token, .. }
            | Self::RunLivePageCommand { token, .. }
            | Self::ContinueLivePageRuntimeCommandLifecycle { token, .. }
            | Self::WaitLivePageNetworkIdle { token, .. }
            | Self::WaitLivePageDomStable { token, .. }
            | Self::WaitLivePageSelector { token, .. }
            | Self::WaitLivePageScriptTruthy { token, .. }
            | Self::WaitLivePageRuntimeExpressionAwait { token, .. }
            | Self::WaitLivePageSubresourceResponse { token, .. }
            | Self::WaitLivePageChildFrameLifecycle { token, .. } => Some(*token),
            _ => None,
        }
    }

    fn committed_page_view_deadline(&self) -> Option<Instant> {
        match self {
            Self::WaitLivePageNetworkIdle { deadline, .. }
            | Self::WaitLivePageDomStable { deadline, .. }
            | Self::WaitLivePageSelector { deadline, .. }
            | Self::WaitLivePageScriptTruthy { deadline, .. }
            | Self::WaitLivePageRuntimeExpressionAwait { deadline, .. }
            | Self::WaitLivePageSubresourceResponse { deadline, .. }
            | Self::WaitLivePageChildFrameLifecycle { deadline, .. } => Some(*deadline),
            _ => None,
        }
    }

    fn is_page_creation_lifecycle_observer_for(&self, token: RendererPageToken) -> bool {
        match self {
            Self::ContinueAttachedPageCreationLifecycle { pending, .. } => pending.token == token,
            Self::WaitLifecycleNavigation(wait) => wait.token() == token,
            Self::ContinueLivePageNavigationPostParseLifecycle {
                token: observer_token,
                completion: LivePagePendingNavigationCompletion::CompletePageCreation { .. },
                ..
            } => *observer_token == token,
            _ => false,
        }
    }

    fn detach_navigation_command_observer(self) -> (Self, bool) {
        match self {
            Self::FollowLivePagePendingLocationNavigation {
                token,
                stage,
                follow_count,
                completion,
            } => {
                let (completion, detached) = completion.detach_command_observer();
                (
                    Self::FollowLivePagePendingLocationNavigation {
                        token,
                        stage,
                        follow_count,
                        completion,
                    },
                    detached,
                )
            }
            Self::ContinueLivePagePendingLocationNavigationPhaseOne {
                token,
                follow_count,
                completion,
            } => {
                let (completion, detached) = completion.detach_command_observer();
                (
                    Self::ContinueLivePagePendingLocationNavigationPhaseOne {
                        token,
                        follow_count,
                        completion,
                    },
                    detached,
                )
            }
            Self::ContinueLivePageNavigationPostParseLifecycle {
                token,
                document,
                target_stage,
                follow_count,
                completion,
            } => {
                let (completion, detached) = completion.detach_command_observer();
                (
                    Self::ContinueLivePageNavigationPostParseLifecycle {
                        token,
                        document,
                        target_stage,
                        follow_count,
                        completion,
                    },
                    detached,
                )
            }
            other => (other, false),
        }
    }
}

fn live_page_command_should_follow_pending_navigation(command: &RendererPageCommand) -> bool {
    // CDP Runtime.evaluate must not use the owner command tail as a generic
    // lifecycle pump. High-level Page API evaluation has an explicit follow
    // variant because callers expect a location.assign side effect to replace
    // the live page state. Input dispatch commands are CDP protocol
    // commands, not page-load commands: they may enqueue a pending location
    // navigation, but the browser/protocol navigation pipeline owns starting,
    // loading, and committing that navigation.
    matches!(
        command,
        RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation { .. }
            | RendererPageCommand::EvaluateExpressionInExecutionContextAndFollowPendingNavigation { .. }
    )
}

fn live_page_command_requires_materialized_child_realms(command: &RendererPageCommand) -> bool {
    matches!(command, RendererPageCommand::Inspector(envelope) if envelope.requires_materialized_child_realms())
}

const fn page_creation_navigation_reply_policy(
    dispatch: RendererTopLevelNavigationDispatch,
) -> NavigationReplyPolicy {
    match dispatch {
        RendererTopLevelNavigationDispatch::DelegateToBrowser => {
            NavigationReplyPolicy::ReturnWithPendingNavigation
        }
        RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter => {
            NavigationReplyPolicy::FollowBeforeReply
        }
    }
}

fn renderer_page_command_timing_label(command: &RendererPageCommand) -> Option<&'static str> {
    command.cdp_nav_timing_label()
}

fn runtime_command_output_scope_owned_by_dispatch(
    scope_before_dispatch: Option<PageVmRuntimeCommandOutputScopeId>,
    scope_after_dispatch: Option<PageVmRuntimeCommandOutputScopeId>,
) -> Option<PageVmRuntimeCommandOutputScopeId> {
    scope_after_dispatch.filter(|scope_id| Some(*scope_id) != scope_before_dispatch)
}

fn owner_command_timing_label(command: &RendererOwnerCommand) -> Option<&'static str> {
    match command {
        RendererOwnerCommand::RunAsyncPageCommand { command, .. }
        | RendererOwnerCommand::RunProtocolPageCommand { command, .. } => {
            renderer_page_command_timing_label(command)
        }
        _ => None,
    }
}

impl From<Result<RendererOwnerReply>> for RenderRuntimeDispatchOutcome {
    fn from(value: Result<RendererOwnerReply>) -> Self {
        Self::Reply(Box::new(value))
    }
}

fn renderer_command_admission_page_token(
    command: &RendererOwnerCommand,
) -> Option<RendererPageToken> {
    match command {
        RendererOwnerCommand::RunAsyncPageCommand { token, .. }
        | RendererOwnerCommand::RunProtocolPageCommand { token, .. }
        | RendererOwnerCommand::WaitForNetworkIdle { token, .. }
        | RendererOwnerCommand::WaitForDomStable { token, .. } => Some(*token),
        _ => None,
    }
}

fn merge_command_admission_output_predecessor(
    mut result: Result<RendererOwnerReply>,
    predecessor: Option<RendererOutputFence>,
) -> Result<RendererOwnerReply> {
    if let (Some(predecessor), Ok(RendererOwnerReply::AsyncPageCommandRan(output))) =
        (predecessor, &mut result)
    {
        output.merge_renderer_output_predecessor(predecessor);
    }
    result
}

#[derive(Debug)]
pub(super) struct RendererOwnerState {
    pub(super) page_table: RendererPageTable,
    pub(super) local_executor: JsLocalExecutor,
    pub(super) next_page_id: Arc<AtomicU64>,
    pub(super) page_wake_tx: mpsc::UnboundedSender<RendererOwnerWake>,
    pub(super) render_runtime_admission: std::sync::OnceLock<RenderRuntimeHandle>,
    pub(super) inspector_io_wake_tx: mpsc::UnboundedSender<RendererInspectorIoOwnerWake>,
    pub(super) browser_context_runtime: RendererBrowserContextRuntime,
    pub(super) owner_local_host_id: RendererOwnerLocalHostId,
    layout_policy: Mutex<RendererOwnerLayoutPolicyState>,
    context_shutdown_notify: tokio::sync::Notify,
    #[cfg(test)]
    command_dispatch_gate: Mutex<Option<RendererCommandDispatchGateForTesting>>,
    #[cfg(debug_assertions)]
    pub(super) owner_local_thread_id: Mutex<Option<ThreadId>>,
}

#[derive(Debug, Clone, Copy, Default)]
struct RendererOwnerLayoutPolicyState {
    policy: Option<LayoutPolicy>,
}

#[cfg(test)]
#[derive(Debug)]
struct RendererCommandDispatchGateForTesting {
    entered_tx: crossbeam_channel::Sender<()>,
    release_rx: crossbeam_channel::Receiver<()>,
}

impl Drop for RendererOwnerState {
    fn drop(&mut self) {
        self.page_table
            .terminate_and_cancel_all_contexts(RendererPageContextCancelReason::ContextDropped);
    }
}

#[derive(Clone, Debug)]
pub struct RendererOwnerHandle {
    pub(super) state: Arc<RendererOwnerState>,
    render_runtime: RenderRuntimeHandle,
}

fn page_turn_trigger_log_label(trigger: PageTurnTrigger) -> &'static str {
    match trigger.producer_source() {
        Some(RendererOwnerWakeSource::SchedulerContinuation) => "scheduler-continuation",
        Some(RendererOwnerWakeSource::NetworkingTask) => "page-networking-task-wake",
        Some(RendererOwnerWakeSource::ParseTimeDocumentScriptWork) => {
            "parse-time-document-script-task-wake"
        }
        Some(RendererOwnerWakeSource::DomManipulationTask) => "dom-manipulation-task-wake",
        Some(RendererOwnerWakeSource::UserInteractionTask) => "user-interaction-task-wake",
        Some(RendererOwnerWakeSource::FileReadingTask) => "file-reading-task-wake",
        Some(RendererOwnerWakeSource::MiscPlatformApiTask) => "misc-platform-api-task-wake",
        Some(RendererOwnerWakeSource::NavigationAndTraversalTask) => {
            "navigation-and-traversal-task-wake"
        }
        Some(RendererOwnerWakeSource::DedicatedWorkerClientEvent) => {
            "dedicated-worker-client-event-wake"
        }
        Some(RendererOwnerWakeSource::SharedWorkerClientEvent) => "shared-worker-client-event-wake",
        Some(RendererOwnerWakeSource::ServiceWorkerInternalTask) => {
            "service-worker-internal-task-wake"
        }
        Some(RendererOwnerWakeSource::ServiceWorkerClientMessage) => {
            "service-worker-client-message-wake"
        }
        Some(RendererOwnerWakeSource::WebCryptoTask) => "webcrypto-task-wake",
        Some(RendererOwnerWakeSource::IndexedDbTask) => "indexed-db-task-wake",
        Some(RendererOwnerWakeSource::OpfsTask) => "opfs-task-wake",
        Some(RendererOwnerWakeSource::InternalLoadingTask) => "internal-loading-task-wake",
        Some(RendererOwnerWakeSource::MainDocumentRuntimeTask) => "main-document-runtime-task-wake",
        Some(RendererOwnerWakeSource::ChildModuleDependencyFetchStart) => {
            "child-module-dependency-fetch-start-wake"
        }
        Some(RendererOwnerWakeSource::ChildModuleScriptTerminal) => {
            "child-module-script-terminal-wake"
        }
        Some(RendererOwnerWakeSource::ChildModulepreloadEventAction) => {
            "child-modulepreload-event-action-wake"
        }
        Some(RendererOwnerWakeSource::ChildFrameTask) => "child-frame-task-wake",
        Some(RendererOwnerWakeSource::V8ForegroundTask) => "v8-foreground-task-wake",
        Some(RendererOwnerWakeSource::ModuleReaction) => "module-reaction-wake",
        Some(RendererOwnerWakeSource::WindowMessageTask) => "window-message-task-wake",
        Some(RendererOwnerWakeSource::MessagePortDelivery) => "message-port-delivery-wake",
        Some(RendererOwnerWakeSource::RenderingUpdateTask) => "rendering-update-task-wake",
        Some(RendererOwnerWakeSource::MediaElementEventTask) => "media-element-event-task-wake",
        Some(RendererOwnerWakeSource::DynamicImportOwnerAction) => {
            "child-dynamic-import-owner-action-wake"
        }
        Some(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::SelectedTaskOutput,
        )) => "selected-task-output-wake",
        Some(RendererOwnerWakeSource::ModulepreloadStart) => "child-modulepreload-start-wake",
        Some(RendererOwnerWakeSource::Runtime(RendererOwnerRuntimeActivitySource::Timer)) => {
            "timer-wake"
        }
        Some(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::NavigationAndTraversal,
        )) => "navigation-and-traversal-output-wake",
        Some(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::RenderingUpdate,
        )) => "rendering-update-output-wake",
        Some(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::MediaElementEvent,
        )) => "media-element-event-output-wake",
        Some(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::DomManipulation,
        )) => "dom-manipulation-output-wake",
        Some(RendererOwnerWakeSource::Runtime(RendererOwnerRuntimeActivitySource::Networking)) => {
            "networking-output-wake"
        }
        Some(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::UserInteraction,
        )) => "user-interaction-output-wake",
        Some(RendererOwnerWakeSource::Runtime(RendererOwnerRuntimeActivitySource::FileReading)) => {
            "file-reading-output-wake"
        }
        Some(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::MiscPlatformApi,
        )) => "misc-platform-api-output-wake",
        Some(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::WindowMessage,
        )) => "window-message-wake",
        Some(RendererOwnerWakeSource::Runtime(RendererOwnerRuntimeActivitySource::IndexedDb)) => {
            "indexed-db-wake"
        }
        Some(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::InternalLoading,
        )) => "internal-loading-wake",
        Some(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::DocumentReplacement,
        )) => "document-replacement-wake",
        Some(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::ModuleReaction,
        )) => "module-reaction-wake",
        Some(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::V8ForegroundTask,
        )) => "v8-foreground-task-wake",
        Some(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::DocumentLifecycleTurn,
        )) => "document-lifecycle-turn-wake",
        Some(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::ChildRealmMaterialization,
        )) => "child-realm-materialization-wake",
        None => "deadline",
    }
}

fn loader_for_new_page(
    loader: &ResourceRequestClient,
    extra_http_headers: &[(String, String)],
    network_offline: bool,
    blocked_url_patterns: &[String],
) -> ResourceRequestClient {
    // A new target/Page may reuse the browser transport and memory cache, but
    // it must never inherit another Page's mutable policy by Arc identity.
    // Child Documents are created below this boundary and continue to clone
    // the resulting Page adapter intentionally.
    let page_loader = loader.fork_with_isolated_page_network_policy();
    page_loader.set_extra_http_headers(extra_http_headers);
    page_loader.set_network_offline(network_offline);
    page_loader.set_blocked_url_patterns(blocked_url_patterns);
    page_loader
}

impl RendererOwnerHandle {
    /// Selects the immutable layout policy for this renderer owner.
    ///
    /// A browser owner may configure the default policy before its first Page
    /// is constructed. Page construction seals the value so a shared renderer
    /// owner can never host Pages with conflicting browser-level policies.
    pub fn configure_layout_policy(&self, policy: LayoutPolicy) -> Result<()> {
        let mut state = self.state.layout_policy.lock();
        if let Some(configured) = state.policy {
            ensure!(
                configured == policy,
                "renderer owner layout policy is configured as {:?}, cannot change it to {:?}",
                configured,
                policy
            );
        } else {
            state.policy = Some(policy);
        }
        Ok(())
    }

    pub fn layout_policy(&self) -> LayoutPolicy {
        self.state.layout_policy.lock().policy.unwrap_or_default()
    }

    fn seal_layout_policy_for_page_creation(&self) -> LayoutPolicy {
        let mut state = self.state.layout_policy.lock();
        *state.policy.get_or_insert_with(LayoutPolicy::default)
    }

    async fn run_owner_lane_local_task<R, F>(&self, future: F) -> Result<R>
    where
        R: 'static,
        F: Future<Output = Result<R>> + 'static,
    {
        run_named_owner_local_task(
            self.state.local_executor.clone(),
            "owner-lane local task channel closed",
            future,
        )
        .await
    }

    async fn install_page_vm_on_owner_lane(
        &self,
        owner_local_context: RendererOwnerLocalContext,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        page_vm: PageVm,
        pending_download: Option<RendererPendingDownloadActivation>,
        lifecycle_decision: Option<PageVmInitStage>,
    ) -> Result<RendererPendingPageCreation> {
        self.run_owner_lane_local_task(async move {
            install_page_vm_on_bound_owner_local_store(
                &owner_local_context,
                requested_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                response_status,
                response_headers,
                page_vm,
                pending_download,
                lifecycle_decision,
            )
        })
        .await
    }

    async fn install_phase_one_blocked_page_on_owner_lane(
        &self,
        owner_local_context: RendererOwnerLocalContext,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        pending_navigation: PageVmPendingPhaseOneNavigation,
        lifecycle_decision: Option<PageVmInitStage>,
    ) -> Result<RendererPendingPageCreation> {
        self.run_owner_lane_local_task(async move {
            install_phase_one_blocked_page_on_bound_owner_local_store(
                &owner_local_context,
                requested_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                response_status,
                response_headers,
                pending_navigation,
                lifecycle_decision,
            )
        })
        .await
    }

    async fn finalize_pending_page_creation_on_owner_lane(
        &self,
        pending: RendererPendingPageCreation,
    ) -> Result<RendererAttachedPage> {
        let token = pending.token;
        let commit = self
            .run_owner_lane_local_task(async move {
                Ok(finalize_pending_page_creation_on_bound_owner_local_store(
                    pending,
                ))
            })
            .await?;
        let finalized =
            commit.publish_then_finalize(|output| self.publish_renderer_output(output))?;
        if finalized.resume_parked_page_turn {
            self.signal_internal_page_turn_source(
                token,
                RendererOwnerWakeSource::SchedulerContinuation,
            );
        }
        Ok(finalized.attached_page)
    }

    async fn resolve_pending_page_creation_on_owner_lane(
        &self,
        pending: RendererPendingPageCreation,
        document: RendererDocumentLifecycleIdentity,
        target_stage: PageVmInitStage,
        navigation_reply_policy: NavigationReplyPolicy,
    ) -> Result<RendererPageCreationResolution> {
        self.run_owner_lane_local_task(async move {
            Ok(resolve_pending_page_creation_on_bound_owner_local_store(
                pending,
                document,
                target_stage,
                navigation_reply_policy,
            ))
        })
        .await
    }

    async fn install_page_vm_and_begin_post_parse_lifecycle(
        &self,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        page_vm: PageVm,
        page_tasks: Vec<PostParsePageOwnedWork>,
        stage: PageVmInitStage,
        started: Instant,
        reply_boundary: crate::RendererReplyBoundary,
        lifecycle_decider: Option<RendererLifecycleDecider>,
        top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
    ) -> Result<(RendererPendingPageCreation, DocumentLifecycleTurnOutcome)> {
        let owner_local_context = self.owner_local_context()?;
        let pending = self
            .install_page_vm_on_owner_lane(
                owner_local_context,
                requested_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                response_status,
                response_headers,
                page_vm,
                None,
                reply_boundary.waits_for_stage().then_some(stage),
            )
            .await?;
        let pending = pending.with_lifecycle_decider(stage, lifecycle_decider);
        let token = pending.token;
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => {
                remove_page_on_bound_owner_local_store(token);
                return Err(error);
            }
        };
        entry.set_top_level_navigation_dispatch(top_level_navigation_dispatch);
        let (entry, begin_result) = begin_post_parse_lifecycle_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            page_tasks,
            stage,
            started,
        )
        .await;
        self.restore_live_page_entry(token, entry);
        match begin_result {
            Ok(outcome) => Ok((pending, outcome)),
            Err(error) => {
                remove_page_on_bound_owner_local_store(token);
                Err(error)
            }
        }
    }

    async fn continue_pending_phase_one_page_creation(
        &self,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        pending_navigation: PageVmPendingPhaseOneNavigation,
        stage: PageVmInitStage,
        reply_boundary: crate::RendererReplyBoundary,
        lifecycle_decider: Option<RendererLifecycleDecider>,
        top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
        navigation_reply_policy: NavigationReplyPolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let owner_local_context = match self.owner_local_context() {
            Ok(context) => context,
            Err(error) => return Err(error).into(),
        };
        let pending = match self
            .install_phase_one_blocked_page_on_owner_lane(
                owner_local_context,
                requested_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                response_status,
                response_headers,
                pending_navigation,
                reply_boundary.waits_for_stage().then_some(stage),
            )
            .await
        {
            Ok(pending) => pending,
            Err(error) => return Err(error).into(),
        };
        let pending = pending.with_lifecycle_decider(stage, lifecycle_decider);
        let token = pending.token;
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        entry.set_top_level_navigation_dispatch(top_level_navigation_dispatch);
        self.restore_live_page_entry(token, entry);

        if matches!(reply_boundary, crate::RendererReplyBoundary::DocumentCommit) {
            self.publish_pending_page_creation_and_continue(
                pending,
                RenderRuntimePageCreationContinuation::AfterCommittedDocumentResponse {
                    turn: Box::new(
                        RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                            token,
                            follow_count: 0,
                            completion:
                                LivePagePendingNavigationCompletion::PublishedPageCreation {
                                    navigation_reply_policy,
                                },
                        },
                    ),
                    wake_token: token,
                },
            )
            .await
        } else {
            let admission =
                pending_phase_one_admission_after_restore_on_bound_owner_local_store(token);
            self.signal_pending_phase_one_admission(token, admission);
            RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                turn: Box::new(
                    RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                        token,
                        follow_count: 0,
                        completion: LivePagePendingNavigationCompletion::CompletePageCreation {
                            pending,
                            navigation_reply_policy,
                        },
                    },
                ),
                wake_token: token,
            }
        }
    }

    async fn continue_page_creation_with_pending_navigation(
        &self,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        page_vm: PageVm,
        stage: PageVmInitStage,
        reply_boundary: crate::RendererReplyBoundary,
        lifecycle_decider: Option<RendererLifecycleDecider>,
        top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
        navigation_reply_policy: NavigationReplyPolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let owner_local_context = match self.owner_local_context() {
            Ok(context) => context,
            Err(error) => return Err(error).into(),
        };
        let pending = match self
            .install_page_vm_on_owner_lane(
                owner_local_context,
                requested_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                response_status,
                response_headers,
                page_vm,
                None,
                reply_boundary.waits_for_stage().then_some(stage),
            )
            .await
        {
            Ok(pending) => pending,
            Err(error) => return Err(error).into(),
        };

        let pending = pending.with_lifecycle_decider(stage, lifecycle_decider);
        let token = pending.token;
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        entry.set_top_level_navigation_dispatch(top_level_navigation_dispatch);

        if navigation_reply_policy.returns_with_pending_navigation() {
            self.restore_live_page_entry(token, entry);
            return self.finish_pending_page_creation(pending).await;
        }

        if matches!(reply_boundary, crate::RendererReplyBoundary::DocumentCommit) {
            self.restore_live_page_entry(token, entry);
            self.publish_pending_page_creation_and_continue(
                pending,
                RenderRuntimePageCreationContinuation::next_turn(
                    RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                        token,
                        stage,
                        follow_count: 0,
                        completion: LivePagePendingNavigationCompletion::PublishedPageCreation {
                            navigation_reply_policy,
                        },
                    },
                ),
            )
            .await
        } else {
            self.continue_live_page_pending_navigation(
                token,
                entry,
                stage,
                0,
                LivePagePendingNavigationCompletion::CompletePageCreation {
                    pending,
                    navigation_reply_policy,
                },
            )
        }
    }

    async fn finish_pending_page_creation(
        &self,
        mut pending: RendererPendingPageCreation,
    ) -> RenderRuntimeDispatchOutcome {
        if let Some((target_stage, decider)) = pending.take_lifecycle_decider() {
            return self
                .apply_lifecycle_decision(pending, target_stage, decider)
                .await;
        }

        self.finalize_pending_page_creation_reply(pending).await
    }

    async fn finalize_pending_page_creation_reply(
        &self,
        pending: RendererPendingPageCreation,
    ) -> RenderRuntimeDispatchOutcome {
        let token = pending.token;
        match self
            .finalize_pending_page_creation_on_owner_lane(pending)
            .await
        {
            Ok(attached_page) => {
                self.reply_with_attached_page_and_resume_lifecycle_if_needed(attached_page)
            }
            Err(error) => self.retire_failed_page_creation(token, error).await,
        }
    }

    fn reply_with_attached_page_and_resume_lifecycle_if_needed(
        &self,
        attached_page: RendererAttachedPage,
    ) -> RenderRuntimeDispatchOutcome {
        let token = attached_page.token;
        let lifecycle = attached_page.creation_artifacts.lifecycle_snapshot;
        let document = lifecycle.into();
        let should_resume_lifecycle = lifecycle.load.is_none() && lifecycle.terminated.is_none();
        if should_resume_lifecycle {
            RenderRuntimeDispatchOutcome::PageCreatedAndContinueNavigation {
                page: Box::new(attached_page),
                continuation: RenderRuntimePageCreationContinuation::next_turn(
                    RenderRuntimeTurn::ResumeLivePageDocumentLifecycleAfterReply {
                        token,
                        document,
                    },
                ),
            }
        } else {
            Ok(RendererOwnerReply::PageCreated(Box::new(attached_page))).into()
        }
    }

    async fn retire_failed_page_creation(
        &self,
        token: RendererPageToken,
        error: anyhow::Error,
    ) -> RenderRuntimeDispatchOutcome {
        let _ = remove_page_on_bound_owner_local_store_via_local_task(
            self.state.local_executor.clone(),
            token,
        )
        .await;
        Err(error).into()
    }

    async fn publish_pending_page_creation_and_continue(
        &self,
        pending: RendererPendingPageCreation,
        continuation: RenderRuntimePageCreationContinuation,
    ) -> RenderRuntimeDispatchOutcome {
        let token = pending.token;
        if matches!(
            continuation.turn(),
            RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                follow_count: 0,
                ..
            }
        ) {
            let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
                Ok(entry) => entry,
                Err(error) => return Err(error).into(),
            };
            entry.begin_standalone_navigation_follow();
            self.restore_live_page_entry(token, entry);
        }
        match self
            .finalize_pending_page_creation_on_owner_lane(pending)
            .await
        {
            Ok(attached_page) => RenderRuntimeDispatchOutcome::PageCreatedAndContinueNavigation {
                page: Box::new(attached_page),
                continuation,
            },
            Err(error) => self.retire_failed_page_creation(token, error).await,
        }
    }

    async fn continue_attached_page_creation_lifecycle_turn(
        &self,
        pending: RendererPendingPageCreation,
        document: RendererDocumentLifecycleIdentity,
        target_stage: PageVmInitStage,
        navigation_reply_policy: NavigationReplyPolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let token = pending.token;
        let resolution = self
            .resolve_pending_page_creation_on_owner_lane(
                pending,
                document,
                target_stage,
                navigation_reply_policy,
            )
            .await;
        let resolution = match resolution {
            Ok(resolution) => resolution,
            Err(error) => return self.retire_failed_page_creation(token, error).await,
        };
        let (resolution, retire_page_after_publication) =
            resolution.publish_then_resolve(|output| self.publish_renderer_output(output));
        match resolution {
            PageCreationResolution::Finalized {
                attached,
                resume_parked_page_turn,
            } => {
                debug_assert!(!retire_page_after_publication);
                if resume_parked_page_turn {
                    self.signal_internal_page_turn_source(
                        token,
                        RendererOwnerWakeSource::SchedulerContinuation,
                    );
                }
                self.reply_with_attached_page_and_resume_lifecycle_if_needed(attached)
            }
            PageCreationResolution::Waiting { pending, document } => {
                debug_assert!(!retire_page_after_publication);
                RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                    turn: Box::new(RenderRuntimeTurn::ContinueAttachedPageCreationLifecycle {
                        pending,
                        document,
                        target_stage,
                        navigation_reply_policy,
                    }),
                    wake_token: token,
                }
            }
            PageCreationResolution::LifecycleDecisionRequired { pending } => {
                debug_assert!(!retire_page_after_publication);
                self.finish_pending_page_creation(pending).await
            }
            PageCreationResolution::Retired { failure } => {
                debug_assert!(retire_page_after_publication);
                let _ = remove_page_on_bound_owner_local_store_via_local_task(
                    self.state.local_executor.clone(),
                    token,
                )
                .await;
                Err(failure.into_error()).into()
            }
            PageCreationResolution::EntryUnavailable { error } => {
                debug_assert!(!retire_page_after_publication);
                Err(error).into()
            }
        }
    }

    async fn continue_live_page_navigation_post_parse_lifecycle_turn(
        &self,
        token: RendererPageToken,
        document: RendererDocumentLifecycleIdentity,
        target_stage: PageVmInitStage,
        follow_count: usize,
        completion: LivePagePendingNavigationCompletion,
    ) -> RenderRuntimeDispatchOutcome {
        let retire_page_on_failure = completion.retires_page_on_navigation_failure();
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let observation = observe_document_lifecycle_on_entry(&mut entry, document, target_stage);
        match observation {
            DocumentLifecycleObserverOutcome::NavigationPending
                if completion.returns_with_pending_location_navigation() =>
            {
                self.finish_live_page_navigation_completion(token, entry, completion)
                    .await
            }
            DocumentLifecycleObserverOutcome::Reached => {
                self.finish_live_page_navigation_completion(token, entry, completion)
                    .await
            }
            DocumentLifecycleObserverOutcome::NavigationPending => self
                .continue_live_page_pending_navigation(
                    token,
                    entry,
                    target_stage,
                    follow_count + 1,
                    completion,
                ),
            DocumentLifecycleObserverOutcome::Pending => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                    turn: Box::new(
                        RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                            token,
                            document,
                            target_stage,
                            follow_count,
                            completion,
                        },
                    ),
                    wake_token: token,
                }
            }
            DocumentLifecycleObserverOutcome::DocumentReplaced { document } => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                    turn: Box::new(
                        RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                            token,
                            document,
                            target_stage,
                            follow_count,
                            completion,
                        },
                    ),
                    wake_token: token,
                }
            }
            DocumentLifecycleObserverOutcome::Interrupted(termination) => {
                let error = anyhow!(
                    "renderer document lifecycle was interrupted before {target_stage:?}: {:?}",
                    termination.reason
                );
                self.finish_live_page_navigation_failure(
                    token,
                    entry,
                    retire_page_on_failure,
                    LivePageNavigationFailureDisposition::ReturnToInitiator(error),
                )
                .await
            }
            DocumentLifecycleObserverOutcome::MissingResident => {
                let error = anyhow!(
                    "renderer document lifecycle resident disappeared while exact Document {document:?} was still pending before {target_stage:?}"
                );
                self.finish_live_page_navigation_failure(
                    token,
                    entry,
                    retire_page_on_failure,
                    LivePageNavigationFailureDisposition::ReturnToInitiator(error),
                )
                .await
            }
        }
    }

    pub(crate) fn new(
        local_executor: JsLocalExecutor,
        next_page_id: Arc<AtomicU64>,
        browser_context_runtime: RendererBrowserContextRuntime,
    ) -> (Self, RenderRuntimeOwner) {
        let (page_wake_tx, page_wake_rx) = mpsc::unbounded_channel();
        let (inspector_io_wake_tx, inspector_io_wake_rx) = mpsc::unbounded_channel();
        let (shared_worker_wake_tx, shared_worker_wake_rx) = shared_worker_owner_wake_channel();
        browser_context_runtime.add_shared_worker_owner_wake_sender(shared_worker_wake_tx);
        let (service_worker_wake_tx, service_worker_wake_rx) = service_worker_owner_wake_channel();
        browser_context_runtime.add_service_worker_owner_wake_sender(service_worker_wake_tx);
        let owner_local_host_id = RendererOwnerLocalHostId::new(
            NEXT_RENDERER_OWNER_LOCAL_HOST_ID.fetch_add(1, Ordering::Relaxed),
        );
        browser_context_runtime.set_shared_worker_owner_local_host_id(owner_local_host_id);
        let state = Arc::new(RendererOwnerState {
            page_table: RendererPageTable::default(),
            local_executor,
            next_page_id,
            page_wake_tx,
            render_runtime_admission: std::sync::OnceLock::new(),
            inspector_io_wake_tx,
            browser_context_runtime,
            owner_local_host_id,
            layout_policy: Mutex::new(RendererOwnerLayoutPolicyState::default()),
            context_shutdown_notify: tokio::sync::Notify::new(),
            #[cfg(test)]
            command_dispatch_gate: Mutex::new(None),
            #[cfg(debug_assertions)]
            owner_local_thread_id: Mutex::new(None),
        });
        let provisional = Self {
            state,
            render_runtime: RenderRuntimeHandle::disconnected(),
        };
        let render_runtime_owner = RenderRuntimeOwner::spawn(
            provisional.clone(),
            page_wake_rx,
            inspector_io_wake_rx,
            shared_worker_wake_rx,
            service_worker_wake_rx,
        );
        let render_runtime = render_runtime_owner.handle();
        provisional
            .state
            .render_runtime_admission
            .set(render_runtime.clone())
            .expect("render runtime admission must be initialized exactly once");
        (
            Self {
                render_runtime,
                ..provisional
            },
            render_runtime_owner,
        )
    }

    #[cfg(debug_assertions)]
    pub(super) fn bind_or_check_local_runtime_thread(&self) -> Result<ThreadId> {
        let current_thread_id = std::thread::current().id();
        let mut owner_local_thread_id = self.state.owner_local_thread_id.lock();
        match *owner_local_thread_id {
            Some(thread_id) => {
                ensure!(
                    thread_id == current_thread_id,
                    "renderer owner local runtime was entered on a different thread"
                );
                Ok(thread_id)
            }
            None => {
                *owner_local_thread_id = Some(current_thread_id);
                Ok(current_thread_id)
            }
        }
    }

    pub(super) fn owner_local_context(&self) -> Result<RendererOwnerLocalContext> {
        let render_runtime = self
            .state
            .render_runtime_admission
            .get()
            .cloned()
            .ok_or_else(|| anyhow!("render runtime admission is not initialized"))?;
        Ok(RendererOwnerLocalContext {
            owner_state: self.state.clone(),
            render_runtime,
            local_host_id: self.state.owner_local_host_id,
            #[cfg(debug_assertions)]
            local_thread_id: self.bind_or_check_local_runtime_thread()?,
        })
    }

    pub fn refresh_page_view_for_testing(&self, view: RendererPageView) -> Result<()> {
        self.state.page_table.refresh(
            view.page_id,
            view.vm_creation_id,
            view.view_generation,
            view.page_state.requested_url.clone(),
            view.page_state.final_url.clone(),
            view.page_state.document_title.clone(),
            view.page_state.status,
        )
    }

    pub fn remove_page_for_testing(&self, page_id: PageId) {
        self.state.page_table.remove(page_id);
    }

    pub fn allocate_page_id(&self) -> PageId {
        PageId::new(self.state.next_page_id.fetch_add(1, Ordering::Relaxed))
    }

    pub(crate) fn allocate_page_reservation_token(&self) -> RendererPageReservationToken {
        RendererPageReservationToken::new(self.state.owner_local_host_id, self.allocate_page_id())
    }

    pub fn set_renderer_output_transport_sender(
        &self,
        sender: super::RendererOutputTransportSender,
    ) {
        self.state
            .browser_context_runtime
            .set_renderer_output_transport_sender(sender);
    }

    fn publish_renderer_output(&self, output: RendererOutputPublication) {
        if let Some(sender) = self
            .state
            .browser_context_runtime
            .renderer_output_transport_sender()
        {
            let _ = output.publish_to(&sender);
        }
    }

    /// Completes the protocol-side owner reservation after renderer bootstrap
    /// has either opened its concrete stream or failed before doing so.
    ///
    /// The journal and this marker share one FIFO transport. Protocol therefore
    /// observes `Opened` before this release on success, while an early failure
    /// produces only the release. Never move this to the navigation completion
    /// channel: that independent channel cannot order against stream opening.
    fn release_page_output_reservation(&self, reservation: RendererPageReservationToken) {
        if let Some(sender) = self
            .state
            .browser_context_runtime
            .renderer_output_transport_sender()
        {
            let _ = sender.send(RendererOutputTransportMessage::page_reservation_released(
                reservation.local_host_id(),
                reservation.page_id(),
            ));
        }
    }

    fn signal_pending_phase_one_admission(
        &self,
        wake_token: RendererPageToken,
        admission: PhaseOneResidenceAdmission,
    ) {
        match admission {
            // The parser already froze and published the exact Fetch pause
            // record while restoring this Page residence. Protocol continuation,
            // not a second source-shaped wake, admits the parser afterwards.
            PhaseOneResidenceAdmission::ParserBlockingSourceLoad => {}
            PhaseOneResidenceAdmission::ReadyPageTurn => self.signal_internal_page_turn_source(
                wake_token,
                RendererOwnerWakeSource::SchedulerContinuation,
            ),
            PhaseOneResidenceAdmission::WaitingForProducer => {}
        }
    }

    fn signal_internal_document_lifecycle_turn(&self, token: RendererPageToken) {
        self.signal_internal_page_turn_source(
            token,
            RendererOwnerWakeSource::Runtime(
                RendererOwnerRuntimeActivitySource::DocumentLifecycleTurn,
            ),
        );
    }

    fn post_response_document_lifecycle_continuation(
        &self,
        token: RendererPageToken,
        document: RendererDocumentLifecycleIdentity,
    ) -> RendererPageCommandPostResponseContinuation {
        let page_wake_tx = self.state.page_wake_tx.clone();
        RendererPageCommandPostResponseContinuation::new(move || {
            let _ = page_wake_tx.send(RendererOwnerWake::post_response_document_lifecycle(
                token, document,
            ));
        })
    }

    fn signal_internal_document_lifecycle_turn_if_resident(
        &self,
        token: RendererPageToken,
        document: RendererDocumentLifecycleIdentity,
    ) {
        let Ok(mut entry) = take_entry_for_command_on_bound_owner_local_store(token) else {
            return;
        };
        let should_signal = entry.pending_document_lifecycle_identity() == Some(document);
        self.restore_live_page_entry(token, entry);
        if should_signal {
            self.signal_internal_document_lifecycle_turn(token);
        }
    }

    fn signal_internal_page_turn_source(
        &self,
        token: RendererPageToken,
        source: RendererOwnerWakeSource,
    ) {
        let _ = self
            .state
            .page_wake_tx
            .send(RendererOwnerWake::page(token, source));
    }

    fn signal_replacement_document_view_settled(
        &self,
        token: RendererPageToken,
        vm_creation_id: u64,
    ) {
        let _ = self
            .state
            .page_wake_tx
            .send(RendererOwnerWake::replacement_document_view_settled(
                token,
                vm_creation_id,
            ));
    }

    fn owner_wake_sender_for_page(
        &self,
        owner_local_context: &RendererOwnerLocalContext,
        page_id: PageId,
    ) -> RendererOwnerWakeSender {
        RendererOwnerWakeSender::new(
            self.state.page_wake_tx.clone(),
            renderer_page_token_for_owner_context(owner_local_context, page_id),
        )
    }

    pub fn build_create_html_page_request(
        &self,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        loader: &ResourceRequestClient,
        web_storage: crate::RendererWebStorageHandles,
        final_url: Url,
        html: String,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: Vec<(String, String)>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
        stage: PageVmInitStage,
    ) -> RendererCreateHtmlPageRequest {
        self.build_create_html_page_request_with_env(
            self.allocate_page_reservation_token(),
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            loader,
            web_storage,
            final_url,
            html,
            document_start_scripts,
            runtime_bindings,
            runtime_inspector_session_restore_snapshots,
            extra_http_headers,
            None,
            None,
            false,
            false,
            1.0,
            crate::protocol_types::EmulatedMediaOverrides::default(),
            None,
            network_offline,
            blocked_url_patterns,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            stage,
        )
    }

    pub fn build_create_html_page_request_with_env(
        &self,
        page_reservation: RendererPageReservationToken,
        requested_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        loader: &ResourceRequestClient,
        web_storage: crate::RendererWebStorageHandles,
        final_url: Url,
        html: String,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        bypass_content_security_policy: bool,
        cpu_throttling_rate: f64,
        emulated_media: crate::protocol_types::EmulatedMediaOverrides,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
        stage: PageVmInitStage,
    ) -> RendererCreateHtmlPageRequest {
        RendererCreateHtmlPageRequest {
            page_reservation,
            root_frame_id: None,
            main_document_commit: None,
            top_level_storage_key: None,
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            loader: loader.clone(),
            web_storage,
            final_url,
            html,
            document_start_scripts,
            runtime_bindings,
            runtime_inspector_session_restore_snapshots,
            runtime_isolated_worlds: Vec::new(),
            permission_overrides: Vec::new(),
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            bypass_content_security_policy,
            cpu_throttling_rate,
            network_offline,
            blocked_url_patterns,
            indexed_db_manager: None,
            storage_bucket_store: None,
            emulated_media,
            idle_override: None,
            viewport_surface,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            layout_policy: self.seal_layout_policy_for_page_creation(),
            wpt_extensions_enabled: false,
            stage,
            reply_boundary: crate::RendererReplyBoundary::Stage,
            lifecycle_decider: None,
            top_level_navigation_dispatch:
                RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            reserved_service_worker_client: None,
        }
    }

    pub fn build_create_streaming_raw_page_request(
        &self,
        requested_url: Url,
        final_url: Url,
        navigation_initiator_url: Option<Url>,
        navigation_redirected: bool,
        navigation_redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        loader: &ResourceRequestClient,
        web_storage: crate::RendererWebStorageHandles,
        raw_body: ExternalRawDocumentBodyStream,
        document_start_scripts: Vec<DocumentStartScript>,
        runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        extra_http_headers: Vec<(String, String)>,
        locale_override: Option<String>,
        timezone_override: Option<String>,
        script_execution_disabled: bool,
        bypass_content_security_policy: bool,
        cpu_throttling_rate: f64,
        emulated_media: crate::protocol_types::EmulatedMediaOverrides,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
        fetch_subresource_interception_enabled: bool,
        fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
        stage: PageVmInitStage,
    ) -> RendererCreateStreamingRawPageRequest {
        RendererCreateStreamingRawPageRequest {
            root_frame_id: None,
            main_document_commit: None,
            requested_url,
            final_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            loader: loader.clone(),
            web_storage,
            raw_body,
            document_start_scripts,
            runtime_bindings,
            runtime_inspector_session_restore_snapshots,
            runtime_isolated_worlds: Vec::new(),
            permission_overrides: Vec::new(),
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            bypass_content_security_policy,
            cpu_throttling_rate,
            emulated_media,
            idle_override: None,
            viewport_surface,
            network_offline,
            blocked_url_patterns,
            indexed_db_manager: None,
            storage_bucket_store: None,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            layout_policy: self.seal_layout_policy_for_page_creation(),
            wpt_extensions_enabled: false,
            stage,
            reply_boundary: crate::RendererReplyBoundary::Stage,
            lifecycle_decider: None,
            top_level_navigation_dispatch:
                RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            navigation_reply_policy: NavigationReplyPolicy::FollowBeforeReply,
            reserved_service_worker_client: None,
        }
    }

    pub fn refresh_page_view_on_slot_for_testing(
        &self,
        slot: &RendererPageSlotHandle,
        view: RendererPageView,
    ) -> Result<()> {
        ensure!(
            self.state.page_table.owns_slot(slot),
            "renderer owner does not own slot for page {}",
            slot.page_id().as_u64()
        );
        slot.refresh_owned_view(view)
    }

    pub async fn dispatch_command(
        &self,
        command: RendererOwnerCommand,
    ) -> Result<RendererOwnerReply> {
        if moli_trace::cdp_nav_timing_enabled()
            && let Some(command_label) = owner_command_timing_label(&command)
        {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                command = command_label,
                stage = "owner_command_enqueue",
            );
        }
        let reply_rx = self.enqueue_command_with_reply(command)?;
        reply_rx
            .await
            .map_err(|_| anyhow!("render runtime reply channel closed"))?
    }

    /// Enqueue a renderer command without waiting for the reply, returning the
    /// reply channel so the caller can `await` it later (or hand it off to a
    /// different code path). This enables fire-then-defer patterns where the
    /// renderer thread can process the work in parallel with conn-side
    /// bookkeeping.
    pub fn enqueue_command_with_reply(
        &self,
        command: RendererOwnerCommand,
    ) -> Result<oneshot::Receiver<Result<RendererOwnerReply>>> {
        if let Err(error) = self.ensure_context_owner_accepts_commands() {
            self.release_rejected_command_output_reservation(&command);
            return Err(error);
        }
        if moli_trace::cdp_nav_timing_enabled()
            && let Some(command_label) = owner_command_timing_label(&command)
        {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                command = command_label,
                stage = "owner_command_enqueue_deferred",
            );
        }
        match self.render_runtime.enqueue_owned(command) {
            Ok(reply_rx) => Ok(reply_rx),
            Err(error) => {
                let (command, error) = error.into_parts();
                self.release_rejected_command_output_reservation(&command);
                Err(error)
            }
        }
    }

    fn context_shutdown_started(&self) -> bool {
        self.state.page_table.is_terminal()
    }

    fn ensure_context_owner_accepts_commands(&self) -> Result<()> {
        ensure!(
            !self.context_shutdown_started(),
            "renderer browser context owner has been dropped"
        );
        Ok(())
    }

    pub(crate) fn terminate_for_context_owner_shutdown(&self) {
        // Closing command admission and draining the queue use the same
        // RenderRuntimeState lock. Every command therefore belongs to exactly
        // one side of this boundary: rejected with ownership returned to the
        // caller, or durably enqueued for the renderer terminal drain.
        self.render_runtime.close_admission();
        self.state
            .page_table
            .terminate_and_cancel_all_contexts(RendererPageContextCancelReason::ContextDropped);
        self.state.context_shutdown_notify.notify_one();
    }

    #[cfg(test)]
    pub(crate) fn install_command_dispatch_gate_for_testing(
        &self,
    ) -> (
        crossbeam_channel::Receiver<()>,
        crossbeam_channel::Sender<()>,
    ) {
        let (entered_tx, entered_rx) = crossbeam_channel::bounded(1);
        let (release_tx, release_rx) = crossbeam_channel::bounded(1);
        let previous = self.state.command_dispatch_gate.lock().replace(
            RendererCommandDispatchGateForTesting {
                entered_tx,
                release_rx,
            },
        );
        assert!(
            previous.is_none(),
            "renderer command test gate already installed"
        );
        (entered_rx, release_tx)
    }

    #[cfg(test)]
    pub(crate) fn close_command_admission_for_testing(&self) {
        self.render_runtime.close_admission();
    }

    #[cfg(test)]
    fn wait_on_command_dispatch_gate_for_testing(&self) {
        let gate = self.state.command_dispatch_gate.lock().take();
        let Some(gate) = gate else {
            return;
        };
        let _ = gate.entered_tx.send(());
        let _ = gate.release_rx.recv();
    }

    fn release_rejected_command_output_reservation(&self, command: &RendererOwnerCommand) {
        let reservation = match command {
            RendererOwnerCommand::CreateHtmlPage(request) => Some(request.page_reservation),
            RendererOwnerCommand::PrepareStreamingRawDocument { token, .. } => Some(*token),
            _ => None,
        };
        if let Some(reservation) = reservation {
            self.release_page_output_reservation(reservation);
        }
    }

    async fn dispatch_command_inline_on_owner_local_store(
        &self,
        command: RendererOwnerCommand,
        owner_local_store: &mut RendererOwnerLocalStore,
    ) -> RenderRuntimeDispatchOutcome {
        match command {
            RendererOwnerCommand::CreateHtmlPage(request) => {
                let reservation = request.page_reservation;
                let outcome = self
                    .create_page_reply_from_html_request_on_owner_local_store(
                        request,
                        owner_local_store,
                    )
                    .await;
                self.release_page_output_reservation(reservation);
                outcome
            }
            RendererOwnerCommand::PrepareStreamingRawDocument { token, request } => {
                let outcome = self
                    .prepare_renderer_document_on_owner_local_store(
                        token,
                        request,
                        owner_local_store,
                    )
                    .await;
                self.release_page_output_reservation(token);
                outcome
            }
            RendererOwnerCommand::UpdatePreparedRendererDocumentCommitConfiguration {
                token,
                configuration,
            } => {
                if token.local_host_id() != self.state.owner_local_host_id {
                    return Err(anyhow!(
                        "prepared document configuration update belongs to renderer owner {}, not {}",
                        token.local_host_id().as_u64(),
                        self.state.owner_local_host_id.as_u64()
                    ))
                    .into();
                }
                match owner_local_store
                    .update_prepared_document_commit_configuration(token, configuration)
                    .map(|()| {
                        RendererOwnerReply::PreparedRendererDocumentCommitConfigurationUpdated
                    }) {
                    Ok(reply) => Ok(reply).into(),
                    Err(error) => Err(error).into(),
                }
            }
            RendererOwnerCommand::CommitPreparedRendererDocument { permit } => {
                let token = permit.prepared_document();
                if token.local_host_id() != self.state.owner_local_host_id {
                    return Err(anyhow!(
                        "prepared document commit permit belongs to renderer owner {}, not {}",
                        token.local_host_id().as_u64(),
                        self.state.owner_local_host_id.as_u64()
                    ))
                    .into();
                }
                match owner_local_store.take_prepared_document(token) {
                    Ok(residence) => {
                        self.create_page_reply_from_prepared_document_on_owner_local_store(
                            token.page_id(),
                            residence,
                            owner_local_store,
                        )
                        .await
                    }
                    Err(error) => Err(error).into(),
                }
            }
            RendererOwnerCommand::CancelPreparedRendererDocument { token } => {
                if token.local_host_id() != self.state.owner_local_host_id {
                    return Err(anyhow!(
                        "prepared document cancellation belongs to renderer owner {}, not {}",
                        token.local_host_id().as_u64(),
                        self.state.owner_local_host_id.as_u64()
                    ))
                    .into();
                }
                owner_local_store.cancel_prepared_document(token);
                Ok(RendererOwnerReply::PreparedRendererDocumentCanceled).into()
            }
            command @ (RendererOwnerCommand::RunAsyncPageCommand { .. }
            | RendererOwnerCommand::RunProtocolPageCommand { .. }) => {
                let (token, command, capture_policy) = match command {
                    RendererOwnerCommand::RunAsyncPageCommand { token, command } => (
                        token,
                        command,
                        super::RendererPageStateCapturePolicy::FullReport,
                    ),
                    RendererOwnerCommand::RunProtocolPageCommand { token, command } => (
                        token,
                        command,
                        super::RendererPageStateCapturePolicy::ProtocolTurn,
                    ),
                    _ => unreachable!("combined renderer page command pattern must match"),
                };
                if moli_trace::cdp_nav_timing_enabled()
                    && let Some(command_label) = renderer_page_command_timing_label(&command)
                {
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        command = command_label,
                        page_id = token.page_id.as_u64(),
                        stage = "owner_command_received",
                    );
                }
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::RunLivePageCommand {
                        token,
                        command,
                        capture_policy,
                    },
                ))
            }
            RendererOwnerCommand::FinalizeRuntimeInspectorSessionDetach {
                token,
                inspector_session_id,
                pause_guard: _pause_guard,
            } => {
                let mut entry = match checkout_entry_for_owner_turn_on_bound_owner_local_store(
                    token,
                ) {
                    Ok(entry) => entry,
                    Err(
                        LivePageEntryCheckoutError::Retired | LivePageEntryCheckoutError::Missing,
                    ) => {
                        return Ok(RendererOwnerReply::RuntimeInspectorSessionDetachFinalized(
                            false,
                        ))
                        .into();
                    }
                    Err(LivePageEntryCheckoutError::Busy) => {
                        return Err(anyhow!(
                            "renderer page {} remained checked out while finalizing Inspector session detach",
                            token.page_id.as_u64()
                        ))
                        .into();
                    }
                };
                let detached = entry
                    .page_vm_mut()
                    .detach_runtime_inspector_session(inspector_session_id.as_deref());
                restore_entry_after_command_on_bound_owner_local_store(token, entry);
                Ok(RendererOwnerReply::RuntimeInspectorSessionDetachFinalized(
                    detached,
                ))
                .into()
            }
            RendererOwnerCommand::WaitForNetworkIdle {
                token,
                timeout_ms,
                loader,
            } => {
                let deadline = match checked_live_page_wait_deadline(timeout_ms, "networkidle") {
                    Ok(deadline) => deadline,
                    Err(error) => return Err(error).into(),
                };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageNetworkIdle {
                        token,
                        state: PageVmNetworkIdleWaitState::default(),
                        deadline,
                        loader,
                    },
                ))
            }
            RendererOwnerCommand::WaitForDomStable {
                token,
                timeout_ms,
                loader,
            } => {
                let deadline = match checked_live_page_wait_deadline(timeout_ms, "domstable") {
                    Ok(deadline) => deadline,
                    Err(error) => return Err(error).into(),
                };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageDomStable {
                        token,
                        state: PageVmDomStableWaitState::default(),
                        deadline,
                        loader,
                    },
                ))
            }
            RendererOwnerCommand::RemovePage { token } => {
                remove_page_on_bound_owner_local_store_via_local_task(
                    self.state.local_executor.clone(),
                    token,
                )
                .await
                .map(|()| RendererOwnerReply::PageRemoved)
                .into()
            }
            RendererOwnerCommand::TestingCurrentPageState { token } => {
                owner_local_store_session(owner_local_store)
                    .current_page_state_for_testing(token)
                    .map(RendererOwnerReply::TestingCurrentPageState)
                    .into()
            }
            RendererOwnerCommand::TestingRendererPageView { token } => {
                owner_local_store_session(owner_local_store)
                    .renderer_page_view_for_testing(token)
                    .map(RendererOwnerReply::TestingRendererPageView)
                    .into()
            }
            RendererOwnerCommand::TestingOwnerSlot { token } => {
                owner_local_store_session(owner_local_store)
                    .owner_slot_for_testing(token)
                    .map(RendererOwnerReply::TestingOwnerSlot)
                    .into()
            }
            RendererOwnerCommand::TestingHostInstanceKey { token } => {
                owner_local_store_session(owner_local_store)
                    .host_instance_key_for_testing(token)
                    .map(RendererOwnerReply::TestingHostInstanceKey)
                    .into()
            }
            RendererOwnerCommand::TestingHostUniqueDocumentIsolateCount { token } => {
                owner_local_store_session(owner_local_store)
                    .host_unique_document_isolate_count_for_testing(token)
                    .map(RendererOwnerReply::TestingHostUniqueDocumentIsolateCount)
                    .into()
            }
            #[cfg(test)]
            RendererOwnerCommand::TestingDeferredPageVmDropPendingCount => {
                Ok(RendererOwnerReply::TestingDeferredPageVmDropPendingCount(
                    deferred_page_vm_drop_pending_count_for_testing(),
                ))
                .into()
            }
        }
    }

    fn enqueue_inspector_main_receiver_wake(
        wake: RendererInspectorMainOwnerWake,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        pending_turns.push_back(RenderRuntimePendingTurn {
            reply_tx: None,
            turn: RenderRuntimeTurn::RunInspectorMainReceiver { wake },
            allow_command_overtake: false,
            command_admission_output_predecessor: None,
        });
    }

    pub(crate) async fn run_render_runtime_loop(
        &self,
        mut rx: mpsc::UnboundedReceiver<RenderRuntimeEnvelope>,
        mut page_wake_rx: mpsc::UnboundedReceiver<RendererOwnerWake>,
        mut inspector_io_wake_rx: mpsc::UnboundedReceiver<RendererInspectorIoOwnerWake>,
        mut shared_worker_wake_rx: mpsc::UnboundedReceiver<SharedWorkerRuntimeOwnerWake>,
        mut service_worker_wake_rx: mpsc::UnboundedReceiver<ServiceWorkerRuntimeOwnerWake>,
    ) {
        let loop_future = async {
            let mut owner_local_store = RendererOwnerLocalStore::default();
            let _owner_local_store_binding =
                bind_render_runtime_owner_local_store(&mut owner_local_store);
            let mut pending_turns = RenderRuntimePendingTurnQueue::default();
            let mut parked_turns: VecDeque<RenderRuntimeParkedTurn> = VecDeque::new();
            // A ready producer and an expired deadline are two admission
            // reasons for the same Page scheduler, not two priority classes.
            // Alternate which one is polled first so neither can remain
            // hidden behind a continuously ready command queue or the other
            // admission reason.
            let mut page_admission_preference = PageTurnAdmissionPreference::ProducerWake;
            loop {
                if self.context_shutdown_started() {
                    while let Some(pending_turn) = pending_turns.pop_front() {
                        self.cancel_pending_turn_on_owner_local_store(pending_turn.turn);
                        if let Some(reply_tx) = pending_turn.reply_tx {
                            let _ = reply_tx.send(Err(anyhow!(
                                "renderer browser context was dropped while work was pending"
                            )));
                        }
                    }
                    for parked_turn in parked_turns.drain(..) {
                        self.cancel_pending_turn_on_owner_local_store(parked_turn.turn);
                        if let Some(reply_tx) = parked_turn.reply_tx {
                            let _ = reply_tx.send(Err(anyhow!(
                                "renderer browser context was dropped while work was parked"
                            )));
                        }
                    }
                    while let Ok(envelope) = rx.try_recv() {
                        if let RenderRuntimeEnvelope::Command { command, reply_tx } = envelope {
                            self.release_rejected_command_output_reservation(&command);
                            let _ = reply_tx.send(Err(anyhow!(
                                "renderer browser context was dropped before command dispatch"
                            )));
                        }
                    }
                    break;
                }
                self.enqueue_due_parked_turns(&mut parked_turns, &mut pending_turns);
                // Commands are also represented as bounded pending turns. A
                // Page wake/deadline must be admitted before selecting the
                // next pending turn, otherwise a stream of command
                // continuations can keep the Page scheduler invisible even
                // though its durable source is ready. At most one Page turn
                // is admitted here; that turn itself still lets one ready
                // command overtake at its owner boundary.
                if !pending_turns.has_page_owner_turn() {
                    match self.try_admit_ready_page_turn(
                        &mut page_admission_preference,
                        &mut page_wake_rx,
                        &mut parked_turns,
                        &mut pending_turns,
                    ) {
                        ReadyPageTurnAdmission::Admitted | ReadyPageTurnAdmission::NoneReady => {}
                        ReadyPageTurnAdmission::WakeChannelClosed => break,
                    }
                }
                // Renderer housekeeping is a separate owner lane. It is
                // admitted after Page work so an expired memory-maintenance
                // deadline cannot masquerade as, or outrank, an HTML task.
                if !pending_turns.has_owner_maintenance_turn() {
                    self.enqueue_one_due_owner_maintenance_turn(&mut pending_turns);
                }
                // IO Inspector ingress is independent of normal owner command
                // admission. Give at most one command an owner execution
                // chance at each boundary, then continue with the ordinary
                // pending turn so sustained IO cannot starve MainThread work.
                match inspector_io_wake_rx.try_recv() {
                    Ok(wake) => dispatch_inspector_io_owner_wake(wake),
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => {}
                }
                if let Some(mut pending_turn) = pending_turns.pop_front() {
                    if pending_turn.allow_command_overtake
                        && pending_turn.turn.page_turn_should_yield_to_ready_command()
                    {
                        // A protocol reply may observe page progress, but it does not
                        // own the page scheduler. Every bounded page turn returns the
                        // entry to its stable slot, so already-arrived commands can be
                        // admitted before the same continuation runs another turn.
                        match rx.try_recv() {
                            Ok(envelope) => {
                                pending_turn.allow_command_overtake = false;
                                pending_turns.push_front(pending_turn);
                                self.dispatch_envelope_on_owner_local_store(
                                    envelope,
                                    &mut owner_local_store,
                                    &mut pending_turns,
                                    &mut parked_turns,
                                )
                                .await;
                                continue;
                            }
                            Err(mpsc::error::TryRecvError::Empty) => {}
                            Err(mpsc::error::TryRecvError::Disconnected) => {}
                        }
                    }
                    if pending_turn
                        .reply_tx
                        .as_ref()
                        .is_some_and(|tx| tx.is_closed())
                    {
                        self.detach_navigation_or_cancel_pending_turn(
                            pending_turn.turn,
                            &mut pending_turns,
                        );
                        continue;
                    }
                    if let Some(token) = pending_turn.turn.committed_page_view_command_token()
                        && parked_turns.iter().any(|turn| {
                            turn.wake_token == token
                                && matches!(
                                    turn.condition,
                                    RenderRuntimeParkCondition::CommittedDocumentParserContinuation {
                                        ..
                                    }
                                )
                        })
                    {
                        // A command observed the committed Page through the
                        // browser-side response path, but its parser handoff
                        // has not run yet. Park the command on the same durable
                        // Page activity edge. The response-release wake puts
                        // the parser turn and continuation in front of it.
                        parked_turns.push_back(RenderRuntimeParkedTurn {
                            reply_tx: pending_turn.reply_tx,
                            turn: pending_turn.turn,
                            wake_token: token,
                            ready_at: None,
                            condition: RenderRuntimeParkCondition::PageActivity,
                            command_admission_output_predecessor: pending_turn
                                .command_admission_output_predecessor
                                .take(),
                        });
                        continue;
                    }
                    if let Some(token) = pending_turn.turn.committed_page_view_command_token()
                        && let Some(expected_vm_creation_id) =
                            owner_local_store.page_uncommitted_vm_creation_id(token)
                        && pending_turn
                            .turn
                            .committed_page_view_deadline()
                            .is_none_or(|deadline| deadline > Instant::now())
                    {
                        tracing::trace!(
                            page_id = token.page_id.as_u64(),
                            expected_vm_creation_id,
                            "parked page command until the replacement PageVm commits"
                        );
                        let ready_at = pending_turn.turn.committed_page_view_deadline();
                        parked_turns.push_back(RenderRuntimeParkedTurn {
                            reply_tx: pending_turn.reply_tx,
                            turn: pending_turn.turn,
                            wake_token: token,
                            ready_at,
                            condition:
                                RenderRuntimeParkCondition::ReplacementDocumentViewSettlement {
                                    expected_vm_creation_id,
                                },
                            command_admission_output_predecessor: pending_turn
                                .command_admission_output_predecessor
                                .take(),
                        });
                        continue;
                    }
                    let completed_page_turn_token = pending_turn.page_owner_token();
                    let outcome = self
                        .run_pending_turn_on_owner_local_store(pending_turn.turn)
                        .await;
                    match outcome {
                        RenderRuntimeDispatchOutcome::Reply(result) => {
                            let result = merge_command_admission_output_predecessor(
                                *result,
                                pending_turn.command_admission_output_predecessor.take(),
                            );
                            if let Some(reply_tx) = pending_turn.reply_tx {
                                let _ = reply_tx.send(result);
                            } else if let Err(error) = result {
                                tracing::debug!(
                                    "background pending turn finished with error: {error}"
                                );
                            }
                        }
                        RenderRuntimeDispatchOutcome::InspectorMainCommandClaimed {
                            reply_tx,
                            turn,
                            command_admission_output_predecessor,
                        } => {
                            debug_assert!(pending_turn.reply_tx.is_none());
                            pending_turns.push_front(RenderRuntimePendingTurn {
                                reply_tx: Some(reply_tx),
                                turn: *turn,
                                allow_command_overtake: false,
                                command_admission_output_predecessor,
                            });
                        }
                        RenderRuntimeDispatchOutcome::PageCreatedAndContinueNavigation {
                            mut page,
                            continuation,
                        } => {
                            match pending_turn.reply_tx {
                                Some(reply_tx) => {
                                    let token = page.token;
                                    if continuation.requires_committed_document_response_release() {
                                        page.defer_committed_document_parser_until_response(
                                            self.state.page_wake_tx.clone(),
                                        );
                                    }
                                    if reply_tx
                                        .send(Ok(RendererOwnerReply::PageCreated(page)))
                                        .is_ok()
                                    {
                                        self.enqueue_page_creation_continuation(
                                            continuation,
                                            &mut pending_turns,
                                            &mut parked_turns,
                                        );
                                    } else {
                                        // The attached Page never crossed its
                                        // ownership handoff, so there is no
                                        // browser-side handle that can retire
                                        // it later. This differs from a live
                                        // navigation command observer going
                                        // away after Page publication: that
                                        // continuation is detached below and
                                        // keeps running in the background.
                                        remove_page_on_bound_owner_local_store(token);
                                    }
                                }
                                None => {
                                    // A Page already published at an earlier
                                    // reply boundary owns this background
                                    // navigation. Its replacement commit may
                                    // produce another attached-page boundary,
                                    // but there is intentionally no command
                                    // observer to notify. Keep the renderer
                                    // continuation alive instead of treating
                                    // the absent observer as a cancelled
                                    // initial Page creation.
                                    drop(page);
                                    pending_turns.push_back(RenderRuntimePendingTurn {
                                        reply_tx: None,
                                        turn: continuation.into_turn(),
                                        allow_command_overtake: true,
                                        command_admission_output_predecessor: None,
                                    });
                                }
                            }
                        }
                        RenderRuntimeDispatchOutcome::BackgroundComplete(result) => {
                            if let Some(reply_tx) = pending_turn.reply_tx {
                                let reply = match result {
                                    Ok(()) => Err(anyhow!(
                                        "command-owned turn unexpectedly completed as background work"
                                    )),
                                    Err(error) => Err(anyhow!(
                                        "command-owned turn unexpectedly completed as background work: {error}"
                                    )),
                                };
                                let _ = reply_tx.send(reply);
                            } else if let Err(error) = result {
                                tracing::debug!(
                                    "background pending turn finished with error: {error}"
                                );
                            }
                        }
                        RenderRuntimeDispatchOutcome::PageCreationNavigationFailurePublished {
                            token,
                            failure,
                        } => {
                            if let Some(reply_tx) = pending_turn.reply_tx {
                                let _ = reply_tx.send(Err(anyhow!(failure.to_string())));
                            } else {
                                tracing::debug!(
                                    "background navigation published a concrete failure: {failure}"
                                );
                            }
                            self.enqueue_parked_turns_for_wake(
                                token,
                                &mut parked_turns,
                                &mut pending_turns,
                            );
                        }
                        RenderRuntimeDispatchOutcome::ContinueNextTurn(turn) => {
                            if pending_turn
                                .reply_tx
                                .as_ref()
                                .is_some_and(|tx| tx.is_closed())
                            {
                                self.detach_navigation_or_cancel_pending_turn(
                                    *turn,
                                    &mut pending_turns,
                                );
                            } else {
                                pending_turns.push_back(RenderRuntimePendingTurn {
                                    reply_tx: pending_turn.reply_tx,
                                    turn: *turn,
                                    allow_command_overtake: true,
                                    command_admission_output_predecessor: pending_turn
                                        .command_admission_output_predecessor
                                        .take(),
                                });
                            }
                        }
                        RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                            turn,
                            wake_token,
                            ready_at,
                        } => {
                            self.park_or_cancel_turn(
                                pending_turn.reply_tx,
                                *turn,
                                wake_token,
                                Some(ready_at),
                                RenderRuntimeParkCondition::PageActivity,
                                pending_turn.command_admission_output_predecessor.take(),
                                &mut parked_turns,
                                &mut pending_turns,
                            );
                        }
                        RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                            turn,
                            wake_token,
                        } => {
                            self.park_or_cancel_turn(
                                pending_turn.reply_tx,
                                *turn,
                                wake_token,
                                None,
                                RenderRuntimeParkCondition::PageActivity,
                                pending_turn.command_admission_output_predecessor.take(),
                                &mut parked_turns,
                                &mut pending_turns,
                            );
                        }
                        RenderRuntimeDispatchOutcome::ContinueCommittedDocumentParserAfterPageWake {
                            turn,
                            wake_token,
                        } => {
                            self.park_or_cancel_turn(
                                pending_turn.reply_tx,
                                *turn,
                                wake_token,
                                None,
                                RenderRuntimeParkCondition::CommittedDocumentParserContinuation {
                                    parser_unblocked: true,
                                },
                                pending_turn.command_admission_output_predecessor.take(),
                                &mut parked_turns,
                                &mut pending_turns,
                            );
                        }
                    }
                    if let Some(token) = completed_page_turn_token {
                        // A bounded Page turn may have published the exact
                        // lifecycle milestone page creation is waiting for.
                        // Re-admit only that one-shot observer at this owner
                        // boundary, before a producer wake can advance the
                        // Page through a follow-up turn. Other waits retain
                        // their existing wake/deadline contracts.
                        self.enqueue_parked_page_creation_observers_after_page_turn(
                            token,
                            &mut parked_turns,
                            &mut pending_turns,
                        );
                    }
                    continue;
                }

                // SharedWorker service-lane completions can make a page-visible
                // command result ready. Do not let sustained CDP polling starve
                // this owner-level wake behind the command queue.
                match shared_worker_wake_rx.try_recv() {
                    Ok(wake) => {
                        self.handle_shared_worker_runtime_wake(wake, &mut pending_turns);
                        continue;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }

                match service_worker_wake_rx.try_recv() {
                    Ok(wake) => {
                        self.handle_service_worker_runtime_wake(wake, &mut pending_turns);
                        continue;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }

                match rx.try_recv() {
                    Ok(envelope) => {
                        self.dispatch_envelope_on_owner_local_store(
                            envelope,
                            &mut owner_local_store,
                            &mut pending_turns,
                            &mut parked_turns,
                        )
                        .await;
                        continue;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }

                let next_owner_deadline = earliest_deadline(
                    earliest_deadline(
                        self.compute_next_page_task_deadline(),
                        self.compute_next_owner_maintenance_deadline(),
                    ),
                    self.next_parked_turn_deadline(&parked_turns),
                );
                tokio::select! {
                    _ = self.state.context_shutdown_notify.notified() => {}
                    envelope_opt = rx.recv() => {
                        let Some(envelope) = envelope_opt else {
                            break;
                        };
                        self.dispatch_envelope_on_owner_local_store(
                            envelope,
                            &mut owner_local_store,
                            &mut pending_turns,
                            &mut parked_turns,
                        )
                        .await;
                    }
                    wake_opt = page_wake_rx.recv() => {
                        let Some(wake) = wake_opt else {
                            break;
                        };
                        page_admission_preference = PageTurnAdmissionPreference::Deadline;
                        self.handle_page_owner_wake(
                            wake,
                            &mut parked_turns,
                            &mut pending_turns,
                        );
                    }
                    inspector_io_wake_opt = inspector_io_wake_rx.recv() => {
                        let Some(wake) = inspector_io_wake_opt else {
                            break;
                        };
                        dispatch_inspector_io_owner_wake(wake);
                    }
                    shared_worker_wake_opt = shared_worker_wake_rx.recv() => {
                        let Some(wake) = shared_worker_wake_opt else {
                            break;
                        };
                        self.handle_shared_worker_runtime_wake(wake, &mut pending_turns);
                    }
                    service_worker_wake_opt = service_worker_wake_rx.recv() => {
                        let Some(wake) = service_worker_wake_opt else {
                            break;
                        };
                        self.handle_service_worker_runtime_wake(wake, &mut pending_turns);
                    }
                    _ = sleep_until_or_forever(next_owner_deadline) => {
                        self.enqueue_due_parked_turns(&mut parked_turns, &mut pending_turns);
                        if self.enqueue_due_page_turns(&mut pending_turns) {
                            page_admission_preference = PageTurnAdmissionPreference::ProducerWake;
                        }
                        self.enqueue_one_due_owner_maintenance_turn(&mut pending_turns);
                    }
                }
            }
        };
        loop_future.await
    }

    async fn dispatch_envelope_on_owner_local_store(
        &self,
        envelope: RenderRuntimeEnvelope,
        owner_local_store: &mut RendererOwnerLocalStore,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
    ) {
        let (command, reply_tx) = match envelope {
            RenderRuntimeEnvelope::Command { command, reply_tx } => (*command, reply_tx),
            RenderRuntimeEnvelope::InspectorMainReceiverWake(wake) => {
                Self::enqueue_inspector_main_receiver_wake(wake, pending_turns);
                return;
            }
        };
        #[cfg(test)]
        self.wait_on_command_dispatch_gate_for_testing();
        if self.context_shutdown_started() {
            self.release_rejected_command_output_reservation(&command);
            let _ = reply_tx.send(Err(anyhow!(
                "renderer browser context was dropped before command dispatch"
            )));
            return;
        }
        if reply_tx.is_closed()
            && matches!(
                &command,
                RendererOwnerCommand::RunAsyncPageCommand { command, .. }
                    | RendererOwnerCommand::RunProtocolPageCommand { command, .. }
                    if command.interruptible_by_javascript_dialog()
            )
        {
            return;
        }
        let removed_page_token = match &command {
            RendererOwnerCommand::RemovePage { token } => Some(*token),
            _ => None,
        };
        let mut command_admission_output_predecessor =
            renderer_command_admission_page_token(&command)
                .and_then(renderer_output_fence_for_tail_on_bound_owner_local_store);
        let command_future =
            Box::pin(self.dispatch_command_inline_on_owner_local_store(command, owner_local_store));
        let outcome = command_future.await;
        if self.context_shutdown_started() {
            self.cancel_dispatch_outcome_for_context_shutdown(outcome);
            let _ = reply_tx.send(Err(anyhow!(
                "renderer browser context was dropped during command dispatch"
            )));
            return;
        }
        match outcome {
            RenderRuntimeDispatchOutcome::Reply(result) => {
                let result = merge_command_admission_output_predecessor(
                    *result,
                    command_admission_output_predecessor.take(),
                );
                let _ = reply_tx.send(result);
            }
            RenderRuntimeDispatchOutcome::InspectorMainCommandClaimed {
                reply_tx: main_reply_tx,
                turn,
                command_admission_output_predecessor: _,
            } => {
                self.cancel_pending_turn_on_owner_local_store(*turn);
                let _ = main_reply_tx.send(Err(anyhow!(
                    "Inspector Main receiver outcome escaped into owner command dispatch"
                )));
                let _ = reply_tx.send(Err(anyhow!(
                    "renderer command unexpectedly entered the Inspector Main receiver"
                )));
            }
            RenderRuntimeDispatchOutcome::PageCreatedAndContinueNavigation {
                mut page,
                continuation,
            } => {
                let token = page.token;
                if continuation.requires_committed_document_response_release() {
                    page.defer_committed_document_parser_until_response(
                        self.state.page_wake_tx.clone(),
                    );
                }
                if reply_tx
                    .send(Ok(RendererOwnerReply::PageCreated(page)))
                    .is_ok()
                {
                    self.enqueue_page_creation_continuation(
                        continuation,
                        pending_turns,
                        parked_turns,
                    );
                } else {
                    remove_page_on_bound_owner_local_store(token);
                }
            }
            RenderRuntimeDispatchOutcome::BackgroundComplete(result) => {
                let reply = match result {
                    Ok(()) => Err(anyhow!(
                        "renderer command unexpectedly completed as background work"
                    )),
                    Err(error) => Err(anyhow!(
                        "renderer command unexpectedly completed as background work: {error}"
                    )),
                };
                let _ = reply_tx.send(reply);
            }
            RenderRuntimeDispatchOutcome::PageCreationNavigationFailurePublished {
                token,
                failure,
            } => {
                let _ = reply_tx.send(Err(anyhow!(failure.to_string())));
                self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
            }
            RenderRuntimeDispatchOutcome::ContinueNextTurn(turn) => {
                if reply_tx.is_closed() {
                    self.detach_navigation_or_cancel_pending_turn(*turn, pending_turns);
                } else {
                    pending_turns.push_back(RenderRuntimePendingTurn {
                        reply_tx: Some(reply_tx),
                        turn: *turn,
                        // This is the command's first owner turn, not a
                        // continuation returning to the queue. Run it before
                        // admitting another envelope so a Page wake produced
                        // by the command cannot inherit an already-pending
                        // command and then yield to a second one.
                        allow_command_overtake: false,
                        command_admission_output_predecessor: command_admission_output_predecessor
                            .take(),
                    });
                }
            }
            RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                turn,
                wake_token,
                ready_at,
            } => {
                self.park_or_cancel_turn(
                    Some(reply_tx),
                    *turn,
                    wake_token,
                    Some(ready_at),
                    RenderRuntimeParkCondition::PageActivity,
                    command_admission_output_predecessor.take(),
                    parked_turns,
                    pending_turns,
                );
            }
            RenderRuntimeDispatchOutcome::ContinueAfterPageWake { turn, wake_token } => {
                self.park_or_cancel_turn(
                    Some(reply_tx),
                    *turn,
                    wake_token,
                    None,
                    RenderRuntimeParkCondition::PageActivity,
                    command_admission_output_predecessor.take(),
                    parked_turns,
                    pending_turns,
                );
            }
            RenderRuntimeDispatchOutcome::ContinueCommittedDocumentParserAfterPageWake {
                turn,
                wake_token,
            } => {
                self.park_or_cancel_turn(
                    Some(reply_tx),
                    *turn,
                    wake_token,
                    None,
                    RenderRuntimeParkCondition::CommittedDocumentParserContinuation {
                        parser_unblocked: true,
                    },
                    command_admission_output_predecessor.take(),
                    parked_turns,
                    pending_turns,
                );
            }
        }
        if let Some(token) = removed_page_token {
            self.enqueue_all_parked_turns_for_page(token, parked_turns, pending_turns);
        }
    }

    fn cancel_dispatch_outcome_for_context_shutdown(&self, outcome: RenderRuntimeDispatchOutcome) {
        match outcome {
            RenderRuntimeDispatchOutcome::PageCreatedAndContinueNavigation {
                page,
                continuation,
            } => {
                drop(page);
                self.cancel_pending_turn_on_owner_local_store(continuation.into_turn());
            }
            RenderRuntimeDispatchOutcome::ContinueNextTurn(turn)
            | RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline { turn, .. }
            | RenderRuntimeDispatchOutcome::ContinueAfterPageWake { turn, .. }
            | RenderRuntimeDispatchOutcome::ContinueCommittedDocumentParserAfterPageWake {
                turn,
                ..
            } => {
                self.cancel_pending_turn_on_owner_local_store(*turn);
            }
            RenderRuntimeDispatchOutcome::InspectorMainCommandClaimed {
                reply_tx,
                turn,
                command_admission_output_predecessor: _,
            } => {
                drop(reply_tx);
                self.cancel_pending_turn_on_owner_local_store(*turn);
            }
            RenderRuntimeDispatchOutcome::Reply(_)
            | RenderRuntimeDispatchOutcome::BackgroundComplete(_)
            | RenderRuntimeDispatchOutcome::PageCreationNavigationFailurePublished { .. } => {}
        }
    }

    pub fn materialize_page_created_reply_parts(
        &self,
        reply: RendererOwnerReply,
    ) -> Result<(
        RendererPageHandle,
        Arc<RendererPageState>,
        RendererPageCreationDiagnostics,
        RendererPageCreationArtifacts,
        Option<RendererPendingDownloadActivation>,
    )> {
        match reply {
            RendererOwnerReply::PageCreated(reply) => Ok((*reply).into_parts(
                self.state.local_executor.clone(),
                self.render_runtime.clone(),
            )),
            _ => Err(anyhow!(
                "renderer owner returned non-page-creation reply for page creation request"
            )),
        }
    }

    fn restore_live_page_entry(&self, token: RendererPageToken, mut entry: LivePageEntry) {
        // Freeze this owner turn before making the Page resident again. The
        // concrete publication is already source-bound, so a later lifecycle
        // turn never needs to rescan or publish output on this turn's behalf.
        let output = entry.page_vm_mut().settle_renderer_output_publication();
        restore_entry_after_command_on_bound_owner_local_store(token, entry);
        if let Some(output) = output {
            self.publish_renderer_output(output);
        }
    }

    fn retire_pending_phase_one_page_entry(
        &self,
        token: RendererPageToken,
        entry: LivePageEntry,
        reason: &str,
    ) {
        // Retirement becomes sticky while the checked-out entry still owns
        // its PageVm. Rejecting the navigation then consumes the live type and
        // returns a teardown-only entry that cannot reach live restoration.
        remove_page_on_bound_owner_local_store(token);
        let entry = entry.reject_pending_phase_one_navigation(reason);
        restore_retiring_entry_after_command_on_bound_owner_local_store(token, entry);
    }

    fn finish_retiring_phase_one_navigation_failure(
        &self,
        token: RendererPageToken,
        mut entry: RetiringPageEntry,
        error: anyhow::Error,
    ) -> RenderRuntimeDispatchOutcome {
        entry.settle_standalone_navigation_follow(false);
        tracing::warn!(
            page_id = token.page_id.as_u64(),
            failure = %error,
            "retiring page after phase-one navigation lost its active PageVm"
        );
        remove_page_on_bound_owner_local_store(token);
        restore_retiring_entry_after_command_on_bound_owner_local_store(token, entry);
        Err(error).into()
    }

    async fn finish_live_page_navigation_failure(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
        retire_page_on_failure: bool,
        disposition: LivePageNavigationFailureDisposition,
    ) -> RenderRuntimeDispatchOutcome {
        entry.settle_standalone_navigation_follow(false);
        let had_uncommitted_page_vm = entry.has_uncommitted_page_vm();
        let navigation_committed = entry
            .vm
            .as_ref()
            .is_none_or(|page_vm| !page_vm.has_live_script_vm());
        let should_retire_page = navigation_committed || retire_page_on_failure;
        // Page creation owns the strong failure observer while the stable Page
        // slot owns only its weak publisher. Publish before requesting Page
        // retirement: restoring a checked-out committed entry tears down that
        // slot, but the parked creation turn must retain the concrete terminal
        // and be woken instead of falling through to its generic timeout.
        let page_creation_publication = match &disposition {
            LivePageNavigationFailureDisposition::PublishToPageCreation(failure) => Some(
                publish_page_navigation_failure_on_bound_owner_local_store(token, *failure),
            ),
            LivePageNavigationFailureDisposition::ReturnToInitiator(_)
            | LivePageNavigationFailureDisposition::ReportBackground(_) => None,
        };
        if should_retire_page {
            tracing::warn!(
                page_id = token.page_id.as_u64(),
                failure = %disposition,
                "retiring page after committed navigation failed to bootstrap"
            );
            // Mark the stable slot retiring while this turn still owns the
            // entry. Restoring then tears down the detached shell without ever
            // publishing it as the page's active runtime.
            remove_page_on_bound_owner_local_store(token);
        }
        if had_uncommitted_page_vm && !should_retire_page {
            // Cross-creation publication is legal only at the typed response
            // commit transition. Reaching failure with a mismatched stable
            // identity means that transition itself failed; do not invent a
            // late commit or roll back to the terminated Document.
            tracing::error!(
                page_id = token.page_id.as_u64(),
                failure = %disposition,
                "retiring page after replacement Document commit failed to publish"
            );
            remove_page_on_bound_owner_local_store(token);
        }
        self.restore_live_page_entry(token, entry);
        match disposition {
            LivePageNavigationFailureDisposition::PublishToPageCreation(failure) => {
                match page_creation_publication
                    .expect("Page-creation failure disposition must publish before retirement")
                {
                    Ok(PageCreationNavigationFailurePublication::Recorded) => {
                        RenderRuntimeDispatchOutcome::PageCreationNavigationFailurePublished {
                            token,
                            failure,
                        }
                    }
                    Ok(PageCreationNavigationFailurePublication::AlreadyRecorded) => {
                        RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()))
                    }
                    Ok(PageCreationNavigationFailurePublication::NoActiveCreationObserver) => {
                        RenderRuntimeDispatchOutcome::BackgroundComplete(Err(anyhow!(
                            "unobserved background navigation failure: {failure}"
                        )))
                    }
                    Err(error) => RenderRuntimeDispatchOutcome::BackgroundComplete(Err(error)),
                }
            }
            LivePageNavigationFailureDisposition::ReturnToInitiator(error) => Err(error).into(),
            LivePageNavigationFailureDisposition::ReportBackground(failure) => {
                RenderRuntimeDispatchOutcome::BackgroundComplete(Err(anyhow!(failure.to_string())))
            }
        }
    }

    /// Compute the earliest Page-owned task deadline. This combines JavaScript
    /// timers with typed delayed sources and contains no renderer housekeeping
    /// deadline, so every admission has a concrete Page scheduler candidate.
    fn compute_next_page_task_deadline(&self) -> Option<Instant> {
        next_page_task_deadline_on_bound_owner_local_store()
    }

    /// Compute the next renderer-owner housekeeping deadline. Maintenance is
    /// selected by its own residence and never enters Page task arbitration.
    fn compute_next_owner_maintenance_deadline(&self) -> Option<Instant> {
        next_owner_maintenance_deadline_on_bound_owner_local_store()
    }

    fn enqueue_page_creation_continuation(
        &self,
        continuation: RenderRuntimePageCreationContinuation,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
    ) {
        match continuation {
            RenderRuntimePageCreationContinuation::NextTurn(turn) => {
                pending_turns.push_back(RenderRuntimePendingTurn {
                    reply_tx: None,
                    turn: *turn,
                    allow_command_overtake: true,
                    command_admission_output_predecessor: None,
                });
            }
            RenderRuntimePageCreationContinuation::AfterCommittedDocumentResponse {
                turn,
                wake_token,
            } => {
                parked_turns.push_back(RenderRuntimeParkedTurn {
                    reply_tx: None,
                    turn: *turn,
                    wake_token,
                    ready_at: None,
                    condition: RenderRuntimeParkCondition::CommittedDocumentParserContinuation {
                        parser_unblocked: false,
                    },
                    command_admission_output_predecessor: None,
                });
            }
        }
    }

    fn park_or_cancel_turn(
        &self,
        reply_tx: Option<oneshot::Sender<Result<RendererOwnerReply>>>,
        turn: RenderRuntimeTurn,
        wake_token: RendererPageToken,
        ready_at: Option<Instant>,
        condition: RenderRuntimeParkCondition,
        command_admission_output_predecessor: Option<RendererOutputFence>,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        if reply_tx.as_ref().is_some_and(|tx| tx.is_closed()) {
            self.detach_navigation_or_cancel_pending_turn(turn, pending_turns);
            return;
        }
        parked_turns.push_back(RenderRuntimeParkedTurn {
            reply_tx,
            turn,
            wake_token,
            ready_at,
            condition,
            command_admission_output_predecessor,
        });
    }

    /// Admit one detached page turn. Producer wakes remain in the
    /// owner channel while a page turn is pending, so every legacy wake gets a
    /// bounded adapter turn without being merged into an inferred continuation.
    /// Already-arrived commands can preempt that turn at its owner boundary.
    fn enqueue_page_turn(
        &self,
        token: RendererPageToken,
        trigger: PageTurnTrigger,
        allow_command_overtake: bool,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        if let Some(turn) = self.admit_page_turn(token, trigger, allow_command_overtake) {
            pending_turns.push_back(turn);
        }
    }

    fn enqueue_page_turn_before_commands(
        &self,
        token: RendererPageToken,
        trigger: PageTurnTrigger,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        if let Some(turn) = self.admit_page_turn(token, trigger, false) {
            pending_turns.push_front(turn);
        }
    }

    fn admit_page_turn(
        &self,
        token: RendererPageToken,
        trigger: PageTurnTrigger,
        allow_command_overtake: bool,
    ) -> Option<RenderRuntimePendingTurn> {
        match schedule_page_turn_on_bound_owner_local_store(token, trigger) {
            RendererPageTurnAdmission::EnqueueOwnerTurn => {}
            RendererPageTurnAdmission::AlreadyScheduled => {
                panic!(
                    "renderer page {} received a second owner admission before its scheduled turn",
                    token.page_id.as_u64()
                );
            }
            RendererPageTurnAdmission::Retired | RendererPageTurnAdmission::MissingPage => {
                tracing::trace!(
                    page_id = token.page_id.as_u64(),
                    ?trigger,
                    "discarded stale page turn trigger"
                );
                return None;
            }
        }
        Some(RenderRuntimePendingTurn {
            reply_tx: None,
            turn: RenderRuntimeTurn::RunPageTurn { token },
            allow_command_overtake,
            command_admission_output_predecessor: None,
        })
    }

    fn handle_page_owner_wake(
        &self,
        wake: RendererOwnerWake,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        match wake {
            RendererOwnerWake::Page { token, source } => {
                if parked_turns.iter().any(|turn| {
                    turn.wake_token == token
                        && turn.condition.blocks_page_activity_until_parser_unblocked()
                }) {
                    // The source payload remains resident. Its edge wake is
                    // reconciled after the commit caller observes the Page,
                    // so parser work cannot outrun the DocumentCommit reply.
                    return;
                }
                let allow_command_overtake = !parked_turns.iter().any(|turn| {
                    turn.wake_token == token
                        && turn.condition.admits_page_activity()
                        && !turn.condition.allows_command_overtake()
                });
                let committed_document_parser_handoff = self
                    .enqueue_unblocked_committed_document_parser_continuation_before_commands(
                        token,
                        parked_turns,
                        pending_turns,
                    );
                if matches!(source, RendererOwnerWakeSource::ParseTimeDocumentScriptWork) {
                    // Parse-time script payloads live inside the parked
                    // continuation. Admitting a generic Page turn for this
                    // source would manufacture a second consumer.
                    self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
                    return;
                }
                if matches!(source, RendererOwnerWakeSource::InternalLoadingTask)
                    && !snapshot_due_page_task_tokens_on_bound_owner_local_store(Instant::now())
                        .contains(&token)
                {
                    // Delayed internal-loading payloads are posted while the
                    // Page is checked out. Restoration has already accepted
                    // the payload into the derived deadline index; the route
                    // wake only brings an idle owner loop back to that index.
                    // Do not manufacture an empty Page turn before it is due.
                    self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
                    return;
                }
                if source
                    == RendererOwnerWakeSource::Runtime(
                        RendererOwnerRuntimeActivitySource::DocumentLifecycleTurn,
                    )
                {
                    // The previous lifecycle turn has already published its
                    // milestone before admitting this follow-up. Let parked
                    // lifecycle observers capture that settled boundary
                    // before the next lifecycle turn can advance into load.
                    self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
                    self.enqueue_page_turn(
                        token,
                        PageTurnTrigger::producer(source),
                        allow_command_overtake,
                        pending_turns,
                    );
                    return;
                }
                if committed_document_parser_handoff {
                    self.enqueue_page_turn_before_commands(
                        token,
                        PageTurnTrigger::producer(source),
                        pending_turns,
                    );
                    self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
                    return;
                }
                self.enqueue_page_turn(
                    token,
                    PageTurnTrigger::producer(source),
                    allow_command_overtake,
                    pending_turns,
                );
                self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
            }
            RendererOwnerWake::PostResponseDocumentLifecycle { token, document } => {
                if !release_post_response_document_lifecycle_on_bound_owner_local_store(
                    token, document,
                ) {
                    tracing::trace!(
                        page_id = token.page_id.as_u64(),
                        ?document,
                        "discarded stale post-response lifecycle continuation"
                    );
                    return;
                }
                let trigger = PageTurnTrigger::producer(RendererOwnerWakeSource::Runtime(
                    RendererOwnerRuntimeActivitySource::DocumentLifecycleTurn,
                ));
                match schedule_page_turn_on_bound_owner_local_store(token, trigger) {
                    RendererPageTurnAdmission::EnqueueOwnerTurn => {
                        pending_turns.push_back(RenderRuntimePendingTurn {
                            reply_tx: None,
                            turn: RenderRuntimeTurn::RunPageTurn { token },
                            allow_command_overtake: true,
                            command_admission_output_predecessor: None,
                        });
                    }
                    RendererPageTurnAdmission::AlreadyScheduled => {
                        // Opening the exact resident is enough. The previously
                        // scheduled Page turn will observe it during arbitration.
                    }
                    RendererPageTurnAdmission::Retired | RendererPageTurnAdmission::MissingPage => {
                        return;
                    }
                }
                self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
            }
            RendererOwnerWake::CommittedDocumentParserUnblocked { token } => {
                let mut unblocked = false;
                for parked_turn in parked_turns
                    .iter_mut()
                    .filter(|turn| turn.wake_token == token)
                {
                    unblocked |= parked_turn.condition.unblock_committed_document_parser();
                }
                if !unblocked {
                    tracing::trace!(
                        page_id = token.page_id.as_u64(),
                        "discarded stale committed-Document parser release"
                    );
                    return;
                }
                let admission =
                    pending_phase_one_admission_after_restore_on_bound_owner_local_store(token);
                self.signal_pending_phase_one_admission(token, admission);
            }
            RendererOwnerWake::RuntimeInspectorResponsePublication { token, publication } => {
                let renderer_output_predecessor =
                    renderer_output_fence_for_tail_on_bound_owner_local_store(token);
                if let Err(completion) = publication.commit(renderer_output_predecessor) {
                    tracing::debug!(
                        page_id = token.page_id.as_u64(),
                        call_id = completion.call_id,
                        "discarded late Runtime response because its protocol receiver was closed"
                    );
                }
            }
            RendererOwnerWake::TopLevelNavigationHandoff { token, handoff } => {
                // A command-owned wait that is already resident for this Page
                // gets first refusal. Its continuation can claim the exact
                // navigation descriptor and retain its own completion policy.
                // The queued owner turn is the explicit background fallback
                // when no such consumer takes ownership.
                self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
                pending_turns.push_back(RenderRuntimePendingTurn {
                    reply_tx: None,
                    turn: RenderRuntimeTurn::ClaimLivePageTopLevelNavigationHandoff {
                        token,
                        handoff,
                    },
                    allow_command_overtake: false,
                    command_admission_output_predecessor: None,
                });
            }
            RendererOwnerWake::ReplacementDocumentViewSettled {
                token,
                vm_creation_id,
            } => {
                self.enqueue_parked_turns_for_replacement_view_settlement(
                    token,
                    vm_creation_id,
                    parked_turns,
                    pending_turns,
                );
            }
        }
    }

    fn claim_top_level_navigation_handoff(
        &self,
        token: RendererPageToken,
        handoff: RendererTopLevelNavigationHandoff,
    ) -> RenderRuntimeDispatchOutcome {
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => {
                tracing::trace!(
                    page_id = token.page_id.as_u64(),
                    ?handoff,
                    "discarded top-level navigation handoff: {error}"
                );
                return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
            }
        };
        let claimed = entry.begin_standalone_navigation_follow_from_handoff(handoff);
        self.restore_live_page_entry(token, entry);
        if !claimed {
            tracing::trace!(
                page_id = token.page_id.as_u64(),
                ?handoff,
                "ignored stale, delegated, or already-owned top-level navigation handoff"
            );
            return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
        }
        RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
            RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                token,
                stage: PageVmInitStage::Load,
                follow_count: 0,
                completion: LivePagePendingNavigationCompletion::Background,
            },
        ))
    }

    fn try_admit_ready_page_turn(
        &self,
        preference: &mut PageTurnAdmissionPreference,
        page_wake_rx: &mut mpsc::UnboundedReceiver<RendererOwnerWake>,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) -> ReadyPageTurnAdmission {
        let preferred = match preference {
            PageTurnAdmissionPreference::ProducerWake => self.try_admit_page_producer_wake(
                preference,
                page_wake_rx,
                parked_turns,
                pending_turns,
            ),
            PageTurnAdmissionPreference::Deadline => {
                self.try_admit_due_page_turn(preference, pending_turns)
            }
        };
        if preferred != ReadyPageTurnAdmission::NoneReady {
            return preferred;
        }
        match preference {
            PageTurnAdmissionPreference::ProducerWake => {
                self.try_admit_due_page_turn(preference, pending_turns)
            }
            PageTurnAdmissionPreference::Deadline => self.try_admit_page_producer_wake(
                preference,
                page_wake_rx,
                parked_turns,
                pending_turns,
            ),
        }
    }

    fn try_admit_page_producer_wake(
        &self,
        preference: &mut PageTurnAdmissionPreference,
        page_wake_rx: &mut mpsc::UnboundedReceiver<RendererOwnerWake>,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) -> ReadyPageTurnAdmission {
        match page_wake_rx.try_recv() {
            Ok(wake) => {
                self.handle_page_owner_wake(wake, parked_turns, pending_turns);
                *preference = PageTurnAdmissionPreference::Deadline;
                ReadyPageTurnAdmission::Admitted
            }
            Err(mpsc::error::TryRecvError::Empty) => ReadyPageTurnAdmission::NoneReady,
            Err(mpsc::error::TryRecvError::Disconnected) => {
                ReadyPageTurnAdmission::WakeChannelClosed
            }
        }
    }

    fn try_admit_due_page_turn(
        &self,
        preference: &mut PageTurnAdmissionPreference,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) -> ReadyPageTurnAdmission {
        if self.enqueue_due_page_turns(pending_turns) {
            *preference = PageTurnAdmissionPreference::ProducerWake;
            ReadyPageTurnAdmission::Admitted
        } else {
            ReadyPageTurnAdmission::NoneReady
        }
    }

    fn enqueue_shared_worker_service_lane_turn(
        &self,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        pending_turns.push_back(RenderRuntimePendingTurn {
            reply_tx: None,
            turn: RenderRuntimeTurn::DrainSharedWorkerServiceLane,
            allow_command_overtake: false,
            command_admission_output_predecessor: None,
        });
    }

    fn enqueue_service_worker_service_lane_turn(
        &self,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        pending_turns.push_back(RenderRuntimePendingTurn {
            reply_tx: None,
            turn: RenderRuntimeTurn::DrainServiceWorkerServiceLane,
            allow_command_overtake: false,
            command_admission_output_predecessor: None,
        });
    }

    fn handle_shared_worker_runtime_wake(
        &self,
        wake: SharedWorkerRuntimeOwnerWake,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        match wake {
            SharedWorkerRuntimeOwnerWake::ServiceLane => {
                self.enqueue_shared_worker_service_lane_turn(pending_turns);
            }
        }
    }

    fn handle_service_worker_runtime_wake(
        &self,
        wake: ServiceWorkerRuntimeOwnerWake,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        match wake {
            ServiceWorkerRuntimeOwnerWake::ServiceLane => {
                self.enqueue_service_worker_service_lane_turn(pending_turns);
            }
        }
    }

    fn next_parked_turn_deadline(
        &self,
        parked_turns: &VecDeque<RenderRuntimeParkedTurn>,
    ) -> Option<Instant> {
        parked_turns.iter().filter_map(|turn| turn.ready_at).min()
    }

    fn enqueue_parked_turn(
        &self,
        parked_turn: RenderRuntimeParkedTurn,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        if let Some(turn) = self.pending_turn_from_parked(parked_turn, pending_turns) {
            pending_turns.push_back(turn);
        }
    }

    fn pending_turn_from_parked(
        &self,
        parked_turn: RenderRuntimeParkedTurn,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) -> Option<RenderRuntimePendingTurn> {
        if parked_turn
            .reply_tx
            .as_ref()
            .is_some_and(|tx| tx.is_closed())
        {
            self.detach_navigation_or_cancel_pending_turn(parked_turn.turn, pending_turns);
            None
        } else {
            let allow_command_overtake = parked_turn.condition.allows_command_overtake();
            Some(RenderRuntimePendingTurn {
                reply_tx: parked_turn.reply_tx,
                turn: parked_turn.turn,
                allow_command_overtake,
                command_admission_output_predecessor: parked_turn
                    .command_admission_output_predecessor,
            })
        }
    }

    fn enqueue_unblocked_committed_document_parser_continuation_before_commands(
        &self,
        token: RendererPageToken,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) -> bool {
        let mut index = 0;
        let mut admitted = 0usize;
        while index < parked_turns.len() {
            let matches = parked_turns[index].wake_token == token
                && parked_turns[index]
                    .condition
                    .is_unblocked_committed_document_parser_continuation();
            if matches {
                let parked_turn = parked_turns
                    .remove(index)
                    .expect("matching committed-Document continuation must remain parked");
                if let Some(turn) = self.pending_turn_from_parked(parked_turn, pending_turns) {
                    pending_turns.push_front(turn);
                    admitted += 1;
                }
            } else {
                index += 1;
            }
        }
        debug_assert!(
            admitted <= 1,
            "one Page cannot retain multiple committed-Document parser continuations"
        );
        admitted != 0
    }

    fn enqueue_due_parked_turns(
        &self,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        let now = Instant::now();
        let mut index = 0;
        while index < parked_turns.len() {
            let ready = parked_turns[index]
                .ready_at
                .is_some_and(|ready_at| ready_at <= now)
                || parked_turns[index]
                    .reply_tx
                    .as_ref()
                    .is_some_and(|tx| tx.is_closed());
            if ready {
                let Some(parked_turn) = parked_turns.remove(index) else {
                    break;
                };
                self.enqueue_parked_turn(parked_turn, pending_turns);
            } else {
                index += 1;
            }
        }
    }

    fn enqueue_parked_turns_for_wake(
        &self,
        token: RendererPageToken,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        let mut index = 0;
        while index < parked_turns.len() {
            let should_wake = (parked_turns[index].wake_token == token
                && parked_turns[index].condition.admits_page_activity())
                || parked_turns[index]
                    .reply_tx
                    .as_ref()
                    .is_some_and(|tx| tx.is_closed());
            if should_wake {
                let Some(parked_turn) = parked_turns.remove(index) else {
                    break;
                };
                self.enqueue_parked_turn(parked_turn, pending_turns);
            } else {
                index += 1;
            }
        }
    }

    fn enqueue_parked_turns_for_replacement_view_settlement(
        &self,
        token: RendererPageToken,
        vm_creation_id: u64,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        let mut index = 0;
        while index < parked_turns.len() {
            let should_wake = (parked_turns[index].wake_token == token
                && parked_turns[index]
                    .condition
                    .admits_replacement_view_settlement(vm_creation_id))
                || parked_turns[index]
                    .reply_tx
                    .as_ref()
                    .is_some_and(|tx| tx.is_closed());
            if should_wake {
                let Some(parked_turn) = parked_turns.remove(index) else {
                    break;
                };
                self.enqueue_parked_turn(parked_turn, pending_turns);
            } else {
                index += 1;
            }
        }
    }

    fn enqueue_all_parked_turns_for_page(
        &self,
        token: RendererPageToken,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        let mut index = 0;
        while index < parked_turns.len() {
            let should_wake = parked_turns[index].wake_token == token
                || parked_turns[index]
                    .reply_tx
                    .as_ref()
                    .is_some_and(|tx| tx.is_closed());
            if should_wake {
                let Some(parked_turn) = parked_turns.remove(index) else {
                    break;
                };
                self.enqueue_parked_turn(parked_turn, pending_turns);
            } else {
                index += 1;
            }
        }
    }

    fn enqueue_parked_page_creation_observers_after_page_turn(
        &self,
        token: RendererPageToken,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        let mut index = 0;
        while index < parked_turns.len() {
            let should_observe = parked_turns[index].wake_token == token
                && parked_turns[index]
                    .turn
                    .is_page_creation_lifecycle_observer_for(token);
            if should_observe {
                let Some(parked_turn) = parked_turns.remove(index) else {
                    break;
                };
                self.enqueue_parked_turn(parked_turn, pending_turns);
            } else {
                index += 1;
            }
        }
    }

    async fn run_one_document_lifecycle_turn(
        &self,
        token: RendererPageToken,
        entry: LivePageEntry,
        source_label: &'static str,
        displaced_ordinary: RendererDisplacedOrdinaryTurn,
    ) -> RenderRuntimeDispatchOutcome {
        let executor = entry.page_vm().local_executor.clone();
        let (mut entry, advance_result) =
            advance_document_lifecycle_one_page_turn_via_local_task(executor, entry).await;
        let outcome = match advance_result {
            Ok(outcome) => outcome,
            Err(error) => {
                let output = entry.page_vm_mut().settle_renderer_output_publication();
                restore_entry_after_document_lifecycle_on_bound_owner_local_store(
                    token,
                    entry,
                    displaced_ordinary.requires_reconsideration(),
                );
                if let Some(output) = output {
                    self.publish_renderer_output(output);
                }
                if displaced_ordinary.requires_reconsideration() {
                    self.signal_internal_page_turn_source(
                        token,
                        RendererOwnerWakeSource::SchedulerContinuation,
                    );
                }
                tracing::error!(
                    source = source_label,
                    page_id = token.page_id.as_u64(),
                    "document lifecycle owner turn failed: {error}"
                );
                return RenderRuntimeDispatchOutcome::BackgroundComplete(Err(error));
            }
        };
        let action = outcome.action;
        let readiness = outcome.readiness;
        let output_ordering = action.renderer_output_ordering();
        let top_level_navigation_dispatch = entry.top_level_navigation_dispatch();
        let should_resume_ordinary = displaced_ordinary.requires_reconsideration()
            && !matches!(
                action,
                DocumentLifecycleTurnAction::RequestedTopLevelNavigation { .. }
            )
            && !matches!(readiness, DocumentLifecycleTurnReadiness::Runnable { .. });
        let should_follow_pending_location_navigation = matches!(
            action,
            DocumentLifecycleTurnAction::RequestedTopLevelNavigation { .. }
        ) && entry
            .begin_standalone_navigation_follow();
        let delegated_top_level_navigation = if matches!(
            action,
            DocumentLifecycleTurnAction::RequestedTopLevelNavigation { .. }
        ) && matches!(
            top_level_navigation_dispatch,
            RendererTopLevelNavigationDispatch::DelegateToBrowser
        ) {
            match entry
                .page_vm_mut()
                .vm_mut()
                .publish_pending_non_javascript_location_navigation()
            {
                Ok(published) => published,
                Err(error) => {
                    let concrete_output = entry
                        .page_vm_mut()
                        .settle_renderer_output_publication()
                        .map(|output| output.with_ordering(output_ordering));
                    self.restore_live_page_entry(token, entry);
                    if let Some(output) = concrete_output {
                        self.publish_renderer_output(output);
                    }
                    return RenderRuntimeDispatchOutcome::BackgroundComplete(Err(error));
                }
            }
        } else {
            false
        };
        let concrete_output = entry
            .page_vm_mut()
            .settle_renderer_output_publication()
            .map(|output| output.with_ordering(output_ordering));
        restore_entry_after_document_lifecycle_on_bound_owner_local_store(
            token,
            entry,
            should_resume_ordinary,
        );
        if let Some(output) = concrete_output {
            self.publish_renderer_output(output);
        }

        match readiness {
            DocumentLifecycleTurnReadiness::Runnable { .. } => {
                self.signal_internal_document_lifecycle_turn(token);
            }
            DocumentLifecycleTurnReadiness::Blocked { .. }
            | DocumentLifecycleTurnReadiness::Idle
                if should_resume_ordinary =>
            {
                self.signal_internal_page_turn_source(
                    token,
                    RendererOwnerWakeSource::SchedulerContinuation,
                );
            }
            DocumentLifecycleTurnReadiness::Blocked { .. }
            | DocumentLifecycleTurnReadiness::Idle => {}
        }

        tracing::debug!(
            source = source_label,
            page_id = token.page_id.as_u64(),
            ?action,
            ?readiness,
            "completed one exact-Document lifecycle owner turn"
        );

        if let DocumentLifecycleTurnAction::RequestedTopLevelNavigation { stage, .. } = action {
            if delegated_top_level_navigation {
                return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
            }
            if !should_follow_pending_location_navigation {
                return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
            }
            return RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                    token,
                    stage,
                    follow_count: 0,
                    completion: LivePagePendingNavigationCompletion::Background,
                },
            ));
        }

        RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()))
    }

    fn finish_page_turn(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
        source_label: &'static str,
        output_ordering: RendererOutputPublicationOrdering,
    ) -> RenderRuntimeDispatchOutcome {
        let has_pending_document_lifecycle_turn =
            entry.pending_document_lifecycle_identity().is_some();
        if matches!(
            entry.top_level_navigation_dispatch(),
            RendererTopLevelNavigationDispatch::DelegateToBrowser
        ) && let Err(error) = entry
            .page_vm_mut()
            .vm_mut()
            .publish_pending_non_javascript_location_navigation()
        {
            let concrete_output = entry
                .page_vm_mut()
                .settle_renderer_output_publication()
                .map(|output| output.with_ordering(output_ordering));
            restore_entry_after_command_on_bound_owner_local_store(token, entry);
            if let Some(output) = concrete_output {
                self.publish_renderer_output(output);
            }
            return RenderRuntimeDispatchOutcome::BackgroundComplete(Err(error));
        }
        let concrete_output = entry
            .page_vm_mut()
            .settle_renderer_output_publication()
            .map(|output| output.with_ordering(output_ordering));
        restore_entry_after_command_on_bound_owner_local_store(token, entry);
        if let Some(output) = concrete_output {
            self.publish_renderer_output(output);
        }

        let readiness = page_turn_readiness_after_restore_on_bound_owner_local_store(token);
        let next_turn = readiness
            .map(|readiness| readiness.next_turn(has_pending_document_lifecycle_turn))
            .unwrap_or(PageOwnerNextTurn::None);

        match next_turn {
            PageOwnerNextTurn::Ordinary => {
                self.signal_internal_page_turn_source(
                    token,
                    RendererOwnerWakeSource::SchedulerContinuation,
                );
            }
            PageOwnerNextTurn::DocumentLifecycle => {
                self.signal_internal_document_lifecycle_turn(token);
            }
            PageOwnerNextTurn::None => {}
        }

        tracing::debug!(
            source = source_label,
            page_id = token.page_id.as_u64(),
            ?readiness,
            ?next_turn,
            "completed one ordinary Page turn"
        );
        RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()))
    }

    async fn run_one_page_turn(&self, token: RendererPageToken) -> RenderRuntimeDispatchOutcome {
        let (entry, trigger, scheduled_turn) =
            match checkout_scheduled_page_turn_on_bound_owner_local_store(token) {
                Ok(turn) => turn,
                Err(RendererPageTurnCheckoutError::Busy) => {
                    panic!(
                        "renderer page {} remained checked out at a serialized page-turn boundary",
                        token.page_id.as_u64()
                    );
                }
                Err(RendererPageTurnCheckoutError::NotScheduled) => {
                    panic!(
                        "renderer page {} reached an owner turn without a page-local admission",
                        token.page_id.as_u64()
                    );
                }
                Err(
                    RendererPageTurnCheckoutError::Retired | RendererPageTurnCheckoutError::Missing,
                ) => {
                    return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
                }
            };
        let source_label = page_turn_trigger_log_label(trigger);
        let scheduled_task = match scheduled_turn {
            RendererPageScheduledTurn::DocumentLifecycle { displaced_ordinary } => {
                return self
                    .run_one_document_lifecycle_turn(token, entry, source_label, displaced_ordinary)
                    .await;
            }
            RendererPageScheduledTurn::SpentWake => {
                self.restore_live_page_entry(token, entry);
                tracing::trace!(
                    source = source_label,
                    page_id = token.page_id.as_u64(),
                    "settled spent Page wake without entering the local executor"
                );
                return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
            }
            RendererPageScheduledTurn::Ordinary(scheduled_task) => *scheduled_task,
        };
        // Freeze the exact Document at task selection. A timer callback may
        // synchronously replace the Document; its protocol output still
        // belongs after the load boundary of the Document that authorized
        // this owner turn, never whichever Document is current at settlement.
        let turn_source_document = entry.page_vm().document_lifecycle.identity();
        let output_ordering = if matches!(
            &scheduled_task,
            crate::page_task_queue::RendererPageSchedulerTask::Timer { .. }
        ) {
            RendererOutputPublicationOrdering::AfterPendingPageLoad {
                source_document: turn_source_document,
            }
        } else {
            RendererOutputPublicationOrdering::Unconstrained
        };
        let executor = entry.page_vm().local_executor.clone();
        let loader = entry.page_vm().request_client.clone();
        let (mut entry, advance_result) =
            advance_page_owner_one_turn_via_local_task(executor, entry, scheduled_task, loader)
                .await;

        if let Err(error) = advance_result {
            let has_pending_document_lifecycle_turn =
                has_pending_document_lifecycle_turn_on_entry(&mut entry);
            let output = entry
                .page_vm_mut()
                .settle_renderer_output_publication()
                .map(|output| output.with_ordering(output_ordering));
            self.restore_live_page_entry(token, entry);
            if let Some(output) = output {
                self.publish_renderer_output(output);
            }
            let readiness = page_turn_readiness_after_restore_on_bound_owner_local_store(token);
            let next_turn = readiness
                .map(|readiness| readiness.next_turn(has_pending_document_lifecycle_turn))
                .unwrap_or(PageOwnerNextTurn::None);
            tracing::debug!(
                source = source_label,
                page_id = token.page_id.as_u64(),
                ?readiness,
                ?next_turn,
                "Page scheduler turn failed: {error}"
            );
            match next_turn {
                PageOwnerNextTurn::Ordinary => self.signal_internal_page_turn_source(
                    token,
                    RendererOwnerWakeSource::SchedulerContinuation,
                ),
                PageOwnerNextTurn::DocumentLifecycle => {
                    self.signal_internal_document_lifecycle_turn(token);
                }
                PageOwnerNextTurn::None => {}
            }
            return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
        }

        self.finish_page_turn(token, entry, source_label, output_ordering)
    }

    async fn run_one_owner_maintenance_turn(
        &self,
        task: RendererOwnerMaintenanceTask,
    ) -> RenderRuntimeDispatchOutcome {
        let token = task.token();
        let entry = match checkout_entry_for_owner_turn_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(LivePageEntryCheckoutError::Busy) => {
                // A maintenance task may wait behind other bounded owner
                // turns, but those turns always restore the Page entry before
                // returning or parking their continuation. Reaching this arm
                // would therefore mean two owner turns retained the same Page
                // entry concurrently, not a condition maintenance can retry.
                panic!(
                    "renderer page {} remained checked out at a serialized owner-maintenance boundary",
                    token.page_id.as_u64()
                );
            }
            Err(LivePageEntryCheckoutError::Retired | LivePageEntryCheckoutError::Missing) => {
                return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
            }
        };
        let executor = entry.page_vm().local_executor.clone();
        let (entry, action_result) =
            execute_owner_maintenance_task_on_local_lane(executor, entry, task).await;

        // Rearm the stable maintenance residence regardless of the V8
        // notification result. Leaving the admitted deadline unresolved would
        // either stop future maintenance or recreate the expired-deadline
        // owner spin this lane is designed to eliminate.
        let settlement_result =
            settle_owner_maintenance_task_on_bound_owner_local_store(task, Instant::now());
        // Maintenance does not own protocol/runtime output publication. A
        // plain residence restore keeps that boundary with the Page task or
        // command that produced the output.
        restore_entry_after_command_on_bound_owner_local_store(token, entry);

        let result = match (action_result, settlement_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(action_error), Ok(())) => Err(action_error),
            (Ok(()), Err(settlement_error)) => Err(settlement_error),
            (Err(action_error), Err(settlement_error)) => Err(anyhow!(
                "owner maintenance failed ({action_error:#}) and its residence settlement also failed ({settlement_error:#})"
            )),
        };
        RenderRuntimeDispatchOutcome::BackgroundComplete(result)
    }

    /// Admit every Page whose scheduler-owned task deadline is due.
    ///
    /// The selected turn still arbitrates and executes exactly one ready
    /// source; future deadlines remain resident in the owner-local index.
    fn enqueue_due_page_turns(&self, pending_turns: &mut RenderRuntimePendingTurnQueue) -> bool {
        // Future work remains only in its Page-owned residence and the
        // owner-local deadline index. A deadline must actually be due before
        // admission.
        let due_at_or_before = Instant::now();
        let tokens = snapshot_due_page_task_tokens_on_bound_owner_local_store(due_at_or_before);
        let admitted = !tokens.is_empty();
        for token in tokens {
            self.enqueue_page_turn(token, PageTurnTrigger::Deadline, true, pending_turns);
        }
        admitted
    }

    /// Claim at most one due housekeeping residence. Other due Pages remain
    /// indexed and are admitted on subsequent owner-loop iterations, so the
    /// lane is bounded and cannot drain ahead of Page work or commands.
    fn enqueue_one_due_owner_maintenance_turn(
        &self,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) -> bool {
        let Some(task) =
            claim_due_owner_maintenance_task_on_bound_owner_local_store(Instant::now())
        else {
            return false;
        };
        pending_turns.push_back(RenderRuntimePendingTurn {
            reply_tx: None,
            turn: RenderRuntimeTurn::RunOwnerMaintenance { task },
            allow_command_overtake: true,
            command_admission_output_predecessor: None,
        });
        true
    }

    fn checkout_live_page_entry_for_cancellation(
        &self,
        token: RendererPageToken,
        continuation: &'static str,
    ) -> Option<LivePageEntry> {
        match checkout_entry_for_owner_turn_on_bound_owner_local_store(token) {
            Ok(entry) => Some(entry),
            Err(LivePageEntryCheckoutError::Retired | LivePageEntryCheckoutError::Missing) => None,
            Err(LivePageEntryCheckoutError::Busy) => {
                panic!(
                    "renderer page {} remained checked out while cancelling {continuation}",
                    token.page_id.as_u64()
                )
            }
        }
    }

    fn detach_navigation_or_cancel_pending_turn(
        &self,
        turn: RenderRuntimeTurn,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        let (turn, detached) = turn.detach_navigation_command_observer();
        if detached {
            pending_turns.push_back(RenderRuntimePendingTurn {
                reply_tx: None,
                turn,
                allow_command_overtake: true,
                command_admission_output_predecessor: None,
            });
        } else {
            self.cancel_pending_turn_on_owner_local_store(turn);
        }
    }

    fn cancel_pending_turn_on_owner_local_store(&self, turn: RenderRuntimeTurn) {
        match turn {
            RenderRuntimeTurn::WaitLivePageScriptTruthy {
                token,
                pending_call: Some(pending_call),
                ..
            }
            | RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                token,
                pending_call: Some(pending_call),
                ..
            } => {
                if let Some(mut entry) = self
                    .checkout_live_page_entry_for_cancellation(token, "pending runtime evaluation")
                {
                    entry
                        .page_vm_mut()
                        .cancel_pending_runtime_evaluate(Some(pending_call));
                    self.restore_live_page_entry(token, entry);
                }
            }
            RenderRuntimeTurn::ContinueAttachedPageCreationLifecycle { pending, .. } => {
                remove_page_on_bound_owner_local_store(pending.token);
            }
            RenderRuntimeTurn::WaitLifecycleNavigation(wait) => {
                remove_page_on_bound_owner_local_store(wait.token());
            }
            RenderRuntimeTurn::ContinueLivePageRuntimeCommandLifecycle {
                token, scope_id, ..
            } => {
                if let Some(mut entry) = self
                    .checkout_live_page_entry_for_cancellation(token, "runtime command lifecycle")
                {
                    entry
                        .page_vm_mut()
                        .abandon_pending_runtime_command_lifecycle(scope_id);
                    self.restore_live_page_entry(token, entry);
                }
            }
            RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                token, ..
            } => {
                if let Some(entry) = self.checkout_live_page_entry_for_cancellation(
                    token,
                    "phase-one location navigation",
                ) {
                    self.retire_pending_phase_one_page_entry(
                        token,
                        entry,
                        "Location navigation was cancelled.",
                    );
                }
            }
            RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle { token, .. } => {
                remove_page_on_bound_owner_local_store(token);
            }
            RenderRuntimeTurn::FinishHtmlCreatePage { .. }
            | RenderRuntimeTurn::DrainSharedWorkerServiceLane
            | RenderRuntimeTurn::DrainServiceWorkerServiceLane
            | RenderRuntimeTurn::RunPageTurn { .. }
            | RenderRuntimeTurn::RunOwnerMaintenance { .. }
            | RenderRuntimeTurn::RunInspectorMainReceiver { .. }
            | RenderRuntimeTurn::RunDevToolsMainCommand { .. }
            | RenderRuntimeTurn::RunLivePageCommand { .. }
            | RenderRuntimeTurn::ResumeLivePageDocumentLifecycleAfterReply { .. }
            | RenderRuntimeTurn::WaitLivePageNetworkIdle { .. }
            | RenderRuntimeTurn::WaitLivePageDomStable { .. }
            | RenderRuntimeTurn::WaitLivePageSelector { .. }
            | RenderRuntimeTurn::WaitLivePageScriptTruthy {
                pending_call: None, ..
            }
            | RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                pending_call: None, ..
            }
            | RenderRuntimeTurn::WaitLivePageSubresourceResponse { .. }
            | RenderRuntimeTurn::WaitLivePageChildFrameLifecycle { .. }
            | RenderRuntimeTurn::ClaimLivePageTopLevelNavigationHandoff { .. } => {}
            RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                token,
                completion,
                ..
            } => {
                if completion.retires_page_on_navigation_failure() {
                    remove_page_on_bound_owner_local_store(token);
                }
            }
        }
    }

    async fn finish_live_page_entry_with_page_state(
        &self,
        token: RendererPageToken,
        entry: LivePageEntry,
        reply: RendererPageReply,
        turn_records: Vec<PendingRendererOutputRecord>,
    ) -> RenderRuntimeDispatchOutcome {
        self.finish_live_page_entry_with_page_state_and_continuation(
            token,
            entry,
            reply,
            turn_records,
            None,
            super::RendererPageStateCapturePolicy::FullReport,
        )
        .await
    }

    async fn finish_live_page_entry_with_page_state_and_continuation(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
        reply: RendererPageReply,
        turn_records: Vec<PendingRendererOutputRecord>,
        post_response_continuation: Option<RendererPageCommandPostResponseContinuation>,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let runtime_command_output = entry.page_vm_mut().take_runtime_command_output();
        let command_produced_records = !turn_records.is_empty();
        entry.page_vm().append_renderer_output_records(turn_records);
        let (mut entry, page_state_result) = commit_page_state_on_entry_via_local_task_with_policy(
            self.state.local_executor.clone(),
            entry,
            capture_policy,
        )
        .await;
        let concrete_output = entry.page_vm_mut().settle_renderer_output_publication();
        assert!(
            !command_produced_records || concrete_output.is_some(),
            "renderer command records must settle into the concrete Page stream before completion"
        );
        // Chromium flushes every notification already queued by the current
        // DevTools command before exposing its response. The concrete batch
        // settled here is the exact Moli equivalent of that queue
        // prefix. Do not require a Runtime-only causal marker: DOM commands
        // and WebAPI side effects (for example mutation, CSP and Log records)
        // are produced during this owner turn but do not all carry a
        // `RendererRuntimeCommandCausalIdentity`.
        //
        // This remains a narrow fence. It names this command turn's one
        // publication, not a Page-wide watermark, and a later independent
        // task necessarily settles into a later stream sequence.
        let renderer_output_cursor = concrete_output
            .as_ref()
            .map(RendererOutputPublication::cursor);
        let renderer_output_predecessor = renderer_output_cursor
            .map(|cursor| entry.page_vm().declare_renderer_output_fence(cursor));
        self.restore_live_page_entry(token, entry);
        if let Some(output) = concrete_output {
            self.publish_renderer_output(output);
        }
        match page_state_result {
            Ok(page_state) => match RendererCommandTurnOutput::new(
                reply,
                page_state,
                runtime_command_output,
                post_response_continuation,
                renderer_output_predecessor,
            ) {
                Ok(output) => Ok(RendererOwnerReply::AsyncPageCommandRan(Box::new(output))).into(),
                Err(error) => Err(error).into(),
            },
            Err(error) => Err(error).into(),
        }
    }

    fn merge_pending_download_into_reply(
        &self,
        reply: RendererPageReply,
        download: RendererPendingDownloadActivation,
    ) -> RendererPageReply {
        match reply {
            RendererPageReply::InputDispatchOutcome(mut outcome) => {
                outcome.pending_download = Some(download);
                RendererPageReply::InputDispatchOutcome(outcome)
            }
            other => other,
        }
    }

    async fn finish_live_page_navigation_completion(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
        completion: LivePagePendingNavigationCompletion,
    ) -> RenderRuntimeDispatchOutcome {
        entry.settle_standalone_navigation_follow(true);
        match completion {
            LivePagePendingNavigationCompletion::Background
            | LivePagePendingNavigationCompletion::PublishedPageCreation { .. } => {
                let lifecycle_to_resume = entry.active_page_vm().and_then(|page_vm| {
                    let lifecycle = page_vm.document_lifecycle.current_snapshot();
                    (lifecycle.load.is_none() && lifecycle.terminated.is_none())
                        .then_some(RendererDocumentLifecycleIdentity::from(lifecycle))
                });
                let (entry, page_state_result) = commit_page_state_on_entry_via_local_task(
                    self.state.local_executor.clone(),
                    entry,
                )
                .await;
                self.restore_live_page_entry(token, entry);
                match page_state_result {
                    Ok(_) => {
                        if let Some(document) = lifecycle_to_resume {
                            return RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                                RenderRuntimeTurn::ResumeLivePageDocumentLifecycleAfterReply {
                                    token,
                                    document,
                                },
                            ));
                        }
                        RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()))
                    }
                    Err(error) => RenderRuntimeDispatchOutcome::BackgroundComplete(Err(error)),
                }
            }
            LivePagePendingNavigationCompletion::CompletePageCreation { pending, .. } => {
                self.restore_live_page_entry(token, entry);
                self.finish_pending_page_creation(pending).await
            }
            LivePagePendingNavigationCompletion::ReplyWithSnapshot {
                reply,
                capture_policy,
            } => {
                self.finish_live_page_entry_with_page_state_and_continuation(
                    token,
                    entry,
                    *reply,
                    Vec::new(),
                    None,
                    capture_policy,
                )
                .await
            }
            LivePagePendingNavigationCompletion::ContinueNetworkIdle { deadline, loader } => {
                if let Err(error) = self.refresh_live_page_before_wait(token, entry).await {
                    return Err(error).into();
                }
                self.wait_live_page_network_idle_turn(
                    token,
                    PageVmNetworkIdleWaitState::default(),
                    deadline,
                    loader,
                )
                .await
            }
            LivePagePendingNavigationCompletion::ContinueDomStable { deadline, loader } => {
                if let Err(error) = self.refresh_live_page_before_wait(token, entry).await {
                    return Err(error).into();
                }
                self.wait_live_page_dom_stable_turn(
                    token,
                    PageVmDomStableWaitState::default(),
                    deadline,
                    loader,
                )
                .await
            }
            LivePagePendingNavigationCompletion::ContinueSubresourceResponse {
                criteria,
                deadline,
                loader,
                capture_policy,
            } => {
                if let Err(error) = self.refresh_live_page_before_wait(token, entry).await {
                    return Err(error).into();
                }
                self.wait_live_page_subresource_response_turn(
                    token,
                    criteria,
                    deadline,
                    loader,
                    capture_policy,
                )
                .await
            }
        }
    }

    /// Capture the already-published replacement before a host-facing wait
    /// resumes. Cross-creation publication is deliberately forbidden here;
    /// it belongs to the typed response/Document commit transition.
    async fn refresh_live_page_before_wait(
        &self,
        token: RendererPageToken,
        entry: LivePageEntry,
    ) -> Result<()> {
        let (entry, page_state_result) =
            commit_page_state_on_entry_via_local_task(self.state.local_executor.clone(), entry)
                .await;
        self.restore_live_page_entry(token, entry);
        page_state_result?;
        Ok(())
    }

    async fn finish_live_page_navigation_download(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
        completion: LivePagePendingNavigationCompletion,
        download: RendererPendingDownloadActivation,
    ) -> RenderRuntimeDispatchOutcome {
        entry.settle_standalone_navigation_follow(true);
        match completion {
            LivePagePendingNavigationCompletion::Background
            | LivePagePendingNavigationCompletion::PublishedPageCreation { .. } => {
                self.restore_live_page_entry(token, entry);
                drop(download);
                RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()))
            }
            LivePagePendingNavigationCompletion::CompletePageCreation { pending, .. } => {
                self.restore_live_page_entry(token, entry);
                self.finish_pending_page_creation(pending.with_pending_download(download))
                    .await
            }
            LivePagePendingNavigationCompletion::ReplyWithSnapshot {
                reply,
                capture_policy,
            } => {
                let reply = *reply;
                let reply = self.merge_pending_download_into_reply(reply, download);
                self.finish_live_page_entry_with_page_state_and_continuation(
                    token,
                    entry,
                    reply,
                    Vec::new(),
                    None,
                    capture_policy,
                )
                .await
            }
            LivePagePendingNavigationCompletion::ContinueNetworkIdle { .. }
            | LivePagePendingNavigationCompletion::ContinueDomStable { .. }
            | LivePagePendingNavigationCompletion::ContinueSubresourceResponse { .. } => {
                self.restore_live_page_entry(token, entry);
                Err(anyhow!(
                    "location navigation resolved to a download while waiting on page state"
                ))
                .into()
            }
        }
    }

    fn continue_live_page_pending_navigation(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
        stage: PageVmInitStage,
        follow_count: usize,
        completion: LivePagePendingNavigationCompletion,
    ) -> RenderRuntimeDispatchOutcome {
        if follow_count == 0 {
            entry.begin_standalone_navigation_follow();
        }
        self.restore_live_page_entry(token, entry);
        RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
            RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                token,
                stage,
                follow_count,
                completion,
            },
        ))
    }

    async fn run_live_page_command_turn(
        &self,
        token: RendererPageToken,
        command: RendererPageCommand,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        match command {
            RendererPageCommand::WaitForSelector {
                selector,
                timeout_ms,
                loader,
            } => {
                let deadline = match checked_live_page_wait_deadline(timeout_ms, "selector") {
                    Ok(deadline) => deadline,
                    Err(error) => return Err(error).into(),
                };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageSelector {
                        token,
                        selector,
                        deadline,
                        loader,
                        capture_policy,
                    },
                ))
            }
            RendererPageCommand::WaitForScriptTruthy {
                expression,
                timeout_ms,
                loader,
            } => {
                let deadline = match checked_live_page_wait_deadline(timeout_ms, "script truthy") {
                    Ok(deadline) => deadline,
                    Err(error) => return Err(error).into(),
                };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageScriptTruthy {
                        token,
                        expression,
                        pending_call: None,
                        deadline,
                        loader,
                        capture_policy,
                    },
                ))
            }
            RendererPageCommand::WaitForSubresourceResponse {
                criteria,
                timeout_ms,
                loader,
            } => {
                let deadline =
                    match checked_live_page_wait_deadline(timeout_ms, "subresource response") {
                        Ok(deadline) => deadline,
                        Err(error) => return Err(error).into(),
                    };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageSubresourceResponse {
                        token,
                        criteria,
                        deadline,
                        loader,
                        capture_policy,
                    },
                ))
            }
            RendererPageCommand::CompleteChildFrameLifecycleWorkBestEffort {
                timeout_ms,
                loader: _,
            } => {
                let deadline = match checked_live_page_wait_deadline(
                    timeout_ms,
                    "child-frame lifecycle best-effort observation",
                ) {
                    Ok(deadline) => deadline,
                    Err(error) => return Err(error).into(),
                };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageChildFrameLifecycle {
                        token,
                        deadline,
                        capture_policy,
                    },
                ))
            }
            RendererPageCommand::EvaluateExpression {
                expression,
                await_promise: true,
            } => {
                let deadline = match checked_live_page_wait_deadline(
                    LIVE_PAGE_RUNTIME_EXPRESSION_AWAIT_TIMEOUT_MS,
                    "runtime expression awaitPromise",
                ) {
                    Ok(deadline) => deadline,
                    Err(error) => return Err(error).into(),
                };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                        token,
                        execution_context_id: None,
                        expression,
                        pending_call: None,
                        deadline,
                        follow_pending_navigation: false,
                        capture_policy,
                    },
                ))
            }
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression,
                await_promise: true,
            } => {
                let deadline = match checked_live_page_wait_deadline(
                    LIVE_PAGE_RUNTIME_EXPRESSION_AWAIT_TIMEOUT_MS,
                    "runtime expression awaitPromise",
                ) {
                    Ok(deadline) => deadline,
                    Err(error) => return Err(error).into(),
                };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                        token,
                        execution_context_id: None,
                        expression,
                        pending_call: None,
                        deadline,
                        follow_pending_navigation: true,
                        capture_policy,
                    },
                ))
            }
            RendererPageCommand::EvaluateExpressionInExecutionContext {
                execution_context_id,
                expression,
                await_promise: true,
            } => {
                let deadline = match checked_live_page_wait_deadline(
                    LIVE_PAGE_RUNTIME_EXPRESSION_AWAIT_TIMEOUT_MS,
                    "runtime expression awaitPromise",
                ) {
                    Ok(deadline) => deadline,
                    Err(error) => return Err(error).into(),
                };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                        token,
                        execution_context_id: Some(execution_context_id),
                        expression,
                        pending_call: None,
                        deadline,
                        follow_pending_navigation: false,
                        capture_policy,
                    },
                ))
            }
            RendererPageCommand::EvaluateExpressionInExecutionContextAndFollowPendingNavigation {
                execution_context_id,
                expression,
                await_promise: true,
            } => {
                let deadline = match checked_live_page_wait_deadline(
                    LIVE_PAGE_RUNTIME_EXPRESSION_AWAIT_TIMEOUT_MS,
                    "runtime expression awaitPromise",
                ) {
                    Ok(deadline) => deadline,
                    Err(error) => return Err(error).into(),
                };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                        token,
                        execution_context_id: Some(execution_context_id),
                        expression,
                        pending_call: None,
                        deadline,
                        follow_pending_navigation: true,
                        capture_policy,
                    },
                ))
            }
            command => {
                return self
                    .run_live_page_command_turn_inline(token, command, capture_policy)
                    .await;
            }
        }
    }

    async fn run_live_page_command_turn_inline(
        &self,
        token: RendererPageToken,
        command: RendererPageCommand,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        if live_page_command_requires_materialized_child_realms(&command)
            && entry
                .page_vm()
                .vm()
                .has_pending_child_frame_realm_materialization()
        {
            // V8's Runtime.enable calls beginEnsureAllContextsInGroup() and
            // then reportAllContexts(). Chromium already has each live local
            // frame's ScriptState by this boundary. Moli creates child
            // realms lazily, so an earlier exact-Document materialization
            // task is a real prerequisite of that report, not an event to be
            // reconstructed from a later realm snapshot.
            //
            // Return the Page entry and let the ordinary scheduler consume
            // its typed child-frame task. The command resumes only after a
            // Page turn; this preserves the task's authorization, checkpoint
            // and concrete-output ownership instead of running its body from
            // the protocol command or manufacturing a second producer.
            self.restore_live_page_entry(token, entry);
            return RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                turn: Box::new(RenderRuntimeTurn::RunLivePageCommand {
                    token,
                    command,
                    capture_policy,
                }),
                wake_token: token,
            };
        }
        let timing_command_label = renderer_page_command_timing_label(&command);
        let timing_started = timing_command_label.and_then(|command_label| {
            moli_trace::cdp_nav_timing_enabled().then(|| {
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    command = command_label,
                    page_id = token.page_id.as_u64(),
                    stage = "owner_command_turn_start",
                );
                Instant::now()
            })
        });
        let should_follow_pending_navigation =
            live_page_command_should_follow_pending_navigation(&command);
        let scope_before_dispatch = entry.page_vm().pending_runtime_command_output_scope_id();
        let (mut entry, reply_result) = dispatch_async_command_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            command,
        )
        .await;
        if let (Some(command_label), Some(started)) = (timing_command_label, timing_started) {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                command = command_label,
                page_id = token.page_id.as_u64(),
                elapsed_ms = started.elapsed().as_millis(),
                stage = "owner_command_vm_dispatch_done",
            );
        }
        let _ = entry.slot.cancel_in_flight_command(token.page_id);
        let owned_runtime_command_scope = runtime_command_output_scope_owned_by_dispatch(
            scope_before_dispatch,
            entry.page_vm().pending_runtime_command_output_scope_id(),
        );
        let RendererPageCommandDispatch {
            reply,
            replacement_lifecycle,
            turn_records,
        } = match reply_result {
            Ok(dispatch) => dispatch,
            Err(error) => {
                if let Some(scope_id) = owned_runtime_command_scope {
                    entry
                        .page_vm_mut()
                        .abandon_pending_runtime_command_lifecycle(scope_id);
                }
                self.restore_live_page_entry(token, entry);
                return Err(error).into();
            }
        };
        if let Some(scope_id) = owned_runtime_command_scope {
            let has_pending_document_lifecycle_turn =
                has_pending_document_lifecycle_turn_on_entry(&mut entry);
            self.restore_live_page_entry(token, entry);
            if has_pending_document_lifecycle_turn {
                self.signal_internal_document_lifecycle_turn(token);
            }
            return RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                turn: Box::new(RenderRuntimeTurn::ContinueLivePageRuntimeCommandLifecycle {
                    token,
                    scope_id,
                    reply: Box::new(reply),
                    should_follow_pending_navigation,
                    turn_records,
                    capture_policy,
                }),
                wake_token: token,
            };
        }
        self.complete_live_page_command_turn(
            token,
            entry,
            reply,
            should_follow_pending_navigation,
            turn_records,
            replacement_lifecycle,
            capture_policy,
        )
        .await
    }

    async fn continue_live_page_runtime_command_lifecycle_turn(
        &self,
        token: RendererPageToken,
        scope_id: PageVmRuntimeCommandOutputScopeId,
        reply: RendererPageReply,
        should_follow_pending_navigation: bool,
        turn_records: Vec<PendingRendererOutputRecord>,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let (entry, advance_result) = advance_runtime_command_lifecycle_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            scope_id,
        )
        .await;
        match advance_result {
            Ok(PageVmRuntimeCommandLifecycleAdvance::Pending) => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                    turn: Box::new(RenderRuntimeTurn::ContinueLivePageRuntimeCommandLifecycle {
                        token,
                        scope_id,
                        reply: Box::new(reply),
                        should_follow_pending_navigation,
                        turn_records,
                        capture_policy,
                    }),
                    wake_token: token,
                }
            }
            Ok(PageVmRuntimeCommandLifecycleAdvance::Completed) => {
                self.complete_live_page_command_turn(
                    token,
                    entry,
                    reply,
                    should_follow_pending_navigation,
                    turn_records,
                    None,
                    capture_policy,
                )
                .await
            }
            Err(error) => {
                self.restore_live_page_entry(token, entry);
                Err(error).into()
            }
        }
    }

    async fn complete_live_page_command_turn(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
        reply: RendererPageReply,
        should_follow_pending_navigation: bool,
        turn_records: Vec<PendingRendererOutputRecord>,
        replacement_lifecycle: Option<DocumentLifecycleTurnOutcome>,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        if should_follow_pending_navigation
            && entry.page_vm().vm().has_pending_location_navigation()
        {
            debug_assert!(
                turn_records.is_empty(),
                "commands that follow a pending navigation cannot drop command-turn output records"
            );
            return self.continue_live_page_pending_navigation(
                token,
                entry,
                PageVmInitStage::Load,
                0,
                LivePagePendingNavigationCompletion::ReplyWithSnapshot {
                    reply: Box::new(reply),
                    capture_policy,
                },
            );
        }
        let post_response_continuation =
            match replacement_lifecycle.map(|outcome| outcome.readiness) {
                Some(
                    DocumentLifecycleTurnReadiness::Runnable { document }
                    | DocumentLifecycleTurnReadiness::Blocked { document },
                ) => {
                    if let Err(error) = entry.defer_document_lifecycle_until_response(document) {
                        self.restore_live_page_entry(token, entry);
                        return Err(error).into();
                    }
                    Some(self.post_response_document_lifecycle_continuation(token, document))
                }
                Some(DocumentLifecycleTurnReadiness::Idle) | None => None,
            };
        self.finish_live_page_entry_with_page_state_and_continuation(
            token,
            entry,
            reply,
            turn_records,
            post_response_continuation,
            capture_policy,
        )
        .await
    }

    async fn wait_live_page_network_idle_turn(
        &self,
        token: RendererPageToken,
        state: PageVmNetworkIdleWaitState,
        deadline: Instant,
        loader: ResourceRequestClient,
    ) -> RenderRuntimeDispatchOutcome {
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.restore_live_page_entry(token, entry);
            return Err(anyhow!("timed out waiting for networkidle")).into();
        }
        let (entry, wait_result) = advance_network_idle_wait_turn_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            state,
            remaining,
        )
        .await;
        match wait_result {
            Ok(PageVmNetworkIdleWaitAdvance::Completed) => {
                self.finish_live_page_entry_with_page_state(
                    token,
                    entry,
                    RendererPageReply::Unit,
                    Vec::new(),
                )
                .await
            }
            Ok(PageVmNetworkIdleWaitAdvance::TriggeredNavigation) => self
                .continue_live_page_pending_navigation(
                    token,
                    entry,
                    PageVmInitStage::Load,
                    0,
                    LivePagePendingNavigationCompletion::ContinueNetworkIdle { deadline, loader },
                ),
            Ok(PageVmNetworkIdleWaitAdvance::Progressed { state }) => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageNetworkIdle {
                        token,
                        state,
                        deadline,
                        loader,
                    },
                ))
            }
            Ok(PageVmNetworkIdleWaitAdvance::Waiting { sleep_for, state }) => {
                if Instant::now() >= deadline {
                    self.restore_live_page_entry(token, entry);
                    Err(anyhow!("timed out waiting for networkidle")).into()
                } else {
                    self.restore_live_page_entry(token, entry);
                    let ready_at = Instant::now()
                        .checked_add(sleep_for)
                        .unwrap_or(deadline)
                        .min(deadline);
                    RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                        turn: Box::new(RenderRuntimeTurn::WaitLivePageNetworkIdle {
                            token,
                            state,
                            deadline,
                            loader,
                        }),
                        wake_token: token,
                        ready_at,
                    }
                }
            }
            Err(error) => {
                self.restore_live_page_entry(token, entry);
                Err(error).into()
            }
        }
    }

    async fn wait_live_page_dom_stable_turn(
        &self,
        token: RendererPageToken,
        state: PageVmDomStableWaitState,
        deadline: Instant,
        loader: ResourceRequestClient,
    ) -> RenderRuntimeDispatchOutcome {
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.restore_live_page_entry(token, entry);
            return Err(anyhow!("timed out waiting for domstable")).into();
        }
        let (entry, wait_result) = advance_dom_stable_wait_turn_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            state,
            remaining,
        )
        .await;
        match wait_result {
            Ok(PageVmDomStableWaitAdvance::Completed) => {
                self.finish_live_page_entry_with_page_state(
                    token,
                    entry,
                    RendererPageReply::Unit,
                    Vec::new(),
                )
                .await
            }
            Ok(PageVmDomStableWaitAdvance::TriggeredNavigation) => self
                .continue_live_page_pending_navigation(
                    token,
                    entry,
                    PageVmInitStage::Load,
                    0,
                    LivePagePendingNavigationCompletion::ContinueDomStable { deadline, loader },
                ),
            Ok(PageVmDomStableWaitAdvance::Progressed { state }) => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageDomStable {
                        token,
                        state,
                        deadline,
                        loader,
                    },
                ))
            }
            Ok(PageVmDomStableWaitAdvance::Waiting { sleep_for, state }) => {
                if Instant::now() >= deadline {
                    self.restore_live_page_entry(token, entry);
                    Err(anyhow!("timed out waiting for domstable")).into()
                } else {
                    self.restore_live_page_entry(token, entry);
                    let ready_at = Instant::now()
                        .checked_add(sleep_for)
                        .unwrap_or(deadline)
                        .min(deadline);
                    RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                        turn: Box::new(RenderRuntimeTurn::WaitLivePageDomStable {
                            token,
                            state,
                            deadline,
                            loader,
                        }),
                        wake_token: token,
                        ready_at,
                    }
                }
            }
            Err(error) => {
                self.restore_live_page_entry(token, entry);
                Err(error).into()
            }
        }
    }

    async fn wait_live_page_selector_turn(
        &self,
        token: RendererPageToken,
        selector: String,
        deadline: Instant,
        loader: ResourceRequestClient,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.restore_live_page_entry(token, entry);
            return Err(anyhow!("timed out waiting for selector `{selector}`")).into();
        }
        let (entry, wait_result) = advance_selector_wait_turn_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            selector.clone(),
            remaining,
        )
        .await;
        match wait_result {
            Ok(PageVmCommandWaitAdvance::Completed { node }) => {
                self.finish_live_page_entry_with_page_state_and_continuation(
                    token,
                    entry,
                    RendererPageReply::DocumentQuerySelectorNode(node),
                    Vec::new(),
                    None,
                    capture_policy,
                )
                .await
            }
            Ok(PageVmCommandWaitAdvance::Progressed) => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageSelector {
                        token,
                        selector,
                        deadline,
                        loader,
                        capture_policy,
                    },
                ))
            }
            Ok(PageVmCommandWaitAdvance::Waiting { sleep_for }) => {
                if Instant::now() >= deadline {
                    self.restore_live_page_entry(token, entry);
                    Err(anyhow!("timed out waiting for selector `{selector}`")).into()
                } else {
                    self.restore_live_page_entry(token, entry);
                    let ready_at = Instant::now()
                        .checked_add(sleep_for)
                        .unwrap_or(deadline)
                        .min(deadline);
                    RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                        turn: Box::new(RenderRuntimeTurn::WaitLivePageSelector {
                            token,
                            selector,
                            deadline,
                            loader,
                            capture_policy,
                        }),
                        wake_token: token,
                        ready_at,
                    }
                }
            }
            Err(error) => {
                self.restore_live_page_entry(token, entry);
                Err(error).into()
            }
        }
    }

    async fn wait_live_page_child_frame_lifecycle_turn(
        &self,
        token: RendererPageToken,
        deadline: Instant,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        if entry.page_vm().child_frame_lifecycle_work_is_complete() {
            return self
                .finish_live_page_entry_with_page_state_and_continuation(
                    token,
                    entry,
                    RendererPageReply::Bool(true),
                    Vec::new(),
                    None,
                    capture_policy,
                )
                .await;
        }
        if Instant::now() >= deadline {
            return self
                .finish_live_page_entry_with_page_state_and_continuation(
                    token,
                    entry,
                    RendererPageReply::Bool(false),
                    Vec::new(),
                    None,
                    capture_policy,
                )
                .await;
        }
        self.restore_live_page_entry(token, entry);
        RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
            turn: Box::new(RenderRuntimeTurn::WaitLivePageChildFrameLifecycle {
                token,
                deadline,
                capture_policy,
            }),
            wake_token: token,
            ready_at: deadline,
        }
    }

    async fn wait_live_page_script_truthy_turn(
        &self,
        token: RendererPageToken,
        expression: String,
        pending_call: Option<PendingRuntimeEvaluateCall>,
        deadline: Instant,
        loader: ResourceRequestClient,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            entry
                .page_vm_mut()
                .cancel_pending_runtime_evaluate(pending_call);
            self.restore_live_page_entry(token, entry);
            return Err(anyhow!("timed out waiting for script to become truthy")).into();
        }
        let pending_call_for_error = pending_call;
        let wait_for = remaining.min(LIVE_PAGE_COMMAND_WAIT_TURN_SLICE);
        let (mut entry, wait_result) = advance_script_truthy_wait_turn_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            expression.clone(),
            pending_call,
            wait_for,
        )
        .await;
        match wait_result {
            Ok(PageVmScriptTruthyWaitAdvance::Completed) => {
                self.finish_live_page_entry_with_page_state_and_continuation(
                    token,
                    entry,
                    RendererPageReply::Unit,
                    Vec::new(),
                    None,
                    capture_policy,
                )
                .await
            }
            Ok(PageVmScriptTruthyWaitAdvance::Progressed { pending_call }) => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageScriptTruthy {
                        token,
                        expression,
                        pending_call,
                        deadline,
                        loader,
                        capture_policy,
                    },
                ))
            }
            Ok(PageVmScriptTruthyWaitAdvance::Waiting {
                sleep_for,
                pending_call,
            }) => {
                if Instant::now() >= deadline {
                    entry
                        .page_vm_mut()
                        .cancel_pending_runtime_evaluate(pending_call);
                    self.restore_live_page_entry(token, entry);
                    Err(anyhow!("timed out waiting for script to become truthy")).into()
                } else {
                    self.restore_live_page_entry(token, entry);
                    let ready_at = Instant::now()
                        .checked_add(sleep_for)
                        .unwrap_or(deadline)
                        .min(deadline);
                    RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                        turn: Box::new(RenderRuntimeTurn::WaitLivePageScriptTruthy {
                            token,
                            expression,
                            pending_call,
                            deadline,
                            loader,
                            capture_policy,
                        }),
                        wake_token: token,
                        ready_at,
                    }
                }
            }
            Err(error) => {
                entry
                    .page_vm_mut()
                    .cancel_pending_runtime_evaluate(pending_call_for_error);
                self.restore_live_page_entry(token, entry);
                Err(error).into()
            }
        }
    }

    async fn wait_live_page_runtime_expression_await_turn(
        &self,
        token: RendererPageToken,
        execution_context_id: Option<i64>,
        expression: String,
        pending_call: Option<PendingRuntimeEvaluateCall>,
        deadline: Instant,
        follow_pending_navigation: bool,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            entry
                .page_vm_mut()
                .cancel_pending_runtime_evaluate(pending_call);
            self.restore_live_page_entry(token, entry);
            return Err(anyhow!(
                "timed out waiting for runtime expression awaitPromise"
            ))
            .into();
        }
        let pending_call_for_error = pending_call;
        let (mut entry, wait_result) =
            advance_runtime_expression_await_turn_on_entry_via_local_task(
                self.state.local_executor.clone(),
                entry,
                execution_context_id,
                expression.clone(),
                pending_call,
                remaining,
            )
            .await;
        match wait_result {
            Ok(PageVmRuntimeExpressionAwaitAdvance::Completed { payload }) => {
                let reply = RendererPageReply::RuntimeEvaluationResult(payload);
                if follow_pending_navigation
                    && entry.page_vm().vm().has_pending_location_navigation()
                {
                    self.continue_live_page_pending_navigation(
                        token,
                        entry,
                        PageVmInitStage::Load,
                        0,
                        LivePagePendingNavigationCompletion::ReplyWithSnapshot {
                            reply: Box::new(reply),
                            capture_policy,
                        },
                    )
                } else {
                    self.finish_live_page_entry_with_page_state_and_continuation(
                        token,
                        entry,
                        reply,
                        Vec::new(),
                        None,
                        capture_policy,
                    )
                    .await
                }
            }
            Ok(PageVmRuntimeExpressionAwaitAdvance::Progressed { pending_call }) => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                        token,
                        execution_context_id,
                        expression,
                        pending_call,
                        deadline,
                        follow_pending_navigation,
                        capture_policy,
                    },
                ))
            }
            Ok(PageVmRuntimeExpressionAwaitAdvance::Waiting {
                sleep_for,
                pending_call,
            }) => {
                if Instant::now() >= deadline {
                    entry
                        .page_vm_mut()
                        .cancel_pending_runtime_evaluate(pending_call);
                    self.restore_live_page_entry(token, entry);
                    Err(anyhow!(
                        "timed out waiting for runtime expression awaitPromise"
                    ))
                    .into()
                } else {
                    self.restore_live_page_entry(token, entry);
                    let ready_at = Instant::now()
                        .checked_add(sleep_for)
                        .unwrap_or(deadline)
                        .min(deadline);
                    RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                        turn: Box::new(RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                            token,
                            execution_context_id,
                            expression,
                            pending_call,
                            deadline,
                            follow_pending_navigation,
                            capture_policy,
                        }),
                        wake_token: token,
                        ready_at,
                    }
                }
            }
            Err(error) => {
                entry
                    .page_vm_mut()
                    .cancel_pending_runtime_evaluate(pending_call_for_error);
                self.restore_live_page_entry(token, entry);
                Err(error).into()
            }
        }
    }

    async fn wait_live_page_subresource_response_turn(
        &self,
        token: RendererPageToken,
        criteria: SubresourceResponseWaitCriteria,
        deadline: Instant,
        loader: ResourceRequestClient,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.restore_live_page_entry(token, entry);
            return Err(anyhow!("timed out waiting for subresource response")).into();
        }
        let (entry, wait_result) = advance_subresource_response_wait_turn_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            criteria.clone(),
            remaining,
        )
        .await;
        match wait_result {
            Ok(PageVmSubresourceResponseWaitAdvance::Completed) => {
                self.finish_live_page_entry_with_page_state_and_continuation(
                    token,
                    entry,
                    RendererPageReply::Unit,
                    Vec::new(),
                    None,
                    capture_policy,
                )
                .await
            }
            Ok(PageVmSubresourceResponseWaitAdvance::TriggeredNavigation) => self
                .continue_live_page_pending_navigation(
                    token,
                    entry,
                    PageVmInitStage::Load,
                    0,
                    LivePagePendingNavigationCompletion::ContinueSubresourceResponse {
                        criteria,
                        deadline,
                        loader,
                        capture_policy,
                    },
                ),
            Ok(PageVmSubresourceResponseWaitAdvance::Progressed) => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageSubresourceResponse {
                        token,
                        criteria,
                        deadline,
                        loader,
                        capture_policy,
                    },
                ))
            }
            Ok(PageVmSubresourceResponseWaitAdvance::Waiting { sleep_for }) => {
                if Instant::now() >= deadline {
                    self.restore_live_page_entry(token, entry);
                    Err(anyhow!("timed out waiting for subresource response")).into()
                } else {
                    self.restore_live_page_entry(token, entry);
                    let ready_at = Instant::now()
                        .checked_add(sleep_for)
                        .unwrap_or(deadline)
                        .min(deadline);
                    RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                        turn: Box::new(RenderRuntimeTurn::WaitLivePageSubresourceResponse {
                            token,
                            criteria,
                            deadline,
                            loader,
                            capture_policy,
                        }),
                        wake_token: token,
                        ready_at,
                    }
                }
            }
            Err(error) => {
                self.restore_live_page_entry(token, entry);
                Err(error).into()
            }
        }
    }

    async fn follow_live_page_pending_location_navigation_turn(
        &self,
        token: RendererPageToken,
        stage: PageVmInitStage,
        follow_count: usize,
        completion: LivePagePendingNavigationCompletion,
    ) -> RenderRuntimeDispatchOutcome {
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        if follow_count == 0 {
            entry.begin_standalone_navigation_follow();
        }
        let retire_page_on_failure = completion.retires_page_on_navigation_failure();
        if follow_count >= MAX_PENDING_LOCATION_NAVIGATION_TURNS {
            let failure = PageNavigationOwnerFailure::TooManyChainedLocationNavigations {
                context: completion.chain_limit_error_context(),
            };
            let disposition = match completion.failure_recipient() {
                LivePageNavigationFailureRecipient::PageCreationObserver => {
                    LivePageNavigationFailureDisposition::PublishToPageCreation(failure)
                }
                LivePageNavigationFailureRecipient::Background => {
                    LivePageNavigationFailureDisposition::ReportBackground(failure)
                }
                LivePageNavigationFailureRecipient::Initiator => {
                    LivePageNavigationFailureDisposition::ReturnToInitiator(anyhow!(
                        failure.to_string()
                    ))
                }
            };
            return self
                .finish_live_page_navigation_failure(
                    token,
                    entry,
                    retire_page_on_failure,
                    disposition,
                )
                .await;
        }
        let (entry, follow_result) =
            follow_pending_location_navigation_one_turn_on_entry_via_local_task(
                self.state.local_executor.clone(),
                entry,
                stage,
            )
            .await;
        let LivePageNavigationFollowTurn {
            outcome: follow_outcome,
            document_commit,
        } = match follow_result {
            Ok(turn) => turn,
            Err(error) => {
                return self
                    .finish_live_page_navigation_failure(
                        token,
                        entry,
                        retire_page_on_failure,
                        LivePageNavigationFailureDisposition::ReturnToInitiator(error),
                    )
                    .await;
            }
        };
        let dispatch = match follow_outcome {
            LivePageNavigationFollowOutcome::Completed => {
                self.finish_live_page_navigation_completion(token, entry, completion)
                    .await
            }
            LivePageNavigationFollowOutcome::PostParseLifecycle {
                target_stage,
                outcome,
            } => {
                self.handle_live_page_post_parse_lifecycle_outcome(
                    token,
                    entry,
                    target_stage,
                    follow_count,
                    completion,
                    outcome,
                )
                .await
            }
            LivePageNavigationFollowOutcome::Download(download) => {
                self.finish_live_page_navigation_download(token, entry, completion, download)
                    .await
            }
            LivePageNavigationFollowOutcome::PendingPhaseOne { wake_token } => {
                self.restore_live_page_entry(token, entry);
                let admission =
                    pending_phase_one_admission_after_restore_on_bound_owner_local_store(
                        wake_token,
                    );
                self.signal_pending_phase_one_admission(wake_token, admission);
                RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                    turn: Box::new(
                        RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                            token,
                            follow_count,
                            completion,
                        },
                    ),
                    wake_token,
                }
            }
            LivePageNavigationFollowOutcome::TriggeredNavigation { stage } => self
                .continue_live_page_pending_navigation(
                    token,
                    entry,
                    stage,
                    follow_count + 1,
                    completion,
                ),
        };
        if let Some(document_commit) = document_commit {
            tracing::debug!(
                page_id = token.page_id.as_u64(),
                navigation_handoff = ?document_commit.navigation_handoff,
                vm_creation_id = document_commit.vm_creation_id,
                view_generation = document_commit.view_generation,
                "published standalone replacement Document"
            );
            self.signal_replacement_document_view_settled(token, document_commit.vm_creation_id);
        }
        dispatch
    }

    async fn handle_live_page_post_parse_lifecycle_outcome(
        &self,
        token: RendererPageToken,
        entry: LivePageEntry,
        target_stage: PageVmInitStage,
        follow_count: usize,
        completion: LivePagePendingNavigationCompletion,
        outcome: DocumentLifecycleTurnOutcome,
    ) -> RenderRuntimeDispatchOutcome {
        match outcome {
            DocumentLifecycleTurnOutcome {
                action: DocumentLifecycleTurnAction::RequestedTopLevelNavigation { stage, .. },
                ..
            } => {
                if completion.returns_with_pending_location_navigation() {
                    self.finish_live_page_navigation_completion(token, entry, completion)
                        .await
                } else {
                    self.continue_live_page_pending_navigation(
                        token,
                        entry,
                        stage,
                        follow_count + 1,
                        completion,
                    )
                }
            }
            DocumentLifecycleTurnOutcome {
                readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                ..
            } => {
                self.restore_live_page_entry(token, entry);
                self.signal_internal_document_lifecycle_turn(token);
                RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                    turn: Box::new(
                        RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                            token,
                            document,
                            target_stage,
                            follow_count,
                            completion,
                        },
                    ),
                    wake_token: token,
                }
            }
            DocumentLifecycleTurnOutcome {
                readiness: DocumentLifecycleTurnReadiness::Blocked { document },
                ..
            } => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                    turn: Box::new(
                        RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                            token,
                            document,
                            target_stage,
                            follow_count,
                            completion,
                        },
                    ),
                    wake_token: token,
                }
            }
            DocumentLifecycleTurnOutcome {
                readiness: DocumentLifecycleTurnReadiness::Idle,
                ..
            } => {
                self.finish_live_page_navigation_completion(token, entry, completion)
                    .await
            }
        }
    }

    async fn run_pending_turn_on_owner_local_store(
        &self,
        turn: RenderRuntimeTurn,
    ) -> RenderRuntimeDispatchOutcome {
        match turn {
            RenderRuntimeTurn::FinishHtmlCreatePage {
                requested_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                response_status,
                response_headers,
                page_vm,
                page_tasks,
                stage,
                started,
                reply_boundary,
                lifecycle_decider,
                top_level_navigation_dispatch,
                navigation_reply_policy,
            } => {
                let (pending, begin_outcome) = match self
                    .install_page_vm_and_begin_post_parse_lifecycle(
                        requested_url,
                        navigation_initiator_url,
                        navigation_redirected,
                        navigation_redirect_count,
                        response_status,
                        response_headers,
                        *page_vm,
                        page_tasks,
                        stage,
                        started,
                        reply_boundary,
                        lifecycle_decider,
                        top_level_navigation_dispatch,
                    )
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(error) => return Err(error).into(),
                };
                match begin_outcome {
                    DocumentLifecycleTurnOutcome {
                        readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                        ..
                    } => {
                        let target_stage = stage;
                        if matches!(reply_boundary, crate::RendererReplyBoundary::DocumentCommit) {
                            let token = pending.token;
                            self.signal_internal_document_lifecycle_turn(token);
                            self.publish_pending_page_creation_and_continue(
                                pending,
                                RenderRuntimePageCreationContinuation::next_turn(
                                    RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                                        token,
                                        document,
                                        target_stage,
                                        follow_count: 0,
                                        completion:
                                            LivePagePendingNavigationCompletion::PublishedPageCreation {
                                                navigation_reply_policy,
                                            },
                                    },
                                ),
                            )
                            .await
                        } else {
                            let token = pending.token;
                            self.signal_internal_document_lifecycle_turn(token);
                            RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                                wake_token: token,
                                turn: Box::new(
                                    RenderRuntimeTurn::ContinueAttachedPageCreationLifecycle {
                                        pending,
                                        document,
                                        target_stage,
                                        navigation_reply_policy,
                                    },
                                ),
                            }
                        }
                    }
                    DocumentLifecycleTurnOutcome {
                        readiness: DocumentLifecycleTurnReadiness::Blocked { document },
                        ..
                    } => {
                        let target_stage = stage;
                        if matches!(reply_boundary, crate::RendererReplyBoundary::DocumentCommit) {
                            let token = pending.token;
                            self.publish_pending_page_creation_and_continue(
                                pending,
                                RenderRuntimePageCreationContinuation::next_turn(
                                    RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                                        token,
                                        document,
                                        target_stage,
                                        follow_count: 0,
                                        completion:
                                            LivePagePendingNavigationCompletion::PublishedPageCreation {
                                                navigation_reply_policy,
                                            },
                                    },
                                ),
                            )
                            .await
                        } else {
                            RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                                wake_token: pending.token,
                                turn: Box::new(
                                    RenderRuntimeTurn::ContinueAttachedPageCreationLifecycle {
                                        pending,
                                        document,
                                        target_stage,
                                        navigation_reply_policy,
                                    },
                                ),
                            }
                        }
                    }
                    DocumentLifecycleTurnOutcome {
                        action:
                            DocumentLifecycleTurnAction::RequestedTopLevelNavigation { stage, .. },
                        ..
                    } => {
                        if matches!(reply_boundary, crate::RendererReplyBoundary::DocumentCommit) {
                            if navigation_reply_policy.returns_with_pending_navigation() {
                                self.finish_pending_page_creation(pending).await
                            } else {
                                let token = pending.token;
                                self.publish_pending_page_creation_and_continue(
                                    pending,
                                    RenderRuntimePageCreationContinuation::next_turn(
                                        RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                                            token,
                                            stage,
                                            follow_count: 0,
                                            completion:
                                                LivePagePendingNavigationCompletion::PublishedPageCreation {
                                                    navigation_reply_policy,
                                                },
                                        },
                                    ),
                                )
                                .await
                            }
                        } else if navigation_reply_policy.returns_with_pending_navigation() {
                            self.finish_pending_page_creation(pending).await
                        } else {
                            let token = pending.token;
                            let entry =
                                match take_entry_for_command_on_bound_owner_local_store(token) {
                                    Ok(entry) => entry,
                                    Err(error) => return Err(error).into(),
                                };
                            self.continue_live_page_pending_navigation(
                                token,
                                entry,
                                stage,
                                0,
                                LivePagePendingNavigationCompletion::CompletePageCreation {
                                    pending,
                                    navigation_reply_policy,
                                },
                            )
                        }
                    }
                    DocumentLifecycleTurnOutcome {
                        readiness: DocumentLifecycleTurnReadiness::Idle,
                        ..
                    } => self.finish_pending_page_creation(pending).await,
                }
            }
            RenderRuntimeTurn::ContinueAttachedPageCreationLifecycle {
                pending,
                document,
                target_stage,
                navigation_reply_policy,
            } => {
                self.continue_attached_page_creation_lifecycle_turn(
                    pending,
                    document,
                    target_stage,
                    navigation_reply_policy,
                )
                .await
            }
            RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                token,
                follow_count,
                completion,
            } => {
                let retire_page_on_failure = completion.retires_page_on_navigation_failure();
                let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
                    Ok(entry) => entry,
                    Err(error) => return Err(error).into(),
                };
                let advance = advance_pending_phase_one_navigation_on_entry_via_local_task(
                    self.state.local_executor.clone(),
                    entry,
                )
                .await;
                let (entry, advance_result) = match advance {
                    PendingPhaseOneEntryAdvance::Live { entry, result } => (entry, result),
                    PendingPhaseOneEntryAdvance::Retiring { entry, error } => {
                        return self
                            .finish_retiring_phase_one_navigation_failure(token, entry, error);
                    }
                };
                match advance_result {
                    Ok(LivePagePendingNavigationPhaseOneAdvance::Pending { wake_token }) => {
                        self.restore_live_page_entry(token, entry);
                        let admission =
                            pending_phase_one_admission_after_restore_on_bound_owner_local_store(
                                wake_token,
                            );
                        let continues_committed_document_parser_prefix = completion
                            .continues_committed_document_parser_prefix()
                            && admission == PhaseOneResidenceAdmission::ReadyPageTurn;
                        self.signal_pending_phase_one_admission(wake_token, admission);
                        let turn = Box::new(
                            RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                                token,
                                follow_count,
                                completion,
                            },
                        );
                        if continues_committed_document_parser_prefix {
                            RenderRuntimeDispatchOutcome::ContinueCommittedDocumentParserAfterPageWake {
                                turn,
                                wake_token,
                            }
                        } else {
                            RenderRuntimeDispatchOutcome::ContinueAfterPageWake { turn, wake_token }
                        }
                    }
                    Ok(LivePagePendingNavigationPhaseOneAdvance::TriggeredNavigation { stage }) => {
                        if completion.returns_with_pending_location_navigation() {
                            self.finish_live_page_navigation_completion(token, entry, completion)
                                .await
                        } else {
                            self.continue_live_page_pending_navigation(
                                token,
                                entry,
                                stage,
                                follow_count + 1,
                                completion,
                            )
                        }
                    }
                    Ok(LivePagePendingNavigationPhaseOneAdvance::PostParseLifecycle {
                        target_stage,
                        outcome: lifecycle_outcome,
                    }) => {
                        self.handle_live_page_post_parse_lifecycle_outcome(
                            token,
                            entry,
                            target_stage,
                            follow_count,
                            completion,
                            lifecycle_outcome,
                        )
                        .await
                    }
                    Err(error) => {
                        self.finish_live_page_navigation_failure(
                            token,
                            entry,
                            retire_page_on_failure,
                            LivePageNavigationFailureDisposition::ReturnToInitiator(error),
                        )
                        .await
                    }
                }
            }
            RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                token,
                document,
                target_stage,
                follow_count,
                completion,
            } => {
                self.continue_live_page_navigation_post_parse_lifecycle_turn(
                    token,
                    document,
                    target_stage,
                    follow_count,
                    completion,
                )
                .await
            }
            RenderRuntimeTurn::DrainSharedWorkerServiceLane => {
                self.state
                    .browser_context_runtime
                    .drain_shared_worker_service_lane();
                RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()))
            }
            RenderRuntimeTurn::DrainServiceWorkerServiceLane => {
                self.state
                    .browser_context_runtime
                    .drain_service_worker_service_lane();
                RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()))
            }
            RenderRuntimeTurn::RunPageTurn { token } => self.run_one_page_turn(token).await,
            RenderRuntimeTurn::RunOwnerMaintenance { task } => {
                self.run_one_owner_maintenance_turn(task).await
            }
            RenderRuntimeTurn::RunInspectorMainReceiver { wake } => {
                let Some(dispatch) = dispatch_inspector_main_owner_wake(wake) else {
                    return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
                };
                let (token, capture_policy, envelope, first_dispatch, reply_tx) =
                    dispatch.into_parts();
                let command_admission_output_predecessor =
                    renderer_output_fence_for_tail_on_bound_owner_local_store(token);
                RenderRuntimeDispatchOutcome::InspectorMainCommandClaimed {
                    reply_tx,
                    command_admission_output_predecessor,
                    turn: Box::new(RenderRuntimeTurn::RunDevToolsMainCommand {
                        token,
                        command: envelope.into_payload(),
                        first_dispatch,
                        capture_policy,
                    }),
                }
            }
            RenderRuntimeTurn::RunDevToolsMainCommand {
                token,
                command,
                mut first_dispatch,
                capture_policy,
            } => {
                // Crossing into the Page owner's concrete command dispatcher
                // is the Main receiver's first-dispatch boundary.
                let _post_dispatch_wake = first_dispatch.release_for_dispatch();
                self.run_live_page_command_turn(token, command, capture_policy)
                    .await
            }
            RenderRuntimeTurn::RunLivePageCommand {
                token,
                command,
                capture_policy,
            } => {
                self.run_live_page_command_turn(token, command, capture_policy)
                    .await
            }
            RenderRuntimeTurn::ContinueLivePageRuntimeCommandLifecycle {
                token,
                scope_id,
                reply,
                should_follow_pending_navigation,
                turn_records,
                capture_policy,
            } => {
                self.continue_live_page_runtime_command_lifecycle_turn(
                    token,
                    scope_id,
                    *reply,
                    should_follow_pending_navigation,
                    turn_records,
                    capture_policy,
                )
                .await
            }
            RenderRuntimeTurn::ResumeLivePageDocumentLifecycleAfterReply { token, document } => {
                self.signal_internal_document_lifecycle_turn_if_resident(token, document);
                RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()))
            }
            RenderRuntimeTurn::WaitLivePageNetworkIdle {
                token,
                state,
                deadline,
                loader,
            } => {
                self.wait_live_page_network_idle_turn(token, state, deadline, loader)
                    .await
            }
            RenderRuntimeTurn::WaitLivePageDomStable {
                token,
                state,
                deadline,
                loader,
            } => {
                self.wait_live_page_dom_stable_turn(token, state, deadline, loader)
                    .await
            }
            RenderRuntimeTurn::WaitLifecycleNavigation(wait) => {
                self.wait_lifecycle_navigation_turn(wait).await
            }
            RenderRuntimeTurn::WaitLivePageSelector {
                token,
                selector,
                deadline,
                loader,
                capture_policy,
            } => {
                self.wait_live_page_selector_turn(token, selector, deadline, loader, capture_policy)
                    .await
            }
            RenderRuntimeTurn::WaitLivePageScriptTruthy {
                token,
                expression,
                pending_call,
                deadline,
                loader,
                capture_policy,
            } => {
                self.wait_live_page_script_truthy_turn(
                    token,
                    expression,
                    pending_call,
                    deadline,
                    loader,
                    capture_policy,
                )
                .await
            }
            RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                token,
                execution_context_id,
                expression,
                pending_call,
                deadline,
                follow_pending_navigation,
                capture_policy,
            } => {
                self.wait_live_page_runtime_expression_await_turn(
                    token,
                    execution_context_id,
                    expression,
                    pending_call,
                    deadline,
                    follow_pending_navigation,
                    capture_policy,
                )
                .await
            }
            RenderRuntimeTurn::WaitLivePageSubresourceResponse {
                token,
                criteria,
                deadline,
                loader,
                capture_policy,
            } => {
                self.wait_live_page_subresource_response_turn(
                    token,
                    criteria,
                    deadline,
                    loader,
                    capture_policy,
                )
                .await
            }
            RenderRuntimeTurn::WaitLivePageChildFrameLifecycle {
                token,
                deadline,
                capture_policy,
            } => {
                self.wait_live_page_child_frame_lifecycle_turn(token, deadline, capture_policy)
                    .await
            }
            RenderRuntimeTurn::ClaimLivePageTopLevelNavigationHandoff { token, handoff } => {
                self.claim_top_level_navigation_handoff(token, handoff)
            }
            RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                token,
                stage,
                follow_count,
                completion,
            } => {
                self.follow_live_page_pending_location_navigation_turn(
                    token,
                    stage,
                    follow_count,
                    completion,
                )
                .await
            }
        }
    }

    async fn create_page_reply_from_html_request_on_owner_local_store(
        &self,
        request: RendererCreateHtmlPageRequest,
        _owner_local_store: &mut RendererOwnerLocalStore,
    ) -> RenderRuntimeDispatchOutcome {
        let RendererCreateHtmlPageRequest {
            page_reservation,
            root_frame_id,
            main_document_commit,
            top_level_storage_key,
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            loader,
            web_storage,
            final_url,
            html,
            document_start_scripts,
            runtime_bindings,
            runtime_inspector_session_restore_snapshots,
            runtime_isolated_worlds,
            permission_overrides,
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            bypass_content_security_policy,
            cpu_throttling_rate,
            network_offline,
            blocked_url_patterns,
            indexed_db_manager,
            storage_bucket_store,
            emulated_media,
            idle_override,
            viewport_surface,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            layout_policy,
            wpt_extensions_enabled,
            stage,
            reply_boundary,
            lifecycle_decider,
            top_level_navigation_dispatch,
            reserved_service_worker_client,
        } = request;
        if page_reservation.local_host_id() != self.state.owner_local_host_id {
            return Err(anyhow!(
                "page reservation belongs to renderer owner {}, not {}",
                page_reservation.local_host_id().as_u64(),
                self.state.owner_local_host_id.as_u64()
            ))
            .into();
        }
        if lifecycle_decider.is_some()
            && (!matches!(reply_boundary, crate::RendererReplyBoundary::Stage)
                || !matches!(
                    top_level_navigation_dispatch,
                    RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter
                ))
        {
            return Err(anyhow!(
                "a lifecycle decider requires a standalone lifecycle-boundary page creation"
            ))
            .into();
        }
        let loader = loader_for_new_page(
            &loader,
            &extra_http_headers,
            network_offline,
            &blocked_url_patterns,
        );
        if response_headers_indicate_download(&response_headers) {
            return Err(anyhow!(
                "external raw streaming page request received download headers; CDP navigation must branch downloads before renderer page creation"
            ))
            .into();
        }
        let document_content_security_policies = if bypass_content_security_policy {
            Vec::new()
        } else {
            crate::content_security_policy::content_security_policy_headers(&response_headers)
        };
        let response_content_security_policies = if bypass_content_security_policy {
            Vec::new()
        } else {
            response_content_security_policies_from_headers(&response_headers)
        };
        let response_content_security_report_only_policies = if bypass_content_security_policy {
            Vec::new()
        } else {
            response_content_security_report_only_policies_from_headers(&response_headers)
        };
        let response_referrer_policy = response_referrer_policy_from_headers(&response_headers);
        let content_security_reporting_endpoints = if bypass_content_security_policy {
            Default::default()
        } else {
            crate::content_security_policy::content_security_policy_reporting_endpoints_from_headers(
                &response_headers,
                &final_url,
            )
        };
        let cross_origin_embedder_policy =
            crate::cross_origin_isolation::cross_origin_embedder_policy_from_headers(
                &response_headers,
            );
        let document_isolation_policy =
            crate::cross_origin_isolation::document_isolation_policy_from_headers(
                &response_headers,
            );
        let cross_origin_isolated =
            crate::cross_origin_isolation::response_headers_enable_cross_origin_isolation(
                &final_url,
                &response_headers,
            );
        let document_default_language =
            crate::document_language::document_default_language_from_headers(&response_headers);
        let document_last_modified =
            crate::document_last_modified::document_last_modified_from_headers(&response_headers);
        let owner = self.clone();
        let phase_one_result = self
            .run_owner_lane_local_task(async move {
                let page_id = page_reservation.page_id();
                let owner_local_context = owner.owner_local_context()?;
                let owner_wake = owner.owner_wake_sender_for_page(&owner_local_context, page_id);
                let renderer_document_isolate_allocator =
                    RendererDocumentIsolateAllocator::new(owner_local_context.clone(), page_id);
                let runtime_hooks = PageVmRuntimeHooks::with_owner_wake(
                    owner_wake,
                    owner.state.browser_context_runtime.clone(),
                )
                .with_renderer_document_isolate_allocator(renderer_document_isolate_allocator);
                let local_executor = owner.state.local_executor.clone();
                debug!(stage = ?stage, %final_url, "starting page VM creation from html");
                let env = PageVmEnvConfig {
                    web_storage,
                    document_start_scripts,
                    runtime_bindings,
                    runtime_inspector_session_restore_snapshots,
                    runtime_isolated_worlds,
                    permission_overrides,
                    extra_http_headers,
                    document_content_security_policies,
                    response_content_security_policies,
                    response_content_security_report_only_policies,
                    response_referrer_policy,
                    content_security_reporting_endpoints,
                    cross_origin_embedder_policy,
                    document_isolation_policy,
                    cross_origin_isolated,
                    document_default_language,
                    document_last_modified,
                    locale_override,
                    timezone_override,
                    script_execution_disabled,
                    bypass_content_security_policy,
                    cpu_throttling_rate,
                    emulated_media,
                    idle_override,
                    viewport_surface,
                    network_offline,
                    blocked_url_patterns,
                    indexed_db_manager,
                    storage_bucket_store,
                    fetch_subresource_interception_enabled,
                    fetch_subresource_interception_resource_type,
                    layout_policy,
                    wpt_extensions_enabled,
                    root_frame_id,
                    main_document_commit,
                    top_level_storage_key,
                    navigation_bootstrap_entry: None,
                    reserved_service_worker_client_id: reserved_service_worker_client
                        .map(RendererReservedServiceWorkerClient::release),
                };
                let bootstrap = Box::pin(async move {
                    let started = Instant::now();
                    ConcurrentParseTimeRuntime::finish_creation_from_html_bootstrap(
                        page_id,
                        local_executor,
                        &loader,
                        &env,
                        runtime_hooks,
                        final_url,
                        stage,
                        html,
                        started,
                    )
                    .await
                });
                PageVm::run_bootstrap_future_on_fresh_local_task(
                    owner.state.local_executor.clone(),
                    "create-page bootstrap local task channel closed",
                    bootstrap,
                )
                .await
            })
            .await;
        match phase_one_result {
            Ok(ParseTimePageVmCreationOutcome::PendingPhaseOne(residence))
                if !matches!(&residence, PendingPhaseOneResidence::OpenStreaming(_)) =>
            {
                self.continue_pending_phase_one_page_creation(
                    requested_url,
                    navigation_initiator_url,
                    navigation_redirected,
                    navigation_redirect_count,
                    response_status,
                    response_headers,
                    PageVmPendingPhaseOneNavigation::new(
                        residence,
                        PageVmFollowedNavigationMetadata::default(),
                    ),
                    stage,
                    reply_boundary,
                    lifecycle_decider,
                    top_level_navigation_dispatch,
                    page_creation_navigation_reply_policy(top_level_navigation_dispatch),
                )
                .await
            }
            Ok(ParseTimePageVmCreationOutcome::PendingPhaseOne(_)) => Err(anyhow!(
                "full-body page creation cannot retain an open Document stream"
            ))
            .into(),
            Ok(ParseTimePageVmCreationOutcome::TriggeredNavigation { page_vm, stage }) => {
                self.continue_page_creation_with_pending_navigation(
                    requested_url,
                    navigation_initiator_url,
                    navigation_redirected,
                    navigation_redirect_count,
                    response_status,
                    response_headers,
                    page_vm,
                    stage,
                    reply_boundary,
                    lifecycle_decider,
                    top_level_navigation_dispatch,
                    page_creation_navigation_reply_policy(top_level_navigation_dispatch),
                )
                .await
            }
            Ok(ParseTimePageVmCreationOutcome::ContinuePhaseTwo {
                page_vm,
                page_tasks,
                stage,
                started,
            }) => RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                RenderRuntimeTurn::FinishHtmlCreatePage {
                    requested_url,
                    navigation_initiator_url,
                    navigation_redirected,
                    navigation_redirect_count,
                    response_status,
                    response_headers,
                    page_vm: Box::new(page_vm),
                    page_tasks,
                    stage,
                    started,
                    reply_boundary,
                    lifecycle_decider,
                    top_level_navigation_dispatch,
                    navigation_reply_policy: page_creation_navigation_reply_policy(
                        top_level_navigation_dispatch,
                    ),
                },
            )),
            Err(error) => Err(error).into(),
        }
    }

    async fn prepare_renderer_document_on_owner_local_store(
        &self,
        token: RendererPageReservationToken,
        request: RendererCreateStreamingRawPageRequest,
        owner_local_store: &mut RendererOwnerLocalStore,
    ) -> RenderRuntimeDispatchOutcome {
        if token.local_host_id() != self.state.owner_local_host_id {
            return Err(anyhow!(
                "prepared document belongs to renderer owner {}, not {}",
                token.local_host_id().as_u64(),
                self.state.owner_local_host_id.as_u64()
            ))
            .into();
        }
        let owner = self.clone();
        let residence = self
            .run_owner_lane_local_task(async move {
                let owner_local_context = owner.owner_local_context()?;
                let owner_wake =
                    owner.owner_wake_sender_for_page(&owner_local_context, token.page_id());
                let page_runtime_task_source =
                    crate::page_task_queue::PageRuntimeTaskSource::new(Some(owner_wake));
                let isolate_allocator =
                    RendererDocumentIsolateAllocator::new(owner_local_context, token.page_id());
                let (isolate_bootstrap, isolate_reservation) = isolate_allocator
                    .reserve_renderer_document_isolate(page_runtime_task_source)?;
                Ok(RendererPreparedDocumentResidence {
                    request,
                    isolate_allocator,
                    isolate_bootstrap,
                    isolate_reservation,
                })
            })
            .await;
        match residence {
            Ok(residence) => {
                let renderer_devtools_agent_token =
                    residence.isolate_bootstrap.renderer_devtools_agent_token();
                match owner_local_store.store_prepared_document(token, residence) {
                    Ok(()) => Ok(RendererOwnerReply::PreparedRendererDocumentStored {
                        renderer_devtools_agent_token,
                    })
                    .into(),
                    Err(error) => Err(error).into(),
                }
            }
            Err(error) => Err(error).into(),
        }
    }

    async fn create_page_reply_from_prepared_document_on_owner_local_store(
        &self,
        page_id: PageId,
        residence: RendererPreparedDocumentResidence,
        owner_local_store: &mut RendererOwnerLocalStore,
    ) -> RenderRuntimeDispatchOutcome {
        let RendererPreparedDocumentResidence {
            request,
            isolate_allocator,
            isolate_bootstrap,
            isolate_reservation,
        } = residence;
        self.create_page_reply_from_streaming_raw_request_on_owner_local_store(
            page_id,
            request,
            isolate_allocator,
            isolate_bootstrap,
            isolate_reservation,
            owner_local_store,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_page_reply_from_streaming_raw_request_on_owner_local_store(
        &self,
        page_id: PageId,
        request: RendererCreateStreamingRawPageRequest,
        isolate_allocator: RendererDocumentIsolateAllocator,
        isolate_bootstrap: RendererDocumentIsolateBootstrap,
        isolate_reservation: RendererDocumentIsolateReservation,
        _owner_local_store: &mut RendererOwnerLocalStore,
    ) -> RenderRuntimeDispatchOutcome {
        let RendererCreateStreamingRawPageRequest {
            root_frame_id,
            main_document_commit,
            requested_url,
            final_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            loader,
            web_storage,
            raw_body,
            document_start_scripts,
            runtime_bindings,
            runtime_inspector_session_restore_snapshots,
            runtime_isolated_worlds,
            permission_overrides,
            extra_http_headers,
            locale_override,
            timezone_override,
            script_execution_disabled,
            bypass_content_security_policy,
            cpu_throttling_rate,
            network_offline,
            blocked_url_patterns,
            indexed_db_manager,
            storage_bucket_store,
            emulated_media,
            idle_override,
            viewport_surface,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            layout_policy,
            wpt_extensions_enabled,
            stage,
            reply_boundary,
            lifecycle_decider,
            top_level_navigation_dispatch,
            navigation_reply_policy,
            reserved_service_worker_client,
        } = request;
        if lifecycle_decider.is_some()
            && (!matches!(reply_boundary, crate::RendererReplyBoundary::Stage)
                || !matches!(
                    top_level_navigation_dispatch,
                    RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter
                )
                || !matches!(
                    navigation_reply_policy,
                    NavigationReplyPolicy::FollowBeforeReply
                ))
        {
            return Err(anyhow!(
                "a lifecycle decider requires standalone follow-before-reply page creation"
            ))
            .into();
        }
        let loader = loader_for_new_page(
            &loader,
            &extra_http_headers,
            network_offline,
            &blocked_url_patterns,
        );
        let document_content_security_policies = if bypass_content_security_policy {
            Vec::new()
        } else {
            crate::content_security_policy::content_security_policy_headers(&response_headers)
        };
        let response_content_security_policies = if bypass_content_security_policy {
            Vec::new()
        } else {
            response_content_security_policies_from_headers(&response_headers)
        };
        let response_content_security_report_only_policies = if bypass_content_security_policy {
            Vec::new()
        } else {
            response_content_security_report_only_policies_from_headers(&response_headers)
        };
        let response_referrer_policy = response_referrer_policy_from_headers(&response_headers);
        let content_security_reporting_endpoints = if bypass_content_security_policy {
            Default::default()
        } else {
            crate::content_security_policy::content_security_policy_reporting_endpoints_from_headers(
                &response_headers,
                &final_url,
            )
        };
        let cross_origin_embedder_policy =
            crate::cross_origin_isolation::cross_origin_embedder_policy_from_headers(
                &response_headers,
            );
        let document_isolation_policy =
            crate::cross_origin_isolation::document_isolation_policy_from_headers(
                &response_headers,
            );
        let cross_origin_isolated =
            crate::cross_origin_isolation::response_headers_enable_cross_origin_isolation(
                &final_url,
                &response_headers,
            );
        let document_default_language =
            crate::document_language::document_default_language_from_headers(&response_headers);
        let document_last_modified =
            crate::document_last_modified::document_last_modified_from_headers(&response_headers);
        let owner = self.clone();
        let phase_one_result = self
            .run_owner_lane_local_task(async move {
                let owner_local_context = owner.owner_local_context()?;
                let owner_wake = owner.owner_wake_sender_for_page(&owner_local_context, page_id);
                let runtime_hooks = PageVmRuntimeHooks::with_owner_wake(
                    owner_wake,
                    owner.state.browser_context_runtime.clone(),
                )
                .with_renderer_document_isolate_allocator(isolate_allocator)
                .with_prepared_renderer_document_isolate(isolate_bootstrap, isolate_reservation)?;
                let local_executor = owner.state.local_executor.clone();
                let env = PageVmEnvConfig {
                    web_storage,
                    document_start_scripts,
                    runtime_bindings,
                    runtime_inspector_session_restore_snapshots,
                    runtime_isolated_worlds,
                    permission_overrides,
                    extra_http_headers,
                    document_content_security_policies,
                    response_content_security_policies,
                    response_content_security_report_only_policies,
                    response_referrer_policy,
                    content_security_reporting_endpoints,
                    cross_origin_embedder_policy,
                    document_isolation_policy,
                    cross_origin_isolated,
                    document_default_language,
                    document_last_modified,
                    locale_override,
                    timezone_override,
                    script_execution_disabled,
                    bypass_content_security_policy,
                    cpu_throttling_rate,
                    emulated_media,
                    idle_override,
                    viewport_surface,
                    network_offline,
                    blocked_url_patterns,
                    indexed_db_manager,
                    storage_bucket_store,
                    fetch_subresource_interception_enabled,
                    fetch_subresource_interception_resource_type,
                    layout_policy,
                    wpt_extensions_enabled,
                    root_frame_id,
                    main_document_commit,
                    top_level_storage_key: None,
                    navigation_bootstrap_entry: None,
                    reserved_service_worker_client_id: reserved_service_worker_client
                        .map(RendererReservedServiceWorkerClient::release),
                };
                let bootstrap = Box::pin(async move {
                    let started = Instant::now();
                    ConcurrentParseTimeRuntime::create_external_raw_document_response_at_reply_boundary(
                        page_id,
                        local_executor,
                        &loader,
                        &env,
                        runtime_hooks,
                        stage,
                        started,
                        final_url,
                        response_status,
                        response_headers,
                        raw_body,
                        reply_boundary,
                    )
                    .await
                });
                PageVm::run_bootstrap_future_on_fresh_local_task(
                    owner.state.local_executor.clone(),
                    "create-page external raw streaming bootstrap local task channel closed",
                    bootstrap,
                )
                .await
            })
            .await;
        match phase_one_result {
            Ok(StreamingNavigationPageCreationResult::Download(_)) => Err(anyhow!(
                "external raw streaming page request produced a download; CDP navigation must branch downloads before renderer page creation"
            ))
            .into(),
            Ok(StreamingNavigationPageCreationResult::Html(result)) => {
                let StreamingHtmlPageCreationResult {
                    response_status,
                    response_headers,
                    outcome,
                } = *result;
                match outcome {
                    ParseTimePageVmCreationOutcome::PendingPhaseOne(residence) => {
                        self.continue_pending_phase_one_page_creation(
                            requested_url,
                            navigation_initiator_url,
                            navigation_redirected,
                            navigation_redirect_count,
                            response_status,
                            response_headers,
                            PageVmPendingPhaseOneNavigation::new(
                                residence,
                                PageVmFollowedNavigationMetadata::default(),
                            ),
                            stage,
                            reply_boundary,
                            lifecycle_decider,
                            top_level_navigation_dispatch,
                            navigation_reply_policy,
                        )
                        .await
                    }
                    ParseTimePageVmCreationOutcome::TriggeredNavigation { page_vm, stage } => {
                        self.continue_page_creation_with_pending_navigation(
                            requested_url,
                            navigation_initiator_url,
                            navigation_redirected,
                            navigation_redirect_count,
                            response_status,
                            response_headers,
                            page_vm,
                            stage,
                            reply_boundary,
                            lifecycle_decider,
                            top_level_navigation_dispatch,
                            navigation_reply_policy,
                        )
                        .await
                    }
                    ParseTimePageVmCreationOutcome::ContinuePhaseTwo {
                        page_vm,
                        page_tasks,
                        stage,
                        started,
                    } => RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                        RenderRuntimeTurn::FinishHtmlCreatePage {
                            requested_url,
                            navigation_initiator_url,
                            navigation_redirected,
                            navigation_redirect_count,
                            response_status,
                            response_headers,
                            page_vm: Box::new(page_vm),
                            page_tasks,
                            stage,
                            started,
                            reply_boundary,
                            lifecycle_decider,
                            top_level_navigation_dispatch,
                            navigation_reply_policy,
                        },
                    )),
                }
            }
            Err(error) => Err(error).into(),
        }
    }

    pub fn record(&self, page_id: PageId) -> Option<RendererPageRecord> {
        self.state.page_table.record(page_id)
    }

    pub fn len(&self) -> usize {
        self.state.page_table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn command_epoch(&self, page_id: PageId) -> Option<u64> {
        self.state.page_table.command_epoch(page_id)
    }

    pub fn in_flight_command_epoch(&self, page_id: PageId) -> Option<u64> {
        self.state.page_table.in_flight_command_epoch(page_id)
    }
}

async fn sleep_until_or_forever(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending::<()>().await,
    }
}

fn earliest_deadline(left: Option<Instant>, right: Option<Instant>) -> Option<Instant> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(deadline), None) | (None, Some(deadline)) => Some(deadline),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::owner_maintenance::RendererPageOwnerMaintenanceResidence;

    #[test]
    fn live_page_wait_deadline_does_not_convert_huge_timeout_to_now() {
        let before = Instant::now();
        let result = checked_live_page_wait_deadline(u64::MAX, "selector");

        match result {
            Ok(deadline) => assert!(
                deadline > before,
                "huge timeout must not become an already-expired deadline"
            ),
            Err(error) => assert_eq!(error.to_string(), "selector timeout is too large"),
        }
    }

    #[test]
    fn live_page_wait_deadline_accepts_regular_timeout() {
        let deadline = checked_live_page_wait_deadline(1, "selector")
            .expect("small timeout should fit in Instant range");

        assert!(deadline > Instant::now());
    }

    #[test]
    fn shared_worker_service_lane_turn_does_not_yield_to_ready_commands() {
        assert!(
            !RenderRuntimeTurn::DrainSharedWorkerServiceLane
                .page_turn_should_yield_to_ready_command(),
            "SharedWorker load completions can make page commands ready, so the service lane must not be starved by command polling"
        );
    }

    #[test]
    fn page_turn_allows_one_ready_command_overtake() {
        let turn = RenderRuntimeTurn::RunPageTurn {
            token: RendererPageToken::new_for_testing(PageId::new_for_testing(7)),
        };

        assert!(
            turn.page_turn_should_yield_to_ready_command(),
            "ordinary detached page activity should still let ready commands run between turns"
        );
    }

    #[test]
    fn replacement_view_waiter_ignores_generic_activity_and_wrong_document_settlement() {
        let condition = RenderRuntimeParkCondition::ReplacementDocumentViewSettlement {
            expected_vm_creation_id: 41,
        };

        assert!(!condition.admits_page_activity());
        assert!(!condition.admits_replacement_view_settlement(40));
        assert!(condition.admits_replacement_view_settlement(41));
        assert!(RenderRuntimeParkCondition::PageActivity.admits_page_activity());
    }

    #[test]
    fn owner_maintenance_turn_has_a_bounded_pending_lane() {
        let now = Instant::now();
        let token = RendererPageToken::new_for_testing(PageId::new_for_testing(9));
        let mut residence = RendererPageOwnerMaintenanceResidence::new(now);
        let deadline = residence
            .indexed_deadline()
            .expect("maintenance residence should publish a deadline");
        let task = residence
            .claim_if_due(token, deadline)
            .expect("maintenance deadline should be claimable");
        let mut pending = RenderRuntimePendingTurnQueue::default();

        pending.push_back(RenderRuntimePendingTurn {
            reply_tx: None,
            turn: RenderRuntimeTurn::RunOwnerMaintenance { task },
            allow_command_overtake: true,
            command_admission_output_predecessor: None,
        });

        assert!(pending.has_owner_maintenance_turn());
        let turn = pending
            .pop_front()
            .expect("maintenance lane should retain its admitted turn");
        assert!(turn.is_owner_maintenance_turn());
        assert!(
            turn.turn.page_turn_should_yield_to_ready_command(),
            "housekeeping may let one ready command overtake before it runs"
        );
        assert!(!pending.has_owner_maintenance_turn());
    }

    #[test]
    fn runtime_command_lifecycle_scope_is_owned_only_by_its_creating_dispatch() {
        let existing = PageVmRuntimeCommandOutputScopeId(11);
        let created = PageVmRuntimeCommandOutputScopeId(12);

        assert_eq!(
            runtime_command_output_scope_owned_by_dispatch(None, Some(created)),
            Some(created)
        );
        assert_eq!(
            runtime_command_output_scope_owned_by_dispatch(Some(existing), Some(existing)),
            None,
            "an unrelated page command must not inherit an existing Runtime lifecycle scope"
        );
        assert_eq!(
            runtime_command_output_scope_owned_by_dispatch(Some(existing), Some(created)),
            Some(created),
            "a newly installed scope must remain attributable to the dispatch that replaced it"
        );
    }
}
