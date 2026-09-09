use super::access::run_named_owner_local_task;
use super::document_lifecycle_turn::DocumentLifecycleObserverOutcome;
use super::navigation::{
    PageCreationNavigationFailurePublication, PageCreationResolution, PageNavigationOwnerFailure,
};
use super::owner_local::RendererAttachedPage;
use super::owner_local_store::{
    CommittedNavigationEntry, LivePageEntry, LivePageEntryCheckoutError,
    LivePageNavigationFailureRecipient, LivePageNavigationFollowEntryAdvance,
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
use crate::document_runtime::DocumentPolicyContainer;
use crate::page_task_queue::{
    PostParsePageOwnedWork, RendererOwnerWake, RendererOwnerWakeSender, RendererOwnerWakeSource,
    RendererTopLevelNavigationHandoff,
};
use crate::render_runtime::{RenderRuntimeEnvelope, RenderRuntimeHandle, RenderRuntimeOwner};
use crate::script_vm::{
    PendingRuntimeEvaluateCall, RendererDocumentIsolateBootstrap, RuntimeEvaluateResultMode,
    dispatch_inspector_io_owner_wake, dispatch_inspector_main_owner_wake,
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
mod owner_commands;
mod owner_loop;
mod page_commands;
mod page_creation;
mod page_turn_completion;
mod scheduler;

use self::lifecycle_decision::PendingLifecycleNavigation;

#[derive(Debug, Clone)]
pub struct RendererPreparedDocumentCommitConfiguration {
    pub document_start_scripts: Vec<DocumentStartScript>,
    pub runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    pub runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
    pub runtime_isolated_worlds: Vec<crate::protocol_types::RuntimeIsolatedWorldDefinition>,
    pub permission_overrides: Vec<crate::protocol_types::PermissionOverrideRegistration>,
    pub extra_http_headers: moli_fetch::RequestHeaders,
    pub script_execution_disabled: bool,
    pub bypass_content_security_policy: bool,
    pub emulated_media: crate::protocol_types::EmulatedMediaOverrides,
    pub idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    pub navigator_overrides: moli_page_types::NavigatorOverrides,
    pub viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    pub document_activity: moli_page_types::DocumentActivity,
    pub browser_resource_runtime: crate::network::BrowserResourceRuntime,
    pub navigator_identity: moli_browser_profile::BrowserIdentityProfile,
    pub network_offline: bool,
    pub bypass_service_worker: bool,
    pub cache_disabled: bool,
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
    pub response_headers: Vec<(String, Vec<u8>)>,
    pub loader: ResourceRequestClient,
    pub navigator_identity: moli_browser_profile::BrowserIdentityProfile,
    pub web_storage: crate::RendererWebStorageHandles,
    pub final_url: Url,
    pub html: String,
    pub document_start_scripts: Vec<DocumentStartScript>,
    pub runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    pub runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
    pub runtime_isolated_worlds: Vec<crate::protocol_types::RuntimeIsolatedWorldDefinition>,
    pub permission_overrides: Vec<crate::protocol_types::PermissionOverrideRegistration>,
    pub extra_http_headers: moli_fetch::RequestHeaders,
    pub script_execution_disabled: bool,
    pub bypass_content_security_policy: bool,
    pub network_offline: bool,
    pub blocked_url_patterns: Vec<String>,
    pub indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    pub storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    pub emulated_media: crate::protocol_types::EmulatedMediaOverrides,
    pub idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    pub navigator_overrides: moli_page_types::NavigatorOverrides,
    pub viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    pub document_activity: moli_page_types::DocumentActivity,
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
    pub(super) document_replacement:
        Option<Arc<super::document_replacement::RendererDocumentReplacementScope>>,
    pub root_frame_id: Option<String>,
    pub main_document_commit: Option<RendererMainDocumentCommit>,
    pub requested_url: Url,
    pub final_url: Url,
    pub navigation_initiator_url: Option<Url>,
    pub navigation_redirected: bool,
    pub navigation_redirect_count: usize,
    pub navigation_redirect_chain: Vec<crate::protocol_types::NavigationRedirect>,
    pub response_status: u16,
    pub response_headers: Vec<(String, Vec<u8>)>,
    pub loader: ResourceRequestClient,
    pub navigator_identity: moli_browser_profile::BrowserIdentityProfile,
    pub web_storage: crate::RendererWebStorageHandles,
    pub raw_body: ExternalRawDocumentBodyStream,
    pub document_start_scripts: Vec<DocumentStartScript>,
    pub runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    pub runtime_inspector_session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
    pub runtime_isolated_worlds: Vec<crate::protocol_types::RuntimeIsolatedWorldDefinition>,
    pub permission_overrides: Vec<crate::protocol_types::PermissionOverrideRegistration>,
    pub extra_http_headers: moli_fetch::RequestHeaders,
    pub script_execution_disabled: bool,
    pub bypass_content_security_policy: bool,
    pub network_offline: bool,
    pub blocked_url_patterns: Vec<String>,
    pub indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    pub storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    pub emulated_media: crate::protocol_types::EmulatedMediaOverrides,
    pub idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    pub navigator_overrides: moli_page_types::NavigatorOverrides,
    pub viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    pub document_activity: moli_page_types::DocumentActivity,
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
    /// disconnected a DevTools session and suspended both of its ingress lanes.
    /// Replacement frontend work remains queued behind this lifecycle task so
    /// it cannot reuse the V8 session before destruction completes.
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
    RuntimeInspectorSessionResponseSettled {
        output: Box<RendererCommandTurnOutput>,
        response_succeeded: bool,
    },
    RuntimeInspectorSessionErrorSettled(RendererOutputFence),
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
    /// One ordinary Page scheduler turn has restored its stable residence.
    /// The admission bit is carried across that restore so the owner can join
    /// a selected parser task directly to its logical phase-one consumer.
    PageTurnComplete {
        result: Result<()>,
        parser_continuation_admitted: bool,
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

impl LivePageNavigationFailureDisposition {
    fn publish_page_creation_failure(
        &self,
        token: RendererPageToken,
    ) -> Option<Result<PageCreationNavigationFailurePublication>> {
        match self {
            Self::PublishToPageCreation(failure) => Some(
                publish_page_navigation_failure_on_bound_owner_local_store(token, *failure),
            ),
            Self::ReturnToInitiator(_) | Self::ReportBackground(_) => None,
        }
    }

    fn into_dispatch_outcome(
        self,
        token: RendererPageToken,
        page_creation_publication: Option<Result<PageCreationNavigationFailurePublication>>,
    ) -> RenderRuntimeDispatchOutcome {
        match self {
            Self::PublishToPageCreation(failure) => {
                match page_creation_publication
                    .expect("Page-creation failure disposition must publish before restoration")
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
            Self::ReturnToInitiator(error) => Err(error).into(),
            Self::ReportBackground(failure) => {
                RenderRuntimeDispatchOutcome::BackgroundComplete(Err(anyhow!(failure.to_string())))
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

    fn promote_phase_one_parser_continuation(&mut self, token: RendererPageToken) -> bool {
        let Some(index) = self
            .turns
            .iter()
            .position(|pending| pending.turn.is_phase_one_continuation_for(token))
        else {
            return false;
        };
        let mut pending = self
            .turns
            .remove(index)
            .expect("selected phase-one continuation must remain pending");
        self.page_owner_turn_count -= usize::from(pending.is_page_owner_turn());
        self.owner_maintenance_turn_count -= usize::from(pending.is_owner_maintenance_turn());
        pending.allow_command_overtake = false;
        self.push_front(pending);
        true
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuntimeEvaluationNavigationPolicy {
    DoNotFollow,
    Follow,
}

impl RuntimeEvaluationNavigationPolicy {
    const fn follows_pending_navigation(self) -> bool {
        matches!(self, Self::Follow)
    }
}

struct RuntimeExpressionAwaitSpec {
    execution_context_id: Option<i64>,
    expression: String,
    result_mode: RuntimeEvaluateResultMode,
    navigation_policy: RuntimeEvaluationNavigationPolicy,
}

struct RendererPostResponseOwnerWork {
    document_lifecycle: Option<RendererDocumentLifecycleIdentity>,
    navigation_handoff: Option<RendererTopLevelNavigationHandoff>,
}

impl RendererPostResponseOwnerWork {
    const fn is_empty(&self) -> bool {
        self.document_lifecycle.is_none() && self.navigation_handoff.is_none()
    }
}

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
        response_headers: Vec<(String, Vec<u8>)>,
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
        result_mode: RuntimeEvaluateResultMode,
        navigation_policy: RuntimeEvaluationNavigationPolicy,
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

    fn is_phase_one_continuation_for(&self, token: RendererPageToken) -> bool {
        matches!(
            self,
            Self::ContinueLivePagePendingLocationNavigationPhaseOne {
                token: continuation_token,
                ..
            } if *continuation_token == token
        )
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
    match command {
        RendererPageCommand::Native(command) => {
            live_page_command_requires_materialized_child_realms(&command.operation.command)
        }
        RendererPageCommand::Inspector(envelope) => envelope.requires_materialized_child_realms(),
        _ => false,
    }
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
    pub(super) devtools_target_shutdown_registry:
        crate::devtools::target::RendererDevToolsTargetShutdownRegistry,
    pub(super) owner_local_host_id: RendererOwnerLocalHostId,
    layout_policy: Mutex<RendererOwnerLayoutPolicyState>,
    context_shutdown_notify: tokio::sync::Notify,
    #[cfg(test)]
    command_dispatch_gate: Mutex<Option<RendererCommandDispatchGateForTesting>>,
    #[cfg(test)]
    publish_next_command_output_before_settlement: std::sync::atomic::AtomicBool,
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
        Some(RendererOwnerWakeSource::BitmapTask) => "bitmap-task-wake",
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

fn loader_for_new_document(
    loader: &ResourceRequestClient,
    extra_http_headers: &moli_fetch::RequestHeaders,
    network_offline: bool,
    blocked_url_patterns: &[String],
) -> ResourceRequestClient {
    // The NavigationEngine already supplies a target-owned loader. A new
    // Document isolates mutable request policy while retaining that stable
    // target's renderer memory-cache partition across navigation.
    let document_loader = loader.fork_with_isolated_document_network_policy();
    document_loader.set_extra_http_headers(extra_http_headers);
    document_loader.set_network_offline(network_offline);
    document_loader.set_blocked_url_patterns(blocked_url_patterns);
    document_loader
}

impl RendererOwnerHandle {}

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
    fn selected_parser_admission_precedes_an_ordinary_same_page_turn() {
        let parser_token = RendererPageToken::new_for_testing(PageId::new_for_testing(8));
        let unrelated_token = RendererPageToken::new_for_testing(PageId::new_for_testing(9));
        let mut pending = RenderRuntimePendingTurnQueue::default();
        pending.push_back(RenderRuntimePendingTurn {
            reply_tx: None,
            turn: RenderRuntimeTurn::RunPageTurn {
                token: parser_token,
            },
            allow_command_overtake: true,
            command_admission_output_predecessor: None,
        });
        pending.push_back(RenderRuntimePendingTurn {
            reply_tx: None,
            turn: RenderRuntimeTurn::RunPageTurn {
                token: unrelated_token,
            },
            allow_command_overtake: true,
            command_admission_output_predecessor: None,
        });
        pending.push_back(RenderRuntimePendingTurn {
            reply_tx: None,
            turn: RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                token: parser_token,
                follow_count: 0,
                completion: LivePagePendingNavigationCompletion::Background,
            },
            allow_command_overtake: true,
            command_admission_output_predecessor: None,
        });

        assert!(pending.promote_phase_one_parser_continuation(parser_token));
        let promoted = pending
            .pop_front()
            .expect("admitted parser must retain its logical driver");
        assert!(promoted.turn.is_phase_one_continuation_for(parser_token));
        assert!(
            !promoted.allow_command_overtake,
            "no command or unrelated Page turn may split a selected parser task from its phase-one consumer"
        );
        assert_eq!(
            pending
                .pop_front()
                .and_then(|pending| pending.page_owner_token()),
            Some(parser_token),
            "ordinary Networking work for the same Page must remain behind the admitted parser driver"
        );
        assert_eq!(
            pending
                .pop_front()
                .and_then(|pending| pending.page_owner_token()),
            Some(unrelated_token)
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
