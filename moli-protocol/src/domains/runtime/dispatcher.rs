use std::collections::HashSet;

use serde_json::{Map, Value, json};

use crate::devtools_runtime::{
    DevToolsBidiChannelProperties, DevToolsCallFunctionCommand, DevToolsCommand,
    DevToolsCommandContext, DevToolsCommandResult, DevToolsDomNodeReference, DevToolsError,
    DevToolsErrorKind, DevToolsEvaluateScriptCommand, DevToolsGetFrameOwnerCommand,
    DevToolsGetRealmsCommand, DevToolsGetRealmsResult, DevToolsLocateNodesCommand,
    DevToolsLocateNodesLocator, DevToolsLocateNodesResult, DevToolsLocateNodesTextMatch,
    DevToolsProtocol, DevToolsRealmId, DevToolsReleaseObjectsCommand, DevToolsRemoteHandleId,
    DevToolsRemoteValue, DevToolsResolveNodeCommand, DevToolsResultOwnership,
    DevToolsScriptException, DevToolsScriptResult, DevToolsSerializationOptions, DevToolsTargetId,
    RuntimeExecutionContextEvent, is_webdriver_bidi_node_shared_id,
    webdriver_bidi_node_shared_id_for_backend_node_id,
};
use moli_core::page::{
    BidiPreloadChannelHandoff, DocumentNodeObjectSnapshot, DocumentNodeSnapshot,
    MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH, RendererCommandTurnOutput,
    RendererDomBidiNodeBindingResolution, RendererRuntimeCommandOutput,
    RendererRuntimeInspectorMessage,
};
use moli_page_types::RendererInspectorResponseDelivery;

use crate::conn::{
    BackgroundCommandResponsePayload, BackgroundCommandResponsePayloadRef, BackgroundProtocolEvent,
    BidiChannelListenerResidence, BidiChannelOwnerAction, BidiChannelPageOwner, CdpConnection,
    CdpRendererCommandAccess, CdpRendererCommandPolicy, CdpSchedulerEvent, CdpSessionRoute,
    ClaimedPendingInspectorAwait, Cmd, CommandOwnerScope, CompletedMoliDiagnosticsDispatch,
    CompletedRuntimeBindingPageCommandDispatch, CompletedRuntimeChildDefaultContextLookupDispatch,
    CompletedRuntimeEnableEventsDispatch, CompletedRuntimeProtocolMessageDispatch,
    CompletedServiceWorkerRuntimeProtocolMessageDispatch,
    CompletedSharedWorkerRuntimeProtocolMessageDispatch, DevToolsCommandDispatchOutcome,
    DevToolsCommandExecutionOutput, DuplicatePendingRendererCommand, InspectorCommandDispatch,
    NoneSessionOwnerRouteOverrideScope, ParsedCdpCommand, PendingBidiChannelListener,
    PendingMoliDiagnosticsDispatch, PendingRuntimeBindingPageCommandDispatch,
    PendingRuntimeChildDefaultContextLookupDispatch, PendingRuntimeEnableEventsDispatch,
    PendingRuntimeProtocolMessageDispatch, PendingServiceWorkerRuntimeProtocolMessageDispatch,
    PendingSharedWorkerRuntimeProtocolMessageDispatch, ProfilerInspectorCommand,
    RendererCommandDescriptor, RuntimeBindingDefinition, RuntimeEnableReplayEvent,
    RuntimeInspectorAsyncCompletionReceiver, RuntimeInspectorResponseReady,
    ServiceWorkerRuntimeExceptionSnapshot, SessionOwnerRuntimeFrontendEnableResult,
    monotonic_timestamp_seconds, renderer_command_turn_frontend_protocol_response,
    runtime_remote_object_ids_in_map,
};
use crate::domains::actions::{ConsoleAction, HeapProfilerAction, RuntimeAction};
use crate::domains::command_output::{
    CommandOutputPlan, devtools_error_from_cdp_error_parts, devtools_error_from_cdp_error_value,
};
use crate::domains::console::apply_console_output_state_for_session;
use crate::domains::observable_output::{
    advance_runtime_observable_cursors_to_current_for_session_owner,
    runtime_console_api_called_background_event, runtime_console_message_type_and_text,
    runtime_exception_thrown_background_event,
};
use crate::domains::runtime_context_events::{
    RuntimeContextProtocolEvent, apply_runtime_context_protocol_event_side_effects_typed,
    emit_runtime_context_protocol_background_event_typed,
    should_emit_child_default_context_inventory_replay_once,
};

const SHARED_WORKER_RUNTIME_BINDING_REPLAY_COMMAND_ID_BASE: u64 = 900_000_000;
const WEBDRIVER_BIDI_FILE_PROMPT_HANDLER_PARAM: &str = "__moliWebDriverBidiFilePromptHandler";
const LOCATE_NODES_START_NODE_OBJECT_GROUP: &str = "moli-locate-nodes-start-nodes";

enum LocateNodesStartNodeInput {
    Raw(Value),
    Reference(DevToolsDomNodeReference),
}
use super::{
    bidi_nodes::{
        BidiNodeSerializationOptions, bidi_node_remote_value_from_deep_serialized_remote_value,
        bidi_node_remote_value_from_snapshot, bidi_node_remote_value_shared_id,
        bidi_node_serialization_options, bidi_node_shared_id_for_snapshot,
        bidi_node_snapshot_for_shared_id_async, bidi_node_value_from_snapshot,
        devtools_serialization_options_for_node_probe,
    },
    bindings::{
        AddBindingParams, clear_runtime_binding_definitions_for_session_owner,
        persist_runtime_binding_definition_for_session_owner,
        remove_runtime_binding_definitions_for_session_owner,
    },
    command_classification::{
        MainRuntimeCommand, MainRuntimeInspectorCommand, RuntimeBindingCommand,
        RuntimeDevToolsScriptCommand, RuntimeInspectorPayloadPreparation, WorkerRuntimeCommand,
        WorkerRuntimeCommandKind,
    },
    evaluate::can_dispatch,
};

pub(crate) struct PendingRuntimeCommandDispatch {
    command_id: Option<u64>,
    action: &'static str,
    owner_scope: CommandOwnerScope,
    object_group: Option<String>,
    release_object_ids: Vec<String>,
    release_object_group: Option<String>,
    await_promise: bool,
    wait_for_deferred_reply: bool,
    pending: PendingRuntimeCommandKind,
}

pub(crate) struct CompletedRuntimeCommandDispatch {
    command_id: Option<u64>,
    action: &'static str,
    owner_scope: CommandOwnerScope,
    object_group: Option<String>,
    release_object_ids: Vec<String>,
    release_object_group: Option<String>,
    await_promise: bool,
    wait_for_deferred_reply: bool,
    completed: CompletedRuntimeCommandKind,
}

#[derive(Debug, Clone)]
struct DevToolsRuntimeTarget {
    route: CdpSessionRoute,
    execution_context_id: Option<i64>,
    window_context_id: Option<DevToolsTargetId>,
}

struct DevToolsRuntimeCommandDispatchState {
    internal_command_id: u64,
    command_context: DevToolsCommandContext,
    result_kind: DevToolsRuntimeCommandResultKind,
    result_ownership: DevToolsResultOwnership,
    serialization_options: Option<DevToolsSerializationOptions>,
    target: DevToolsRuntimeTarget,
    target_realm: Option<DevToolsRealmId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DevToolsRuntimeCommandResultKind {
    Script,
    Empty,
}

pub struct PendingDevToolsRuntimeCommandDispatch {
    state: DevToolsRuntimeCommandDispatchState,
    pending: PendingRuntimeCommandDispatch,
    interleaved_protocol_events: Vec<BackgroundProtocolEvent>,
    scheduler_events: Vec<CdpSchedulerEvent>,
}

pub struct CompletedDevToolsRuntimeCommandDispatch {
    state: DevToolsRuntimeCommandDispatchState,
    completed: CompletedRuntimeCommandDispatch,
    interleaved_protocol_events: Vec<BackgroundProtocolEvent>,
}

pub enum DevToolsRuntimeCommandTaskStep {
    Pending(Box<PendingDevToolsRuntimeCommandDispatch>),
    Complete(Box<DevToolsCommandDispatchOutcome>),
}

impl PendingDevToolsRuntimeCommandDispatch {
    pub fn take_scheduler_events(&mut self) -> Vec<CdpSchedulerEvent> {
        std::mem::take(&mut self.scheduler_events)
    }

    pub fn internal_command_id(&self) -> u64 {
        self.state.internal_command_id
    }

    pub fn command_id(&self) -> Option<u64> {
        self.pending.command_id()
    }

    pub fn session_id(&self) -> Option<&str> {
        self.pending.session_id()
    }

    pub fn waits_for_scheduler_deferred_inspector_reply(&self) -> bool {
        self.pending.waits_for_scheduler_deferred_inspector_reply()
    }

    pub fn take_scheduler_deferred_inspector_reply_events(
        &mut self,
    ) -> Vec<BackgroundProtocolEvent> {
        self.pending
            .take_scheduler_deferred_inspector_reply_events()
    }

    pub fn take_scheduler_deferred_inspector_reply_receiver(
        &mut self,
    ) -> Option<RuntimeInspectorAsyncCompletionReceiver> {
        self.pending
            .take_scheduler_deferred_inspector_reply_receiver()
    }

    pub async fn route_scheduler_deferred_inspector_response(
        &mut self,
        conn: &mut CdpConnection,
        response: crate::conn::RuntimeInspectorResponseReady,
    ) -> bool {
        self.pending
            .route_scheduler_deferred_inspector_response(conn, response)
            .await
    }

    pub async fn wait_for_scheduler_deferred_inspector_reply_receiver(
        &mut self,
        conn: &mut CdpConnection,
    ) -> Result<(), String> {
        self.pending
            .wait_for_scheduler_deferred_inspector_reply_receiver(conn)
            .await
    }

    pub fn complete_scheduler_deferred_inspector_reply(
        self,
        conn: &mut CdpConnection,
    ) -> CompletedDevToolsRuntimeCommandDispatch {
        CompletedDevToolsRuntimeCommandDispatch {
            state: self.state,
            completed: self
                .pending
                .complete_scheduler_deferred_inspector_reply(conn),
            interleaved_protocol_events: self.interleaved_protocol_events,
        }
    }

    pub fn forget_scheduler_deferred_inspector_reply(self, conn: &mut CdpConnection) {
        self.pending.forget_scheduler_deferred_inspector_reply(conn);
    }

    pub async fn wait(self) -> CompletedDevToolsRuntimeCommandDispatch {
        CompletedDevToolsRuntimeCommandDispatch {
            state: self.state,
            completed: self.pending.wait().await,
            interleaved_protocol_events: self.interleaved_protocol_events,
        }
    }
}

impl CompletedDevToolsRuntimeCommandDispatch {
    pub fn append_interleaved_protocol_events(&mut self, events: Vec<BackgroundProtocolEvent>) {
        self.interleaved_protocol_events.extend(events);
    }
}

struct DevToolsWindowRemoteCandidate {
    deep_serialized_value: Option<Value>,
}

pub(crate) enum RuntimeCommandTaskStep {
    Pending(Box<PendingRuntimeCommandDispatch>),
    Complete(CommandOutputPlan),
}

impl RuntimeCommandTaskStep {
    fn with_owner_scope(mut self, owner_scope: CommandOwnerScope) -> Self {
        if let RuntimeCommandTaskStep::Pending(pending) = &mut self {
            pending.owner_scope = owner_scope;
        }
        self
    }
}

enum PendingRuntimeCommandKind {
    Inspector {
        pending: PendingRuntimeProtocolMessageDispatch,
    },
    InspectorDeferredReply {
        routed_output: RuntimeInspectorRoutedOutput,
        renderer_response_rx: Option<RuntimeInspectorAsyncCompletionReceiver>,
        claimed_await: Option<ClaimedPendingInspectorAwait>,
        // False when V8's nested pause loop, rather than the Page actor,
        // consumed the Inspector command.
        page_owner_access_allowed: bool,
    },
    SharedWorkerInspector {
        pending: PendingSharedWorkerRuntimeProtocolMessageDispatch,
        binding_effect: Option<SharedWorkerRuntimeBindingEffect>,
    },
    ServiceWorkerInspector {
        pending: PendingServiceWorkerRuntimeProtocolMessageDispatch,
    },
    MoliDiagnostics(PendingMoliDiagnosticsDispatch),
    Enable(PendingRuntimeEnableEventsDispatch),
    BindingInspector {
        task: RuntimeBindingCommandTask,
        pending: PendingRuntimeProtocolMessageDispatch,
    },
    BindingContextLookup {
        task: RuntimeBindingCommandTask,
        pending: PendingRuntimeChildDefaultContextLookupDispatch,
    },
    BindingPage {
        task: RuntimeBindingCommandTask,
        pending: PendingRuntimeBindingPageCommandDispatch,
    },
}

enum CompletedRuntimeCommandKind {
    Inspector {
        completed: Result<CompletedRuntimeProtocolMessageDispatch, String>,
    },
    InspectorDeferredReplyReady {
        routed_output: RuntimeInspectorRoutedOutput,
        page_owner_access_allowed: bool,
    },
    SharedWorkerInspector {
        completed: Result<CompletedSharedWorkerRuntimeProtocolMessageDispatch, String>,
        binding_effect: Option<SharedWorkerRuntimeBindingEffect>,
    },
    ServiceWorkerInspector {
        completed: Result<CompletedServiceWorkerRuntimeProtocolMessageDispatch, String>,
    },
    MoliDiagnostics(Result<CompletedMoliDiagnosticsDispatch, String>),
    Enable(Result<CompletedRuntimeEnableEventsDispatch, String>),
    BindingInspector {
        task: RuntimeBindingCommandTask,
        completed: Result<CompletedRuntimeProtocolMessageDispatch, String>,
    },
    BindingContextLookup {
        task: RuntimeBindingCommandTask,
        completed: Result<CompletedRuntimeChildDefaultContextLookupDispatch, String>,
    },
    BindingPage {
        task: RuntimeBindingCommandTask,
        completed: Result<CompletedRuntimeBindingPageCommandDispatch, String>,
    },
}

#[derive(Default)]
struct RuntimeInspectorRoutedOutput {
    events: Vec<BackgroundProtocolEvent>,
    post_response_events: Vec<BackgroundProtocolEvent>,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
}

impl RuntimeInspectorRoutedOutput {
    fn append_ordered_events(&mut self, events: Vec<BackgroundProtocolEvent>) {
        self.events.extend(events);
    }

    fn append_post_response_events(&mut self, events: Vec<BackgroundProtocolEvent>) {
        self.post_response_events.extend(events);
    }

    fn set_renderer_output_predecessor(&mut self, predecessor: moli_core::RendererOutputFence) {
        predecessor.merge_into_same_stream_tail(&mut self.renderer_output_predecessor);
    }

    fn events(&self) -> &[BackgroundProtocolEvent] {
        &self.events
    }

    fn background_event_count(&self) -> usize {
        self.events
            .iter()
            .filter(|event| event.protocol_message_id().is_none())
            .count()
    }

    fn take_events_ready_before_command_response(
        &mut self,
        command_id: Option<u64>,
    ) -> Vec<BackgroundProtocolEvent> {
        let pending_events = std::mem::take(&mut self.events);
        let mut ready_events = Vec::new();
        for event in pending_events {
            if command_id.is_some_and(|command_id| event.protocol_message_id() == Some(command_id))
            {
                self.events.push(event);
            } else {
                ready_events.push(event);
            }
        }
        ready_events
    }

    fn take_ready_background_events_for_command(
        &mut self,
        command_id: Option<u64>,
    ) -> Vec<BackgroundProtocolEvent> {
        self.take_events_ready_before_command_response(command_id)
    }

    fn command_response_succeeded(&self, command_id: Option<u64>) -> bool {
        command_response_succeeded_for_events(&self.events, command_id)
    }

    fn register_object_group_for_success(
        &self,
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        object_group: Option<&str>,
    ) {
        let Some(object_group) = object_group else {
            return;
        };
        for event in &self.events {
            if let Some((_, _, BackgroundCommandResponsePayloadRef::Success { result })) =
                event.command_response_payload_ref()
            {
                conn.register_runtime_remote_object_ids_from_value_for_session_owner_with_group(
                    session_id,
                    result,
                    object_group,
                );
            } else if let Some(message) = event.protocol_message() {
                conn.register_runtime_remote_object_ids_from_value_for_session_owner_with_group(
                    session_id,
                    message,
                    object_group,
                );
            }
        }
    }

    fn push_ordered_into_plan(self, plan: &mut CommandOutputPlan, command_id: Option<u64>) {
        for event in self.events {
            push_runtime_protocol_event_or_background_event(plan, command_id, event);
        }
        plan.extend_post_response_events(self.post_response_events);
        if let Some(predecessor) = self.renderer_output_predecessor {
            plan.set_renderer_output_predecessor(predecessor);
        }
    }

    fn push_background_events_before_response_events(
        self,
        plan: &mut CommandOutputPlan,
        command_id: Option<u64>,
    ) {
        let (response_events, background_events): (Vec<_>, Vec<_>) =
            self.events.into_iter().partition(|event| {
                command_id.is_some_and(|command_id| event.protocol_message_id() == Some(command_id))
            });
        for event in background_events {
            plan.push_background_event(event);
        }
        for event in response_events {
            push_runtime_protocol_event_or_background_event(plan, command_id, event);
        }
        plan.extend_post_response_events(self.post_response_events);
    }
}

#[derive(Clone)]
enum SharedWorkerRuntimeBindingEffect {
    Add {
        name: String,
        execution_context_name: Option<String>,
    },
    Remove {
        name: String,
    },
}

#[derive(Clone)]
enum RuntimeBindingPhase {
    LivePageUpdate,
    StoredBindingsApply,
}

#[derive(Clone)]
enum RuntimeBindingCommandResponse {
    Success,
    Error { code: i32, message: String },
}

impl RuntimeBindingCommandResponse {
    fn empty_success() -> Self {
        Self::Success
    }

    fn succeeded(&self) -> bool {
        matches!(self, Self::Success)
    }

    fn push_into_plan(self, plan: &mut CommandOutputPlan) {
        match self {
            Self::Success => plan.push_success(),
            Self::Error { code, message } => plan.push_error(code, message),
        }
    }
}

#[derive(Clone)]
struct RuntimeBindingCommandTask {
    action: RuntimeBindingCommand,
    renderer_policy: CdpRendererCommandPolicy,
    phase: RuntimeBindingPhase,
    name: String,
    execution_context_name: Option<String>,
    execution_context_id: Option<i64>,
    inspector_json: Option<String>,
    command_response: Option<RuntimeBindingCommandResponse>,
    should_persist: bool,
    skip_live_page_update_after_inspector_success: bool,
}

#[derive(Clone)]
struct RuntimeCommandCompletionMeta {
    command_id: Option<u64>,
    action: &'static str,
    owner_scope: CommandOwnerScope,
    object_group: Option<String>,
    release_object_ids: Vec<String>,
    release_object_group: Option<String>,
    await_promise: bool,
    wait_for_deferred_reply: bool,
}

impl From<&CompletedRuntimeCommandDispatch> for RuntimeCommandCompletionMeta {
    fn from(completed: &CompletedRuntimeCommandDispatch) -> Self {
        Self {
            command_id: completed.command_id,
            action: completed.action,
            owner_scope: completed.owner_scope.clone(),
            object_group: completed.object_group.clone(),
            release_object_ids: completed.release_object_ids.clone(),
            release_object_group: completed.release_object_group.clone(),
            await_promise: completed.await_promise,
            wait_for_deferred_reply: completed.wait_for_deferred_reply,
        }
    }
}

impl RuntimeCommandCompletionMeta {
    fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }
}

impl PendingRuntimeCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }

    pub(crate) fn executes_page_javascript(&self) -> bool {
        matches!(
            self.action,
            "evaluate" | "callFunctionOn" | "awaitPromise" | "runScript"
        )
    }

    pub(crate) fn waits_for_scheduler_deferred_inspector_reply(&self) -> bool {
        matches!(
            self.pending,
            PendingRuntimeCommandKind::InspectorDeferredReply { .. }
        )
    }

    pub(crate) fn deferred_reply_page_owner_access_allowed(&self) -> bool {
        matches!(
            self.pending,
            PendingRuntimeCommandKind::InspectorDeferredReply {
                page_owner_access_allowed: true,
                ..
            }
        )
    }

    pub(crate) fn append_scheduler_deferred_inspector_reply_output(
        &mut self,
        response_events: Vec<BackgroundProtocolEvent>,
        events: Vec<BackgroundProtocolEvent>,
    ) {
        let PendingRuntimeCommandKind::InspectorDeferredReply {
            routed_output,
            renderer_response_rx: _,
            claimed_await: _,
            page_owner_access_allowed: _,
        } = &mut self.pending
        else {
            return;
        };
        routed_output.append_ordered_events(response_events);
        routed_output.append_ordered_events(events);
    }

    pub(crate) fn take_scheduler_deferred_inspector_reply_events(
        &mut self,
    ) -> Vec<BackgroundProtocolEvent> {
        let command_id = self.command_id;
        let PendingRuntimeCommandKind::InspectorDeferredReply {
            routed_output,
            renderer_response_rx: _,
            claimed_await: _,
            page_owner_access_allowed: _,
        } = &mut self.pending
        else {
            return Vec::new();
        };
        routed_output.take_ready_background_events_for_command(command_id)
    }

    pub(crate) fn take_scheduler_deferred_inspector_reply_receiver(
        &mut self,
    ) -> Option<RuntimeInspectorAsyncCompletionReceiver> {
        let PendingRuntimeCommandKind::InspectorDeferredReply {
            renderer_response_rx,
            ..
        } = &mut self.pending
        else {
            return None;
        };
        renderer_response_rx.take()
    }

    pub(crate) async fn route_scheduler_deferred_inspector_response(
        &mut self,
        conn: &mut CdpConnection,
        response: crate::conn::RuntimeInspectorResponseReady,
    ) -> bool {
        let Some(command_id) = self.command_id else {
            return false;
        };
        if response.command_id() != command_id {
            return false;
        }
        let owner_scope = self.owner_scope.clone();
        let session_id = self.session_id().map(str::to_owned);
        let mut response_events = Vec::new();
        let mut background_events = Vec::new();
        let mut route_scope = owner_scope.enter(conn);
        let (routed, renderer_output_predecessor) = route_scope
            .conn_mut()
            .route_scheduler_deferred_runtime_inspector_response_into(
                response,
                session_id.as_deref(),
                &mut response_events,
                &mut background_events,
            )
            .await;
        drop(route_scope);
        if let Some(predecessor) = renderer_output_predecessor {
            let PendingRuntimeCommandKind::InspectorDeferredReply { routed_output, .. } =
                &mut self.pending
            else {
                unreachable!("deferred Inspector response requires deferred command state")
            };
            routed_output.set_renderer_output_predecessor(predecessor);
        }
        self.append_scheduler_deferred_inspector_reply_output(response_events, background_events);
        routed
    }

    async fn wait_for_scheduler_deferred_inspector_reply_receiver(
        &mut self,
        conn: &mut CdpConnection,
    ) -> Result<(), String> {
        let command_id = self
            .command_id
            .ok_or_else(|| "RuntimeDeferredInspectorReplyMissingCommandId".to_owned())?;
        let Some(response_rx) = self.take_scheduler_deferred_inspector_reply_receiver() else {
            return Err("RuntimeDeferredInspectorReplyMissingRendererResponse".to_owned());
        };
        let response = response_rx
            .await
            .map_err(|_| "RuntimeDeferredInspectorResponseCanceled".to_owned());
        let session_id = self.session_id().map(str::to_owned);
        let routed = self
            .route_scheduler_deferred_inspector_response(
                conn,
                crate::conn::RuntimeInspectorResponseReady::new(
                    command_id,
                    session_id.as_deref(),
                    response,
                ),
            )
            .await;
        debug_assert!(routed);
        Ok(())
    }

    pub(crate) fn complete_scheduler_deferred_inspector_reply(
        self,
        conn: &mut CdpConnection,
    ) -> CompletedRuntimeCommandDispatch {
        let owner_scope = self.owner_scope;
        let completed = match self.pending {
            PendingRuntimeCommandKind::InspectorDeferredReply {
                routed_output,
                renderer_response_rx: _,
                claimed_await,
                page_owner_access_allowed,
            } => {
                let mut route_scope = owner_scope.enter(conn);
                route_scope
                    .conn_mut()
                    .complete_claimed_pending_inspector_await_for_scheduler_deferred_reply(
                        claimed_await,
                        routed_output.events(),
                    );
                CompletedRuntimeCommandKind::InspectorDeferredReplyReady {
                    routed_output,
                    page_owner_access_allowed,
                }
            }
            _ => {
                unreachable!(
                    "deferred inspector reply completion requires a deferred reply pending step"
                )
            }
        };
        CompletedRuntimeCommandDispatch {
            command_id: self.command_id,
            action: self.action,
            owner_scope,
            object_group: self.object_group,
            release_object_ids: self.release_object_ids,
            release_object_group: self.release_object_group,
            await_promise: self.await_promise,
            wait_for_deferred_reply: self.wait_for_deferred_reply,
            completed,
        }
    }

    pub(crate) fn forget_scheduler_deferred_inspector_reply(self, conn: &mut CdpConnection) {
        let session_id = self.owner_scope.session_id().map(str::to_owned);
        let owner_scope = self.owner_scope.clone();
        let mut route_scope = owner_scope.enter(conn);
        match self.pending {
            PendingRuntimeCommandKind::InspectorDeferredReply { claimed_await, .. } => {
                route_scope
                    .conn_mut()
                    .cancel_claimed_pending_inspector_await_for_scheduler_deferred_reply(
                        claimed_await,
                        "forgotten",
                    );
            }
            _ => {
                if let Some(command_id) = self.command_id {
                    route_scope
                        .conn_mut()
                        .forget_pending_inspector_await(command_id, session_id.as_deref());
                }
            }
        }
    }

    pub(crate) async fn wait(self) -> CompletedRuntimeCommandDispatch {
        CompletedRuntimeCommandDispatch {
            command_id: self.command_id,
            action: self.action,
            owner_scope: self.owner_scope,
            object_group: self.object_group,
            release_object_ids: self.release_object_ids,
            release_object_group: self.release_object_group,
            await_promise: self.await_promise,
            wait_for_deferred_reply: self.wait_for_deferred_reply,
            completed: match self.pending {
                PendingRuntimeCommandKind::Inspector { pending } => {
                    CompletedRuntimeCommandKind::Inspector {
                        completed: pending.wait().await,
                    }
                }
                PendingRuntimeCommandKind::InspectorDeferredReply {
                    routed_output,
                    renderer_response_rx: _,
                    claimed_await: _,
                    page_owner_access_allowed,
                } => CompletedRuntimeCommandKind::InspectorDeferredReplyReady {
                    routed_output,
                    page_owner_access_allowed,
                },
                PendingRuntimeCommandKind::SharedWorkerInspector {
                    pending,
                    binding_effect,
                } => CompletedRuntimeCommandKind::SharedWorkerInspector {
                    completed: pending.wait().await,
                    binding_effect,
                },
                PendingRuntimeCommandKind::ServiceWorkerInspector { pending } => {
                    CompletedRuntimeCommandKind::ServiceWorkerInspector {
                        completed: pending.wait().await,
                    }
                }
                PendingRuntimeCommandKind::MoliDiagnostics(pending) => {
                    CompletedRuntimeCommandKind::MoliDiagnostics(pending.wait().await)
                }
                PendingRuntimeCommandKind::Enable(pending) => {
                    CompletedRuntimeCommandKind::Enable(pending.wait().await)
                }
                PendingRuntimeCommandKind::BindingInspector { task, pending } => {
                    CompletedRuntimeCommandKind::BindingInspector {
                        task,
                        completed: pending.wait().await,
                    }
                }
                PendingRuntimeCommandKind::BindingContextLookup { task, pending } => {
                    CompletedRuntimeCommandKind::BindingContextLookup {
                        task,
                        completed: pending.wait().await,
                    }
                }
                PendingRuntimeCommandKind::BindingPage { task, pending } => {
                    CompletedRuntimeCommandKind::BindingPage {
                        task,
                        completed: pending.wait().await,
                    }
                }
            },
        }
    }
}

impl CompletedRuntimeCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }

    pub(crate) fn page_owner_access_allowed(&self) -> bool {
        match &self.completed {
            CompletedRuntimeCommandKind::Inspector {
                completed: Ok(completed),
            } => completed.page_owner_access_allowed(),
            CompletedRuntimeCommandKind::InspectorDeferredReplyReady {
                page_owner_access_allowed,
                ..
            } => *page_owner_access_allowed,
            _ => true,
        }
    }
}

fn command_response_succeeded(
    messages: &[RendererRuntimeInspectorMessage],
    command_id: Option<u64>,
) -> bool {
    let Some(command_id) = command_id else {
        return false;
    };
    messages.iter().any(|message| {
        let RendererRuntimeInspectorMessage::Protocol(message) = message else {
            return false;
        };
        message.get("id").and_then(Value::as_u64) == Some(command_id)
            && message.get("result").is_some()
    })
}

fn command_response_succeeded_for_events(
    events: &[BackgroundProtocolEvent],
    command_id: Option<u64>,
) -> bool {
    let Some(command_id) = command_id else {
        return false;
    };
    events.iter().any(|event| {
        if let Some((event_command_id, _, payload)) = event.command_response_payload_ref()
            && event_command_id == Some(command_id)
        {
            return matches!(payload, BackgroundCommandResponsePayloadRef::Success { .. });
        }
        event.protocol_message().is_some_and(|message| {
            message.get("id").and_then(Value::as_u64) == Some(command_id)
                && message.get("result").is_some()
        })
    })
}

fn runtime_object_group_from_params(params: Option<&Map<String, Value>>) -> Option<&str> {
    params?.get("objectGroup").and_then(Value::as_str)
}

fn runtime_object_id_from_params(params: Option<&Map<String, Value>>) -> Option<&str> {
    params?.get("objectId").and_then(Value::as_str)
}

fn runtime_promise_object_id_from_params(params: Option<&Map<String, Value>>) -> Option<&str> {
    params?.get("promiseObjectId").and_then(Value::as_str)
}

fn runtime_prototype_object_id_from_params(params: Option<&Map<String, Value>>) -> Option<&str> {
    params?.get("prototypeObjectId").and_then(Value::as_str)
}

fn runtime_object_group_for_command_result(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    action: RuntimeAction,
) -> Option<String> {
    match action {
        RuntimeAction::Evaluate => runtime_object_group_from_params(cmd.params).map(str::to_owned),
        RuntimeAction::CallFunctionOn => runtime_object_group_from_params(cmd.params)
            .map(str::to_owned)
            .or_else(|| {
                conn.runtime_remote_object_group_for_session_owner(
                    cmd.session_id,
                    runtime_object_id_from_params(cmd.params)?,
                )
            }),
        RuntimeAction::GetProperties => conn.runtime_remote_object_group_for_session_owner(
            cmd.session_id,
            runtime_object_id_from_params(cmd.params)?,
        ),
        RuntimeAction::AwaitPromise => conn.runtime_remote_object_group_for_session_owner(
            cmd.session_id,
            runtime_promise_object_id_from_params(cmd.params)?,
        ),
        RuntimeAction::RunScript => runtime_object_group_from_params(cmd.params).map(str::to_owned),
        RuntimeAction::QueryObjects => runtime_object_group_from_params(cmd.params)
            .map(str::to_owned)
            .or_else(|| {
                conn.runtime_remote_object_group_for_session_owner(
                    cmd.session_id,
                    runtime_prototype_object_id_from_params(cmd.params)?,
                )
            }),
        _ => None,
    }
}

fn runtime_command_awaits_promise(cmd: &Cmd<'_>, action: RuntimeAction) -> bool {
    action == RuntimeAction::AwaitPromise
        || cmd
            .params
            .and_then(|params| params.get("awaitPromise"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

pub(crate) fn try_start_runtime_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<RuntimeCommandTaskStep> {
    let Some(action) = cmd.parse_action::<RuntimeAction>() else {
        return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32601,
            "UnknownMethod",
        )));
    };
    if matches!(
        conn.session_route(cmd.session_id),
        Some(
            CdpSessionRoute::SharedWorkerTarget { .. }
                | CdpSessionRoute::DedicatedWorkerTarget { .. }
        )
    ) {
        return try_start_shared_worker_runtime_command_dispatch(conn, cmd, action);
    }
    if matches!(
        conn.session_route(cmd.session_id),
        Some(CdpSessionRoute::ServiceWorkerTarget { .. })
    ) {
        return try_start_service_worker_runtime_command_dispatch(conn, cmd, action);
    }
    let command = MainRuntimeCommand::classify(action);
    if command.requires_v8_method_support_check() && !can_dispatch(cmd) {
        return Some(RuntimeCommandTaskStep::Complete(
            runtime_inspector_error_plan(cmd.id, "UnknownMethod".to_owned()),
        ));
    }
    match command {
        MainRuntimeCommand::Enable => try_start_pending_runtime_enable_command(conn, cmd),
        MainRuntimeCommand::Disable => Some(start_pending_runtime_disable_command(conn, cmd)),
        MainRuntimeCommand::Binding(binding) => {
            try_start_pending_runtime_binding_command(conn, cmd, binding)
        }
        MainRuntimeCommand::DiscardConsoleEntries => {
            Some(start_runtime_discard_console_entries_command(conn, cmd))
        }
        MainRuntimeCommand::RunIfWaitingForDebugger => {
            Some(start_runtime_run_if_waiting_for_debugger_command(conn, cmd))
        }
        MainRuntimeCommand::DevToolsScript(command) => Some(
            start_cdp_devtools_script_runtime_command(conn, cmd, command),
        ),
        MainRuntimeCommand::Inspector(command) => {
            Some(start_main_runtime_inspector_command(conn, cmd, command))
        }
    }
}

fn start_main_runtime_inspector_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command: MainRuntimeInspectorCommand,
) -> RuntimeCommandTaskStep {
    let action = command.action();
    let action_label = action.label();
    let await_promise = runtime_command_awaits_promise(cmd, action);
    let inspector_json =
        match prepare_runtime_inspector_payload(conn, cmd, command.payload_preparation()) {
            Ok(json) => json,
            Err(message) => {
                return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                    cmd.id, message,
                ));
            }
        };
    let object_group = runtime_object_group_for_command_result(conn, cmd, action);
    let release_object_ids = if action == RuntimeAction::ReleaseObject {
        cmd.params
            .map(runtime_remote_object_ids_in_map)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let release_object_group = if action == RuntimeAction::ReleaseObjectGroup {
        runtime_object_group_from_params(cmd.params).map(str::to_owned)
    } else {
        None
    };
    let session_owner_route = if await_promise {
        conn.runtime_await_owner_route_for_session(cmd.session_id)
    } else {
        None
    };
    let pre_registered_await = match pre_register_runtime_await_if_needed(
        conn,
        await_promise,
        cmd.id,
        cmd.session_id,
        object_group.as_deref(),
        action_label,
    ) {
        Ok(command_id) => command_id,
        Err(error) => {
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
                -32600,
                error.to_string(),
            ));
        }
    };
    let pending = match start_pending_runtime_routable_inspector_dispatch(
        conn,
        cmd,
        inspector_json,
        if await_promise {
            RendererInspectorResponseDelivery::CommandReply
        } else {
            cmd.terminal_response_delivery(RendererInspectorResponseDelivery::DevToolsSession)
        },
    ) {
        Ok(pending) => pending,
        Err(message) => {
            forget_pre_registered_runtime_await(conn, pre_registered_await, cmd.session_id);
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };
    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: action_label,
        owner_scope: CommandOwnerScope::from_session_and_owner_route(
            cmd.session_id,
            session_owner_route,
        ),
        object_group,
        release_object_ids,
        release_object_group,
        await_promise,
        wait_for_deferred_reply: await_promise,
        pending: PendingRuntimeCommandKind::Inspector { pending },
    }))
}

pub(crate) fn start_profiler_inspector_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command: ProfilerInspectorCommand,
) -> RuntimeCommandTaskStep {
    let dispatch = command.runtime_dispatch(cmd.id, cmd.params);
    if matches!(
        conn.session_route(cmd.session_id),
        Some(
            CdpSessionRoute::SharedWorkerTarget { .. }
                | CdpSessionRoute::DedicatedWorkerTarget { .. }
        )
    ) {
        return start_profiler_inspector_shared_worker_command_dispatch(conn, cmd, dispatch);
    }
    if matches!(
        conn.session_route(cmd.session_id),
        Some(CdpSessionRoute::ServiceWorkerTarget { .. })
    ) {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "ServiceWorkerTargetRuntimeNotImplemented",
        ));
    }

    if !can_dispatch(cmd) {
        return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
            cmd.id,
            "UnknownMethod".to_owned(),
        ));
    }

    let action = dispatch.protocol_method();
    let pending = match start_pending_runtime_inspector_dispatch_with_delivery(
        conn,
        cmd,
        dispatch.into_inspector_json(),
        cmd.terminal_response_delivery(RendererInspectorResponseDelivery::DevToolsSession),
    ) {
        Ok(pending) => pending,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action,
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::Inspector { pending },
    }))
}

pub(crate) fn start_heap_profiler_inspector_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: HeapProfilerAction,
) -> RuntimeCommandTaskStep {
    if matches!(
        conn.session_route(cmd.session_id),
        Some(
            CdpSessionRoute::SharedWorkerTarget { .. }
                | CdpSessionRoute::DedicatedWorkerTarget { .. }
        )
    ) {
        return start_heap_profiler_inspector_shared_worker_command_dispatch(conn, cmd, action);
    }
    if matches!(
        conn.session_route(cmd.session_id),
        Some(CdpSessionRoute::ServiceWorkerTarget { .. })
    ) {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "ServiceWorkerTargetRuntimeNotImplemented",
        ));
    }

    if !can_dispatch(cmd) {
        return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
            cmd.id,
            "UnknownMethod".to_owned(),
        ));
    }

    if !conn
        .runtime_session_owner_slot(cmd.session_id)
        .is_ok_and(|slot| slot.has_loaded_page())
    {
        return RuntimeCommandTaskStep::Complete(match action {
            HeapProfilerAction::Enable | HeapProfilerAction::Disable => {
                CommandOutputPlan::success()
            }
            HeapProfilerAction::AddInspectedHeapObject
            | HeapProfilerAction::CollectGarbage
            | HeapProfilerAction::GetHeapObjectId
            | HeapProfilerAction::GetObjectByHeapObjectId
            | HeapProfilerAction::GetSamplingProfile
            | HeapProfilerAction::StartSampling
            | HeapProfilerAction::StartTrackingHeapObjects
            | HeapProfilerAction::StopSampling
            | HeapProfilerAction::StopTrackingHeapObjects
            | HeapProfilerAction::TakeHeapSnapshot => {
                CommandOutputPlan::error(-32000, "NoDocumentLoaded")
            }
            HeapProfilerAction::MoliDiagnostics | HeapProfilerAction::MoliResetIdleEngine => {
                runtime_inspector_error_plan(
                    cmd.id,
                    "UnsupportedHeapProfilerInspectorCommand".to_owned(),
                )
            }
        });
    }

    let method = heap_profiler_action_protocol_method(action);
    let object_group = heap_profiler_object_group_for_command_result(cmd, action);
    let pending = match start_pending_runtime_inspector_dispatch_with_delivery(
        conn,
        cmd,
        cmd.json.to_owned(),
        RendererInspectorResponseDelivery::CommandReply,
    ) {
        Ok(pending) => pending,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: method,
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: true,
        pending: PendingRuntimeCommandKind::Inspector { pending },
    }))
}

pub(crate) fn start_debugger_inspector_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    inspector_json: String,
) -> RuntimeCommandTaskStep {
    if matches!(
        conn.session_route(cmd.session_id),
        Some(
            CdpSessionRoute::SharedWorkerTarget { .. }
                | CdpSessionRoute::DedicatedWorkerTarget { .. }
                | CdpSessionRoute::ServiceWorkerTarget { .. }
        )
    ) {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "WorkerDebuggerNotImplemented",
        ));
    }
    if !can_dispatch(cmd) {
        return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
            cmd.id,
            "UnknownMethod".to_owned(),
        ));
    }

    let pending = match cmd.renderer_policy().access() {
        CdpRendererCommandAccess::MainThread => {
            start_pending_runtime_inspector_dispatch_with_delivery(
                conn,
                cmd,
                inspector_json,
                cmd.terminal_response_delivery(RendererInspectorResponseDelivery::DevToolsSession),
            )
        }
        CdpRendererCommandAccess::Io => start_pending_runtime_io_inspector_dispatch(
            conn,
            cmd,
            inspector_json,
            cmd.terminal_response_delivery(RendererInspectorResponseDelivery::DevToolsSession),
        ),
        CdpRendererCommandAccess::OwnerIndependent => Err(
            "an owner-independent command cannot enter the Debugger Inspector dispatcher"
                .to_owned(),
        ),
    };
    let pending = match pending {
        Ok(pending) => pending,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: "Debugger.inspectorCommand",
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::Inspector { pending },
    }))
}

fn start_heap_profiler_inspector_shared_worker_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: HeapProfilerAction,
) -> RuntimeCommandTaskStep {
    let Some(session_id) = cmd.session_id else {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    };
    if conn
        .shared_worker_target_for_session(Some(session_id))
        .is_none()
    {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    }

    if !can_dispatch(cmd) {
        return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(
            "UnknownMethod".to_owned(),
        ));
    }

    let pending =
        match start_shared_worker_frontend_inspector_dispatch(conn, cmd, cmd.json.to_owned()) {
            Ok(pending) => pending,
            Err(message)
                if matches!(
                    action,
                    HeapProfilerAction::Enable | HeapProfilerAction::Disable
                ) && (message == "NoDocumentLoaded"
                    || worker_runtime_is_unavailable(&message)) =>
            {
                return RuntimeCommandTaskStep::Complete(CommandOutputPlan::success());
            }
            Err(message) => {
                return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message));
            }
        };

    let object_group = heap_profiler_object_group_for_command_result(cmd, action);
    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: heap_profiler_action_protocol_method(action),
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: true,
        pending: PendingRuntimeCommandKind::SharedWorkerInspector {
            pending,
            binding_effect: None,
        },
    }))
}

fn heap_profiler_action_protocol_method(action: HeapProfilerAction) -> &'static str {
    match action {
        HeapProfilerAction::AddInspectedHeapObject => "HeapProfiler.addInspectedHeapObject",
        HeapProfilerAction::Enable => "HeapProfiler.enable",
        HeapProfilerAction::Disable => "HeapProfiler.disable",
        HeapProfilerAction::CollectGarbage => "HeapProfiler.collectGarbage",
        HeapProfilerAction::GetHeapObjectId => "HeapProfiler.getHeapObjectId",
        HeapProfilerAction::GetObjectByHeapObjectId => "HeapProfiler.getObjectByHeapObjectId",
        HeapProfilerAction::GetSamplingProfile => "HeapProfiler.getSamplingProfile",
        HeapProfilerAction::StartSampling => "HeapProfiler.startSampling",
        HeapProfilerAction::StartTrackingHeapObjects => "HeapProfiler.startTrackingHeapObjects",
        HeapProfilerAction::StopSampling => "HeapProfiler.stopSampling",
        HeapProfilerAction::StopTrackingHeapObjects => "HeapProfiler.stopTrackingHeapObjects",
        HeapProfilerAction::TakeHeapSnapshot => "HeapProfiler.takeHeapSnapshot",
        HeapProfilerAction::MoliDiagnostics | HeapProfilerAction::MoliResetIdleEngine => {
            "HeapProfiler.moliExtension"
        }
    }
}

fn heap_profiler_object_group_for_command_result(
    cmd: &Cmd<'_>,
    action: HeapProfilerAction,
) -> Option<String> {
    match action {
        HeapProfilerAction::GetObjectByHeapObjectId => {
            runtime_object_group_from_params(cmd.params).map(str::to_owned)
        }
        _ => None,
    }
}

fn start_profiler_inspector_shared_worker_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    dispatch: InspectorCommandDispatch,
) -> RuntimeCommandTaskStep {
    let Some(session_id) = cmd.session_id else {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    };
    if conn
        .shared_worker_target_for_session(Some(session_id))
        .is_none()
    {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    }

    if !can_dispatch(cmd) {
        return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
            cmd.id,
            "UnknownMethod".to_owned(),
        ));
    }

    let action = dispatch.protocol_method();
    let pending = match start_shared_worker_frontend_inspector_dispatch(
        conn,
        cmd,
        dispatch.into_inspector_json(),
    ) {
        Ok(pending) => pending,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action,
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::SharedWorkerInspector {
            pending,
            binding_effect: None,
        },
    }))
}

pub(crate) fn start_console_inspector_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: ConsoleAction,
) -> RuntimeCommandTaskStep {
    if matches!(
        conn.session_route(cmd.session_id),
        Some(
            CdpSessionRoute::SharedWorkerTarget { .. }
                | CdpSessionRoute::DedicatedWorkerTarget { .. }
        )
    ) {
        return start_console_inspector_shared_worker_command_dispatch(conn, cmd, action);
    }
    if matches!(
        conn.session_route(cmd.session_id),
        Some(CdpSessionRoute::ServiceWorkerTarget { .. })
    ) {
        return start_console_inspector_service_worker_command_dispatch(conn, cmd, action);
    }

    if !can_dispatch(cmd) {
        return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
            cmd.id,
            "UnknownMethod".to_owned(),
        ));
    }

    let pending = match start_pending_runtime_inspector_dispatch_with_delivery(
        conn,
        cmd,
        cmd.json.to_owned(),
        cmd.terminal_response_delivery(RendererInspectorResponseDelivery::DevToolsSession),
    ) {
        Ok(pending) => pending,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: console_action_protocol_method(action),
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::Inspector { pending },
    }))
}

fn console_action_protocol_method(action: ConsoleAction) -> &'static str {
    match action {
        ConsoleAction::Enable => "Console.enable",
        ConsoleAction::Disable => "Console.disable",
        ConsoleAction::ClearMessages => "Console.clearMessages",
    }
}

fn console_action_from_protocol_method(method: &str) -> Option<ConsoleAction> {
    match method {
        "Console.enable" => Some(ConsoleAction::Enable),
        "Console.disable" => Some(ConsoleAction::Disable),
        "Console.clearMessages" => Some(ConsoleAction::ClearMessages),
        _ => None,
    }
}

fn start_console_inspector_shared_worker_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: ConsoleAction,
) -> RuntimeCommandTaskStep {
    let Some(session_id) = cmd.session_id else {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    };
    if conn
        .shared_worker_target_for_session(Some(session_id))
        .is_none()
    {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    }

    if !can_dispatch(cmd) {
        return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(
            "UnknownMethod".to_owned(),
        ));
    }

    let pending =
        match start_shared_worker_frontend_inspector_dispatch(conn, cmd, cmd.json.to_owned()) {
            Ok(pending) => pending,
            Err(message) if worker_runtime_is_unavailable(&message) => {
                if !apply_console_output_state_for_session(conn, cmd.session_id, action) {
                    return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(
                        "UnknownSession".to_owned(),
                    ));
                }
                return RuntimeCommandTaskStep::Complete(CommandOutputPlan::success());
            }
            Err(message) => {
                return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message));
            }
        };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: console_action_protocol_method(action),
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::SharedWorkerInspector {
            pending,
            binding_effect: None,
        },
    }))
}

fn start_console_inspector_service_worker_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: ConsoleAction,
) -> RuntimeCommandTaskStep {
    let Some(session_id) = cmd.session_id else {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    };
    if conn
        .service_worker_target_for_session(Some(session_id))
        .is_none()
    {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        ));
    }

    if !can_dispatch(cmd) {
        return RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(
            "UnknownMethod".to_owned(),
        ));
    }

    let pending =
        match start_service_worker_frontend_inspector_dispatch(conn, cmd, cmd.json.to_owned()) {
            Ok(pending) => pending,
            Err(message) if message == "ServiceWorkerRuntimeUnavailable" => {
                if !apply_console_output_state_for_session(conn, cmd.session_id, action) {
                    return RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(
                        "UnknownSession".to_owned(),
                    ));
                }
                return RuntimeCommandTaskStep::Complete(CommandOutputPlan::success());
            }
            Err(message) => {
                return RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(
                    message,
                ));
            }
        };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: console_action_protocol_method(action),
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::ServiceWorkerInspector { pending },
    }))
}

pub(crate) fn start_moli_diagnostics_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> RuntimeCommandTaskStep {
    let pending = match conn.start_moli_diagnostics() {
        Ok(pending) => pending,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
        }
    };
    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: "moliDiagnostics",
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::MoliDiagnostics(pending),
    }))
}

fn try_start_pending_runtime_enable_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<RuntimeCommandTaskStep> {
    let session_owner_route = if cmd.session_id.is_none() {
        conn.none_session_owner_route_override()
    } else {
        None
    };
    let has_loaded_page = match conn.runtime_session_owner_slot(cmd.session_id) {
        Ok(slot) => slot.has_loaded_page(),
        Err(_) if cmd.session_id.is_some() => {
            return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
                -32001,
                "Unknown sessionId",
            )));
        }
        Err(_) => {
            match conn.set_runtime_frontend_enabled_for_session_owner(cmd.session_id, true) {
                SessionOwnerRuntimeFrontendEnableResult::Handled => {}
                SessionOwnerRuntimeFrontendEnableResult::UnknownSession => {
                    return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32001,
                        "Unknown sessionId",
                    )));
                }
            }
            return Some(RuntimeCommandTaskStep::Complete(
                CommandOutputPlan::success(),
            ));
        }
    };
    if !has_loaded_page {
        if conn.can_defer_initial_document_page_build() {
            match conn.set_runtime_frontend_enabled_for_session_owner(cmd.session_id, true) {
                SessionOwnerRuntimeFrontendEnableResult::Handled => {}
                SessionOwnerRuntimeFrontendEnableResult::UnknownSession => {
                    return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32001,
                        "Unknown sessionId",
                    )));
                }
            }
            return Some(RuntimeCommandTaskStep::Complete(
                CommandOutputPlan::success(),
            ));
        }
        return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "NoDocumentLoaded",
        )));
    }
    Some(start_pending_runtime_enable_events_phase(
        conn,
        cmd.id,
        cmd.session_id,
        session_owner_route,
    ))
}

fn start_pending_runtime_enable_events_phase(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    session_id: Option<&str>,
    session_owner_route: Option<CdpSessionRoute>,
) -> RuntimeCommandTaskStep {
    match conn.start_runtime_enable_events_for_session_owner(session_id) {
        Ok(pending) => RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
            command_id,
            action: "enable",
            owner_scope: CommandOwnerScope::from_session_and_owner_route(
                session_id,
                session_owner_route,
            ),
            object_group: None,
            release_object_ids: Vec::new(),
            release_object_group: None,
            await_promise: false,
            wait_for_deferred_reply: false,
            pending: PendingRuntimeCommandKind::Enable(pending),
        })),
        Err(message) if message == "NoDocumentLoaded" => {
            RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
        }
        Err(message) => RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message)),
    }
}

fn runtime_remove_binding_should_skip_live_page_update(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> bool {
    conn.target_runtime_session_state_for_session(session_id)
        .is_some_and(|state| state.runtime_frontend_enabled)
}

fn try_start_pending_runtime_binding_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: RuntimeBindingCommand,
) -> Option<RuntimeCommandTaskStep> {
    let session_owner_route = if cmd.session_id.is_none() {
        conn.none_session_owner_route_override()
    } else {
        None
    };
    let (name, execution_context_name, execution_context_id) = match action {
        RuntimeBindingCommand::Add => {
            let params = match cmd.get_params::<AddBindingParams>() {
                Ok(Some(params)) => params,
                _ => {
                    return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32602,
                        "InvalidParams",
                    )));
                }
            };
            (
                params.name,
                params.execution_context_name,
                params.execution_context_id,
            )
        }
        RuntimeBindingCommand::Remove => {
            let params = match cmd
                .get_params::<chromiumoxide_cdp::cdp::js_protocol::runtime::RemoveBindingParams>()
            {
                Ok(Some(params)) => params,
                _ => {
                    return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32602,
                        "InvalidParams",
                    )));
                }
            };
            (params.name, None, None)
        }
    };
    if conn.browser_contexts().next().is_none() {
        return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        )));
    }
    if matches!(action, RuntimeBindingCommand::Add)
        && execution_context_id.is_some()
        && execution_context_name.is_some()
    {
        return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32602,
            "executionContextName is mutually exclusive with executionContextId",
        )));
    }
    let should_persist =
        matches!(action, RuntimeBindingCommand::Remove) || execution_context_id.is_none();
    let mut task = RuntimeBindingCommandTask {
        action,
        renderer_policy: cmd.renderer_policy(),
        phase: RuntimeBindingPhase::LivePageUpdate,
        name,
        execution_context_name,
        execution_context_id,
        inspector_json: Some(cmd.json.to_owned()),
        command_response: None,
        should_persist,
        skip_live_page_update_after_inspector_success: false,
    };
    let live_page_update_unavailable = conn
        .runtime_session_owner_slot(cmd.session_id)
        .is_ok_and(|slot| !slot.has_loaded_page())
        || should_persist
            && conn.renderer_document_navigation_is_suspended_for_session_owner(cmd.session_id);
    if live_page_update_unavailable {
        task.command_response = Some(RuntimeBindingCommandResponse::empty_success());
        let meta = RuntimeCommandCompletionMeta {
            command_id: cmd.id,
            action: task.action.label(),
            owner_scope: CommandOwnerScope::from_session_and_owner_route(
                cmd.session_id,
                session_owner_route.clone(),
            ),
            object_group: None,
            release_object_ids: Vec::new(),
            release_object_group: None,
            await_promise: false,
            wait_for_deferred_reply: false,
        };
        return Some(complete_runtime_binding_after_live_update(conn, meta, task));
    }
    if let Some(execution_context_id) = execution_context_id {
        return start_pending_runtime_binding_context_lookup_phase(
            conn,
            cmd.id,
            cmd.session_id,
            task,
            execution_context_id,
            session_owner_route,
        );
    }
    if matches!(task.action, RuntimeBindingCommand::Add)
        || matches!(task.action, RuntimeBindingCommand::Remove)
            && runtime_remove_binding_should_skip_live_page_update(conn, cmd.session_id)
    {
        task.skip_live_page_update_after_inspector_success = true;
    }
    let action_label = action.label();
    let pending = match action {
        RuntimeBindingCommand::Add => start_pending_runtime_context_resolved_inspector_dispatch(
            conn,
            cmd,
            action_label,
            cmd.json.to_owned(),
        ),
        RuntimeBindingCommand::Remove => start_pending_runtime_inspector_dispatch_with_delivery(
            conn,
            cmd,
            cmd.json.to_owned(),
            RendererInspectorResponseDelivery::CommandReply,
        ),
    };
    let pending = match pending {
        Ok(pending) => pending,
        Err(message) => {
            return Some(RuntimeCommandTaskStep::Complete(
                runtime_inspector_error_plan(cmd.id, message),
            ));
        }
    };
    Some(RuntimeCommandTaskStep::Pending(Box::new(
        PendingRuntimeCommandDispatch {
            command_id: cmd.id,
            action: action_label,
            owner_scope: CommandOwnerScope::from_session_and_owner_route(
                cmd.session_id,
                session_owner_route,
            ),
            object_group: None,
            release_object_ids: Vec::new(),
            release_object_group: None,
            await_promise: false,
            wait_for_deferred_reply: false,
            pending: PendingRuntimeCommandKind::BindingInspector { task, pending },
        },
    )))
}

fn start_pending_runtime_routable_inspector_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    inspector_json: String,
    response_delivery: RendererInspectorResponseDelivery,
) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
    match cmd.renderer_policy().access() {
        CdpRendererCommandAccess::MainThread => {
            start_pending_runtime_inspector_dispatch_with_delivery(
                conn,
                cmd,
                inspector_json,
                response_delivery,
            )
        }
        CdpRendererCommandAccess::Io => start_pending_runtime_io_inspector_dispatch(
            conn,
            cmd,
            inspector_json,
            response_delivery,
        ),
        CdpRendererCommandAccess::OwnerIndependent => Err(
            "an owner-independent command cannot enter the Runtime Inspector dispatcher".to_owned(),
        ),
    }
}

fn start_pending_runtime_io_inspector_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    inspector_json: String,
    response_delivery: RendererInspectorResponseDelivery,
) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
    if let Some(command_id) = cmd.id {
        let descriptor = RendererCommandDescriptor::from_frontend_policy(
            inspector_json,
            cmd.renderer_policy(),
            response_delivery,
        );
        conn.start_runtime_io_protocol_message_for_session_owner_with_deferred_response(
            cmd.session_id,
            descriptor,
            command_id,
        )
    } else {
        conn.start_runtime_io_protocol_message_for_session_owner(cmd.session_id, inspector_json)
    }
}

fn start_pending_runtime_context_resolved_inspector_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: &'static str,
    inspector_json: String,
) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
    start_pending_runtime_context_resolved_inspector_dispatch_with_delivery(
        conn,
        cmd,
        action,
        inspector_json,
        RendererInspectorResponseDelivery::CommandReply,
    )
}

fn start_pending_runtime_context_resolved_inspector_dispatch_with_delivery(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: &'static str,
    inspector_json: String,
    response_delivery: RendererInspectorResponseDelivery,
) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
    if let Some(command_id) = cmd.id {
        let descriptor = RendererCommandDescriptor::from_frontend_policy(
            inspector_json,
            cmd.renderer_policy(),
            response_delivery,
        );
        conn.start_runtime_protocol_message_with_context_resolution_for_session_owner_with_deferred_response(
            cmd.session_id,
            action,
            descriptor,
            command_id,
        )
    } else {
        conn.start_runtime_protocol_message_with_context_resolution_for_session_owner(
            cmd.session_id,
            action,
            inspector_json,
        )
    }
}

fn start_pending_runtime_inspector_dispatch_with_delivery(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    inspector_json: String,
    response_delivery: RendererInspectorResponseDelivery,
) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
    if let Some(command_id) = cmd.id {
        let descriptor = RendererCommandDescriptor::from_frontend_policy(
            inspector_json,
            cmd.renderer_policy(),
            response_delivery,
        );
        conn.start_runtime_protocol_message_for_session_owner_with_deferred_response(
            cmd.session_id,
            descriptor,
            command_id,
        )
    } else {
        conn.start_runtime_protocol_message_for_session_owner(cmd.session_id, inspector_json)
    }
}

fn start_shared_worker_frontend_inspector_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    inspector_json: String,
) -> Result<PendingSharedWorkerRuntimeProtocolMessageDispatch, String> {
    if let Some(command_id) = cmd.id {
        let descriptor = RendererCommandDescriptor::from_frontend_policy(
            inspector_json,
            cmd.renderer_policy(),
            RendererInspectorResponseDelivery::CommandReply,
        );
        conn.start_shared_worker_runtime_protocol_message_for_session_with_deferred_response(
            cmd.session_id,
            descriptor,
            command_id,
        )
    } else {
        conn.start_shared_worker_runtime_protocol_message_for_session(
            cmd.session_id,
            inspector_json,
        )
    }
}

fn start_service_worker_frontend_inspector_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    inspector_json: String,
) -> Result<PendingServiceWorkerRuntimeProtocolMessageDispatch, String> {
    if let Some(command_id) = cmd.id {
        let descriptor = RendererCommandDescriptor::from_frontend_policy(
            inspector_json,
            cmd.renderer_policy(),
            RendererInspectorResponseDelivery::CommandReply,
        );
        conn.start_service_worker_runtime_protocol_message_for_session_with_deferred_response(
            cmd.session_id,
            descriptor,
            command_id,
        )
    } else {
        conn.start_service_worker_runtime_protocol_message_for_session(
            cmd.session_id,
            inspector_json,
        )
    }
}

fn start_cdp_devtools_script_runtime_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command_kind: RuntimeDevToolsScriptCommand,
) -> RuntimeCommandTaskStep {
    let (browser_context_id, target_id) =
        devtools_runtime_owner_identity_for_session(conn, cmd.session_id);
    let action = command_kind.action();
    let await_promise = runtime_command_awaits_promise(cmd, action);
    let command = match command_kind {
        RuntimeDevToolsScriptCommand::Evaluate => {
            DevToolsCommand::EvaluateScript(build_cdp_evaluate_script_command(
                cmd,
                target_id.as_deref(),
                browser_context_id.as_deref(),
                await_promise,
            ))
        }
        RuntimeDevToolsScriptCommand::CallFunctionOn => {
            DevToolsCommand::CallFunction(build_cdp_call_function_command(
                cmd,
                target_id.as_deref(),
                browser_context_id.as_deref(),
            ))
        }
    };
    let inspector_json = match prepare_pending_devtools_runtime_inspector_json(conn, cmd, &command)
    {
        Ok(json) => json,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };
    start_devtools_runtime_command(
        conn,
        cmd,
        command,
        inspector_json,
        await_promise,
        if await_promise {
            RendererInspectorResponseDelivery::CommandReply
        } else {
            cmd.terminal_response_delivery(RendererInspectorResponseDelivery::DevToolsSession)
        },
    )
}

fn prepare_pending_devtools_runtime_inspector_json(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    command: &DevToolsCommand,
) -> Result<String, String> {
    match command {
        DevToolsCommand::EvaluateScript(_) => Ok(cmd.json.to_owned()),
        DevToolsCommand::CallFunction(command) => {
            prepare_pending_devtools_call_function_json(conn, cmd, command)
        }
        _ => Err("UnsupportedDevToolsCommand".to_owned()),
    }
}

fn start_devtools_runtime_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command: DevToolsCommand,
    inspector_json: String,
    wait_for_deferred_reply: bool,
    response_delivery: RendererInspectorResponseDelivery,
) -> RuntimeCommandTaskStep {
    let (action, action_label, await_promise) = match &command {
        DevToolsCommand::EvaluateScript(command) => {
            (RuntimeAction::Evaluate, "evaluate", command.await_promise)
        }
        DevToolsCommand::CallFunction(command) => (
            RuntimeAction::CallFunctionOn,
            "callFunctionOn",
            command.await_promise,
        ),
        _ => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                cmd.id,
                "UnsupportedDevToolsCommand".to_owned(),
            ));
        }
    };
    let object_group = runtime_object_group_for_command_result(conn, cmd, action);
    let session_owner_route = if await_promise {
        conn.runtime_await_owner_route_for_session(cmd.session_id)
    } else {
        None
    };
    let pre_registered_await = match pre_register_runtime_await_if_needed(
        conn,
        await_promise,
        cmd.id,
        cmd.session_id,
        object_group.as_deref(),
        action_label,
    ) {
        Ok(command_id) => command_id,
        Err(error) => {
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
                -32600,
                error.to_string(),
            ));
        }
    };
    let pending = match start_pending_runtime_context_resolved_inspector_dispatch_with_delivery(
        conn,
        cmd,
        action_label,
        inspector_json,
        response_delivery,
    ) {
        Ok(pending) => pending,
        Err(message) => {
            forget_pre_registered_runtime_await(conn, pre_registered_await, cmd.session_id);
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };
    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: action_label,
        owner_scope: CommandOwnerScope::from_session_and_owner_route(
            cmd.session_id,
            session_owner_route,
        ),
        object_group,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise,
        wait_for_deferred_reply,
        pending: PendingRuntimeCommandKind::Inspector { pending },
    }))
}

fn pre_register_runtime_await_if_needed(
    conn: &mut CdpConnection,
    await_promise: bool,
    command_id: Option<u64>,
    session_id: Option<&str>,
    object_group: Option<&str>,
    action: &'static str,
) -> Result<Option<u64>, DuplicatePendingRendererCommand> {
    if !await_promise {
        return Ok(None);
    }
    let Some(command_id) = command_id else {
        return Ok(None);
    };
    conn.try_register_pending_inspector_await_with_object_group(
        command_id,
        session_id,
        object_group,
    )?;
    conn.register_runtime_await_job(command_id, session_id, object_group, action);
    conn.trace_runtime_await_pending_registered(command_id, session_id);
    Ok(Some(command_id))
}

fn forget_pre_registered_runtime_await(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    session_id: Option<&str>,
) {
    if let Some(command_id) = command_id {
        conn.forget_pending_inspector_await(command_id, session_id);
    }
}

pub(crate) async fn execute_devtools_runtime_command_async_with_protocol_events(
    conn: &mut CdpConnection,
    mut command: DevToolsCommand,
) -> DevToolsCommandExecutionOutput {
    if let DevToolsCommand::GetRealms(command) = command {
        return DevToolsCommandExecutionOutput::new(
            execute_devtools_get_realms_command_async(conn, command).await,
        );
    }
    if let DevToolsCommand::ReleaseObjects(command) = command {
        return DevToolsCommandExecutionOutput::new(
            execute_devtools_release_objects_command_async(conn, command).await,
        );
    }
    if let DevToolsCommand::LocateNodes(command) = command {
        return execute_devtools_locate_nodes_command_async(conn, command).await;
    }
    let result_kind = devtools_runtime_command_result_kind(&command);
    let result_ownership = devtools_runtime_result_ownership(&command);
    let serialization_options = devtools_runtime_serialization_options(&command);
    let target = match devtools_runtime_target_async(conn, &command).await {
        Ok(target) => target,
        Err(error) => return DevToolsCommandExecutionOutput::new(Err(error)),
    };
    // Control commands must reach their Inspector route without first asking
    // the Page owner for realm inventory. That owner can be the JavaScript
    // execution this command exists to interrupt.
    let target_realm = match result_kind {
        DevToolsRuntimeCommandResultKind::Script => {
            devtools_realm_id_for_runtime_target_async(conn, &target).await
        }
        DevToolsRuntimeCommandResultKind::Empty => None,
    };
    let mut route_scope = conn.scoped_none_session_owner_route_override(target.route.clone());
    if let DevToolsCommand::CallFunction(call_function) = &mut command
        && matches!(
            call_function.context.protocol,
            DevToolsProtocol::WebDriverBidi
        )
        && let Err(error) = remap_bidi_node_shared_references_for_target_async(
            route_scope.conn_mut(),
            &target,
            call_function,
            target_realm.as_ref(),
        )
        .await
    {
        return DevToolsCommandExecutionOutput::new(Err(error));
    }
    let validation_result = validate_protocol_neutral_runtime_handle_realms(
        route_scope.conn_mut(),
        &command,
        target_realm.as_ref(),
    );
    drop(route_scope);
    if let Err(error) = validation_result {
        return DevToolsCommandExecutionOutput::new(Err(error));
    }
    let internal_command_id = conn.next_internal_runtime_command_id();
    let mut step =
        start_protocol_neutral_runtime_command(conn, target.clone(), command, internal_command_id)
            .await;
    loop {
        match step {
            RuntimeCommandTaskStep::Complete(mut plan) => {
                let renderer_output_predecessor = plan.take_renderer_output_predecessor();
                let (response, protocol_events) = plan
                    .into_runtime_inspector_response_and_background_events(
                        internal_command_id,
                        None,
                    );
                let Some(response) = response else {
                    return DevToolsCommandExecutionOutput::from_parts(
                        Err(DevToolsError::new(
                            DevToolsErrorKind::Internal,
                            "MissingDevToolsCommandResult",
                        )),
                        protocol_events,
                        renderer_output_predecessor,
                    );
                };
                if result_kind == DevToolsRuntimeCommandResultKind::Empty {
                    return DevToolsCommandExecutionOutput::from_parts(
                        devtools_empty_result_from_response(response),
                        protocol_events,
                        renderer_output_predecessor,
                    );
                }
                let mut result = match devtools_script_result_from_response(
                    response,
                    result_ownership,
                    target_realm.clone(),
                ) {
                    Ok(result) => result,
                    Err(error) => {
                        return DevToolsCommandExecutionOutput::from_parts(
                            Err(error),
                            protocol_events,
                            renderer_output_predecessor,
                        );
                    }
                };
                let mut route_scope =
                    conn.scoped_none_session_owner_route_override(target.route.clone());
                register_devtools_script_result_remote_object(route_scope.conn_mut(), &result);
                materialize_devtools_script_dom_collection_remote_value_async(
                    route_scope.conn_mut(),
                    &mut result,
                    serialization_options.as_ref(),
                    &target,
                    target_realm.as_ref(),
                )
                .await;
                materialize_devtools_script_deep_serialized_root_value_async(
                    route_scope.conn_mut(),
                    &mut result,
                    serialization_options.as_ref(),
                    &target,
                )
                .await;
                materialize_devtools_script_node_remote_value_async(
                    route_scope.conn_mut(),
                    &mut result,
                    serialization_options.as_ref(),
                    &target,
                    target_realm.as_ref(),
                )
                .await;
                materialize_devtools_script_deep_serialized_node_remote_values_async(
                    route_scope.conn_mut(),
                    &mut result,
                    serialization_options.as_ref(),
                    &target,
                    target_realm.as_ref(),
                )
                .await;
                materialize_devtools_script_window_remote_value(&mut result, &target);
                register_devtools_script_result_remote_object_realm(
                    route_scope.conn_mut(),
                    &result,
                    target_realm.as_ref(),
                );
                drop(route_scope);
                return DevToolsCommandExecutionOutput::from_parts(
                    Ok(result),
                    protocol_events,
                    renderer_output_predecessor,
                );
            }
            RuntimeCommandTaskStep::Pending(pending) => {
                let mut pending = *pending;
                let completed = if pending.waits_for_scheduler_deferred_inspector_reply() {
                    if let Err(message) = pending
                        .wait_for_scheduler_deferred_inspector_reply_receiver(conn)
                        .await
                    {
                        pending.forget_scheduler_deferred_inspector_reply(conn);
                        return DevToolsCommandExecutionOutput::from_parts(
                            Err(DevToolsError::new(DevToolsErrorKind::Internal, message)),
                            Vec::new(),
                            None,
                        );
                    }
                    pending.complete_scheduler_deferred_inspector_reply(conn)
                } else {
                    pending.wait().await
                };
                let mut route_scope =
                    conn.scoped_none_session_owner_route_override(target.route.clone());
                step = complete_pending_runtime_command(route_scope.conn_mut(), completed).await;
            }
        }
    }
}

impl CdpConnection {
    pub async fn start_devtools_runtime_command_dispatch(
        &mut self,
        mut command: DevToolsCommand,
    ) -> DevToolsRuntimeCommandTaskStep {
        let command_context = command.context().clone();
        if let DevToolsCommand::GetRealms(command) = command {
            let result = execute_devtools_get_realms_command_async(self, command).await;
            return self
                .complete_devtools_runtime_direct_result(command_context, result, Vec::new(), None)
                .await;
        }
        if let DevToolsCommand::ReleaseObjects(command) = command {
            let result = execute_devtools_release_objects_command_async(self, command).await;
            return self
                .complete_devtools_runtime_direct_result(command_context, result, Vec::new(), None)
                .await;
        }
        if let DevToolsCommand::LocateNodes(command) = command {
            let output = execute_devtools_locate_nodes_command_async(self, command).await;
            let (result, protocol_events, renderer_output_predecessor) = output.into_parts();
            return self
                .complete_devtools_runtime_direct_result(
                    command_context,
                    result,
                    protocol_events,
                    renderer_output_predecessor,
                )
                .await;
        }

        let result_kind = devtools_runtime_command_result_kind(&command);
        let result_ownership = devtools_runtime_result_ownership(&command);
        let serialization_options = devtools_runtime_serialization_options(&command);
        let target = match devtools_runtime_target_async(self, &command).await {
            Ok(target) => target,
            Err(error) => {
                return self
                    .complete_devtools_runtime_direct_result(
                        command_context,
                        Err(error),
                        Vec::new(),
                        None,
                    )
                    .await;
            }
        };
        // Keep the interrupt path free of Page-owner realm lookups. In
        // particular, Runtime.terminateExecution must be able to enter its IO
        // envelope while a MainThread script is not yielding.
        let target_realm = match result_kind {
            DevToolsRuntimeCommandResultKind::Script => {
                devtools_realm_id_for_runtime_target_async(self, &target).await
            }
            DevToolsRuntimeCommandResultKind::Empty => None,
        };
        let mut route_scope = self.scoped_none_session_owner_route_override(target.route.clone());
        if let DevToolsCommand::CallFunction(call_function) = &mut command
            && matches!(
                call_function.context.protocol,
                DevToolsProtocol::WebDriverBidi
            )
            && let Err(error) = remap_bidi_node_shared_references_for_target_async(
                route_scope.conn_mut(),
                &target,
                call_function,
                target_realm.as_ref(),
            )
            .await
        {
            drop(route_scope);
            return self
                .complete_devtools_runtime_direct_result(
                    command_context,
                    Err(error),
                    Vec::new(),
                    None,
                )
                .await;
        }
        let validation_result = validate_protocol_neutral_runtime_handle_realms(
            route_scope.conn_mut(),
            &command,
            target_realm.as_ref(),
        );
        drop(route_scope);
        if let Err(error) = validation_result {
            return self
                .complete_devtools_runtime_direct_result(
                    command_context,
                    Err(error),
                    Vec::new(),
                    None,
                )
                .await;
        }

        let internal_command_id = self.next_internal_runtime_command_id();
        let state = DevToolsRuntimeCommandDispatchState {
            internal_command_id,
            command_context,
            result_kind,
            result_ownership,
            serialization_options,
            target: target.clone(),
            target_realm,
        };
        let step =
            start_protocol_neutral_runtime_command(self, target, command, internal_command_id)
                .await;
        self.complete_devtools_runtime_command_step(state, step, Vec::new())
            .await
    }

    pub async fn complete_devtools_runtime_command_dispatch(
        &mut self,
        completed: CompletedDevToolsRuntimeCommandDispatch,
    ) -> DevToolsRuntimeCommandTaskStep {
        let route = completed.state.target.route.clone();
        let mut route_scope = self.scoped_none_session_owner_route_override(route);
        let step =
            complete_pending_runtime_command(route_scope.conn_mut(), completed.completed).await;
        drop(route_scope);
        self.complete_devtools_runtime_command_step(
            completed.state,
            step,
            completed.interleaved_protocol_events,
        )
        .await
    }

    async fn complete_devtools_runtime_command_step(
        &mut self,
        state: DevToolsRuntimeCommandDispatchState,
        step: RuntimeCommandTaskStep,
        interleaved_protocol_events: Vec<BackgroundProtocolEvent>,
    ) -> DevToolsRuntimeCommandTaskStep {
        match step {
            RuntimeCommandTaskStep::Pending(pending) => DevToolsRuntimeCommandTaskStep::Pending(
                Box::new(PendingDevToolsRuntimeCommandDispatch {
                    state,
                    pending: *pending,
                    interleaved_protocol_events,
                    scheduler_events: self.take_scheduler_events(),
                }),
            ),
            RuntimeCommandTaskStep::Complete(mut plan) => {
                for event in interleaved_protocol_events {
                    push_runtime_protocol_event_or_background_event(
                        &mut plan,
                        Some(state.internal_command_id),
                        event,
                    );
                }
                self.complete_devtools_runtime_command_plan(state, plan)
                    .await
            }
        }
    }

    async fn complete_devtools_runtime_command_plan(
        &mut self,
        state: DevToolsRuntimeCommandDispatchState,
        mut plan: CommandOutputPlan,
    ) -> DevToolsRuntimeCommandTaskStep {
        let renderer_output_predecessor = plan.take_renderer_output_predecessor();
        let (response, protocol_events) = plan
            .into_runtime_inspector_response_and_background_events(state.internal_command_id, None);
        let Some(response) = response else {
            return self
                .complete_devtools_runtime_direct_result(
                    state.command_context,
                    Err(DevToolsError::new(
                        DevToolsErrorKind::Internal,
                        "MissingDevToolsCommandResult",
                    )),
                    protocol_events,
                    renderer_output_predecessor,
                )
                .await;
        };
        if state.result_kind == DevToolsRuntimeCommandResultKind::Empty {
            return self
                .complete_devtools_runtime_direct_result(
                    state.command_context,
                    devtools_empty_result_from_response(response),
                    protocol_events,
                    renderer_output_predecessor,
                )
                .await;
        }
        let mut result = match devtools_script_result_from_response(
            response,
            state.result_ownership,
            state.target_realm.clone(),
        ) {
            Ok(result) => result,
            Err(error) => {
                return self
                    .complete_devtools_runtime_direct_result(
                        state.command_context,
                        Err(error),
                        protocol_events,
                        renderer_output_predecessor,
                    )
                    .await;
            }
        };
        let mut route_scope =
            self.scoped_none_session_owner_route_override(state.target.route.clone());
        register_devtools_script_result_remote_object(route_scope.conn_mut(), &result);
        materialize_devtools_script_dom_collection_remote_value_async(
            route_scope.conn_mut(),
            &mut result,
            state.serialization_options.as_ref(),
            &state.target,
            state.target_realm.as_ref(),
        )
        .await;
        materialize_devtools_script_deep_serialized_root_value_async(
            route_scope.conn_mut(),
            &mut result,
            state.serialization_options.as_ref(),
            &state.target,
        )
        .await;
        materialize_devtools_script_node_remote_value_async(
            route_scope.conn_mut(),
            &mut result,
            state.serialization_options.as_ref(),
            &state.target,
            state.target_realm.as_ref(),
        )
        .await;
        materialize_devtools_script_deep_serialized_node_remote_values_async(
            route_scope.conn_mut(),
            &mut result,
            state.serialization_options.as_ref(),
            &state.target,
            state.target_realm.as_ref(),
        )
        .await;
        materialize_devtools_script_window_remote_value(&mut result, &state.target);
        register_devtools_script_result_remote_object_realm(
            route_scope.conn_mut(),
            &result,
            state.target_realm.as_ref(),
        );
        drop(route_scope);
        self.complete_devtools_runtime_direct_result(
            state.command_context,
            Ok(result),
            protocol_events,
            renderer_output_predecessor,
        )
        .await
    }

    async fn complete_devtools_runtime_direct_result(
        &mut self,
        command_context: DevToolsCommandContext,
        result: Result<DevToolsCommandResult, DevToolsError>,
        protocol_events: Vec<BackgroundProtocolEvent>,
        renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
    ) -> DevToolsRuntimeCommandTaskStep {
        DevToolsRuntimeCommandTaskStep::Complete(Box::new(
            self.finish_devtools_command_dispatch(
                command_context,
                result,
                protocol_events,
                renderer_output_predecessor,
            )
            .await,
        ))
    }
}

async fn execute_devtools_locate_nodes_command_async(
    conn: &mut CdpConnection,
    mut command: DevToolsLocateNodesCommand,
) -> DevToolsCommandExecutionOutput {
    if matches!(&command.locator, DevToolsLocateNodesLocator::Context(_)) {
        return execute_devtools_locate_nodes_context_command_async(conn, command).await;
    }
    let target =
        match devtools_runtime_target_async(conn, &DevToolsCommand::LocateNodes(command.clone()))
            .await
        {
            Ok(target) => target,
            Err(error) => return DevToolsCommandExecutionOutput::new(Err(error)),
        };
    let context = command.context.clone();
    let locator = command.locator.clone();
    let start_nodes = std::mem::take(&mut command.start_nodes);
    let start_node_references = std::mem::take(&mut command.start_node_references);
    let start_node_inputs = match locate_nodes_start_node_inputs_async(
        conn,
        &target,
        start_nodes,
        start_node_references,
    )
    .await
    {
        Ok(inputs) => inputs,
        Err(error) => return DevToolsCommandExecutionOutput::new(Err(error)),
    };
    let (start_node_arguments, start_node_handles) =
        match resolve_locate_nodes_start_node_inputs(conn, &context, start_node_inputs).await {
            Ok(arguments) => arguments,
            Err(error) => return DevToolsCommandExecutionOutput::new(Err(error)),
        };
    let call_function = locate_nodes_call_function_command(command, start_node_arguments);
    let output = Box::pin(execute_devtools_runtime_command_async_with_protocol_events(
        conn,
        DevToolsCommand::CallFunction(call_function),
    ))
    .await;
    let (result, protocol_events, renderer_output_predecessor) = output.into_parts();
    release_locate_nodes_start_node_handles(conn, context, start_node_handles).await;
    let result = match result {
        Ok(result) => {
            locate_nodes_result_from_script_result_async(conn, &target, result, &locator).await
        }
        Err(error) => Err(error),
    };
    DevToolsCommandExecutionOutput::from_parts(result, protocol_events, renderer_output_predecessor)
}

async fn execute_devtools_locate_nodes_context_command_async(
    conn: &mut CdpConnection,
    command: DevToolsLocateNodesCommand,
) -> DevToolsCommandExecutionOutput {
    let DevToolsLocateNodesCommand {
        context,
        locator,
        start_nodes,
        start_node_references,
        serialization_options,
        ..
    } = command;
    let DevToolsLocateNodesLocator::Context(ref frame_id) = locator else {
        return DevToolsCommandExecutionOutput::new(Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "UnexpectedLocateNodesContextLocator",
        )));
    };
    if !start_nodes.is_empty() || !start_node_references.is_empty() {
        return DevToolsCommandExecutionOutput::new(Err(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "Start nodes are not supported",
        )));
    }

    let owner_result = match crate::domains::dom::execute_devtools_dom_command_async(
        conn,
        DevToolsCommand::GetFrameOwner(DevToolsGetFrameOwnerCommand {
            context: context.clone(),
            frame_id: frame_id.clone(),
        }),
    )
    .await
    .map_err(locate_nodes_context_owner_error)
    {
        Ok(result) => result,
        Err(error) => return DevToolsCommandExecutionOutput::new(Err(error)),
    };
    let DevToolsCommandResult::GetFrameOwner(owner_result) = owner_result else {
        return DevToolsCommandExecutionOutput::new(Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "UnexpectedLocateNodesFrameOwnerResult",
        )));
    };
    let owner_node_id = owner_result.node_id;

    let max_dom_depth = serialization_options
        .as_ref()
        .and_then(|options| options.max_dom_depth);
    let (start_node_arguments, start_node_handles) =
        match resolve_locate_nodes_start_node_reference_arguments(
            conn,
            &context,
            vec![DevToolsDomNodeReference::BackendNodeId(
                owner_result.backend_node_id,
            )],
        )
        .await
        .map_err(locate_nodes_context_owner_error)
        {
            Ok(arguments) => arguments,
            Err(error) => return DevToolsCommandExecutionOutput::new(Err(error)),
        };
    let call_function_context = context.clone();
    let call_function = DevToolsCallFunctionCommand {
        context: call_function_context,
        realm_id: None,
        world_name: None,
        object_id: None,
        this_parameter: None,
        function_declaration: r#"function(node) {
            if (!node) {
                throw new Error('Context does not exist');
            }
            return [node];
        }"#
        .to_owned(),
        arguments: start_node_arguments,
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: Some(DevToolsSerializationOptions {
            max_object_depth: Some(1),
            max_dom_depth,
            include_shadow_tree: serialization_options
                .as_ref()
                .and_then(|options| options.include_shadow_tree.clone()),
        }),
    };
    let output = Box::pin(execute_devtools_runtime_command_async_with_protocol_events(
        conn,
        DevToolsCommand::CallFunction(call_function),
    ))
    .await;
    let (result, protocol_events, renderer_output_predecessor) = output.into_parts();
    release_locate_nodes_start_node_handles(conn, context, start_node_handles).await;
    let result = result.and_then(|result| {
        let mut result = locate_nodes_result_from_script_result(result, &locator)?;
        if let DevToolsCommandResult::LocateNodes(result) = &mut result
            && result.node_ids.is_empty()
        {
            result.node_ids.push(owner_node_id);
            if let Some(node) = result.nodes.first_mut() {
                node.node_id = Some(owner_node_id);
            }
        }
        Ok(result)
    });
    DevToolsCommandExecutionOutput::from_parts(result, protocol_events, renderer_output_predecessor)
}

fn locate_nodes_context_owner_error(error: DevToolsError) -> DevToolsError {
    match error.kind {
        DevToolsErrorKind::NoSuchSession | DevToolsErrorKind::NoSuchTarget => error,
        _ => DevToolsError::new(DevToolsErrorKind::InvalidArgument, "Context does not exist"),
    }
}

fn locate_nodes_call_function_command(
    command: DevToolsLocateNodesCommand,
    resolved_start_node_arguments: Vec<Value>,
) -> DevToolsCallFunctionCommand {
    let DevToolsLocateNodesCommand {
        context,
        locator,
        max_node_count,
        start_nodes,
        start_node_references: _,
        serialization_options,
    } = command;
    let max_node_count = max_node_count.unwrap_or(0);
    let mut arguments = locate_nodes_locator_arguments(&locator, max_node_count);
    arguments.extend(resolved_start_node_arguments);
    arguments.extend(start_nodes);
    let max_dom_depth = serialization_options
        .as_ref()
        .and_then(|options| options.max_dom_depth);

    DevToolsCallFunctionCommand {
        context,
        realm_id: None,
        world_name: None,
        object_id: None,
        this_parameter: None,
        function_declaration: locate_nodes_function_declaration(&locator).to_owned(),
        arguments,
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: Some(DevToolsSerializationOptions {
            max_object_depth: Some(2),
            max_dom_depth,
            include_shadow_tree: serialization_options
                .as_ref()
                .and_then(|options| options.include_shadow_tree.clone()),
        }),
    }
}

async fn locate_nodes_start_node_inputs_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    start_nodes: Vec<Value>,
    start_node_references: Vec<DevToolsDomNodeReference>,
) -> Result<Vec<LocateNodesStartNodeInput>, DevToolsError> {
    let mut inputs = start_node_references
        .into_iter()
        .map(LocateNodesStartNodeInput::Reference)
        .collect::<Vec<_>>();
    for node in start_nodes {
        let Some(object) = node.as_object() else {
            return Err(DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                "locateNodes startNodes entries must be node remote values",
            ));
        };
        if let Some(shared_id) = object.get("sharedId").and_then(Value::as_str) {
            if let Some(reference) = renderer_locate_nodes_start_node_reference_for_shared_id_async(
                conn, target, shared_id,
            )
            .await?
            {
                inputs.push(LocateNodesStartNodeInput::Reference(reference));
            } else {
                inputs.push(LocateNodesStartNodeInput::Raw(node));
            }
            continue;
        }
        if object.get("handle").and_then(Value::as_str).is_some() {
            inputs.push(LocateNodesStartNodeInput::Raw(node));
            continue;
        }
        return Err(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "locateNodes startNodes entries must be node remote values",
        ));
    }
    Ok(inputs)
}

async fn renderer_locate_nodes_start_node_reference_for_shared_id_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    shared_id: &str,
) -> Result<Option<DevToolsDomNodeReference>, DevToolsError> {
    let mut route_scope = conn.scoped_none_session_owner_route_override(target.route.clone());
    let result = route_scope
        .conn_mut()
        .document_bidi_node_binding_for_session_owner_async(None, shared_id)
        .await;
    drop(route_scope);
    match result {
        Ok(RendererDomBidiNodeBindingResolution::BackendNodeId(backend_node_id)) => Ok(Some(
            DevToolsDomNodeReference::BackendNodeId(backend_node_id),
        )),
        Ok(RendererDomBidiNodeBindingResolution::NotFound) => Ok(None),
        Err(error) => {
            tracing::debug!(
                %error,
                %shared_id,
                "failed to resolve locateNodes start node through renderer BiDi binding"
            );
            Ok(None)
        }
    }
}

async fn resolve_locate_nodes_start_node_inputs(
    conn: &mut CdpConnection,
    context: &DevToolsCommandContext,
    inputs: Vec<LocateNodesStartNodeInput>,
) -> Result<(Vec<Value>, Vec<DevToolsRemoteHandleId>), DevToolsError> {
    let mut arguments = Vec::with_capacity(inputs.len());
    let mut handles = Vec::new();
    for input in inputs {
        match input {
            LocateNodesStartNodeInput::Raw(value) => arguments.push(value),
            LocateNodesStartNodeInput::Reference(reference) => {
                let resolved = resolve_locate_nodes_start_node_reference_arguments(
                    conn,
                    context,
                    vec![reference],
                )
                .await;
                let (mut resolved_arguments, mut resolved_handles) = match resolved {
                    Ok(resolved) => resolved,
                    Err(error) => {
                        release_locate_nodes_start_node_handles(conn, context.clone(), handles)
                            .await;
                        return Err(error);
                    }
                };
                arguments.append(&mut resolved_arguments);
                handles.append(&mut resolved_handles);
            }
        }
    }
    Ok((arguments, handles))
}

async fn resolve_locate_nodes_start_node_reference_arguments(
    conn: &mut CdpConnection,
    context: &DevToolsCommandContext,
    references: Vec<DevToolsDomNodeReference>,
) -> Result<(Vec<Value>, Vec<DevToolsRemoteHandleId>), DevToolsError> {
    let mut arguments = Vec::with_capacity(references.len());
    let mut handles = Vec::with_capacity(references.len());
    for reference in references {
        let result = crate::domains::dom::execute_devtools_dom_command_async(
            conn,
            DevToolsCommand::ResolveNode(DevToolsResolveNodeCommand {
                context: context.clone(),
                reference,
                execution_context_id: None,
                object_group: Some(LOCATE_NODES_START_NODE_OBJECT_GROUP.to_owned()),
            }),
        )
        .await;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                release_locate_nodes_start_node_handles(conn, context.clone(), handles).await;
                return Err(error);
            }
        };
        let DevToolsCommandResult::ResolveNode(result) = result else {
            release_locate_nodes_start_node_handles(conn, context.clone(), handles).await;
            return Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                "LocateNodesStartNodeResolveReturnedUnexpectedResult",
            ));
        };
        let Some(object_id) = result
            .object
            .get("objectId")
            .and_then(Value::as_str)
            .map(DevToolsRemoteHandleId::from)
        else {
            release_locate_nodes_start_node_handles(conn, context.clone(), handles).await;
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchNode,
                "start node is no longer attached to the DOM",
            ));
        };
        arguments.push(json!({ "handle": object_id.as_str() }));
        handles.push(object_id);
    }
    Ok((arguments, handles))
}

async fn release_locate_nodes_start_node_handles(
    conn: &mut CdpConnection,
    context: DevToolsCommandContext,
    handles: Vec<DevToolsRemoteHandleId>,
) {
    if handles.is_empty() {
        return;
    }
    if let Err(error) = execute_devtools_release_objects_command_async(
        conn,
        DevToolsReleaseObjectsCommand {
            context,
            realm_id: None,
            world_name: None,
            handles,
        },
    )
    .await
    {
        tracing::debug!(?error, "failed to release locateNodes start node handles");
    }
}

fn locate_nodes_locator_arguments(
    locator: &DevToolsLocateNodesLocator,
    max_node_count: u64,
) -> Vec<Value> {
    match locator {
        DevToolsLocateNodesLocator::Css(selector)
        | DevToolsLocateNodesLocator::XPath(selector)
        | DevToolsLocateNodesLocator::TagName(selector) => {
            vec![json!(selector), json!(max_node_count)]
        }
        DevToolsLocateNodesLocator::LinkText { value, match_type } => {
            vec![
                json!(value),
                json!(matches!(match_type, DevToolsLocateNodesTextMatch::Full)),
                json!(max_node_count),
            ]
        }
        DevToolsLocateNodesLocator::Context(_) => {
            unreachable!("context locator is handled before selector delegate")
        }
        DevToolsLocateNodesLocator::InnerText {
            value,
            ignore_case,
            match_type,
            max_depth,
        } => vec![
            json!(value),
            json!(matches!(match_type, DevToolsLocateNodesTextMatch::Full)),
            json!(*ignore_case),
            json!(max_node_count),
            json!(max_depth),
        ],
        DevToolsLocateNodesLocator::Accessibility { role, name } => vec![
            json!(name.as_deref().unwrap_or_default()),
            json!(role.as_deref().unwrap_or_default()),
            json!(max_node_count),
        ],
    }
}

fn locate_nodes_function_declaration(locator: &DevToolsLocateNodesLocator) -> &'static str {
    match locator {
        DevToolsLocateNodesLocator::Context(_) => {
            unreachable!("context locator is handled before selector delegate")
        }
        DevToolsLocateNodesLocator::Css(_) => {
            r#"function(cssSelector, maxNodeCount, ...startNodes) {
                const locatedNodeRecords = (nodes) => {
                    const returned = maxNodeCount === 0 ? nodes : nodes.slice(0, maxNodeCount);
                    return returned.map((node) => ({
                        backendNodeId: __moliHostResolveBackendNodeIdForObject(node),
                        node,
                    }));
                };
                const locateNodesUsingCss = (node) => {
                    if (!(node instanceof HTMLElement ||
                        node instanceof Document ||
                        node instanceof DocumentFragment ||
                        node instanceof SVGElement)) {
                        throw new Error('startNodes in css selector should be HTMLElement, SVGElement or Document or DocumentFragment');
                    }
                    return Array.from(node.querySelectorAll(cssSelector));
                };
                startNodes = startNodes.length > 0
                    ? startNodes.filter(Boolean)
                    : [document];
                const returnedNodes = startNodes.flatMap((startNode) => locateNodesUsingCss(startNode));
                return locatedNodeRecords(returnedNodes);
            }"#
        }
        DevToolsLocateNodesLocator::XPath(_) => {
            r#"function(xPathSelector, maxNodeCount, ...startNodes) {
                const locatedNodeRecords = (nodes) => {
                    const returned = maxNodeCount === 0 ? nodes : nodes.slice(0, maxNodeCount);
                    return returned.map((node) => ({
                        backendNodeId: __moliHostResolveBackendNodeIdForObject(node),
                        node,
                    }));
                };
                const locateNodesUsingXpath = (node) => {
                    const documentForNode = node.nodeType === Node.DOCUMENT_NODE ? node : node.ownerDocument;
                    const xPathResult = documentForNode.evaluate(
                        xPathSelector,
                        node,
                        null,
                        XPathResult.ORDERED_NODE_SNAPSHOT_TYPE,
                        null
                    );
                    const returnedNodes = [];
                    for (let index = 0; index < xPathResult.snapshotLength; index += 1) {
                        returnedNodes.push(xPathResult.snapshotItem(index));
                    }
                    return returnedNodes;
                };
                startNodes = startNodes.length > 0
                    ? startNodes.filter(Boolean)
                    : [document];
                const returnedNodes = startNodes.flatMap((startNode) => locateNodesUsingXpath(startNode));
                return locatedNodeRecords(returnedNodes);
            }"#
        }
        DevToolsLocateNodesLocator::TagName(_) => {
            r#"function(tagName, maxNodeCount, ...startNodes) {
                const locatedNodeRecords = (nodes) => {
                    const returned = maxNodeCount === 0 ? nodes : nodes.slice(0, maxNodeCount);
                    return returned.map((node) => ({
                        backendNodeId: __moliHostResolveBackendNodeIdForObject(node),
                        node,
                    }));
                };
                if (tagName === '') {
                    throw new Error('Unable to locate an element with the tagName ""');
                }
                const locateNodesUsingTagName = (node) => {
                    if (!(node instanceof HTMLElement ||
                        node instanceof Document ||
                        node instanceof DocumentFragment ||
                        node instanceof SVGElement)) {
                        throw new Error('startNodes in tag name should be HTMLElement, SVGElement or Document or DocumentFragment');
                    }
                    return Array.from(node.getElementsByTagName(tagName));
                };
                startNodes = startNodes.length > 0
                    ? startNodes.filter(Boolean)
                    : [document];
                const returnedNodes = startNodes.flatMap((startNode) => locateNodesUsingTagName(startNode));
                return locatedNodeRecords(returnedNodes);
            }"#
        }
        DevToolsLocateNodesLocator::LinkText { .. } => {
            r#"function(linkText, fullMatch, maxNodeCount, ...startNodes) {
                const locatedNodeRecords = (nodes) => {
                    const returned = maxNodeCount === 0 ? nodes : nodes.slice(0, maxNodeCount);
                    return returned.map((node) => ({
                        backendNodeId: __moliHostResolveBackendNodeIdForObject(node),
                        node,
                    }));
                };
                const visibleLinkText = (element) => (element.innerText || element.textContent || '').trim();
                const linkMatches = (element) => {
                    const text = visibleLinkText(element);
                    return fullMatch ? text === linkText : text.includes(linkText);
                };
                const locateNodesUsingLinkText = (node) => {
                    if (!(node instanceof HTMLElement ||
                        node instanceof Document ||
                        node instanceof DocumentFragment ||
                        node instanceof SVGElement)) {
                        throw new Error('startNodes in link text should be HTMLElement, SVGElement or Document or DocumentFragment');
                    }
                    return Array.from(node.getElementsByTagName('a')).filter(linkMatches);
                };
                startNodes = startNodes.length > 0
                    ? startNodes.filter(Boolean)
                    : [document];
                const returnedNodes = startNodes.flatMap((startNode) => locateNodesUsingLinkText(startNode));
                return locatedNodeRecords(returnedNodes);
            }"#
        }
        DevToolsLocateNodesLocator::InnerText { .. } => {
            r#"function(innerTextSelector, fullMatch, ignoreCase, maxNodeCount, maxDepth, ...startNodes) {
                const locatedNodeRecords = (nodes) => {
                    const returned = maxNodeCount === 0 ? nodes : nodes.slice(0, maxNodeCount);
                    return returned.map((node) => ({
                        backendNodeId: __moliHostResolveBackendNodeIdForObject(node),
                        node,
                    }));
                };
                const searchText = ignoreCase ? innerTextSelector.toUpperCase() : innerTextSelector;
                const locateNodesUsingInnerText = (node, currentMaxDepth) => {
                    const returnedNodes = [];
                    if (node instanceof DocumentFragment || node instanceof Document) {
                        for (const child of node.children) {
                            returnedNodes.push(...locateNodesUsingInnerText(child, currentMaxDepth));
                        }
                        return returnedNodes;
                    }
                    if (!(node instanceof HTMLElement)) {
                        return [];
                    }
                    const nodeInnerText = ignoreCase ? node.innerText?.toUpperCase() : node.innerText;
                    if (!nodeInnerText || !nodeInnerText.includes(searchText)) {
                        return [];
                    }
                    const childNodes = Array.from(node.children).filter((child) => child instanceof HTMLElement);
                    if (childNodes.length === 0) {
                        if (!fullMatch || nodeInnerText === searchText) {
                            returnedNodes.push(node);
                        }
                        return returnedNodes;
                    }
                    const childNodeMatches = currentMaxDepth <= 0
                        ? []
                        : childNodes.flatMap((child) => locateNodesUsingInnerText(child, currentMaxDepth - 1));
                    if (childNodeMatches.length === 0) {
                        if (!fullMatch || nodeInnerText === searchText) {
                            returnedNodes.push(node);
                        }
                    } else {
                        returnedNodes.push(...childNodeMatches);
                    }
                    return returnedNodes;
                };
                startNodes = startNodes.length > 0
                    ? startNodes.filter(Boolean)
                    : [document];
                const returnedNodes = startNodes.flatMap((startNode) => locateNodesUsingInnerText(startNode, maxDepth));
                return locatedNodeRecords(returnedNodes);
            }"#
        }
        DevToolsLocateNodesLocator::Accessibility { .. } => {
            r#"function(name, role, maxNodeCount, ...startNodes) {
                const locatedNodeRecords = (nodes) => {
                    const returned = maxNodeCount === 0 ? nodes : nodes.slice(0, maxNodeCount);
                    return returned.map((node) => ({
                        backendNodeId: __moliHostResolveBackendNodeIdForObject(node),
                        node,
                    }));
                };
                const implicitRole = (element) => {
                    const localName = element.localName;
                    if (/^h[1-6]$/.test(localName)) { return 'heading'; }
                    if (localName === 'article') { return 'article'; }
                    if (localName === 'button') { return 'button'; }
                    if (localName === 'input' && String(element.type || 'text') === 'text') { return 'textbox'; }
                    if (localName === 'input' && String(element.type || '') === 'search') { return 'searchbox'; }
                    if (localName === 'a' && element.hasAttribute('href')) { return 'link'; }
                    return '';
                };
                const accessibleRole = (element) => element.getAttribute('role') || implicitRole(element);
                const accessibleName = (element) => {
                    if (element.hasAttribute('aria-label')) { return element.getAttribute('aria-label') || ''; }
                    return (element.innerText || element.textContent || '').trim();
                };
                const returnedNodes = [];
                let aborted = false;
                const collect = (contextNodes) => {
                    if (aborted) { return; }
                    for (const contextNode of contextNodes) {
                        if (!(contextNode instanceof HTMLElement || contextNode instanceof SVGElement)) {
                            continue;
                        }
                        let matches = true;
                        if (role && accessibleRole(contextNode) !== role) { matches = false; }
                        if (name && accessibleName(contextNode) !== name) { matches = false; }
                        if (matches) {
                            if (maxNodeCount !== 0 && returnedNodes.length === maxNodeCount) {
                                aborted = true;
                                break;
                            }
                            returnedNodes.push(contextNode);
                        }
                        collect(Array.from(contextNode.children));
                    }
                };
                startNodes = startNodes.length > 0
                    ? startNodes.filter(Boolean)
                    : Array.from(document.documentElement.children).filter((child) => child instanceof HTMLElement || child instanceof SVGElement);
                collect(startNodes);
                return locatedNodeRecords(returnedNodes);
            }"#
        }
    }
}

async fn locate_nodes_result_from_script_result_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    result: DevToolsCommandResult,
    locator: &DevToolsLocateNodesLocator,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut result = locate_nodes_result_from_script_result(result, locator)?;
    if let DevToolsCommandResult::LocateNodes(result) = &mut result {
        materialize_locate_nodes_result_node_ids_async(conn, target, result).await;
    }
    Ok(result)
}

async fn materialize_locate_nodes_result_node_ids_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    result: &mut DevToolsLocateNodesResult,
) {
    let mut route_scope = conn.scoped_none_session_owner_route_override(target.route.clone());
    for index in 0..result.nodes.len() {
        let shared_id = result.nodes[index]
            .shared_id
            .as_ref()
            .map(|shared_id| shared_id.as_str().to_owned());
        let mut materialized = false;
        if let Some(shared_id) = shared_id {
            let snapshot = route_scope
                .conn_mut()
                .document_node_snapshot_for_runtime_remote_object_id_async(
                    None, &shared_id, 0, false,
                )
                .await;
            if let Ok(Some(snapshot)) = snapshot {
                result.nodes[index].node_id =
                    frontend_node_id_for_locate_node_snapshot(&snapshot.snapshot);
                result.nodes[index].backend_node_id = snapshot
                    .snapshot
                    .backend_node_id
                    .or(result.nodes[index].backend_node_id);
                materialized = true;
            }
        }

        if !materialized
            && let Some(backend_node_id) = result.nodes[index].backend_node_id
            && let Some(snapshot) = locate_nodes_snapshot_for_backend_node_id_async(
                route_scope.conn_mut(),
                backend_node_id,
            )
            .await
        {
            result.nodes[index].node_id =
                frontend_node_id_for_locate_node_snapshot(&snapshot.snapshot);
            result.nodes[index].backend_node_id =
                snapshot.snapshot.backend_node_id.or(Some(backend_node_id));
        }
    }
    drop(route_scope);
    result.node_ids = result
        .nodes
        .iter()
        .filter_map(|node| node.node_id)
        .collect();
}

async fn locate_nodes_snapshot_for_backend_node_id_async(
    conn: &mut CdpConnection,
    backend_node_id: u32,
) -> Option<DocumentNodeObjectSnapshot> {
    let pending = conn
        .loaded_page_mut_for_protocol_access(None)
        .ok()?
        .start_document_node_snapshot_for_backend_node_id(backend_node_id, 0, false)
        .ok()?;
    let completion = pending.wait().await.ok()?;
    conn.loaded_page_mut_for_protocol_access(None)
        .ok()?
        .finish_document_node_snapshot_for_backend_node_id(completion)
        .ok()?
}

fn frontend_node_id_for_locate_node_snapshot(snapshot: &DocumentNodeSnapshot) -> Option<u32> {
    snapshot.frontend_node_id
}

fn locate_nodes_result_from_script_result(
    result: DevToolsCommandResult,
    locator: &DevToolsLocateNodesLocator,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let DevToolsCommandResult::Script(result) = result else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "UnexpectedLocateNodesResult",
        ));
    };
    match *result {
        DevToolsScriptResult::Value(value) => {
            let Some(deep_serialized_value) = value.deep_serialized_value else {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "LocateNodesMissingSerializedArray",
                ));
            };
            let Some(nodes) =
                locate_nodes_remote_values_from_deep_serialized_array(&deep_serialized_value)
            else {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "LocateNodesUnexpectedSerializedArray",
                ));
            };
            Ok(DevToolsCommandResult::LocateNodes(
                DevToolsLocateNodesResult {
                    node_ids: nodes.iter().filter_map(|node| node.node_id).collect(),
                    nodes,
                },
            ))
        }
        DevToolsScriptResult::Exception(exception) => {
            Err(locate_nodes_error_from_exception(exception, locator))
        }
    }
}

fn locate_nodes_error_from_exception(
    exception: DevToolsScriptException,
    locator: &DevToolsLocateNodesLocator,
) -> DevToolsError {
    let text = locate_nodes_exception_text(&exception);
    if locate_nodes_exception_is_invalid_selector(&text, locator) {
        return DevToolsError::new(DevToolsErrorKind::InvalidSelector, text);
    }
    if text
        == "Error: startNodes in css selector should be HTMLElement, SVGElement or Document or DocumentFragment"
        || text
            == "Error: startNodes in tag name should be HTMLElement, SVGElement or Document or DocumentFragment"
    {
        return DevToolsError::new(DevToolsErrorKind::InvalidArgument, text);
    }
    if text == "Error: Context does not exist" {
        return DevToolsError::new(DevToolsErrorKind::InvalidArgument, text);
    }
    DevToolsError::new(
        DevToolsErrorKind::Internal,
        format!("Unexpected error in selector script: {text}"),
    )
}

fn locate_nodes_exception_is_invalid_selector(
    text: &str,
    locator: &DevToolsLocateNodesLocator,
) -> bool {
    (text.starts_with("SyntaxError: Failed to execute 'querySelectorAll' on '")
        && text.ends_with(" is not a valid selector."))
        || text.starts_with("SyntaxError: Failed to execute 'evaluate' on 'Document': ")
        || text.starts_with("DOMException: Failed to execute 'evaluate' on 'Document': ")
        || text.starts_with("DOMException: The string ")
            && text.ends_with(" is not a valid XPath expression.")
        || text == "DOMException" && matches!(locator, DevToolsLocateNodesLocator::XPath(_))
}

fn locate_nodes_exception_text(exception: &DevToolsScriptException) -> String {
    if exception.text != "Uncaught" {
        return exception.text.clone();
    }
    if let Some(description) = exception
        .value
        .as_ref()
        .and_then(|value| value.description.as_ref())
    {
        return description.clone();
    }
    if let Some(value) = exception
        .value
        .as_ref()
        .and_then(|value| value.value.as_str())
    {
        return value.to_owned();
    }
    exception.text.clone()
}

fn locate_nodes_remote_values_from_deep_serialized_array(
    value: &Value,
) -> Option<Vec<DevToolsRemoteValue>> {
    let object = value.as_object()?;
    if object.get("type").and_then(Value::as_str) != Some("array") {
        return None;
    }
    object
        .get("value")?
        .as_array()?
        .iter()
        .map(locate_node_remote_value_from_deep_serialized)
        .collect()
}

fn locate_node_remote_value_from_deep_serialized(value: &Value) -> Option<DevToolsRemoteValue> {
    if let Some((backend_node_id, node)) = locate_node_record_from_deep_serialized(value) {
        let mut remote = locate_node_remote_value_from_deep_serialized_node(node)?;
        remote.backend_node_id = backend_node_id;
        return Some(remote);
    }
    locate_node_remote_value_from_deep_serialized_node(value)
}

fn locate_node_record_from_deep_serialized(value: &Value) -> Option<(Option<u32>, &Value)> {
    let object = value.as_object()?;
    if object.get("type").and_then(Value::as_str) != Some("object") {
        return None;
    }
    let properties = object.get("value")?.as_array()?;
    let node = deep_serialized_property(properties, "node")?;
    let backend_node_id = deep_serialized_property(properties, "backendNodeId")
        .and_then(deep_serialized_number_value)
        .and_then(|id| u32::try_from(id).ok());
    Some((backend_node_id, node))
}

fn deep_serialized_number_value(value: &Value) -> Option<u64> {
    let object = value.as_object()?;
    if object.get("type").and_then(Value::as_str) != Some("number") {
        return None;
    }
    object.get("value")?.as_u64()
}

fn locate_node_remote_value_from_deep_serialized_node(
    value: &Value,
) -> Option<DevToolsRemoteValue> {
    let object = value.as_object()?;
    if object.get("type").and_then(Value::as_str) != Some("node") {
        return None;
    }
    let shared_id = object
        .get("sharedId")
        .and_then(Value::as_str)
        .map(|shared_id| DevToolsRemoteHandleId::from(shared_id.to_owned()));
    Some(DevToolsRemoteValue {
        value: Value::Null,
        handle: None,
        shared_id,
        node_id: None,
        backend_node_id: None,
        window_context: None,
        realm: None,
        remote_type: Some("object".to_owned()),
        remote_subtype: Some("node".to_owned()),
        unserializable_value: None,
        description: None,
        class_name: None,
        deep_serialized_value: None,
        node_value: object.get("value").cloned(),
    })
}

async fn devtools_realm_id_for_runtime_target_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
) -> Option<DevToolsRealmId> {
    devtools_runtime_realm_for_target_async(conn, target)
        .await
        .and_then(|realm| realm.realm_id)
}

async fn devtools_runtime_realm_for_target_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
) -> Option<RuntimeExecutionContextEvent> {
    let mut realms = devtools_realms_for_route_async(conn, target.route.clone())
        .await
        .ok()?;
    if let Some(execution_context_id) = target.execution_context_id {
        return realms
            .into_iter()
            .find(|realm| realm.context_id == Some(execution_context_id));
    }
    if let Some(index) = realms
        .iter()
        .position(|realm| realm.is_default != Some(false))
    {
        return Some(realms.remove(index));
    }
    realms.into_iter().next()
}

fn devtools_runtime_result_ownership(command: &DevToolsCommand) -> DevToolsResultOwnership {
    match command {
        DevToolsCommand::EvaluateScript(command) => command.result_ownership,
        DevToolsCommand::CallFunction(command) => command.result_ownership,
        _ => DevToolsResultOwnership::None,
    }
}

fn devtools_runtime_command_result_kind(
    command: &DevToolsCommand,
) -> DevToolsRuntimeCommandResultKind {
    match command {
        DevToolsCommand::TerminateExecution(_) => DevToolsRuntimeCommandResultKind::Empty,
        _ => DevToolsRuntimeCommandResultKind::Script,
    }
}

fn devtools_runtime_serialization_options(
    command: &DevToolsCommand,
) -> Option<DevToolsSerializationOptions> {
    match command {
        DevToolsCommand::EvaluateScript(command) => command.serialization_options.clone(),
        DevToolsCommand::CallFunction(command) => command.serialization_options.clone(),
        _ => None,
    }
}

#[cfg(test)]
fn devtools_command_has_bidi_script_channel_arguments(command: &DevToolsCommand) -> bool {
    let DevToolsCommand::CallFunction(command) = command else {
        return false;
    };
    matches!(command.context.protocol, DevToolsProtocol::WebDriverBidi)
        && command
            .this_parameter
            .iter()
            .chain(command.arguments.iter())
            .any(bidi_local_value_contains_channel)
}

#[cfg(test)]
fn bidi_local_value_contains_channel(value: &Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    match map.get("type").and_then(Value::as_str) {
        Some("channel") => true,
        Some("array" | "set") => map
            .get("value")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(bidi_local_value_contains_channel)),
        Some("object" | "map") => {
            map.get("value")
                .and_then(Value::as_array)
                .is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        let Some(pair) = entry.as_array() else {
                            return false;
                        };
                        pair.iter().any(bidi_local_value_contains_channel)
                    })
                })
        }
        _ => false,
    }
}

async fn devtools_runtime_target_async(
    conn: &mut CdpConnection,
    command: &DevToolsCommand,
) -> Result<DevToolsRuntimeTarget, DevToolsError> {
    if let DevToolsCommand::TerminateExecution(command) = command {
        let target_id =
            command.context.target_id.as_ref().ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget")
            })?;
        return devtools_runtime_control_target(conn, target_id);
    }
    let (target_id, realm_id, world_name) = match command {
        DevToolsCommand::EvaluateScript(command) => (
            command.context.target_id.as_ref(),
            command.realm_id.as_ref(),
            command.world_name.as_deref(),
        ),
        DevToolsCommand::CallFunction(command) => (
            command.context.target_id.as_ref(),
            command.realm_id.as_ref(),
            command.world_name.as_deref(),
        ),
        DevToolsCommand::LocateNodes(command) => (command.context.target_id.as_ref(), None, None),
        DevToolsCommand::ReleaseObjects(command) => (
            command.context.target_id.as_ref(),
            command.realm_id.as_ref(),
            command.world_name.as_deref(),
        ),
        _ => (None, None, None),
    };
    if let Some(target_id) = target_id {
        return devtools_runtime_context_target_async(conn, target_id, world_name).await;
    }
    if let Some(realm_id) = realm_id {
        return devtools_runtime_realm_target_async(conn, realm_id).await;
    }
    Err(DevToolsError::new(
        DevToolsErrorKind::NoSuchTarget,
        "NoSuchTarget",
    ))
}

fn devtools_runtime_control_target(
    conn: &CdpConnection,
    target_id: &DevToolsTargetId,
) -> Result<DevToolsRuntimeTarget, DevToolsError> {
    // Do not fall back to realm discovery here: it is a MainThread operation
    // and would put the escape hatch behind the work it needs to interrupt.
    let route = conn
        .target_session_route_for_target_id(target_id.as_str())
        .or_else(|| conn.target_session_route_for_child_frame_id(target_id.as_str()))
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
    Ok(DevToolsRuntimeTarget {
        route,
        execution_context_id: None,
        window_context_id: Some(target_id.clone()),
    })
}

async fn devtools_runtime_context_target_async(
    conn: &mut CdpConnection,
    target_id: &DevToolsTargetId,
    world_name: Option<&str>,
) -> Result<DevToolsRuntimeTarget, DevToolsError> {
    if let Some(route) = conn.target_session_route_for_target_id(target_id.as_str()) {
        let execution_context_id = if let Some(world_name) = world_name {
            Some(
                devtools_ensure_runtime_world_async(conn, route.clone(), target_id, world_name)
                    .await?,
            )
        } else {
            None
        };
        return Ok(DevToolsRuntimeTarget {
            route,
            execution_context_id,
            window_context_id: Some(target_id.clone()),
        });
    }

    if let Some(route) = conn.target_session_route_for_child_frame_id(target_id.as_str()) {
        let execution_context_id = if let Some(world_name) = world_name {
            Some(
                devtools_ensure_runtime_world_async(conn, route.clone(), target_id, world_name)
                    .await?,
            )
        } else {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
            let result = route_scope
                .conn_mut()
                .child_default_execution_context_id_for_frame_id_for_session_owner_async(
                    None,
                    target_id.as_str(),
                )
                .await;
            drop(route_scope);
            Some(
                result
                    .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?
                    .ok_or_else(|| {
                        DevToolsError::new(
                            DevToolsErrorKind::Internal,
                            "NoExecutionContextForFrame",
                        )
                    })?,
            )
        };
        return Ok(DevToolsRuntimeTarget {
            route,
            execution_context_id,
            window_context_id: Some(target_id.clone()),
        });
    }

    let routes = devtools_get_realms_routes(conn, None)?;
    for route in routes {
        let realms = devtools_realms_for_search_route_async(conn, route.clone()).await?;
        let Some(default_realm) = realms.iter().find(|realm| {
            realm
                .frame_id
                .as_ref()
                .is_some_and(|frame_id| frame_id.as_str() == target_id.as_str())
                && realm.is_default != Some(false)
        }) else {
            continue;
        };
        let execution_context_id = if let Some(world_name) = world_name {
            Some(
                devtools_ensure_runtime_world_async(conn, route.clone(), target_id, world_name)
                    .await?,
            )
        } else {
            Some(default_realm.context_id.ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::Internal, "NoExecutionContextForFrame")
            })?)
        };
        return Ok(DevToolsRuntimeTarget {
            route,
            execution_context_id,
            window_context_id: default_realm
                .frame_id
                .clone()
                .map(|frame_id| DevToolsTargetId::from(frame_id.into_string())),
        });
    }

    Err(DevToolsError::new(
        DevToolsErrorKind::NoSuchTarget,
        "NoSuchTarget",
    ))
}

async fn devtools_ensure_runtime_world_async(
    conn: &mut CdpConnection,
    route: CdpSessionRoute,
    frame_id: &DevToolsTargetId,
    world_name: &str,
) -> Result<i64, DevToolsError> {
    let mut route_scope = conn.scoped_none_session_owner_route_override(route);
    let result = route_scope
        .conn_mut()
        .runtime_ensure_isolated_world_for_session_owner_async(
            None,
            Some(frame_id.as_str()),
            world_name,
        )
        .await;
    drop(route_scope);
    result.map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))
}

async fn devtools_runtime_realm_target_async(
    conn: &mut CdpConnection,
    realm_id: &DevToolsRealmId,
) -> Result<DevToolsRuntimeTarget, DevToolsError> {
    let routes = devtools_get_realms_routes(conn, None)?;
    for route in routes {
        let realms = devtools_realms_for_search_route_async(conn, route.clone()).await?;
        if let Some(realm) = realms
            .into_iter()
            .find(|realm| realm.realm_id.as_ref() == Some(realm_id))
        {
            let execution_context_id = realm.context_id.ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::Internal, "NoExecutionContextForRealm")
            })?;
            return Ok(DevToolsRuntimeTarget {
                route,
                execution_context_id: Some(execution_context_id),
                window_context_id: realm
                    .frame_id
                    .clone()
                    .map(|frame_id| DevToolsTargetId::from(frame_id.into_string())),
            });
        }
    }
    Err(DevToolsError::new(
        DevToolsErrorKind::NoSuchTarget,
        "NoSuchRealm",
    ))
}

async fn devtools_realms_for_search_route_async(
    conn: &mut CdpConnection,
    route: CdpSessionRoute,
) -> Result<Vec<RuntimeExecutionContextEvent>, DevToolsError> {
    match devtools_realms_for_route_async(conn, route).await {
        Ok(realms) => Ok(realms),
        Err(error)
            if matches!(error.kind, DevToolsErrorKind::Internal)
                && error.message == "NoDocumentLoaded" =>
        {
            Ok(Vec::new())
        }
        Err(error) => Err(error),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum BidiValuePathSegment {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BidiCallFunctionValueRoot {
    This,
    Argument(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BidiNodeSharedReferencePath {
    root: BidiCallFunctionValueRoot,
    path: Vec<BidiValuePathSegment>,
    shared_id: String,
}

async fn remap_bidi_node_shared_references_for_target_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    command: &mut DevToolsCallFunctionCommand,
    realm_id: Option<&DevToolsRealmId>,
) -> Result<(), DevToolsError> {
    let mut references = Vec::new();
    if let Some(this_parameter) = command.this_parameter.as_ref() {
        collect_bidi_node_shared_reference_paths(
            this_parameter,
            BidiCallFunctionValueRoot::This,
            &mut references,
        );
    }
    for (index, argument) in command.arguments.iter().enumerate() {
        collect_bidi_node_shared_reference_paths(
            argument,
            BidiCallFunctionValueRoot::Argument(index),
            &mut references,
        );
    }

    references.sort_by(|left, right| {
        (left.root, &left.path, &left.shared_id).cmp(&(right.root, &right.path, &right.shared_id))
    });
    references.dedup();
    if references.is_empty() {
        return Ok(());
    }

    let execution_context_id = if let Some(execution_context_id) = target.execution_context_id {
        Some(execution_context_id)
    } else {
        let mut route_scope = conn.scoped_none_session_owner_route_override(target.route.clone());
        let result = route_scope
            .conn_mut()
            .runtime_default_or_initial_execution_context_id_for_session_owner_async(None)
            .await;
        drop(route_scope);
        Some(
            result
                .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?
                .ok_or_else(|| {
                    DevToolsError::new(DevToolsErrorKind::Internal, "NoDefaultExecutionContext")
                })?,
        )
    };

    for reference in references {
        let Some(target_shared_id) = remap_bidi_node_shared_id_for_target_async(
            conn,
            execution_context_id,
            &reference.shared_id,
            realm_id,
        )
        .await?
        else {
            continue;
        };
        let Some(value) =
            bidi_call_function_value_at_path_mut(command, reference.root, &reference.path)
        else {
            continue;
        };
        if let Some(map) = value.as_object_mut() {
            map.insert("sharedId".to_owned(), json!(target_shared_id.into_string()));
        }
    }

    Ok(())
}

fn collect_bidi_node_shared_reference_paths(
    value: &Value,
    root: BidiCallFunctionValueRoot,
    out: &mut Vec<BidiNodeSharedReferencePath>,
) {
    let mut stack = vec![(
        value,
        Vec::<BidiValuePathSegment>::new(),
        MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH,
    )];
    while let Some((value, path, remaining_tree_depth)) = stack.pop() {
        let Some(next_tree_depth) = remaining_tree_depth.checked_sub(1) else {
            continue;
        };
        match value {
            Value::Object(map) => {
                if let Some(shared_id) = map.get("sharedId").and_then(Value::as_str) {
                    out.push(BidiNodeSharedReferencePath {
                        root,
                        path,
                        shared_id: shared_id.to_owned(),
                    });
                    continue;
                }
                if map.get("handle").and_then(Value::as_str).is_some() {
                    continue;
                }
                let children = map.iter().collect::<Vec<_>>();
                for (key, child) in children.into_iter().rev() {
                    let mut child_path = path.clone();
                    child_path.push(BidiValuePathSegment::Key(key.clone()));
                    stack.push((child, child_path, next_tree_depth));
                }
            }
            Value::Array(values) => {
                for index in (0..values.len()).rev() {
                    let mut child_path = path.clone();
                    child_path.push(BidiValuePathSegment::Index(index));
                    stack.push((&values[index], child_path, next_tree_depth));
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
}

async fn remap_bidi_node_shared_id_for_target_async(
    conn: &mut CdpConnection,
    execution_context_id: Option<i64>,
    shared_id: &str,
    realm_id: Option<&DevToolsRealmId>,
) -> Result<Option<DevToolsRemoteHandleId>, DevToolsError> {
    let is_internal_bidi_node_id = is_webdriver_bidi_node_shared_id(shared_id);
    let snapshot = bidi_node_snapshot_for_shared_id_async(conn, shared_id, 0, false).await?;
    let Some(snapshot) = snapshot else {
        return if is_internal_bidi_node_id {
            Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchNode,
                "Could not find node with given id",
            ))
        } else {
            Ok(None)
        };
    };

    let Some(backend_node_id) = snapshot.snapshot.backend_node_id else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchNode,
            "Could not find node with given id",
        ));
    };
    let Some(remote_object) = conn
        .runtime_remote_object_for_backend_node_id_async(
            None,
            backend_node_id,
            execution_context_id,
            None,
        )
        .await
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?
    else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchNode,
            "Could not find node with given id",
        ));
    };
    let Some(object_id) = remote_object
        .get("objectId")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchNode,
            "Could not find node with given id",
        ));
    };

    register_remapped_bidi_node_remote_object(conn, &remote_object, &object_id, realm_id);
    Ok(Some(DevToolsRemoteHandleId::from(object_id)))
}

fn register_remapped_bidi_node_remote_object(
    conn: &mut CdpConnection,
    remote_object: &Value,
    object_id: &str,
    realm_id: Option<&DevToolsRealmId>,
) {
    if let Some(realm_id) = realm_id {
        conn.register_runtime_remote_object_ids_for_session_owner_with_realm(
            None,
            vec![object_id.to_owned()],
            realm_id.as_str(),
        );
    } else {
        conn.register_runtime_remote_object_ids_from_value_for_session_owner(None, remote_object);
    }
}

fn bidi_call_function_value_at_path_mut<'a>(
    command: &'a mut DevToolsCallFunctionCommand,
    root: BidiCallFunctionValueRoot,
    path: &[BidiValuePathSegment],
) -> Option<&'a mut Value> {
    let mut value = match root {
        BidiCallFunctionValueRoot::This => command.this_parameter.as_mut()?,
        BidiCallFunctionValueRoot::Argument(index) => command.arguments.get_mut(index)?,
    };
    for segment in path {
        match segment {
            BidiValuePathSegment::Key(key) => {
                value = value.as_object_mut()?.get_mut(key)?;
            }
            BidiValuePathSegment::Index(index) => {
                value = value.as_array_mut()?.get_mut(*index)?;
            }
        }
    }
    Some(value)
}

async fn materialize_bidi_channel_argument_proxies_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    command: &mut DevToolsCallFunctionCommand,
) -> Result<(), String> {
    let realm_id = devtools_realm_id_for_runtime_target_async(conn, target).await;
    if let Some(this_parameter) = command.this_parameter.as_mut() {
        materialize_bidi_channel_value_async(conn, target, realm_id.as_ref(), this_parameter)
            .await?;
    }
    for argument in &mut command.arguments {
        materialize_bidi_channel_value_async(conn, target, realm_id.as_ref(), argument).await?;
    }
    Ok(())
}

async fn materialize_bidi_channel_value_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
    value: &mut Value,
) -> Result<(), String> {
    let Some(map) = value.as_object_mut() else {
        return Ok(());
    };
    if bidi_remote_reference_object_id(map).is_some() {
        return Ok(());
    }
    match map.get("type").and_then(Value::as_str) {
        Some("channel") => {
            let properties = bidi_channel_properties_from_local_value(map)?;
            let proxy_handle = create_bidi_channel_proxy_and_start_listener_async(
                conn, target, realm_id, properties,
            )
            .await?;
            *value = json!({
                "__moliBidiChannelProxy": true,
                "handle": proxy_handle.as_str(),
            });
        }
        Some("array" | "set") => {
            if let Some(items) = map.get_mut("value").and_then(Value::as_array_mut) {
                for item in items {
                    Box::pin(materialize_bidi_channel_value_async(
                        conn, target, realm_id, item,
                    ))
                    .await?;
                }
            }
        }
        Some("object" | "map") => {
            if let Some(entries) = map.get_mut("value").and_then(Value::as_array_mut) {
                for entry in entries {
                    let Some(pair) = entry.as_array_mut() else {
                        continue;
                    };
                    for item in pair {
                        Box::pin(materialize_bidi_channel_value_async(
                            conn, target, realm_id, item,
                        ))
                        .await?;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn bidi_channel_properties_from_local_value(
    map: &serde_json::Map<String, Value>,
) -> Result<DevToolsBidiChannelProperties, String> {
    let channel = map
        .get("value")
        .and_then(Value::as_object)
        .ok_or_else(|| "InvalidChannelArgument".to_owned())?;
    Ok(DevToolsBidiChannelProperties {
        channel: channel
            .get("channel")
            .and_then(Value::as_str)
            .ok_or_else(|| "InvalidChannelArgument".to_owned())?
            .to_owned(),
        ownership: bidi_script_message_ownership_from_value(channel.get("ownership"))?,
        serialization_options: match channel.get("serializationOptions") {
            Some(value) => Some(bidi_script_message_serialization_options_from_value(value)?),
            None => None,
        },
    })
}

pub(crate) async fn start_bidi_preload_channel_listeners_for_execution_context_background_events_async(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    execution_context_id: i64,
    out: &mut Vec<BackgroundProtocolEvent>,
) {
    let handoff_owners = conn.target_owner_bidi_channel_preload_handoffs_for_session(session_id);
    if handoff_owners.is_empty() {
        return;
    }
    let target_id = conn
        .target_owner_identity_for_session(session_id)
        .and_then(|(_, target_id)| target_id)
        .map(DevToolsTargetId::from);
    let route = conn.session_route(session_id).or_else(|| {
        target_id
            .as_ref()
            .and_then(|target_id| conn.target_session_route_for_target_id(target_id.as_str()))
    });
    let Some(route) = route else {
        return;
    };
    let target = DevToolsRuntimeTarget {
        route,
        execution_context_id: Some(execution_context_id),
        window_context_id: target_id,
    };
    let Some(listener_owner) =
        bidi_channel_page_owner_for_runtime_target(conn, &target, session_id)
    else {
        return;
    };
    let realm = devtools_runtime_realm_for_target_async(conn, &target).await;
    let realm_id = realm.as_ref().and_then(|realm| realm.realm_id.clone());
    let listener_target_id = realm
        .as_ref()
        .and_then(|realm| realm.frame_id.clone())
        .map(|frame_id| DevToolsTargetId::from(frame_id.into_string()))
        .or_else(|| target.window_context_id.clone());
    for handoff_owner in handoff_owners {
        let channel_object_group = conn.next_bidi_channel_object_group();
        let proxy_handle = match bidi_preload_channel_proxy_handle_async(
            conn,
            &target,
            realm_id.as_ref(),
            handoff_owner.handoff_id.as_str(),
            handoff_owner.token.as_str(),
            &channel_object_group,
        )
        .await
        {
            Ok(Some(handle)) => handle,
            Ok(None) => {
                tracing::debug!(
                    handoff_id = %handoff_owner.handoff_id,
                    channel = %handoff_owner.channel,
                    execution_context_id,
                    "BiDi preload channel handoff had no proxy handle"
                );
                release_bidi_channel_object_group_for_target_best_effort_async(
                    conn,
                    &target,
                    session_id,
                    &channel_object_group,
                )
                .await;
                continue;
            }
            Err(error) => {
                tracing::debug!(
                    ?error,
                    handoff_id = %handoff_owner.handoff_id,
                    channel = %handoff_owner.channel,
                    execution_context_id,
                    "failed to materialize BiDi preload channel proxy handle"
                );
                release_bidi_channel_object_group_for_target_best_effort_async(
                    conn,
                    &target,
                    session_id,
                    &channel_object_group,
                )
                .await;
                continue;
            }
        };
        let properties = match bidi_preload_channel_properties_from_handoff(&handoff_owner) {
            Ok(properties) => properties,
            Err(error) => {
                tracing::debug!(
                    ?error,
                    handoff_id = %handoff_owner.handoff_id,
                    channel = %handoff_owner.channel,
                    execution_context_id,
                    "skipping invalid BiDi preload channel handoff"
                );
                release_bidi_channel_object_group_for_target_best_effort_async(
                    conn,
                    &target,
                    session_id,
                    &channel_object_group,
                )
                .await;
                continue;
            }
        };
        let listener = match PendingBidiChannelListener::new(
            listener_target_id.clone(),
            realm_id.clone(),
            proxy_handle,
            channel_object_group.clone(),
            properties,
        ) {
            Some(listener) => listener,
            None => {
                tracing::debug!(
                    handoff_id = %handoff_owner.handoff_id,
                    channel = %handoff_owner.channel,
                    execution_context_id,
                    "skipping BiDi preload channel listener without target or realm"
                );
                release_bidi_channel_object_group_for_target_best_effort_async(
                    conn,
                    &target,
                    session_id,
                    &channel_object_group,
                )
                .await;
                continue;
            }
        };
        conn.complete_bidi_channel_owner_action_with_background_events_async(
            BidiChannelOwnerAction::start_listener(BidiChannelListenerResidence::new(
                listener_owner.clone(),
                listener,
            )),
            out,
        )
        .await;
    }
}

async fn bidi_preload_channel_proxy_handle_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
    handoff_id: &str,
    token: &str,
    channel_object_group: &str,
) -> Result<Option<DevToolsRemoteHandleId>, DevToolsError> {
    let value = devtools_probe_remote_value_async(
        conn,
        target.clone(),
        DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: None,
                target_id: None,
                browser_context_id: None,
            },
            realm_id: realm_id.cloned(),
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: bidi_preload_channel_proxy_handle_source().to_owned(),
            arguments: vec![json!(handoff_id), json!(token)],
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::Root,
            object_group: Some(channel_object_group.to_owned()),
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        }),
    )
    .await?;
    Ok(value.and_then(|value| value.handle.or(value.shared_id)))
}

fn bidi_preload_channel_properties_from_handoff(
    handoff: &BidiPreloadChannelHandoff,
) -> Result<DevToolsBidiChannelProperties, String> {
    Ok(DevToolsBidiChannelProperties {
        channel: handoff.channel.clone(),
        ownership: bidi_script_message_ownership_from_str(handoff.ownership.as_deref())?,
        serialization_options: match handoff.serialization_options.as_ref() {
            Some(value) => Some(bidi_script_message_serialization_options_from_value(value)?),
            None => None,
        },
    })
}

async fn release_bidi_channel_object_group_for_target_best_effort_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    session_id: Option<&str>,
    object_group: &str,
) {
    let mut route_scope = conn.scoped_none_session_owner_route_override(target.route.clone());
    route_scope
        .conn_mut()
        .release_bidi_channel_object_group_for_session_owner_best_effort_async(
            session_id,
            object_group,
        )
        .await;
}

fn bidi_channel_page_owner_for_runtime_target(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    session_id: Option<&str>,
) -> Option<BidiChannelPageOwner> {
    let mut route_scope = conn.scoped_none_session_owner_route_override(target.route.clone());
    BidiChannelPageOwner::capture(route_scope.conn_mut(), session_id)
}

async fn create_bidi_channel_proxy_and_start_listener_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
    properties: DevToolsBidiChannelProperties,
) -> Result<DevToolsRemoteHandleId, String> {
    let listener_target_id = target
        .window_context_id
        .clone()
        .ok_or_else(|| "NoSuchBidiChannelTarget".to_owned())?;
    let listener_realm_id = realm_id
        .cloned()
        .ok_or_else(|| "NoSuchBidiChannelRealm".to_owned())?;
    let listener_owner = bidi_channel_page_owner_for_runtime_target(conn, target, None)
        .ok_or_else(|| "NoSuchBidiChannelTarget".to_owned())?;
    let channel_object_group = conn.next_bidi_channel_object_group();
    let proxy_handle = match create_bidi_channel_proxy_async(
        conn,
        target,
        realm_id,
        &channel_object_group,
    )
    .await
    {
        Ok(Some(handle)) => handle,
        Ok(None) => {
            release_bidi_channel_object_group_for_target_best_effort_async(
                conn,
                target,
                None,
                &channel_object_group,
            )
            .await;
            return Err("CannotCreateBidiChannelProxy".to_owned());
        }
        Err(error) => {
            release_bidi_channel_object_group_for_target_best_effort_async(
                conn,
                target,
                None,
                &channel_object_group,
            )
            .await;
            return Err(error);
        }
    };
    let listener = PendingBidiChannelListener::new(
        Some(listener_target_id),
        Some(listener_realm_id),
        proxy_handle.clone(),
        channel_object_group,
        properties,
    )
    .ok_or_else(|| "NoSuchBidiChannelRealm".to_owned())?;
    conn.publish_bidi_channel_listener_start(BidiChannelListenerResidence::new(
        listener_owner,
        listener,
    ));
    Ok(proxy_handle)
}

async fn create_bidi_channel_proxy_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
    channel_object_group: &str,
) -> Result<Option<DevToolsRemoteHandleId>, String> {
    let call_target = if let Some(realm_id) = realm_id
        && target.execution_context_id.is_none()
    {
        devtools_runtime_realm_target_async(conn, realm_id)
            .await
            .map_err(|error| error.message)?
    } else {
        target.clone()
    };
    let command_target_id = if realm_id.is_some() {
        None
    } else {
        call_target.window_context_id.clone()
    };
    let value = Box::pin(devtools_probe_remote_value_async(
        conn,
        call_target,
        DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: None,
                target_id: command_target_id,
                browser_context_id: None,
            },
            realm_id: realm_id.cloned(),
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: format!(
                "function() {{ return {}; }}",
                bidi_channel_proxy_expression_source()
            ),
            arguments: Vec::new(),
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::Root,
            object_group: Some(channel_object_group.to_owned()),
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        }),
    ))
    .await
    .map_err(|error| error.message)?;
    Ok(value.and_then(|value| value.handle.or(value.shared_id)))
}

fn bidi_channel_proxy_expression_source() -> String {
    let mut source = String::new();
    source.push_str("(() => {\n");
    source.push_str(
        "const queue = [];\n\
     let queueNonEmptyResolver = null;\n\
     return {\n\
     async getMessage() {\n\
     const onMessage = queue.length > 0 ? Promise.resolve() : new Promise((resolve) => { queueNonEmptyResolver = resolve; });\n\
     await onMessage;\n\
     return queue.shift();\n\
     },\n\
     sendMessage(message) {\n\
     queue.push(message);\n\
     if (queueNonEmptyResolver !== null) {\n\
     queueNonEmptyResolver();\n\
     queueNonEmptyResolver = null;\n\
     }\n\
     },\n\
     };\n\
     })()",
    );
    source
}

fn bidi_preload_channel_delegate_source(has_channel_handoffs: bool) -> String {
    if !has_channel_handoffs {
        return String::new();
    }
    let mut source = String::new();
    source.push_str(
        "const __moliPutBidiPreloadChannelProxy = (handoffId, token, proxy) => {\n\
     if (typeof handoffId !== 'string' || handoffId.length === 0 || typeof token !== 'string') { return; }\n\
     const take = (providedToken) => {\n\
     if (providedToken !== token) { return undefined; }\n\
     delete globalThis[handoffId];\n\
     return proxy;\n\
     };\n\
     Object.defineProperty(globalThis, handoffId, {\n\
     value: take,\n\
     configurable: true,\n\
     enumerable: false,\n\
     writable: false,\n\
     });\n\
     };\n",
    );
    source.push_str(
        "const __moliCreateBidiChannelDelegate = (properties) => {\n\
     const queue = [];\n\
     let queueNonEmptyResolver = null;\n\
     const proxy = {\n\
     async getMessage() {\n\
     const onMessage = queue.length > 0 ? Promise.resolve() : new Promise((resolve) => { queueNonEmptyResolver = resolve; });\n\
     await onMessage;\n\
     return queue.shift();\n\
     },\n\
     sendMessage(message) {\n\
     queue.push(message);\n\
     if (queueNonEmptyResolver !== null) {\n\
     queueNonEmptyResolver();\n\
     queueNonEmptyResolver = null;\n\
     }\n\
     },\n\
     };\n\
     __moliPutBidiPreloadChannelProxy(properties && properties.handoffId, properties && properties.handoffToken, proxy);\n\
     return proxy.sendMessage.bind(proxy);\n\
     };",
    );
    source
}

fn bidi_preload_channel_proxy_handle_source() -> &'static str {
    "(function(handoffId, token) {\n\
     const take = globalThis[handoffId];\n\
     if (typeof take !== 'function') { return undefined; }\n\
     return take(token);\n\
     })"
}

async fn start_protocol_neutral_runtime_command(
    conn: &mut CdpConnection,
    target: DevToolsRuntimeTarget,
    command: DevToolsCommand,
    internal_command_id: u64,
) -> RuntimeCommandTaskStep {
    let session_owner_route = target.route.clone();
    let mut route_scope = conn.scoped_none_session_owner_route_override(target.route.clone());
    let step = match command {
        DevToolsCommand::EvaluateScript(command) => {
            let params = devtools_evaluate_script_params(&command, target.execution_context_id);
            let json =
                runtime_inspector_command_json(internal_command_id, "Runtime.evaluate", &params);
            let parsed = match parse_synthesized_runtime_command(json) {
                Ok(command) => command,
                Err(message) => {
                    return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                        Some(internal_command_id),
                        message,
                    ));
                }
            };
            let cmd = Cmd::from_parsed(&parsed)
                .expect("synthesized Runtime command must contain a domain separator");
            let command = DevToolsCommand::EvaluateScript(command);
            match prepare_pending_devtools_runtime_inspector_json(
                route_scope.conn_mut(),
                &cmd,
                &command,
            ) {
                Ok(inspector_json) => start_devtools_runtime_command(
                    route_scope.conn_mut(),
                    &cmd,
                    command,
                    inspector_json,
                    runtime_command_awaits_promise(&cmd, RuntimeAction::Evaluate),
                    RendererInspectorResponseDelivery::CommandReply,
                ),
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message))
                }
            }
        }
        DevToolsCommand::CallFunction(mut command) => {
            if matches!(command.context.protocol, DevToolsProtocol::WebDriverBidi)
                && let Err(message) = Box::pin(materialize_bidi_channel_argument_proxies_async(
                    route_scope.conn_mut(),
                    &target,
                    &mut command,
                ))
                .await
            {
                return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                    Some(internal_command_id),
                    message,
                ));
            }
            match devtools_call_function_params_async(
                route_scope.conn_mut(),
                &command,
                target.execution_context_id,
            )
            .await
            {
                Ok(params) => {
                    let json = runtime_inspector_command_json(
                        internal_command_id,
                        "Runtime.callFunctionOn",
                        &params,
                    );
                    let parsed = match parse_synthesized_runtime_command(json) {
                        Ok(command) => command,
                        Err(message) => {
                            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                                Some(internal_command_id),
                                message,
                            ));
                        }
                    };
                    let cmd = Cmd::from_parsed(&parsed)
                        .expect("synthesized Runtime command must contain a domain separator");
                    let command = DevToolsCommand::CallFunction(command);
                    match prepare_pending_devtools_runtime_inspector_json(
                        route_scope.conn_mut(),
                        &cmd,
                        &command,
                    ) {
                        Ok(inspector_json) => start_devtools_runtime_command(
                            route_scope.conn_mut(),
                            &cmd,
                            command,
                            inspector_json,
                            runtime_command_awaits_promise(&cmd, RuntimeAction::CallFunctionOn),
                            RendererInspectorResponseDelivery::CommandReply,
                        ),
                        Err(message) => RuntimeCommandTaskStep::Complete(
                            runtime_inspector_error_plan(cmd.id, message),
                        ),
                    }
                }
                Err(message) => RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                    Some(internal_command_id),
                    message,
                )),
            }
        }
        DevToolsCommand::TerminateExecution(_) => {
            let json = runtime_inspector_command_json(
                internal_command_id,
                "Runtime.terminateExecution",
                &json!({}),
            );
            let parsed = match parse_synthesized_runtime_command(json) {
                Ok(command) => command,
                Err(message) => {
                    return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                        Some(internal_command_id),
                        message,
                    ));
                }
            };
            let cmd = Cmd::from_parsed(&parsed)
                .expect("synthesized Runtime command must contain a domain separator");
            let MainRuntimeCommand::Inspector(command) =
                MainRuntimeCommand::classify(RuntimeAction::TerminateExecution)
            else {
                unreachable!("Runtime.terminateExecution must use an Inspector command route")
            };
            start_main_runtime_inspector_command(route_scope.conn_mut(), &cmd, command)
        }
        _ => RuntimeCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(
            DevToolsError::new(DevToolsErrorKind::Unsupported, "UnsupportedDevToolsCommand"),
        )),
    };
    drop(route_scope);
    step.with_owner_scope(CommandOwnerScope::from_session_and_owner_route(
        None,
        Some(session_owner_route),
    ))
}

fn runtime_inspector_command_json(command_id: u64, method: &str, params: &Value) -> String {
    json!({
        "id": command_id,
        "method": method,
        "params": params,
    })
    .to_string()
}

fn devtools_evaluate_script_params(
    command: &DevToolsEvaluateScriptCommand,
    execution_context_id: Option<i64>,
) -> Value {
    let expression = if command.materialize_bidi_script_result {
        bidi_window_remote_result_expression_source(&command.expression, command.await_promise)
    } else {
        command.expression.clone()
    };
    let mut params = json!({
        "expression": expression,
        "awaitPromise": command.await_promise,
        "returnByValue": devtools_result_ownership_returns_by_value(
            command.result_ownership,
            command.preserve_remote_metadata,
        ),
    });
    if command.user_gesture
        && let Some(map) = params.as_object_mut()
    {
        map.insert("userGesture".to_owned(), Value::Bool(true));
    }
    if let Some(handler) = command.webdriver_bidi_file_prompt_handler.as_deref()
        && let Some(map) = params.as_object_mut()
    {
        map.insert(
            WEBDRIVER_BIDI_FILE_PROMPT_HANDLER_PARAM.to_owned(),
            Value::String(handler.to_owned()),
        );
    }
    apply_devtools_serialization_options(&mut params, command.serialization_options.as_ref());
    if let Some(execution_context_id) = execution_context_id
        && let Some(map) = params.as_object_mut()
    {
        map.insert("contextId".to_owned(), json!(execution_context_id));
    }
    if matches!(command.result_ownership, DevToolsResultOwnership::Root)
        && let Some(map) = params.as_object_mut()
    {
        map.insert("objectGroup".to_owned(), json!("webdriver-bidi"));
    }
    params
}

async fn devtools_call_function_params_async(
    conn: &mut CdpConnection,
    command: &DevToolsCallFunctionCommand,
    execution_context_id: Option<i64>,
) -> Result<Value, String> {
    let deserialize_bidi_local_values =
        devtools_call_function_deserializes_bidi_local_values(command);
    let primary_argument_count = devtools_call_function_arguments(command).len();
    let function_declaration = devtools_call_function_declaration(
        command,
        deserialize_bidi_local_values,
        primary_argument_count,
    );
    let arguments = devtools_call_function_cdp_arguments(command, deserialize_bidi_local_values);
    let mut params = json!({
        "functionDeclaration": function_declaration,
        "arguments": arguments,
        "awaitPromise": command.await_promise,
        "returnByValue": devtools_result_ownership_returns_by_value(
            command.result_ownership,
            command.preserve_remote_metadata,
        ),
    });
    if command.user_gesture
        && let Some(map) = params.as_object_mut()
    {
        map.insert("userGesture".to_owned(), Value::Bool(true));
    }
    if let Some(handler) = command.webdriver_bidi_file_prompt_handler.as_deref()
        && let Some(map) = params.as_object_mut()
    {
        map.insert(
            WEBDRIVER_BIDI_FILE_PROMPT_HANDLER_PARAM.to_owned(),
            Value::String(handler.to_owned()),
        );
    }
    apply_devtools_serialization_options(&mut params, command.serialization_options.as_ref());
    if let Some(map) = params.as_object_mut() {
        if let Some(object_id) = command.object_id.as_ref() {
            map.insert("objectId".to_owned(), json!(object_id.as_str()));
        } else if let Some(execution_context_id) = execution_context_id {
            map.insert("executionContextId".to_owned(), json!(execution_context_id));
        } else if command.realm_id.is_none() {
            let Some(execution_context_id) = conn
                .runtime_default_or_initial_execution_context_id_for_session_owner_async(None)
                .await?
            else {
                return Err("NoDefaultExecutionContext".to_owned());
            };
            map.insert("executionContextId".to_owned(), json!(execution_context_id));
        }
    }
    let object_group = command.object_group.as_deref().or_else(|| {
        matches!(command.result_ownership, DevToolsResultOwnership::Root)
            .then_some("webdriver-bidi")
    });
    if let Some(object_group) = object_group
        && let Some(map) = params.as_object_mut()
    {
        map.insert("objectGroup".to_owned(), json!(object_group));
    }
    Ok(params)
}

fn devtools_call_function_declaration(
    command: &DevToolsCallFunctionCommand,
    deserialize_bidi_local_values: bool,
    primary_argument_count: usize,
) -> String {
    let declaration = if command.this_parameter.is_none() && !deserialize_bidi_local_values {
        command.function_declaration.clone()
    } else if deserialize_bidi_local_values {
        let this_expression = if command.this_parameter.is_some() {
            "__deserialize(__primaryArgs.shift())"
        } else {
            "undefined"
        };
        let call_expression = if command.this_parameter.is_some() {
            "f.apply(deserializedThis, deserializedArgs)"
        } else {
            "f(...deserializedArgs)"
        };
        let deserializer_source = bidi_local_value_deserializer_source();
        format!(
            "(...args) => {{\n\
             function callFunction(f, args) {{\n\
             const __primaryArgs = args.slice(0, {primary_argument_count});\n\
             const __remoteReferences = args.slice({primary_argument_count});\n\
             {deserializer_source}\n\
             const deserializedThis = {this_expression};\n\
             const deserializedArgs = __primaryArgs.map(__deserialize);\n\
             return {call_expression};\n\
             }}\n\
             return callFunction((\n\
             {}\n\
             ), args);\n\
             }}",
            command.function_declaration
        )
    } else {
        format!(
            "(...args) => {{\n\
         function callFunction(f, args) {{\n\
         const deserializedThis = args.shift();\n\
         const deserializedArgs = args;\n\
         return f.apply(deserializedThis, deserializedArgs);\n\
         }}\n\
         return callFunction((\n\
         {}\n\
         ), args);\n\
         }}",
            command.function_declaration
        )
    };
    if command.materialize_bidi_script_result {
        bidi_window_remote_result_function_declaration_source(&declaration, command.await_promise)
    } else {
        declaration
    }
}

fn bidi_local_value_deserializer_source() -> &'static str {
    "const __deserialize = (value) => {\n\
     if (value && value.__moliBidiRemoteReference === true) { return __remoteReferences[value.index]; }\n\
     if (value && value.__moliBidiChannelProxy === true) {\n\
     const proxy = __remoteReferences[value.index];\n\
     return proxy && typeof proxy.sendMessage === 'function'\n\
     ? proxy.sendMessage.bind(proxy)\n\
     : undefined;\n\
     }\n\
     if (!value || value.__moliBidiLocalValue !== true) { return value; }\n\
     switch (value.type) {\n\
     case 'undefined': return undefined;\n\
     case 'null': return null;\n\
     case 'string':\n\
     case 'boolean': return value.value;\n\
     case 'number':\n\
     if (value.value === 'NaN') { return NaN; }\n\
     if (value.value === '-0') { return -0; }\n\
     if (value.value === 'Infinity') { return Infinity; }\n\
     if (value.value === '-Infinity') { return -Infinity; }\n\
     return value.value;\n\
     case 'bigint': return BigInt(value.value);\n\
     case 'date': return new Date(value.value);\n\
     case 'regexp': return new RegExp(value.value.pattern, value.value.flags || '');\n\
     case 'array': return value.value.map(__deserialize);\n\
     case 'object': return Object.fromEntries(value.value.map(([key, item]) => [key, __deserialize(item)]));\n\
     case 'map': return new Map(value.value.map(([key, item]) => [__deserialize(key), __deserialize(item)]));\n\
     case 'set': return new Set(value.value.map(__deserialize));\n\
     case 'channel': return typeof __moliCreateBidiChannelDelegate === 'function'\n\
     ? __moliCreateBidiChannelDelegate(value.value)\n\
     : undefined;\n\
     default: return value.value;\n\
     }\n\
     };"
}

fn bidi_window_remote_result_expression_source(expression: &str, await_promise: bool) -> String {
    let serializer = bidi_window_remote_result_serializer_source();
    let encoded_expression = serde_json::to_string(expression)
        .expect("serializing a string expression to JSON should not fail");
    if await_promise {
        format!(
            "Promise.resolve((0, eval)({encoded_expression})).then((__moliBidiResult) => {{\n\
             {serializer}\n\
             return __moliSerializeBidiWindowRemoteResult(__moliBidiResult);\n\
             }})"
        )
    } else {
        format!(
            "((__moliBidiResult) => {{\n\
             {serializer}\n\
             return __moliSerializeBidiWindowRemoteResult(__moliBidiResult);\n\
             }})(\n\
             (0, eval)({encoded_expression})\n\
             )"
        )
    }
}

fn bidi_window_remote_result_function_declaration_source(
    function_declaration: &str,
    await_promise: bool,
) -> String {
    let serializer = bidi_window_remote_result_serializer_source();
    if await_promise {
        format!(
            "(...args) => {{\n\
             {serializer}\n\
             const __moliBidiFunction = ({function_declaration});\n\
             const __moliBidiResult = Reflect.apply(__moliBidiFunction, this, args);\n\
             return Promise.resolve(__moliBidiResult).then(__moliSerializeBidiWindowRemoteResult);\n\
             }}"
        )
    } else {
        format!(
            "(...args) => {{\n\
             {serializer}\n\
             const __moliBidiFunction = ({function_declaration});\n\
             const __moliBidiResult = Reflect.apply(__moliBidiFunction, this, args);\n\
             return __moliSerializeBidiWindowRemoteResult(__moliBidiResult);\n\
             }}"
        )
    }
}

fn bidi_window_remote_result_serializer_source() -> &'static str {
    "const __moliSerializeBidiWindowRemoteResult = (value) => {\n\
     if (typeof __moliHostBidiWindowRemoteValue === 'function') {\n\
     const windowRemoteValue = __moliHostBidiWindowRemoteValue(value);\n\
     if (windowRemoteValue && windowRemoteValue.__moliBidiRemoteValue === true) {\n\
     return windowRemoteValue;\n\
     }\n\
     }\n\
     return value;\n\
     };"
}

pub(crate) struct BidiPreloadFunctionDeclaration {
    pub(crate) source: String,
    pub(crate) channel_handoffs: Vec<BidiPreloadChannelHandoff>,
}

pub(crate) fn bidi_preload_function_declaration_source(
    function_declaration: &str,
    arguments: &[Value],
) -> Result<Option<BidiPreloadFunctionDeclaration>, String> {
    let mut remote_object_ids = Vec::new();
    let mut channel_handoffs = Vec::new();
    let mut context = BidiLocalValueDescriptorContext {
        remote_object_ids: &mut remote_object_ids,
        preload_channel_handoffs: Some(&mut channel_handoffs),
        preload_channel_error: None,
    };
    let mut descriptors = Vec::with_capacity(arguments.len());
    for argument in arguments {
        let descriptor = bidi_local_value_descriptor_from_devtools_argument(argument, &mut context)
            .or_else(|| cdp_value_from_devtools_argument(argument))
            .or_else(|| {
                argument
                    .as_object()
                    .and_then(|map| map.get("value"))
                    .cloned()
            });
        let Some(descriptor) = descriptor else {
            if let Some(message) = context.preload_channel_error.take() {
                return Err(message);
            }
            return Ok(None);
        };
        descriptors.push(descriptor);
    }
    drop(context);
    if !remote_object_ids.is_empty() {
        return Ok(None);
    }
    let arguments_json = serde_json::to_string(&descriptors).map_err(|error| error.to_string())?;
    let channel_delegate_source =
        bidi_preload_channel_delegate_source(!channel_handoffs.is_empty());
    let deserializer_source = bidi_local_value_deserializer_source();
    let source = format!(
        "(() => {{\n\
         const __remoteReferences = [];\n\
         {channel_delegate_source}\n\
         {deserializer_source}\n\
         const __args = {arguments_json};\n\
         return ({function_declaration})(...__args.map(__deserialize));\n\
         }})();"
    );
    Ok(Some(BidiPreloadFunctionDeclaration {
        source,
        channel_handoffs,
    }))
}

fn bidi_script_message_ownership_from_value(
    value: Option<&Value>,
) -> Result<DevToolsResultOwnership, String> {
    match value {
        None => Ok(DevToolsResultOwnership::None),
        Some(Value::String(value)) if value == "root" => Ok(DevToolsResultOwnership::Root),
        Some(Value::String(value)) if value == "none" => Ok(DevToolsResultOwnership::None),
        Some(_) => Err("InvalidChannelArgument".to_owned()),
    }
}

fn bidi_script_message_ownership_from_str(
    value: Option<&str>,
) -> Result<DevToolsResultOwnership, String> {
    match value {
        None => Ok(DevToolsResultOwnership::None),
        Some("root") => Ok(DevToolsResultOwnership::Root),
        Some("none") => Ok(DevToolsResultOwnership::None),
        Some(_) => Err("InvalidChannelArgument".to_owned()),
    }
}

fn devtools_call_function_arguments(command: &DevToolsCallFunctionCommand) -> Vec<Value> {
    command
        .this_parameter
        .iter()
        .cloned()
        .chain(command.arguments.iter().cloned())
        .collect()
}

fn devtools_call_function_cdp_arguments(
    command: &DevToolsCallFunctionCommand,
    deserialize_bidi_local_values: bool,
) -> Vec<Value> {
    let arguments = devtools_call_function_arguments(command);
    if !deserialize_bidi_local_values {
        return arguments
            .into_iter()
            .map(cdp_call_argument_from_devtools_argument)
            .collect();
    }

    let mut remote_object_ids = Vec::new();
    let mut context = BidiLocalValueDescriptorContext {
        remote_object_ids: &mut remote_object_ids,
        preload_channel_handoffs: None,
        preload_channel_error: None,
    };
    let mut cdp_arguments = arguments
        .into_iter()
        .map(|argument| {
            let descriptor =
                bidi_local_value_descriptor_from_devtools_argument(&argument, &mut context)
                    .or_else(|| cdp_value_from_devtools_argument(&argument))
                    .or_else(|| {
                        argument
                            .as_object()
                            .and_then(|map| map.get("value"))
                            .cloned()
                    })
                    .unwrap_or(argument);
            json!({ "value": descriptor })
        })
        .collect::<Vec<_>>();
    drop(context);
    cdp_arguments.extend(
        remote_object_ids
            .into_iter()
            .map(|object_id| json!({ "objectId": object_id })),
    );
    cdp_arguments
}

fn devtools_call_function_deserializes_bidi_local_values(
    command: &DevToolsCallFunctionCommand,
) -> bool {
    matches!(command.context.protocol, DevToolsProtocol::WebDriverBidi)
        && command
            .this_parameter
            .iter()
            .chain(command.arguments.iter())
            .any(bidi_local_value_needs_js_deserialization)
}

fn bidi_local_value_needs_js_deserialization(argument: &Value) -> bool {
    bidi_local_value_needs_js_deserialization_nested(argument, false)
}

fn bidi_local_value_needs_js_deserialization_nested(argument: &Value, nested: bool) -> bool {
    let Some(map) = argument.as_object() else {
        return false;
    };
    if map.get("__moliBidiChannelProxy").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    if bidi_remote_reference_object_id(map).is_some() {
        return nested;
    }
    match map.get("type").and_then(Value::as_str) {
        Some("date" | "map" | "regexp" | "set") => true,
        Some("channel") => true,
        Some("bigint") => nested,
        Some("number") => {
            nested
                && map
                    .get("value")
                    .and_then(Value::as_str)
                    .is_some_and(is_bidi_unserializable_number)
        }
        Some("array") => map
            .get("value")
            .and_then(Value::as_array)
            .is_some_and(|values| {
                values
                    .iter()
                    .any(|value| bidi_local_value_needs_js_deserialization_nested(value, true))
            }),
        Some("object") => map
            .get("value")
            .and_then(Value::as_array)
            .is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry
                        .as_array()
                        .and_then(|pair| pair.get(1))
                        .is_some_and(|value| {
                            bidi_local_value_needs_js_deserialization_nested(value, true)
                        })
                })
            }),
        _ => false,
    }
}

fn apply_devtools_serialization_options(
    params: &mut Value,
    serialization_options: Option<&DevToolsSerializationOptions>,
) {
    let Some(serialization_options) = serialization_options else {
        return;
    };
    let Some(map) = params.as_object_mut() else {
        return;
    };
    map.insert(
        "serializationOptions".to_owned(),
        devtools_deep_serialization_options_json(serialization_options),
    );
}

pub(crate) fn devtools_deep_serialization_options_json(
    serialization_options: &DevToolsSerializationOptions,
) -> Value {
    let mut options = json!({
        "serialization": "deep",
    });
    if let Some(max_depth) = serialization_options.max_object_depth {
        options["maxDepth"] = json!(max_depth);
    }
    let mut additional_parameters = serde_json::Map::new();
    if let Some(max_dom_depth) = serialization_options.max_dom_depth {
        additional_parameters.insert(
            "maxNodeDepth".to_owned(),
            json!(max_dom_depth.min(i32::MAX as u64)),
        );
    }
    if let Some(include_shadow_tree) = serialization_options.include_shadow_tree.as_deref() {
        additional_parameters.insert("includeShadowTree".to_owned(), json!(include_shadow_tree));
    }
    if !additional_parameters.is_empty() {
        options["additionalParameters"] = Value::Object(additional_parameters);
    }
    options
}

fn devtools_result_ownership_returns_by_value(
    ownership: DevToolsResultOwnership,
    preserve_remote_metadata: bool,
) -> bool {
    !preserve_remote_metadata && !matches!(ownership, DevToolsResultOwnership::Root)
}

fn cdp_call_argument_from_devtools_argument(argument: Value) -> Value {
    let Some(map) = argument.as_object() else {
        return json!({ "value": argument });
    };
    if let Some(object_id) = bidi_remote_reference_object_id(map) {
        return json!({ "objectId": object_id });
    }
    if map.get("type").and_then(Value::as_str) == Some("number")
        && let Some(unserializable_value) = map.get("value").and_then(Value::as_str)
        && is_bidi_unserializable_number(unserializable_value)
    {
        return json!({ "unserializableValue": unserializable_value });
    }
    if map.get("type").and_then(Value::as_str) == Some("bigint")
        && let Some(value) = map.get("value").and_then(Value::as_str)
    {
        let unserializable_value = if value.ends_with('n') {
            value.to_owned()
        } else {
            format!("{value}n")
        };
        return json!({ "unserializableValue": unserializable_value });
    }
    if let Some(value) = cdp_value_from_devtools_argument(&argument) {
        return json!({ "value": value });
    }
    if let Some(value) = map.get("value") {
        return json!({ "value": value.clone() });
    }
    if map.get("type").and_then(Value::as_str) == Some("null") {
        return json!({ "value": null });
    }
    argument
}

fn is_bidi_unserializable_number(value: &str) -> bool {
    matches!(value, "NaN" | "-0" | "Infinity" | "-Infinity")
}

fn bidi_remote_reference_object_id(map: &serde_json::Map<String, Value>) -> Option<&str> {
    map.get("sharedId")
        .or_else(|| map.get("handle"))
        .and_then(Value::as_str)
}

struct BidiLocalValueDescriptorContext<'a> {
    remote_object_ids: &'a mut Vec<String>,
    preload_channel_handoffs: Option<&'a mut Vec<BidiPreloadChannelHandoff>>,
    preload_channel_error: Option<String>,
}

fn bidi_local_value_descriptor_from_devtools_argument(
    argument: &Value,
    context: &mut BidiLocalValueDescriptorContext<'_>,
) -> Option<Value> {
    let Some(map) = argument.as_object() else {
        return Some(argument.clone());
    };
    if map.get("__moliBidiChannelProxy").and_then(Value::as_bool) == Some(true) {
        let object_id = map.get("handle")?.as_str()?;
        let index = context.remote_object_ids.len();
        context.remote_object_ids.push(object_id.to_owned());
        return Some(json!({
            "__moliBidiChannelProxy": true,
            "index": index,
        }));
    }
    if let Some(object_id) = bidi_remote_reference_object_id(map) {
        let index = context.remote_object_ids.len();
        context.remote_object_ids.push(object_id.to_owned());
        return Some(json!({
            "__moliBidiRemoteReference": true,
            "index": index,
        }));
    }
    let type_name = map.get("type").and_then(Value::as_str)?;
    let descriptor_value = match type_name {
        "undefined" | "null" => Value::Null,
        "array" | "set" => Value::Array(
            map.get("value")?
                .as_array()?
                .iter()
                .map(|value| bidi_local_value_descriptor_from_devtools_argument(value, context))
                .collect::<Option<Vec<_>>>()?,
        ),
        "object" => {
            let mut entries = Vec::new();
            for entry in map.get("value")?.as_array()? {
                let pair = entry.as_array()?;
                let [key, value] = pair.as_slice() else {
                    return None;
                };
                entries.push(json!([
                    key.as_str()?,
                    bidi_local_value_descriptor_from_devtools_argument(value, context)?
                ]));
            }
            Value::Array(entries)
        }
        "map" => {
            let mut entries = Vec::new();
            for entry in map.get("value")?.as_array()? {
                let pair = entry.as_array()?;
                let [key, value] = pair.as_slice() else {
                    return None;
                };
                let key = key
                    .as_str()
                    .map(Value::from)
                    .or_else(|| bidi_local_value_descriptor_from_devtools_argument(key, context))?;
                entries.push(json!([
                    key,
                    bidi_local_value_descriptor_from_devtools_argument(value, context)?
                ]));
            }
            Value::Array(entries)
        }
        "regexp" => {
            let regexp = map.get("value")?.as_object()?;
            json!({
                "pattern": regexp.get("pattern")?.as_str()?,
                "flags": regexp.get("flags").and_then(Value::as_str).unwrap_or_default(),
            })
        }
        "channel" => {
            let channel = map.get("value")?.as_object()?;
            let mut descriptor = serde_json::Map::new();
            descriptor.insert(
                "channel".to_owned(),
                json!(channel.get("channel")?.as_str()?),
            );
            if let Some(ownership) = channel.get("ownership").and_then(Value::as_str) {
                descriptor.insert("ownership".to_owned(), json!(ownership));
            }
            if let Some(serialization_options) = channel.get("serializationOptions") {
                descriptor.insert(
                    "serializationOptions".to_owned(),
                    serialization_options.clone(),
                );
            }
            if let Some(handoffs) = context.preload_channel_handoffs.as_deref_mut() {
                let handoff = match new_bidi_preload_channel_handoff(channel) {
                    Ok(handoff) => handoff,
                    Err(message) => {
                        context.preload_channel_error = Some(message);
                        return None;
                    }
                };
                descriptor.insert("handoffId".to_owned(), json!(handoff.handoff_id));
                descriptor.insert("handoffToken".to_owned(), json!(handoff.token));
                handoffs.push(handoff);
            }
            Value::Object(descriptor)
        }
        "string" | "number" | "boolean" | "bigint" | "date" => map.get("value")?.clone(),
        _ => return None,
    };
    Some(json!({
        "__moliBidiLocalValue": true,
        "type": type_name,
        "value": descriptor_value,
    }))
}

fn new_bidi_preload_channel_handoff(
    channel: &serde_json::Map<String, Value>,
) -> Result<BidiPreloadChannelHandoff, String> {
    let properties = bidi_channel_properties_from_channel_map(channel)?;
    let channel_name = channel
        .get("channel")
        .and_then(Value::as_str)
        .ok_or_else(|| "InvalidChannelArgument".to_owned())?
        .to_owned();
    Ok(BidiPreloadChannelHandoff {
        handoff_id: format!(
            "__lmBidiPreloadChannel_{}",
            random_bidi_preload_channel_handoff_id()?
        ),
        token: random_bidi_preload_channel_handoff_id()?,
        channel: channel_name,
        ownership: match properties.ownership {
            DevToolsResultOwnership::Root => Some("root".to_owned()),
            DevToolsResultOwnership::None | DevToolsResultOwnership::ByValue => None,
        },
        serialization_options: channel.get("serializationOptions").cloned(),
    })
}

fn bidi_channel_properties_from_channel_map(
    channel: &serde_json::Map<String, Value>,
) -> Result<DevToolsBidiChannelProperties, String> {
    Ok(DevToolsBidiChannelProperties {
        channel: channel
            .get("channel")
            .and_then(Value::as_str)
            .ok_or_else(|| "InvalidChannelArgument".to_owned())?
            .to_owned(),
        ownership: bidi_script_message_ownership_from_value(channel.get("ownership"))?,
        serialization_options: match channel.get("serializationOptions") {
            Some(value) => Some(bidi_script_message_serialization_options_from_value(value)?),
            None => None,
        },
    })
}

fn random_bidi_preload_channel_handoff_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    moli_crypto::fill_secure_random(&mut bytes)
        .map_err(|error| format!("failed to generate BiDi preload channel id: {error}"))?;
    Ok(hex_bytes(&bytes))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn cdp_value_from_devtools_argument(argument: &Value) -> Option<Value> {
    let Some(map) = argument.as_object() else {
        return Some(argument.clone());
    };
    if bidi_remote_reference_object_id(map).is_some() {
        return None;
    }
    match map.get("type").and_then(Value::as_str) {
        Some("null") => Some(Value::Null),
        Some("array") => map.get("value")?.as_array().and_then(|values| {
            values
                .iter()
                .map(cdp_value_from_devtools_argument)
                .collect::<Option<Vec<_>>>()
                .map(Value::Array)
        }),
        Some("object") => map.get("value")?.as_array().and_then(|entries| {
            let mut object = serde_json::Map::new();
            for entry in entries {
                let pair = entry.as_array()?;
                let [key, value] = pair.as_slice() else {
                    return None;
                };
                object.insert(
                    key.as_str()?.to_owned(),
                    cdp_value_from_devtools_argument(value)?,
                );
            }
            Some(Value::Object(object))
        }),
        Some("string" | "number" | "boolean" | "bigint" | "date") => map.get("value").cloned(),
        None => map.get("value").cloned(),
        _ => None,
    }
}

fn devtools_call_function_remote_object_ids(command: &DevToolsCallFunctionCommand) -> Vec<String> {
    let mut object_ids = Vec::new();
    if let Some(object_id) = command.object_id.as_ref() {
        object_ids.push(object_id.as_str().to_owned());
    }
    if let Some(this_parameter) = &command.this_parameter {
        collect_devtools_remote_object_ids(this_parameter, &mut object_ids);
    }
    for argument in &command.arguments {
        collect_devtools_remote_object_ids(argument, &mut object_ids);
    }
    object_ids.sort();
    object_ids.dedup();
    object_ids
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum RuntimeRemoteReferenceKind {
    Object,
    Node,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RuntimeRemoteReference {
    object_id: String,
    kind: RuntimeRemoteReferenceKind,
}

fn devtools_call_function_remote_references(
    command: &DevToolsCallFunctionCommand,
) -> Vec<RuntimeRemoteReference> {
    let mut references = Vec::new();
    if let Some(object_id) = command.object_id.as_ref() {
        references.push(RuntimeRemoteReference {
            object_id: object_id.as_str().to_owned(),
            kind: RuntimeRemoteReferenceKind::Object,
        });
    }
    if let Some(this_parameter) = &command.this_parameter {
        collect_devtools_remote_references(this_parameter, &mut references);
    }
    for argument in &command.arguments {
        collect_devtools_remote_references(argument, &mut references);
    }
    references.sort();
    references.dedup();
    references
}

fn collect_devtools_remote_object_ids(value: &Value, out: &mut Vec<String>) {
    let mut stack = vec![(value, MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH)];
    while let Some((value, remaining_tree_depth)) = stack.pop() {
        let Some(next_tree_depth) = remaining_tree_depth.checked_sub(1) else {
            continue;
        };
        match value {
            Value::Object(map) => {
                for key in ["objectId", "promiseObjectId"] {
                    if let Some(object_id) = map.get(key).and_then(Value::as_str) {
                        out.push(object_id.to_owned());
                    }
                }
                if let Some(object_id) = bidi_remote_reference_object_id(map) {
                    out.push(object_id.to_owned());
                }
                for child in map.values() {
                    stack.push((child, next_tree_depth));
                }
            }
            Value::Array(values) => {
                for child in values {
                    stack.push((child, next_tree_depth));
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
}

fn collect_devtools_remote_references(value: &Value, out: &mut Vec<RuntimeRemoteReference>) {
    let mut stack = vec![(value, MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH)];
    while let Some((value, remaining_tree_depth)) = stack.pop() {
        let Some(next_tree_depth) = remaining_tree_depth.checked_sub(1) else {
            continue;
        };
        match value {
            Value::Object(map) => {
                let mut found_cdp_remote_reference = false;
                for key in ["objectId", "promiseObjectId"] {
                    if let Some(object_id) = map.get(key).and_then(Value::as_str) {
                        out.push(RuntimeRemoteReference {
                            object_id: object_id.to_owned(),
                            kind: RuntimeRemoteReferenceKind::Object,
                        });
                        found_cdp_remote_reference = true;
                    }
                }
                if found_cdp_remote_reference {
                    continue;
                }
                if let Some(shared_id) = map.get("sharedId").and_then(Value::as_str) {
                    out.push(RuntimeRemoteReference {
                        object_id: shared_id.to_owned(),
                        kind: if map.get("type").and_then(Value::as_str) == Some("node") {
                            RuntimeRemoteReferenceKind::Node
                        } else {
                            RuntimeRemoteReferenceKind::Object
                        },
                    });
                    continue;
                } else if let Some(handle) = map.get("handle").and_then(Value::as_str) {
                    out.push(RuntimeRemoteReference {
                        object_id: handle.to_owned(),
                        kind: RuntimeRemoteReferenceKind::Object,
                    });
                    continue;
                }
                for child in map.values() {
                    stack.push((child, next_tree_depth));
                }
            }
            Value::Array(values) => {
                for child in values {
                    stack.push((child, next_tree_depth));
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
}

fn devtools_script_result_from_response(
    response: Value,
    result_ownership: DevToolsResultOwnership,
    target_realm: Option<DevToolsRealmId>,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if let Some(error) = response.get("error") {
        return Err(devtools_error_from_cdp_error_value(error));
    }
    let result = response.get("result").unwrap_or(&Value::Null);
    if let Some(exception_details) = result.get("exceptionDetails") {
        let mut exception = devtools_script_exception_from_cdp(
            exception_details,
            matches!(result_ownership, DevToolsResultOwnership::Root),
        );
        if exception.realm.is_none() {
            exception.realm = target_realm;
        }
        return Ok(DevToolsCommandResult::Script(Box::new(
            DevToolsScriptResult::Exception(exception),
        )));
    }
    let remote = result.get("result").unwrap_or(&Value::Null);
    Ok(DevToolsCommandResult::Script(Box::new(
        DevToolsScriptResult::Value(devtools_remote_value_from_cdp(
            remote,
            matches!(result_ownership, DevToolsResultOwnership::Root),
            target_realm,
        )),
    )))
}

fn devtools_empty_result_from_response(
    response: Value,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if let Some(error) = response.get("error") {
        return Err(devtools_error_from_cdp_error_value(error));
    }
    Ok(DevToolsCommandResult::Empty)
}

fn validate_protocol_neutral_runtime_handle_realms(
    conn: &CdpConnection,
    command: &DevToolsCommand,
    target_realm: Option<&DevToolsRealmId>,
) -> Result<(), DevToolsError> {
    let DevToolsCommand::CallFunction(command) = command else {
        return Ok(());
    };
    if command.context.protocol != DevToolsProtocol::WebDriverBidi {
        return Ok(());
    }

    let references = devtools_call_function_remote_references(command);

    for reference in references {
        if !conn.runtime_remote_object_id_known_for_session_owner(None, &reference.object_id) {
            return Err(match reference.kind {
                RuntimeRemoteReferenceKind::Node => DevToolsError::new(
                    DevToolsErrorKind::NoSuchNode,
                    "Could not find node with given id",
                ),
                RuntimeRemoteReferenceKind::Object => DevToolsError::new(
                    DevToolsErrorKind::NoSuchHandle,
                    "Cannot find object with given id",
                ),
            });
        }
        if reference.kind == RuntimeRemoteReferenceKind::Object
            && let Some(target_realm) = target_realm
            && let Some(owner_realm) =
                conn.runtime_remote_object_realm_for_session_owner(None, &reference.object_id)
            && owner_realm != target_realm.as_str()
        {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchHandle,
                "Cannot find object with given id",
            ));
        }
    }
    Ok(())
}

fn register_devtools_script_result_remote_object_realm(
    conn: &mut CdpConnection,
    result: &DevToolsCommandResult,
    realm_id: Option<&DevToolsRealmId>,
) {
    let (DevToolsCommandResult::Script(result), Some(realm_id)) = (result, realm_id) else {
        return;
    };
    let DevToolsScriptResult::Value(value) = result.as_ref() else {
        return;
    };
    let Some(remote_object_id) = value.handle.as_ref().or(value.shared_id.as_ref()) else {
        return;
    };
    if let Some(object_id) =
        conn.runtime_remote_object_alias_for_session_owner(None, remote_object_id.as_str())
    {
        conn.register_runtime_remote_object_alias_for_session_owner_with_realm(
            None,
            remote_object_id.as_str().to_owned(),
            object_id,
            realm_id.as_str(),
        );
        return;
    }
    conn.register_runtime_remote_object_ids_for_session_owner_with_realm(
        None,
        vec![remote_object_id.as_str().to_owned()],
        realm_id.as_str(),
    );
}

fn register_devtools_script_result_remote_object(
    conn: &mut CdpConnection,
    result: &DevToolsCommandResult,
) {
    let Some(value) = devtools_script_result_remote_value(result) else {
        return;
    };
    let Some(remote_object_id) = value.handle.as_ref().or(value.shared_id.as_ref()) else {
        return;
    };
    conn.register_runtime_remote_object_ids_from_value_for_session_owner(
        None,
        &json!({ "objectId": remote_object_id.as_str() }),
    );
}

fn materialize_devtools_script_window_remote_value(
    result: &mut DevToolsCommandResult,
    target: &DevToolsRuntimeTarget,
) {
    let Some(target_window_context_id) = target.window_context_id.clone() else {
        return;
    };
    let Some(candidate) = devtools_window_remote_candidate(result) else {
        return;
    };
    let window_context_id = if let Some(window_remote) =
        deep_serialized_bidi_window_remote_result(candidate.deep_serialized_value.as_ref())
    {
        match window_remote {
            BidiWindowRemoteResult::TargetWindow => Some(target_window_context_id),
            BidiWindowRemoteResult::Context(window_context_id) => Some(window_context_id),
        }
    } else {
        None
    };
    let Some(window_context_id) = window_context_id else {
        return;
    };
    if let Some(value) = devtools_script_result_remote_value_mut(result) {
        value.window_context = Some(Box::new(window_context_id));
    }
}

enum BidiWindowRemoteResult {
    TargetWindow,
    Context(DevToolsTargetId),
}

fn deep_serialized_bidi_window_remote_result(
    value: Option<&Value>,
) -> Option<BidiWindowRemoteResult> {
    let value = value?;
    if value.get("type").and_then(Value::as_str) == Some("window") {
        return value
            .get("value")
            .and_then(|value| value.get("context"))
            .and_then(Value::as_str)
            .map(|context| BidiWindowRemoteResult::Context(DevToolsTargetId::from(context)));
    }
    let properties = value.get("value").and_then(Value::as_array)?;
    let marker = deep_serialized_property(properties, "__moliBidiRemoteValue")?;
    if deep_serialized_bool_value(marker) != Some(true) {
        return None;
    }
    let type_value = deep_serialized_property(properties, "type")?;
    if deep_serialized_string_value(type_value).as_deref() != Some("window") {
        return None;
    }
    if let Some(target_window) = deep_serialized_property(properties, "targetWindow")
        && deep_serialized_bool_value(target_window) == Some(true)
    {
        return Some(BidiWindowRemoteResult::TargetWindow);
    }
    let context = deep_serialized_property(properties, "context")?;
    deep_serialized_string_value(context)
        .map(|context| BidiWindowRemoteResult::Context(DevToolsTargetId::from(context)))
}

fn deep_serialized_bool_value(value: &Value) -> Option<bool> {
    let object = value.as_object()?;
    (object.get("type").and_then(Value::as_str) == Some("boolean"))
        .then(|| object.get("value")?.as_bool())
        .flatten()
}

fn deep_serialized_string_value(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    (object.get("type").and_then(Value::as_str) == Some("string"))
        .then(|| object.get("value")?.as_str().map(str::to_owned))
        .flatten()
}

fn devtools_window_remote_candidate(
    result: &DevToolsCommandResult,
) -> Option<DevToolsWindowRemoteCandidate> {
    let value = devtools_script_result_remote_value(result)?;
    Some(DevToolsWindowRemoteCandidate {
        deep_serialized_value: value.deep_serialized_value.clone(),
    })
}

async fn devtools_probe_remote_value_async(
    conn: &mut CdpConnection,
    target: DevToolsRuntimeTarget,
    command: DevToolsCommand,
) -> Result<Option<DevToolsRemoteValue>, DevToolsError> {
    let target_route = target.route.clone();
    let result_ownership = devtools_runtime_result_ownership(&command);
    let internal_command_id = conn.next_internal_runtime_command_id();
    let mut step =
        start_protocol_neutral_runtime_command(conn, target, command, internal_command_id).await;
    loop {
        match step {
            RuntimeCommandTaskStep::Complete(plan) => {
                let (response, _events) = plan
                    .into_runtime_inspector_response_and_background_events(
                        internal_command_id,
                        None,
                    );
                let Some(response) = response else {
                    return Err(DevToolsError::new(
                        DevToolsErrorKind::Internal,
                        "MissingDevToolsCommandResult",
                    ));
                };
                let result =
                    devtools_script_result_from_response(response, result_ownership, None)?;
                let DevToolsCommandResult::Script(result) = result else {
                    return Ok(None);
                };
                let DevToolsScriptResult::Value(value) = *result else {
                    return Ok(None);
                };
                return Ok(Some(value));
            }
            RuntimeCommandTaskStep::Pending(pending) => {
                let completed = pending.wait().await;
                let mut scope = RuntimeProbeCompletionScope::enter(conn, target_route.clone());
                step = complete_pending_runtime_command(scope.conn_mut(), completed).await;
            }
        }
    }
}

struct RuntimeProbeCompletionScope<'a> {
    route_scope: NoneSessionOwnerRouteOverrideScope<'a>,
}

impl<'a> RuntimeProbeCompletionScope<'a> {
    fn enter(conn: &'a mut CdpConnection, target_route: CdpSessionRoute) -> Self {
        Self {
            route_scope: conn.scoped_none_session_owner_route_override(target_route),
        }
    }

    fn conn_mut(&mut self) -> &mut CdpConnection {
        self.route_scope.conn_mut()
    }

    fn restore(&mut self) {
        self.route_scope.restore();
    }
}

impl Drop for RuntimeProbeCompletionScope<'_> {
    fn drop(&mut self) {
        self.restore();
    }
}

async fn materialize_devtools_script_deep_serialized_root_value_async(
    conn: &mut CdpConnection,
    result: &mut DevToolsCommandResult,
    serialization_options: Option<&DevToolsSerializationOptions>,
    target: &DevToolsRuntimeTarget,
) {
    let Some(serialization_options) = serialization_options else {
        return;
    };
    let Some(remote_value) = devtools_script_result_remote_value(result) else {
        return;
    };
    if remote_value.deep_serialized_value.is_some() {
        return;
    }
    if !matches!(remote_value.remote_type.as_deref(), Some("object")) {
        return;
    }
    let Some(root_object_id) = remote_value
        .shared_id
        .as_ref()
        .map(|shared_id| shared_id.as_str().to_owned())
    else {
        return;
    };

    let command = DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: None,
            target_id: target.window_context_id.clone(),
            browser_context_id: None,
        },
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(root_object_id.clone())),
        this_parameter: None,
        function_declaration: "function() { return this; }".to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::ByValue,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: Some(serialization_options.clone()),
    });
    let deep_serialized_value =
        match devtools_probe_remote_value_async(conn, target.clone(), command).await {
            Ok(Some(readback)) => readback.deep_serialized_value,
            Ok(None) => None,
            Err(error) => {
                tracing::debug!(
                    %root_object_id,
                    ?error,
                    "failed to materialize root BiDi deep serialized value"
                );
                None
            }
        };
    let Some(deep_serialized_value) = deep_serialized_value else {
        return;
    };
    if let Some(remote_value) = devtools_script_result_remote_value_mut(result) {
        remote_value.deep_serialized_value = Some(deep_serialized_value);
    }
}

fn devtools_script_result_remote_value(
    result: &DevToolsCommandResult,
) -> Option<&DevToolsRemoteValue> {
    let DevToolsCommandResult::Script(result) = result else {
        return None;
    };
    match result.as_ref() {
        DevToolsScriptResult::Value(value) => Some(value),
        DevToolsScriptResult::Exception(exception) => exception.value.as_ref(),
    }
}

fn devtools_script_result_remote_value_mut(
    result: &mut DevToolsCommandResult,
) -> Option<&mut DevToolsRemoteValue> {
    let DevToolsCommandResult::Script(result) = result else {
        return None;
    };
    match result.as_mut() {
        DevToolsScriptResult::Value(value) => Some(value),
        DevToolsScriptResult::Exception(exception) => exception.value.as_mut(),
    }
}

fn bidi_script_message_serialization_options_from_value(
    value: &Value,
) -> Result<DevToolsSerializationOptions, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "InvalidChannelArgument".to_owned())?;
    let max_object_depth = match object.get("maxObjectDepth") {
        Some(Value::Null) | None => None,
        Some(value) => Some(
            value
                .as_u64()
                .ok_or_else(|| "InvalidChannelArgument".to_owned())?,
        ),
    };
    let max_dom_depth = match object.get("maxDomDepth") {
        Some(Value::Null) | None => None,
        Some(value) => Some(
            value
                .as_u64()
                .ok_or_else(|| "InvalidChannelArgument".to_owned())?,
        ),
    };
    let include_shadow_tree = match object.get("includeShadowTree") {
        None => None,
        Some(Value::String(value)) if matches!(value.as_str(), "none" | "open" | "all") => {
            Some(value.clone())
        }
        Some(_) => return Err("InvalidChannelArgument".to_owned()),
    };
    Ok(DevToolsSerializationOptions {
        max_object_depth,
        max_dom_depth,
        include_shadow_tree,
    })
}

async fn devtools_probe_value_async(
    conn: &mut CdpConnection,
    target: DevToolsRuntimeTarget,
    command: DevToolsCommand,
) -> Result<Option<Value>, DevToolsError> {
    let Some(value) = devtools_probe_remote_value_async(conn, target, command).await? else {
        return Ok(None);
    };
    Ok(Some(value.value))
}

fn is_attribute_node_remote_metadata(
    remote_type: Option<&str>,
    remote_subtype: Option<&str>,
    description: Option<&str>,
    class_name: Option<&str>,
) -> bool {
    remote_type == Some("object")
        && matches!(remote_subtype, None | Some("node"))
        && (matches!(class_name, Some("Attr"))
            || matches!(description, Some("Attr") | Some("[object Attr]")))
}

fn deep_serialized_internal_id(value: &Value) -> Option<String> {
    value
        .get("internalId")
        .or_else(|| value.get("weakLocalObjectReference"))
        .and_then(|reference| {
            reference
                .as_str()
                .map(str::to_owned)
                .or_else(|| reference.as_u64().map(|reference| reference.to_string()))
                .or_else(|| reference.as_i64().map(|reference| reference.to_string()))
        })
}

fn deep_serialized_property<'a>(properties: &'a [Value], name: &str) -> Option<&'a Value> {
    properties.iter().find_map(|property| {
        let pair = property.as_array()?;
        let [key, value] = pair.as_slice() else {
            return None;
        };
        (key.as_str() == Some(name)).then_some(value)
    })
}

#[derive(Clone, Debug)]
struct DeepSerializedNodeCandidatePath {
    json_pointer: String,
    js_path: Vec<Value>,
}

struct DeepSerializedNodeCandidateFrame<'a> {
    value: &'a Value,
    json_pointer: String,
    js_path: Vec<Value>,
    remaining_tree_depth: usize,
}

fn collect_deep_serialized_node_candidate_paths(
    value: &Value,
) -> Vec<DeepSerializedNodeCandidatePath> {
    let mut paths = Vec::new();
    let mut stack = vec![DeepSerializedNodeCandidateFrame {
        value,
        json_pointer: String::new(),
        js_path: Vec::new(),
        remaining_tree_depth: MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH,
    }];
    while let Some(frame) = stack.pop() {
        collect_deep_serialized_node_candidate_path_frame(frame, &mut paths, &mut stack);
    }
    paths.sort_by_key(|path| path.js_path.len());
    paths
}

fn collect_deep_serialized_node_candidate_path_frame<'a>(
    frame: DeepSerializedNodeCandidateFrame<'a>,
    out: &mut Vec<DeepSerializedNodeCandidatePath>,
    stack: &mut Vec<DeepSerializedNodeCandidateFrame<'a>>,
) {
    let DeepSerializedNodeCandidateFrame {
        value,
        json_pointer,
        js_path,
        remaining_tree_depth,
    } = frame;
    let Some(next_tree_depth) = remaining_tree_depth.checked_sub(1) else {
        return;
    };
    if !js_path.is_empty() && is_deep_serialized_node_candidate(value) {
        out.push(DeepSerializedNodeCandidatePath {
            json_pointer: json_pointer.clone(),
            js_path: js_path.clone(),
        });
    }

    let Some(value_type) = value.get("type").and_then(Value::as_str) else {
        return;
    };
    let Some(children) = value.get("value").and_then(Value::as_array) else {
        return;
    };

    let mut pending = Vec::new();
    match value_type {
        "object" => {
            for (index, property) in children.iter().enumerate() {
                let Some(pair) = property.as_array() else {
                    continue;
                };
                let [key, child] = pair.as_slice() else {
                    continue;
                };
                let Some(key) = key.as_str() else {
                    continue;
                };
                let mut child_js_path = js_path.clone();
                child_js_path.push(json!({
                    "kind": "property",
                    "key": key,
                }));
                let value_pointer = json_pointer_child(&json_pointer, "value");
                let property_pointer = json_pointer_child(&value_pointer, index);
                pending.push(DeepSerializedNodeCandidateFrame {
                    value: child,
                    json_pointer: json_pointer_child(&property_pointer, "1"),
                    js_path: child_js_path,
                    remaining_tree_depth: next_tree_depth,
                });
            }
        }
        "array" | "htmlcollection" | "nodelist" => {
            for (index, child) in children.iter().enumerate() {
                let mut child_js_path = js_path.clone();
                child_js_path.push(json!({
                    "kind": "index",
                    "index": index,
                }));
                let value_pointer = json_pointer_child(&json_pointer, "value");
                pending.push(DeepSerializedNodeCandidateFrame {
                    value: child,
                    json_pointer: json_pointer_child(&value_pointer, index),
                    js_path: child_js_path,
                    remaining_tree_depth: next_tree_depth,
                });
            }
        }
        "set" => {
            for (index, child) in children.iter().enumerate() {
                let mut child_js_path = js_path.clone();
                child_js_path.push(json!({
                    "kind": "iterable",
                    "index": index,
                }));
                let value_pointer = json_pointer_child(&json_pointer, "value");
                pending.push(DeepSerializedNodeCandidateFrame {
                    value: child,
                    json_pointer: json_pointer_child(&value_pointer, index),
                    js_path: child_js_path,
                    remaining_tree_depth: next_tree_depth,
                });
            }
        }
        "map" => {
            for (index, entry) in children.iter().enumerate() {
                let Some(pair) = entry.as_array() else {
                    continue;
                };
                let [key, child] = pair.as_slice() else {
                    continue;
                };
                let value_pointer = json_pointer_child(&json_pointer, "value");
                let entry_pointer = json_pointer_child(&value_pointer, index);
                let mut key_js_path = js_path.clone();
                key_js_path.push(json!({
                    "kind": "mapEntry",
                    "index": index,
                    "part": 0,
                }));
                pending.push(DeepSerializedNodeCandidateFrame {
                    value: key,
                    json_pointer: json_pointer_child(&entry_pointer, "0"),
                    js_path: key_js_path,
                    remaining_tree_depth: next_tree_depth,
                });

                let mut child_js_path = js_path.clone();
                child_js_path.push(json!({
                    "kind": "mapEntry",
                    "index": index,
                    "part": 1,
                }));
                pending.push(DeepSerializedNodeCandidateFrame {
                    value: child,
                    json_pointer: json_pointer_child(&entry_pointer, "1"),
                    js_path: child_js_path,
                    remaining_tree_depth: next_tree_depth,
                });
            }
        }
        _ => {}
    }
    for frame in pending.into_iter().rev() {
        stack.push(frame);
    }
}

fn is_deep_serialized_node_candidate(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("object")
}

fn json_pointer_child(parent: &str, segment: impl ToString) -> String {
    let segment = segment.to_string();
    format!("{parent}/{}", escape_json_pointer_segment(&segment))
}

fn escape_json_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn json_pointer_matches_or_descends_from(pointer: &str, ancestor: &str) -> bool {
    pointer == ancestor
        || pointer
            .strip_prefix(ancestor)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

async fn materialize_devtools_script_node_remote_value_async(
    conn: &mut CdpConnection,
    result: &mut DevToolsCommandResult,
    serialization_options: Option<&DevToolsSerializationOptions>,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
) {
    let Some(value) = devtools_script_result_remote_value_mut(result) else {
        return;
    };
    let is_attribute_node = is_attribute_node_remote_metadata(
        value.remote_type.as_deref(),
        value.remote_subtype.as_deref(),
        value.description.as_deref(),
        value.class_name.as_deref(),
    );
    if value.remote_subtype.as_deref() != Some("node") && !is_attribute_node {
        return;
    }
    if materialize_devtools_script_node_remote_value_from_deep_serialized(conn, value, realm_id) {
        return;
    }
    let Some(shared_id) = value
        .shared_id
        .as_ref()
        .map(|shared_id| shared_id.as_str().to_owned())
    else {
        return;
    };
    let node_options = bidi_node_serialization_options(serialization_options);
    let object_snapshot = match conn
        .document_node_snapshot_for_runtime_remote_object_id_async(
            None,
            &shared_id,
            node_options.snapshot_depth,
            true,
        )
        .await
    {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::debug!(%shared_id, %error, "failed to materialize BiDi node remote value");
            None
        }
    };
    let Some(object_snapshot) = object_snapshot else {
        if is_attribute_node {
            match devtools_attribute_node_value_async(conn, target, shared_id.clone()).await {
                Ok(Some(attribute_value)) => {
                    value.remote_subtype = Some("node".to_owned());
                    value.node_value = Some(attribute_value);
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::debug!(
                        %shared_id,
                        ?error,
                        "failed to materialize BiDi attribute node remote value"
                    );
                }
            }
        }
        if value.node_value.is_some() {
            return;
        }
        match devtools_detached_node_value_async(conn, target, shared_id.clone(), &node_options)
            .await
        {
            Ok(Some(node_value)) => {
                value.remote_subtype = Some("node".to_owned());
                value.node_value = Some(node_value);
            }
            Ok(None) => {}
            Err(error) => {
                tracing::debug!(
                    %shared_id,
                    ?error,
                    "failed to materialize detached BiDi node remote value"
                );
            }
        }
        return;
    };
    let Some(canonical_shared_id) = bidi_node_shared_id_for_snapshot(&object_snapshot.snapshot)
    else {
        return;
    };
    register_bidi_node_bindings_for_snapshot_tree(conn, &object_snapshot.snapshot).await;
    register_bidi_node_shared_id_alias(conn, &canonical_shared_id, &shared_id, realm_id);
    value.node_id = object_snapshot.snapshot.frontend_node_id;
    value.backend_node_id = object_snapshot.snapshot.backend_node_id;
    value.shared_id = Some(canonical_shared_id);
    value.node_value = Some(bidi_node_value_from_snapshot(
        &object_snapshot.snapshot,
        &node_options,
    ));
}

async fn materialize_devtools_script_deep_serialized_node_remote_values_async(
    conn: &mut CdpConnection,
    result: &mut DevToolsCommandResult,
    serialization_options: Option<&DevToolsSerializationOptions>,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
) {
    let Some(value) = devtools_script_result_remote_value_mut(result) else {
        return;
    };
    let Some(root_object_id) = value
        .shared_id
        .as_ref()
        .map(|shared_id| shared_id.as_str().to_owned())
    else {
        return;
    };
    let Some(deep_serialized_value) = value.deep_serialized_value.as_mut() else {
        return;
    };
    let candidate_paths = collect_deep_serialized_node_candidate_paths(deep_serialized_value);
    if candidate_paths.is_empty() {
        return;
    }

    let node_options = bidi_node_serialization_options(serialization_options);
    let mut materialized_paths: Vec<String> = Vec::new();
    for candidate_path in candidate_paths {
        if materialized_paths
            .iter()
            .any(|path| json_pointer_matches_or_descends_from(&candidate_path.json_pointer, path))
        {
            continue;
        }

        let internal_id = deep_serialized_value
            .pointer(&candidate_path.json_pointer)
            .and_then(deep_serialized_internal_id);
        let Some(remote_value) =
            materialize_devtools_script_deep_serialized_node_remote_value_async(
                conn,
                target,
                &root_object_id,
                &candidate_path.js_path,
                &node_options,
                serialization_options,
                realm_id,
            )
            .await
        else {
            continue;
        };
        let mut remote_value = remote_value;
        if let (Some(internal_id), Some(map)) = (internal_id, remote_value.as_object_mut()) {
            map.insert("internalId".to_owned(), json!(internal_id));
        }
        if let Some(slot) = deep_serialized_value.pointer_mut(&candidate_path.json_pointer) {
            *slot = remote_value;
            materialized_paths.push(candidate_path.json_pointer);
        }
    }
}

async fn materialize_devtools_script_dom_collection_remote_value_async(
    conn: &mut CdpConnection,
    result: &mut DevToolsCommandResult,
    serialization_options: Option<&DevToolsSerializationOptions>,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
) {
    let Some(root_object_id) = devtools_script_result_remote_value(result)
        .and_then(|value| value.shared_id.as_ref())
        .map(|shared_id| shared_id.as_str().to_owned())
    else {
        return;
    };
    let probe = match devtools_dom_collection_probe_async(conn, target, &root_object_id).await {
        Ok(probe) => probe,
        Err(error) => {
            tracing::debug!(
                %root_object_id,
                ?error,
                "failed to probe BiDi DOM collection remote value"
            );
            None
        }
    };
    let Some(probe) = probe else {
        return;
    };

    let node_options = bidi_node_serialization_options(serialization_options);
    let entries = materialize_bidi_dom_collection_entries_async(
        conn,
        target,
        &root_object_id,
        probe.length,
        &node_options.with_decremented_value_depth(),
        realm_id,
    )
    .await;
    if let Some(remote_value) = devtools_script_result_remote_value_mut(result) {
        remote_value.remote_type = Some("object".to_owned());
        remote_value.remote_subtype = Some(probe.kind.as_bidi_type().to_owned());
        remote_value.node_value = None;
        remote_value.deep_serialized_value = Some(json!({
            "type": probe.kind.as_bidi_type(),
            "value": entries,
        }));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BidiDomCollectionProbe {
    kind: BidiDomCollectionKind,
    length: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BidiDomCollectionKind {
    HtmlCollection,
    NodeList,
}

impl BidiDomCollectionKind {
    fn as_bidi_type(self) -> &'static str {
        match self {
            Self::HtmlCollection => "htmlcollection",
            Self::NodeList => "nodelist",
        }
    }

    fn from_bidi_type(value: &str) -> Option<Self> {
        match value {
            "htmlcollection" => Some(Self::HtmlCollection),
            "nodelist" => Some(Self::NodeList),
            _ => None,
        }
    }
}

async fn devtools_dom_collection_probe_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    root_object_id: &str,
) -> Result<Option<BidiDomCollectionProbe>, DevToolsError> {
    let command = DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: None,
            target_id: target.window_context_id.clone(),
            browser_context_id: None,
        },
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(root_object_id.to_owned())),
        this_parameter: None,
        function_declaration: r#"function() {
            const length = Number(this && this.length);
            if (!Number.isFinite(length) || length < 0) {
                return null;
            }
            function hasPrototypeNamed(value, name) {
                let proto = Object.getPrototypeOf(value);
                const expected = globalThis[name] && globalThis[name].prototype;
                while (proto !== null) {
                    if (expected && proto === expected) {
                        return true;
                    }
                    const ctorName = proto.constructor && proto.constructor.name;
                    if (ctorName === name) {
                        return true;
                    }
                    proto = Object.getPrototypeOf(proto);
                }
                return false;
            }
            let kind = null;
            try {
                if (
                    (typeof HTMLCollection === "function" && this instanceof HTMLCollection) ||
                    hasPrototypeNamed(this, "HTMLCollection") ||
                    (typeof this.item === "function" && typeof this.namedItem === "function")
                ) {
                    kind = "htmlcollection";
                }
            } catch (_) {}
            try {
                if (
                    kind === null &&
                    (
                        (typeof NodeList === "function" && this instanceof NodeList) ||
                        hasPrototypeNamed(this, "NodeList") ||
                        (
                            typeof this.item === "function" &&
                            typeof this.namedItem !== "function" &&
                            (typeof this.forEach === "function" ||
                                typeof this[Symbol.iterator] === "function")
                        )
                    )
                ) {
                    kind = "nodelist";
                }
            } catch (_) {}
            if (kind === null) {
                return null;
            }
            return {
                kind,
                length: Math.min(Math.trunc(length), 4096),
            };
        }"#
        .to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::ByValue,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    });
    let Some(value) = devtools_probe_value_async(conn, target.clone(), command).await? else {
        return Ok(None);
    };
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let Some(kind) = object
        .get("kind")
        .and_then(Value::as_str)
        .and_then(BidiDomCollectionKind::from_bidi_type)
    else {
        return Ok(None);
    };
    let length = object
        .get("length")
        .and_then(Value::as_u64)
        .map(|length| length.min(4096) as usize)
        .unwrap_or(0);
    Ok(Some(BidiDomCollectionProbe { kind, length }))
}

async fn materialize_bidi_dom_collection_entries_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    root_object_id: &str,
    length: usize,
    node_options: &BidiNodeSerializationOptions,
    realm_id: Option<&DevToolsRealmId>,
) -> Vec<Value> {
    let mut entries = Vec::new();
    for index in 0..length {
        let js_path = [json!({
            "kind": "index",
            "index": index,
        })];
        let remote_value = match devtools_deep_serialized_path_remote_value_async(
            conn,
            target,
            root_object_id,
            &js_path,
            Some(&devtools_serialization_options_for_node_probe(node_options)),
        )
        .await
        {
            Ok(remote_value) => remote_value,
            Err(error) => {
                tracing::debug!(
                    %root_object_id,
                    index,
                    ?error,
                    "failed to resolve BiDi DOM collection entry"
                );
                None
            }
        };
        let Some(remote_value) = remote_value else {
            continue;
        };
        let Some(object_id) = remote_value
            .shared_id
            .as_ref()
            .map(|shared_id| shared_id.as_str().to_owned())
        else {
            continue;
        };
        if let Some(remote) =
            bidi_node_remote_value_from_deep_serialized_remote_value(&remote_value)
        {
            register_devtools_script_remote_object_realm(conn, &object_id, realm_id);
            register_bidi_node_remote_value_shared_id_alias(conn, &remote, &object_id, realm_id);
            entries.push(remote);
            continue;
        }
        let object_snapshot = match conn
            .document_node_snapshot_for_runtime_remote_object_id_async(
                None,
                &object_id,
                node_options.snapshot_depth,
                true,
            )
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::debug!(
                    %object_id,
                    index,
                    %error,
                    "failed to materialize BiDi DOM collection entry node"
                );
                None
            }
        };
        let Some(object_snapshot) = object_snapshot else {
            continue;
        };
        register_devtools_script_remote_object_realm(conn, &object_id, realm_id);
        let Some(shared_id) = bidi_node_shared_id_for_snapshot(&object_snapshot.snapshot) else {
            continue;
        };
        register_bidi_node_bindings_for_snapshot_tree(conn, &object_snapshot.snapshot).await;
        register_bidi_node_shared_id_alias(conn, &shared_id, &object_id, realm_id);
        entries.push(bidi_node_remote_value_from_snapshot(
            &object_snapshot.snapshot,
            shared_id,
            node_options,
        ));
    }
    entries
}

async fn materialize_devtools_script_deep_serialized_node_remote_value_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    root_object_id: &str,
    js_path: &[Value],
    node_options: &BidiNodeSerializationOptions,
    serialization_options: Option<&DevToolsSerializationOptions>,
    realm_id: Option<&DevToolsRealmId>,
) -> Option<Value> {
    let remote_value = match devtools_deep_serialized_path_remote_value_async(
        conn,
        target,
        root_object_id,
        js_path,
        serialization_options,
    )
    .await
    {
        Ok(remote_value) => remote_value?,
        Err(error) => {
            tracing::debug!(
                %root_object_id,
                ?js_path,
                ?error,
                "failed to resolve deep serialized BiDi node candidate"
            );
            return None;
        }
    };
    let object_id = remote_value
        .shared_id
        .as_ref()
        .map(|shared_id| shared_id.as_str().to_owned())?;
    if let Some(remote) = bidi_node_remote_value_from_deep_serialized_remote_value(&remote_value) {
        register_devtools_script_remote_object_realm(conn, &object_id, realm_id);
        register_bidi_node_remote_value_shared_id_alias(conn, &remote, &object_id, realm_id);
        return Some(remote);
    }

    let object_snapshot = match conn
        .document_node_snapshot_for_runtime_remote_object_id_async(
            None,
            &object_id,
            node_options.snapshot_depth,
            true,
        )
        .await
    {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::debug!(
                %object_id,
                ?js_path,
                %error,
                "failed to materialize deep serialized BiDi node candidate"
            );
            None
        }
    };

    if let Some(object_snapshot) = object_snapshot {
        register_devtools_script_remote_object_realm(conn, &object_id, realm_id);
        let shared_id = bidi_node_shared_id_for_snapshot(&object_snapshot.snapshot)?;
        register_bidi_node_bindings_for_snapshot_tree(conn, &object_snapshot.snapshot).await;
        register_bidi_node_shared_id_alias(conn, &shared_id, &object_id, realm_id);
        return Some(bidi_node_remote_value_from_snapshot(
            &object_snapshot.snapshot,
            shared_id,
            node_options,
        ));
    }

    let is_attribute_node = is_attribute_node_remote_metadata(
        remote_value.remote_type.as_deref(),
        remote_value.remote_subtype.as_deref(),
        remote_value.description.as_deref(),
        remote_value.class_name.as_deref(),
    );
    if !is_attribute_node {
        let detached_value =
            match devtools_detached_node_value_async(conn, target, object_id.clone(), node_options)
                .await
            {
                Ok(Some(value)) => value,
                Ok(None) => return None,
                Err(error) => {
                    tracing::debug!(
                        %object_id,
                        ?js_path,
                        ?error,
                        "failed to materialize detached deep serialized BiDi node candidate"
                    );
                    return None;
                }
            };
        register_devtools_script_remote_object_realm(conn, &object_id, realm_id);
        return Some(json!({
            "type": "node",
            "sharedId": object_id,
            "value": detached_value,
        }));
    }

    let attribute_value =
        match devtools_attribute_node_value_async(conn, target, object_id.clone()).await {
            Ok(Some(value)) => value,
            Ok(None) => return None,
            Err(error) => {
                tracing::debug!(
                    %object_id,
                    ?js_path,
                    ?error,
                    "failed to materialize deep serialized BiDi attribute node candidate"
                );
                return None;
            }
        };
    register_devtools_script_remote_object_realm(conn, &object_id, realm_id);
    Some(json!({
        "type": "node",
        "sharedId": object_id,
        "value": attribute_value,
    }))
}

async fn devtools_deep_serialized_path_remote_value_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    root_object_id: &str,
    js_path: &[Value],
    serialization_options: Option<&DevToolsSerializationOptions>,
) -> Result<Option<DevToolsRemoteValue>, DevToolsError> {
    let command = DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: None,
            target_id: target.window_context_id.clone(),
            browser_context_id: None,
        },
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(root_object_id.to_owned())),
        this_parameter: None,
        function_declaration: r#"function(path) {
            let value = this;
            for (const step of path) {
                if (value == null || step == null || typeof step !== "object") {
                    return null;
                }
                if (step.kind === "property") {
                    value = value[step.key];
                } else if (step.kind === "index") {
                    value = value[Number(step.index)];
                } else if (step.kind === "iterable") {
                    if (typeof value[Symbol.iterator] !== "function") {
                        return null;
                    }
                    value = Array.from(value)[Number(step.index)];
                } else if (step.kind === "mapEntry") {
                    if (typeof value[Symbol.iterator] !== "function") {
                        return null;
                    }
                    const entry = Array.from(value)[Number(step.index)];
                    if (!entry) {
                        return null;
                    }
                    value = entry[Number(step.part)];
                } else {
                    return null;
                }
            }
            return value == null ? null : value;
        }"#
        .to_owned(),
        arguments: vec![Value::Array(js_path.to_vec())],
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::Root,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: serialization_options.cloned(),
    });
    devtools_probe_remote_value_async(conn, target.clone(), command).await
}

fn materialize_devtools_script_node_remote_value_from_deep_serialized(
    conn: &mut CdpConnection,
    value: &mut DevToolsRemoteValue,
    realm_id: Option<&DevToolsRealmId>,
) -> bool {
    let Some(remote) = bidi_node_remote_value_from_deep_serialized_remote_value(value) else {
        return false;
    };
    let original_object_id = value
        .shared_id
        .as_ref()
        .map(|shared_id| shared_id.as_str().to_owned());
    let Some(shared_id) = bidi_node_remote_value_shared_id(&remote).map(str::to_owned) else {
        return false;
    };
    let Some(node_value) = remote.get("value").cloned() else {
        return false;
    };
    if let Some(original_object_id) = original_object_id.as_deref() {
        register_bidi_node_remote_value_shared_id_alias(
            conn,
            &remote,
            original_object_id,
            realm_id,
        );
    }
    value.remote_type = Some("object".to_owned());
    value.remote_subtype = Some("node".to_owned());
    value.shared_id = Some(DevToolsRemoteHandleId::from(shared_id));
    value.node_value = Some(node_value);
    true
}

fn register_bidi_node_remote_value_shared_id_alias(
    conn: &mut CdpConnection,
    remote: &Value,
    remote_object_id: &str,
    realm_id: Option<&DevToolsRealmId>,
) {
    let Some(shared_id) = bidi_node_remote_value_shared_id(remote) else {
        return;
    };
    register_bidi_node_shared_id_alias(
        conn,
        &DevToolsRemoteHandleId::from(shared_id.to_owned()),
        remote_object_id,
        realm_id,
    );
}

async fn register_bidi_node_bindings_for_snapshot_tree(
    conn: &mut CdpConnection,
    snapshot: &DocumentNodeSnapshot,
) {
    let mut entries = Vec::new();
    let mut stack = vec![snapshot];
    while let Some(snapshot) = stack.pop() {
        if let Some(backend_node_id) = snapshot.backend_node_id {
            entries.push((
                webdriver_bidi_node_shared_id_for_backend_node_id(backend_node_id),
                backend_node_id,
            ));
        }
        stack.extend(snapshot.shadow_roots.iter().rev());
        stack.extend(snapshot.children.iter().rev());
    }

    for (shared_id, backend_node_id) in entries {
        if let Err(error) = conn
            .register_document_bidi_node_binding_for_session_owner_async(
                None,
                shared_id.as_str(),
                backend_node_id,
            )
            .await
        {
            tracing::debug!(
                %error,
                shared_id = shared_id.as_str(),
                backend_node_id,
                "failed to register renderer BiDi node binding"
            );
        }
    }
}

async fn devtools_detached_node_value_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    remote_object_id: String,
    node_options: &BidiNodeSerializationOptions,
) -> Result<Option<Value>, DevToolsError> {
    let command = DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: None,
            target_id: target.window_context_id.clone(),
            browser_context_id: None,
        },
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(remote_object_id)),
        this_parameter: None,
        function_declaration: r#"function(maxDepth) {
            if (!this || typeof this.nodeType !== "number") {
                return null;
            }
            const nodeType = Number(this.nodeType);
            if (!Number.isFinite(nodeType)) {
                return null;
            }
            const childNodes = this.childNodes;
            const childNodeCount = childNodes == null ? 0 : Number(childNodes.length) || 0;
            const value = {
                nodeType,
                childNodeCount,
            };
            if (nodeType === 1) {
                value.localName = this.localName == null ? "" : String(this.localName);
                value.namespaceURI = this.namespaceURI == null ? null : String(this.namespaceURI);
                const attributes = {};
                if (this.attributes) {
                    for (let index = 0; index < this.attributes.length; index++) {
                        const attr = this.attributes[index];
                        if (!attr) {
                            continue;
                        }
                        const localName = attr.localName == null ? String(attr.name ?? "") : String(attr.localName);
                        const name = attr.prefix ? `${attr.prefix}:${localName}` : (attr.name == null ? localName : String(attr.name));
                        attributes[name] = attr.value == null ? "" : String(attr.value);
                    }
                }
                value.attributes = attributes;
                value.shadowRoot = null;
            } else if (nodeType === 3 || nodeType === 4 || nodeType === 7 || nodeType === 8) {
                value.nodeValue = this.nodeValue == null ? "" : String(this.nodeValue);
            } else if (nodeType === 11 && this.mode != null) {
                value.mode = String(this.mode);
            }
            const depth = Number(maxDepth);
            if (depth !== 0 && childNodeCount === 0) {
                value.children = [];
            }
            return value;
        }"#
        .to_owned(),
        arguments: vec![json!(node_options.value_depth)],
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::ByValue,
        object_group: None,
        preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
        serialization_options: None,
    });
    let Some(value) = devtools_probe_value_async(conn, target.clone(), command).await? else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(value))
}

fn register_devtools_script_remote_object_realm(
    conn: &mut CdpConnection,
    remote_object_id: &str,
    realm_id: Option<&DevToolsRealmId>,
) {
    let Some(realm_id) = realm_id else {
        return;
    };
    conn.register_runtime_remote_object_ids_for_session_owner_with_realm(
        None,
        vec![remote_object_id.to_owned()],
        realm_id.as_str(),
    );
}

fn register_bidi_node_shared_id_alias(
    conn: &mut CdpConnection,
    shared_id: &DevToolsRemoteHandleId,
    remote_object_id: &str,
    realm_id: Option<&DevToolsRealmId>,
) {
    let Some(realm_id) = realm_id else {
        return;
    };
    if shared_id.as_str() == remote_object_id || is_webdriver_bidi_node_shared_id(remote_object_id)
    {
        return;
    }
    conn.register_runtime_remote_object_alias_for_session_owner_with_realm(
        None,
        shared_id.as_str().to_owned(),
        remote_object_id.to_owned(),
        realm_id.as_str(),
    );
}

async fn devtools_attribute_node_value_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    remote_object_id: String,
) -> Result<Option<Value>, DevToolsError> {
    let command = DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: None,
            target_id: target.window_context_id.clone(),
            browser_context_id: None,
        },
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(remote_object_id)),
        this_parameter: None,
        function_declaration: r#"function() {
            if (!this || Number(this.nodeType) !== 2) {
                return null;
            }
            const localName = this.localName == null ? String(this.name ?? "") : String(this.localName);
            const namespaceURI = this.namespaceURI == null ? null : String(this.namespaceURI);
            const nodeValue = this.nodeValue == null ? String(this.value ?? "") : String(this.nodeValue);
            return {
                childNodeCount: 0,
                localName,
                namespaceURI,
                nodeType: 2,
                nodeValue,
            };
        }"#
        .to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::ByValue,
        object_group: None,
        preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
        serialization_options: None,
    });
    let Some(value) = devtools_probe_value_async(conn, target.clone(), command).await? else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(value))
}

async fn execute_devtools_release_objects_command_async(
    conn: &mut CdpConnection,
    command: DevToolsReleaseObjectsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let target =
        devtools_runtime_target_async(conn, &DevToolsCommand::ReleaseObjects(command.clone()))
            .await?;
    let target_realm = devtools_realm_id_for_runtime_target_async(conn, &target).await;
    let mut route_scope = conn.scoped_none_session_owner_route_override(target.route);
    let result = release_devtools_objects_on_current_route_async(
        route_scope.conn_mut(),
        &command.handles,
        target_realm.as_ref(),
    )
    .await;
    drop(route_scope);
    result?;
    Ok(DevToolsCommandResult::Empty)
}

async fn release_devtools_objects_on_current_route_async(
    conn: &mut CdpConnection,
    handles: &[DevToolsRemoteHandleId],
    target_realm: Option<&DevToolsRealmId>,
) -> Result<(), DevToolsError> {
    for handle in handles {
        let object_id = handle.as_str().to_owned();
        if !conn.runtime_remote_object_id_known_for_session_owner(None, &object_id) {
            continue;
        }
        if let Some(target_realm) = target_realm
            && let Some(owner_realm) =
                conn.runtime_remote_object_realm_for_session_owner(None, &object_id)
            && owner_realm != target_realm.as_str()
        {
            continue;
        }
        let params = json!({ "objectId": object_id });
        let command_id = conn.next_internal_runtime_command_id();
        let raw_json = runtime_inspector_command_json(command_id, "Runtime.releaseObject", &params);
        let response = dispatch_runtime_inspector_command_response_for_current_route_async(
            conn, raw_json, command_id,
        )
        .await?;
        if let BackgroundCommandResponsePayload::Error { code, message, .. } = response {
            let error = devtools_error_from_cdp_error_parts(Some(i64::from(code)), &message);
            if matches!(error.kind, DevToolsErrorKind::NoSuchHandle) {
                conn.unregister_runtime_remote_object_ids_for_session_owner(None, &[object_id]);
                continue;
            }
            return Err(error);
        }
        conn.unregister_runtime_remote_object_ids_for_session_owner(None, &[object_id]);
    }
    Ok(())
}

async fn dispatch_runtime_inspector_command_response_for_current_route_async(
    conn: &mut CdpConnection,
    raw_json: String,
    command_id: u64,
) -> Result<BackgroundCommandResponsePayload, DevToolsError> {
    let descriptor = RendererCommandDescriptor::from_synthesized_payload(raw_json)
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?;
    let pending = conn
        .start_runtime_protocol_message_for_session_owner_with_deferred_response(
            None, descriptor, command_id,
        )
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?;
    let mut completed = pending
        .wait()
        .await
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?;
    let response_rx = completed.take_deferred_response_receiver().ok_or_else(|| {
        DevToolsError::new(
            DevToolsErrorKind::Internal,
            "MissingRuntimeInspectorResponseReceiver",
        )
    })?;
    let output = conn
        .complete_runtime_protocol_message_for_session_owner_async(completed)
        .await
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?;
    if let Some(message) = output
        .as_ref()
        .and_then(|output| renderer_command_turn_frontend_protocol_response(output, command_id))
    {
        return Ok(BackgroundCommandResponsePayload::from_runtime_inspector_message(message));
    }
    let response = RuntimeInspectorResponseReady::new(
        command_id,
        None,
        response_rx
            .await
            .map_err(|_| "RuntimeInspectorResponseCanceled".to_owned()),
    );
    let Some(mut response) = conn.resolve_runtime_inspector_response_ready(response) else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "RuntimeInspectorResponseMissingCorrelation",
        ));
    };
    if response
        .renderer_agent_attachment_id()
        .is_some_and(|attachment_id| {
            !conn.renderer_agent_attachment_is_current_for_session_owner(None, attachment_id)
        })
    {
        response.replace_with_error("Execution context was destroyed by navigation");
    }
    Ok(response.into_command_response_payload())
}

async fn execute_devtools_get_realms_command_async(
    conn: &mut CdpConnection,
    command: DevToolsGetRealmsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let requested_target_id = command
        .context
        .target_id
        .as_ref()
        .map(|target_id| target_id.as_str().to_owned());
    let routes = devtools_get_realms_routes(conn, command.context.target_id.as_ref())?;
    let requested_target_is_worker_target = routes.iter().any(|route| {
        matches!(
            route,
            CdpSessionRoute::SharedWorkerTarget { .. }
                | CdpSessionRoute::DedicatedWorkerTarget { .. }
                | CdpSessionRoute::ServiceWorkerTarget { .. }
        )
    });
    let mut realms = Vec::new();
    for route in routes {
        realms.extend(devtools_realms_for_route_async(conn, route).await?);
    }
    if let Some(requested_target_id) = requested_target_id.as_deref()
        && !requested_target_is_worker_target
    {
        realms.retain(|realm| {
            realm
                .frame_id
                .as_ref()
                .is_some_and(|frame_id| frame_id.as_str() == requested_target_id)
        });
        if realms.is_empty() {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "NoSuchTarget",
            ));
        }
    }
    if let Some(realm_type) = command.realm_type.as_deref() {
        realms.retain(|realm| devtools_realm_type(realm.context_type.as_deref()) == realm_type);
    }
    dedup_devtools_realms(&mut realms);
    Ok(DevToolsCommandResult::Realms(DevToolsGetRealmsResult {
        realms,
    }))
}

fn dedup_devtools_realms(realms: &mut Vec<RuntimeExecutionContextEvent>) {
    let mut seen = HashSet::new();
    realms.retain(|realm| {
        let key = if let Some(realm_id) = realm.realm_id.as_ref() {
            format!(
                "realm:{}:{}",
                realm
                    .frame_id
                    .as_ref()
                    .map(|frame_id| frame_id.as_str())
                    .unwrap_or_default(),
                realm_id.as_str()
            )
        } else {
            format!(
                "context:{}:{}:{}",
                realm
                    .frame_id
                    .as_ref()
                    .map(|frame_id| frame_id.as_str())
                    .unwrap_or_default(),
                realm
                    .context_id
                    .map(|context_id| context_id.to_string())
                    .unwrap_or_default(),
                realm.context_type.as_deref().unwrap_or_default()
            )
        };
        seen.insert(key)
    });
}

fn devtools_get_realms_routes(
    conn: &CdpConnection,
    target_id: Option<&crate::devtools_runtime::DevToolsTargetId>,
) -> Result<Vec<CdpSessionRoute>, DevToolsError> {
    if let Some(target_id) = target_id {
        if let Some(route) = conn
            .target_session_route_for_target_id(target_id.as_str())
            .or_else(|| conn.target_session_route_for_child_frame_id(target_id.as_str()))
        {
            return Ok(vec![route]);
        }
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "NoSuchTarget",
        ));
    }

    let mut routes = Vec::new();
    for target_info in conn
        .browser_contexts()
        .flat_map(crate::conn::BrowserContext::devtools_target_infos)
    {
        if !matches!(
            target_info.kind,
            crate::devtools_runtime::DevToolsTargetKind::Page
                | crate::devtools_runtime::DevToolsTargetKind::Frame
                | crate::devtools_runtime::DevToolsTargetKind::SharedWorker
                | crate::devtools_runtime::DevToolsTargetKind::Worker
                | crate::devtools_runtime::DevToolsTargetKind::ServiceWorker
        ) {
            continue;
        }
        let Some(target_id) = target_info.target_id else {
            continue;
        };
        if let Some(route) = conn.target_session_route_for_target_id(target_id.as_str()) {
            routes.push(route);
        }
    }
    if routes.is_empty() {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "NoSuchTarget",
        ));
    }
    Ok(routes)
}

async fn devtools_realms_for_route_async(
    conn: &mut CdpConnection,
    route: CdpSessionRoute,
) -> Result<Vec<RuntimeExecutionContextEvent>, DevToolsError> {
    if let CdpSessionRoute::SharedWorkerTarget {
        browser_context_id,
        target_id,
    } = &route
    {
        let target = conn
            .browser_context_by_id(browser_context_id)
            .and_then(|context| context.shared_worker_target(target_id))
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
        return Ok(shared_worker_target_runtime_realm(target)
            .into_iter()
            .collect());
    }
    if let CdpSessionRoute::DedicatedWorkerTarget {
        browser_context_id,
        target_id,
    } = &route
    {
        let target = conn
            .browser_context_by_id(browser_context_id)
            .and_then(|context| context.dedicated_worker_target(target_id))
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
        return Ok(dedicated_worker_target_runtime_realm(target)
            .into_iter()
            .collect());
    }
    if let CdpSessionRoute::ServiceWorkerTarget {
        browser_context_id,
        target_id,
    } = &route
    {
        let target = conn
            .browser_context_by_id(browser_context_id)
            .and_then(|context| context.service_worker_target(target_id))
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
        return Ok(service_worker_target_runtime_realm(target)
            .into_iter()
            .collect());
    }
    let mut route_scope = conn.scoped_none_session_owner_route_override(route);
    let result = route_scope
        .conn_mut()
        .runtime_realm_inventory_for_session_owner_async(None)
        .await
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message));
    drop(route_scope);
    result
}

fn shared_worker_target_runtime_realm(
    target: &crate::conn::SharedWorkerTargetState,
) -> Option<RuntimeExecutionContextEvent> {
    let context_id = target.real_runtime_execution_context_id()?;
    let origin = url::Url::parse(&target.url)
        .ok()
        .map(|url| moli_url::origin_ascii_serialization(&url))
        .unwrap_or_else(|| "null".to_owned());
    Some(RuntimeExecutionContextEvent {
        target_id: Some(DevToolsTargetId::from(target.target_id.as_str())),
        context_id: Some(context_id),
        realm_id: Some(DevToolsRealmId::from(format!(
            "shared-worker-{}",
            target.target_id
        ))),
        frame_id: None,
        origin: Some(origin),
        name: Some(target.name.clone()),
        is_default: Some(true),
        context_type: Some("shared-worker".to_owned()),
        grant_universal_access: None,
    })
}

fn dedicated_worker_target_runtime_realm(
    target: &crate::conn::DedicatedWorkerTargetState,
) -> Option<RuntimeExecutionContextEvent> {
    let mut realm = shared_worker_target_runtime_realm(&target.inner)?;
    realm.realm_id = Some(DevToolsRealmId::from(format!(
        "dedicated-worker-{}",
        target.target_id
    )));
    realm.context_type = Some("worker".to_owned());
    Some(realm)
}

fn service_worker_target_runtime_realm(
    target: &crate::conn::ServiceWorkerTargetState,
) -> Option<RuntimeExecutionContextEvent> {
    let context_id = target.real_runtime_execution_context_id()?;
    let origin = url::Url::parse(&target.script_url)
        .ok()
        .map(|url| moli_url::origin_ascii_serialization(&url))
        .unwrap_or_else(|| "null".to_owned());
    Some(RuntimeExecutionContextEvent {
        target_id: Some(DevToolsTargetId::from(target.target_id.as_str())),
        context_id: Some(context_id),
        realm_id: Some(DevToolsRealmId::from(format!(
            "service-worker-{}",
            target.target_id
        ))),
        frame_id: None,
        origin: Some(origin),
        name: Some(String::new()),
        is_default: Some(true),
        context_type: Some("service-worker".to_owned()),
        grant_universal_access: None,
    })
}

fn devtools_realm_type(context_type: Option<&str>) -> &'static str {
    match context_type {
        Some("dedicated-worker") => "dedicated-worker",
        Some("shared-worker") => "shared-worker",
        Some("service-worker") => "service-worker",
        Some("paint-worklet") => "paint-worklet",
        Some("audio-worklet") => "audio-worklet",
        Some("worklet") => "worklet",
        Some("worker") => "worker",
        _ => "window",
    }
}

fn devtools_remote_value_from_cdp(
    remote: &Value,
    retain_handle: bool,
    realm: Option<DevToolsRealmId>,
) -> DevToolsRemoteValue {
    let object_id = remote.get("objectId").and_then(Value::as_str);
    DevToolsRemoteValue {
        value: cdp_remote_object_value(remote),
        handle: retain_handle
            .then(|| object_id.map(DevToolsRemoteHandleId::from))
            .flatten(),
        shared_id: object_id.map(DevToolsRemoteHandleId::from),
        node_id: None,
        backend_node_id: None,
        window_context: None,
        realm,
        remote_type: remote
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        remote_subtype: remote
            .get("subtype")
            .and_then(Value::as_str)
            .map(str::to_owned),
        unserializable_value: remote
            .get("unserializableValue")
            .and_then(Value::as_str)
            .map(str::to_owned),
        description: remote
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
        class_name: remote
            .get("className")
            .and_then(Value::as_str)
            .map(str::to_owned),
        deep_serialized_value: remote.get("deepSerializedValue").cloned(),
        node_value: None,
    }
}

fn devtools_script_exception_from_cdp(
    exception_details: &Value,
    retain_handle: bool,
) -> DevToolsScriptException {
    let exception = exception_details.get("exception");
    let text = exception_details
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| {
            exception.and_then(|exception| exception.get("description").and_then(Value::as_str))
        })
        .unwrap_or("JavaScript exception")
        .to_owned();
    DevToolsScriptException {
        exception_id: exception_details.get("exceptionId").and_then(Value::as_u64),
        script_id: exception_details
            .get("scriptId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        text,
        value: exception.map(|remote| devtools_remote_value_from_cdp(remote, retain_handle, None)),
        realm: exception_details
            .get("executionContextUniqueId")
            .and_then(Value::as_str)
            .map(DevToolsRealmId::from)
            .or_else(|| {
                exception_details
                    .get("executionContextId")
                    .and_then(Value::as_i64)
                    .map(|id| DevToolsRealmId::from(id.to_string()))
            }),
        line_number: exception_details.get("lineNumber").and_then(Value::as_u64),
        column_number: exception_details
            .get("columnNumber")
            .and_then(Value::as_u64),
        stack_trace: exception_details
            .get("stackTrace")
            .and_then(crate::devtools_runtime::DevToolsStackTrace::from_cdp_value),
    }
}

fn cdp_remote_object_value(remote: &Value) -> Value {
    if let Some(value) = remote.get("value") {
        return value.clone();
    }
    if remote
        .get("subtype")
        .and_then(Value::as_str)
        .is_some_and(|subtype| subtype == "null")
    {
        return Value::Null;
    }
    if let Some(unserializable) = remote.get("unserializableValue").and_then(Value::as_str) {
        return Value::String(unserializable.to_owned());
    }
    match remote.get("type").and_then(Value::as_str) {
        Some("undefined") => Value::Null,
        Some("boolean") => Value::Bool(false),
        Some("number") => json!(0),
        Some("string") => Value::String(String::new()),
        Some("object") | Some("function") => json!({}),
        _ => Value::Null,
    }
}

fn start_pending_runtime_binding_context_lookup_phase(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    session_id: Option<&str>,
    task: RuntimeBindingCommandTask,
    execution_context_id: i64,
    session_owner_route: Option<CdpSessionRoute>,
) -> Option<RuntimeCommandTaskStep> {
    let pending = conn
        .start_child_default_execution_context_lookup_for_session_owner(
            session_id,
            execution_context_id,
        )
        .ok()?;
    Some(RuntimeCommandTaskStep::Pending(Box::new(
        PendingRuntimeCommandDispatch {
            command_id,
            action: task.action.label(),
            owner_scope: CommandOwnerScope::from_session_and_owner_route(
                session_id,
                session_owner_route,
            ),
            object_group: None,
            release_object_ids: Vec::new(),
            release_object_group: None,
            await_promise: false,
            wait_for_deferred_reply: false,
            pending: PendingRuntimeCommandKind::BindingContextLookup { task, pending },
        },
    )))
}

fn start_pending_runtime_binding_inspector_phase(
    conn: &mut CdpConnection,
    completed: &RuntimeCommandCompletionMeta,
    task: RuntimeBindingCommandTask,
) -> RuntimeCommandTaskStep {
    let Some(inspector_json) = task.inspector_json.clone() else {
        return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
            completed.command_id,
            "InvalidParams".to_owned(),
        ));
    };
    let result = match task.action {
        RuntimeBindingCommand::Add => {
            if let Some(command_id) = completed.command_id {
                let descriptor = RendererCommandDescriptor::from_frontend_policy(
                    inspector_json,
                    task.renderer_policy,
                    RendererInspectorResponseDelivery::CommandReply,
                );
                conn.start_runtime_protocol_message_with_context_resolution_for_session_owner_with_deferred_response(
                    completed.session_id(),
                    "addBinding",
                    descriptor,
                    command_id,
                )
            } else {
                conn.start_runtime_protocol_message_with_context_resolution_for_session_owner(
                    completed.session_id(),
                    "addBinding",
                    inspector_json,
                )
            }
        }
        RuntimeBindingCommand::Remove => {
            if let Some(command_id) = completed.command_id {
                let descriptor = RendererCommandDescriptor::from_frontend_policy(
                    inspector_json,
                    task.renderer_policy,
                    RendererInspectorResponseDelivery::CommandReply,
                );
                conn.start_runtime_protocol_message_for_session_owner_with_deferred_response(
                    completed.session_id(),
                    descriptor,
                    command_id,
                )
            } else {
                conn.start_runtime_protocol_message_for_session_owner(
                    completed.session_id(),
                    inspector_json,
                )
            }
        }
    };
    match result {
        Ok(pending) => RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
            command_id: completed.command_id,
            action: task.action.label(),
            owner_scope: completed.owner_scope.clone(),
            object_group: None,
            release_object_ids: Vec::new(),
            release_object_group: None,
            await_promise: false,
            wait_for_deferred_reply: false,
            pending: PendingRuntimeCommandKind::BindingInspector { task, pending },
        })),
        Err(message) if message == "NoDocumentLoaded" => {
            let mut task = task;
            task.command_response = Some(RuntimeBindingCommandResponse::empty_success());
            match start_pending_runtime_binding_page_phase(
                conn,
                completed.command_id,
                completed.session_id(),
                task.clone(),
                completed.owner_scope.clone(),
            ) {
                Some(pending) => RuntimeCommandTaskStep::Pending(Box::new(pending)),
                None => complete_runtime_binding_after_live_update(conn, completed.clone(), task),
            }
        }
        Err(message) => RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
            completed.command_id,
            message,
        )),
    }
}

fn start_pending_runtime_binding_page_phase(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    session_id: Option<&str>,
    task: RuntimeBindingCommandTask,
    owner_scope: CommandOwnerScope,
) -> Option<PendingRuntimeCommandDispatch> {
    let pending = match task.phase {
        RuntimeBindingPhase::LivePageUpdate => match task.action {
            RuntimeBindingCommand::Add => conn.start_install_runtime_binding_for_session_owner(
                session_id,
                &task.name,
                task.execution_context_name.as_deref(),
                task.execution_context_id,
            ),
            RuntimeBindingCommand::Remove => {
                conn.start_remove_runtime_binding_for_session_owner(session_id, &task.name)
            }
        },
        RuntimeBindingPhase::StoredBindingsApply => {
            conn.start_apply_stored_runtime_bindings_for_session_owner(session_id)
        }
    }
    .ok()?;
    Some(PendingRuntimeCommandDispatch {
        command_id,
        action: task.action.label(),
        owner_scope,
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::BindingPage { task, pending },
    })
}

fn prepare_runtime_inspector_payload(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    preparation: RuntimeInspectorPayloadPreparation,
) -> Result<String, String> {
    match preparation {
        RuntimeInspectorPayloadPreparation::Passthrough => Ok(cmd.json.to_owned()),
        RuntimeInspectorPayloadPreparation::ValidateObjectOwner => {
            validate_pending_runtime_object_ownership(conn, cmd)?;
            Ok(cmd.json.to_owned())
        }
        RuntimeInspectorPayloadPreparation::ValidatePrototypeOwner => {
            validate_pending_runtime_prototype_object_ownership(conn, cmd)?;
            Ok(cmd.json.to_owned())
        }
        RuntimeInspectorPayloadPreparation::PrepareCallFunctionOn => {
            prepare_pending_runtime_call_function_on_json(conn, cmd)
        }
    }
}

fn build_cdp_evaluate_script_command(
    cmd: &Cmd<'_>,
    target_id: Option<&str>,
    browser_context_id: Option<&str>,
    await_promise: bool,
) -> DevToolsEvaluateScriptCommand {
    DevToolsEvaluateScriptCommand {
        context: cmd.devtools_command_context(target_id, browser_context_id),
        realm_id: None,
        world_name: None,
        expression: cdp_evaluate_expression_from_params(cmd.params).unwrap_or_default(),
        await_promise,
        user_gesture: cdp_runtime_user_gesture_from_params(cmd.params),
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: cdp_evaluate_result_ownership_from_params(cmd.params),
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    }
}

fn cdp_evaluate_expression_from_params(params: Option<&Map<String, Value>>) -> Option<String> {
    params?
        .get("expression")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn cdp_evaluate_result_ownership_from_params(
    params: Option<&Map<String, Value>>,
) -> DevToolsResultOwnership {
    if params
        .and_then(|params| params.get("returnByValue"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        DevToolsResultOwnership::ByValue
    } else {
        DevToolsResultOwnership::Root
    }
}

fn cdp_runtime_user_gesture_from_params(params: Option<&Map<String, Value>>) -> bool {
    params
        .and_then(|params| params.get("userGesture"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn devtools_runtime_owner_identity_for_session(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> (Option<String>, Option<String>) {
    if let Some(
        CdpSessionRoute::SharedWorkerTarget {
            browser_context_id,
            target_id,
        }
        | CdpSessionRoute::DedicatedWorkerTarget {
            browser_context_id,
            target_id,
        }
        | CdpSessionRoute::ServiceWorkerTarget {
            browser_context_id,
            target_id,
        },
    ) = conn.session_route(session_id)
    {
        return (Some(browser_context_id), Some(target_id));
    }
    conn.target_owner_identity_for_session(session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None))
}

fn prepare_pending_runtime_call_function_on_json(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<String, String> {
    let (browser_context_id, target_id) =
        devtools_runtime_owner_identity_for_session(conn, cmd.session_id);
    let command =
        build_cdp_call_function_command(cmd, target_id.as_deref(), browser_context_id.as_deref());
    prepare_pending_devtools_call_function_json(conn, cmd, &command)
}

fn build_cdp_call_function_command(
    cmd: &Cmd<'_>,
    target_id: Option<&str>,
    browser_context_id: Option<&str>,
) -> DevToolsCallFunctionCommand {
    DevToolsCallFunctionCommand {
        context: cmd.devtools_command_context(target_id, browser_context_id),
        realm_id: cdp_call_function_realm_id_from_params(cmd.params),
        world_name: None,
        object_id: runtime_object_id_from_params(cmd.params).map(DevToolsRemoteHandleId::from),
        this_parameter: None,
        function_declaration: cdp_call_function_declaration_from_params(cmd.params)
            .unwrap_or_default(),
        arguments: cdp_call_function_arguments_from_params(cmd.params),
        await_promise: cmd
            .params
            .and_then(|params| params.get("awaitPromise"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        user_gesture: cdp_runtime_user_gesture_from_params(cmd.params),
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: cdp_evaluate_result_ownership_from_params(cmd.params),
        object_group: runtime_object_group_from_params(cmd.params).map(str::to_owned),
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    }
}

fn cdp_call_function_realm_id_from_params(
    params: Option<&Map<String, Value>>,
) -> Option<DevToolsRealmId> {
    params?
        .get("executionContextId")
        .and_then(Value::as_i64)
        .map(|id| DevToolsRealmId::from(id.to_string()))
}

fn cdp_call_function_declaration_from_params(
    params: Option<&Map<String, Value>>,
) -> Option<String> {
    params?
        .get("functionDeclaration")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn cdp_call_function_arguments_from_params(params: Option<&Map<String, Value>>) -> Vec<Value> {
    params
        .and_then(|params| params.get("arguments"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn prepare_pending_devtools_call_function_json(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    command: &DevToolsCallFunctionCommand,
) -> Result<String, String> {
    let object_ids = devtools_call_function_remote_object_ids(command);
    conn.validate_runtime_remote_object_ids_for_session_owner(cmd.session_id, &object_ids)?;
    Ok(cmd.json.to_owned())
}

fn validate_pending_runtime_object_ownership(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<(), String> {
    let object_ids = cmd
        .params
        .map(runtime_remote_object_ids_in_map)
        .unwrap_or_default();
    conn.validate_runtime_remote_object_ids_for_session_owner(cmd.session_id, &object_ids)
}

fn validate_pending_runtime_prototype_object_ownership(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<(), String> {
    let object_ids = runtime_prototype_object_id_from_params(cmd.params)
        .map(|object_id| vec![object_id.to_owned()])
        .unwrap_or_default();
    conn.validate_runtime_remote_object_ids_for_session_owner(cmd.session_id, &object_ids)
}

pub(crate) async fn complete_pending_runtime_command(
    conn: &mut CdpConnection,
    completed: CompletedRuntimeCommandDispatch,
) -> RuntimeCommandTaskStep {
    let response_flush = crate::conn::CommandResponseFlushContext::default();
    complete_pending_runtime_command_at_response_boundary(conn, completed, &response_flush).await
}

pub(crate) async fn complete_pending_runtime_command_at_response_boundary(
    conn: &mut CdpConnection,
    completed: CompletedRuntimeCommandDispatch,
    response_flush: &crate::conn::CommandResponseFlushContext,
) -> RuntimeCommandTaskStep {
    let owner_scope = completed.owner_scope.clone();
    let mut route_scope = owner_scope.enter(conn);
    Box::pin(complete_pending_runtime_command_inner(
        route_scope.conn_mut(),
        completed,
        response_flush,
    ))
    .await
}

async fn complete_pending_runtime_command_inner(
    conn: &mut CdpConnection,
    completed: CompletedRuntimeCommandDispatch,
    response_flush: &crate::conn::CommandResponseFlushContext,
) -> RuntimeCommandTaskStep {
    let timing_started = moli_trace::cdp_nav_timing_enabled().then(std::time::Instant::now);
    let meta = RuntimeCommandCompletionMeta::from(&completed);
    match completed.completed {
        CompletedRuntimeCommandKind::MoliDiagnostics(completed_diagnostics) => {
            RuntimeCommandTaskStep::Complete(match completed_diagnostics {
                Ok(completed_diagnostics) => {
                    CommandOutputPlan::result(conn.complete_moli_diagnostics(completed_diagnostics))
                }
                Err(message) => CommandOutputPlan::error(-32000, message),
            })
        }
        CompletedRuntimeCommandKind::Enable(completed_enable) => RuntimeCommandTaskStep::Complete(
            complete_pending_runtime_enable_command(conn, meta, completed_enable),
        ),
        CompletedRuntimeCommandKind::BindingInspector {
            task,
            completed: completed_inspector,
        } => {
            Box::pin(complete_pending_runtime_binding_inspector_command(
                conn,
                meta,
                task,
                completed_inspector,
                response_flush,
            ))
            .await
        }
        CompletedRuntimeCommandKind::BindingContextLookup {
            task,
            completed: completed_lookup,
        } => complete_pending_runtime_binding_context_lookup_command(
            conn,
            meta,
            task,
            completed_lookup,
        ),
        CompletedRuntimeCommandKind::BindingPage {
            task,
            completed: completed_page,
        } => complete_pending_runtime_binding_page_command(conn, meta, task, completed_page),
        CompletedRuntimeCommandKind::Inspector {
            completed: completed_inspector,
        } => {
            Box::pin(complete_pending_runtime_inspector_command(
                conn,
                meta,
                completed_inspector,
                timing_started,
                response_flush,
            ))
            .await
        }
        CompletedRuntimeCommandKind::InspectorDeferredReplyReady { routed_output, .. } => {
            RuntimeCommandTaskStep::Complete(
                complete_pending_runtime_deferred_inspector_reply_command(meta, routed_output),
            )
        }
        CompletedRuntimeCommandKind::SharedWorkerInspector {
            completed: completed_inspector,
            binding_effect,
        } => {
            Box::pin(complete_pending_shared_worker_runtime_inspector_command(
                conn,
                meta,
                completed_inspector,
                binding_effect,
                timing_started,
            ))
            .await
        }
        CompletedRuntimeCommandKind::ServiceWorkerInspector {
            completed: completed_inspector,
        } => {
            Box::pin(complete_pending_service_worker_runtime_inspector_command(
                conn,
                meta,
                completed_inspector,
                timing_started,
            ))
            .await
        }
    }
}

async fn route_registered_runtime_response_receiver_into(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    session_id: Option<&str>,
    response_rx: RuntimeInspectorAsyncCompletionReceiver,
    page_owner_access_allowed: bool,
    routed_output: &mut RuntimeInspectorRoutedOutput,
) -> bool {
    let Some(command_id) = command_id else {
        return false;
    };
    let response = RuntimeInspectorResponseReady::new(
        command_id,
        session_id,
        response_rx
            .await
            .map_err(|_| "RuntimeInspectorResponseCanceled".to_owned()),
    );
    let Some(response) = conn.resolve_runtime_inspector_response_ready(response) else {
        return false;
    };
    let (_, output, renderer_output_predecessor) = response.into_renderer_command_output();
    if let Some(predecessor) = renderer_output_predecessor {
        routed_output.set_renderer_output_predecessor(predecessor);
    }
    route_runtime_command_output_into_routed_output(
        conn,
        output,
        Some(command_id),
        session_id,
        page_owner_access_allowed,
        routed_output,
    )
    .await
}

async fn route_runtime_command_output_into_routed_output(
    conn: &mut CdpConnection,
    output: RendererRuntimeCommandOutput,
    command_id: Option<u64>,
    session_id: Option<&str>,
    page_owner_access_allowed: bool,
    routed_output: &mut RuntimeInspectorRoutedOutput,
) -> bool {
    let mut ordered_events = Vec::new();
    let saw_current_response = conn
        .route_renderer_runtime_command_output_with_page_owner_access_into(
            output,
            command_id,
            session_id,
            page_owner_access_allowed,
            &mut ordered_events,
        )
        .await;
    routed_output.append_ordered_events(ordered_events);
    saw_current_response
}

async fn route_renderer_command_turn_output_into_routed_output(
    conn: &mut CdpConnection,
    output: RendererCommandTurnOutput,
    command_id: Option<u64>,
    session_id: Option<&str>,
    response_flush: &crate::conn::CommandResponseFlushContext,
    routed_output: &mut RuntimeInspectorRoutedOutput,
) -> bool {
    let mut ordered_events = Vec::new();
    let mut post_response_events = Vec::new();
    let (saw_current_response, renderer_output_predecessor) = conn
        .route_renderer_command_turn_output_into(
            output,
            command_id,
            session_id,
            response_flush,
            &mut ordered_events,
            &mut post_response_events,
        )
        .await;
    if let Some(predecessor) = renderer_output_predecessor {
        routed_output.set_renderer_output_predecessor(predecessor);
    }
    routed_output.append_ordered_events(ordered_events);
    routed_output.append_post_response_events(post_response_events);
    saw_current_response
}

fn route_inspector_messages_into_routed_output(
    conn: &mut CdpConnection,
    messages: Vec<RendererRuntimeInspectorMessage>,
    command_id: Option<u64>,
    session_id: Option<&str>,
    routed_output: &mut RuntimeInspectorRoutedOutput,
) -> bool {
    let mut saw_current_response = false;
    for message in messages {
        let mut response_events = Vec::new();
        let mut background_events = Vec::new();
        saw_current_response |= conn
            .route_renderer_runtime_inspector_messages_with_background_events_into(
                vec![message],
                command_id,
                session_id,
                &mut response_events,
                &mut background_events,
            );
        routed_output.append_ordered_events(response_events);
        routed_output.append_ordered_events(background_events);
    }
    saw_current_response
}

async fn complete_pending_runtime_inspector_command(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    completed_inspector: Result<CompletedRuntimeProtocolMessageDispatch, String>,
    timing_started: Option<std::time::Instant>,
    response_flush: &crate::conn::CommandResponseFlushContext,
) -> RuntimeCommandTaskStep {
    let (
        messages,
        mut renderer_response_rx,
        page_owner_access_allowed,
        response_delivery,
        session_response_predecessor,
        session_response_succeeded,
    ) = match completed_inspector {
        Ok(mut completed_protocol) => {
            let page_owner_access_allowed = completed_protocol.page_owner_access_allowed();
            let session_response_succeeded = completed_protocol.session_response_succeeded();
            let session_response_predecessor = completed_protocol.session_response_predecessor();
            let response_delivery = completed_protocol.response_delivery();
            let renderer_response_rx = completed_protocol.take_deferred_response_receiver();
            match conn
                .complete_runtime_protocol_message_for_session_owner_async(completed_protocol)
                .await
            {
                Ok(messages) => (
                    messages,
                    renderer_response_rx,
                    page_owner_access_allowed,
                    response_delivery,
                    session_response_predecessor,
                    session_response_succeeded,
                ),
                Err(message) => {
                    if let Some(command_id) = completed.command_id {
                        conn.forget_pending_inspector_await(command_id, completed.session_id());
                    }
                    if session_response_succeeded.is_some() {
                        tracing::debug!(
                            command_id = completed.command_id,
                            session_id = completed.session_id(),
                            error = %message,
                            "ignored browser-side completion error after the renderer session settled the terminal response"
                        );
                        (
                            None,
                            renderer_response_rx,
                            page_owner_access_allowed,
                            response_delivery,
                            session_response_predecessor,
                            session_response_succeeded,
                        )
                    } else {
                        if let Some(command_id) = completed.command_id {
                            let correlation = conn
                                .take_renderer_call_for_frontend_for_session_owner(
                                    completed.session_id(),
                                    command_id,
                                );
                            if correlation.is_none() {
                                tracing::debug!(
                                    command_id,
                                    session_id = completed.session_id(),
                                    error = %message,
                                    "ignored renderer completion error after another route settled the frontend call"
                                );
                                return RuntimeCommandTaskStep::Complete(
                                    CommandOutputPlan::default(),
                                );
                            }
                        }
                        return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                            completed.command_id,
                            message,
                        ));
                    }
                }
            }
        }
        Err(message) => {
            if let Some(command_id) = completed.command_id {
                conn.forget_pending_inspector_await(command_id, completed.session_id());
                let correlation = conn.take_renderer_call_for_frontend_for_session_owner(
                    completed.session_id(),
                    command_id,
                );
                if correlation.is_none() {
                    tracing::debug!(
                        command_id,
                        session_id = completed.session_id(),
                        error = %message,
                        "ignored canceled renderer route after another route settled the frontend call"
                    );
                    return RuntimeCommandTaskStep::Complete(CommandOutputPlan::default());
                }
            }
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                completed.command_id,
                message,
            ));
        }
    };
    let initial_message_count = messages.as_ref().map_or(0, |messages| {
        messages
            .runtime_inspector_output()
            .map_or(0, |messages| messages.len())
    });
    if let Some(started) = timing_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            action = completed.action,
            stage = "runtime_inspector_dispatch_returned",
            output_items = initial_message_count,
            elapsed_ms = started.elapsed().as_millis(),
        );
    }

    let mut plan = CommandOutputPlan::default();
    let mut routed_output = RuntimeInspectorRoutedOutput::default();
    if let Some(predecessor) = session_response_predecessor {
        routed_output.set_renderer_output_predecessor(predecessor);
    }
    let mut saw_current_response = if let Some(messages) = messages {
        route_renderer_command_turn_output_into_routed_output(
            conn,
            messages,
            completed.command_id,
            completed.session_id(),
            response_flush,
            &mut routed_output,
        )
        .await
    } else {
        false
    };
    if saw_current_response {
        renderer_response_rx.take();
    }
    // Target teardown can settle the frontend command from a different
    // command turn before this renderer completion reaches the scheduler. In
    // that case the renderer-call correlation has already been consumed and
    // the deferred response sender can remain alive in the retired Page
    // command. Waiting on its receiver here would stall the entire CDP owner
    // after, for example, Page.crash interrupted an active Runtime.evaluate.
    // Treat the missing correlation as the command's already-settled
    // tombstone; any non-response Inspector output in this late completion is
    // still projected below.
    if !saw_current_response
        && renderer_response_rx.is_some()
        && completed.command_id.is_some_and(|command_id| {
            conn.renderer_runtime_command_cause_for_frontend(completed.session_id(), command_id)
                .is_none()
        })
    {
        renderer_response_rx.take();
        saw_current_response = true;
    }
    if response_delivery == RendererInspectorResponseDelivery::DevToolsSession {
        // The attachment-scoped renderer session stream owns the terminal
        // response. The typed Page completion remains internal state only and
        // must not join or await the legacy per-command receiver.
        renderer_response_rx.take();
    }
    if response_delivery == RendererInspectorResponseDelivery::CommandReply
        && !completed.wait_for_deferred_reply
        && let Some(renderer_response_rx) = renderer_response_rx.take()
    {
        let saw_deferred_response = route_registered_runtime_response_receiver_into(
            conn,
            completed.command_id,
            completed.session_id(),
            renderer_response_rx,
            page_owner_access_allowed,
            &mut routed_output,
        )
        .await;
        saw_current_response |= saw_deferred_response;
    }
    if completed.await_promise {
        conn.trace_runtime_await_initial_dispatch_done(
            completed.command_id,
            completed.session_id(),
            initial_message_count,
            saw_current_response,
        );
    }
    if completed.wait_for_deferred_reply
        && (renderer_response_rx.is_some() || !saw_current_response)
        && completed.command_id.is_some()
    {
        return pending_runtime_deferred_inspector_reply_command(
            conn,
            completed,
            routed_output,
            renderer_response_rx,
            page_owner_access_allowed,
        );
    }
    let succeeded = session_response_succeeded
        .unwrap_or_else(|| routed_output.command_response_succeeded(completed.command_id));
    if succeeded {
        routed_output.register_object_group_for_success(
            conn,
            completed.session_id(),
            completed.object_group.as_deref(),
        );
    }
    if succeeded {
        if let Some(console_action) = console_action_from_protocol_method(completed.action)
            && !apply_console_output_state_for_session(conn, completed.session_id(), console_action)
        {
            let message = format!("ConsoleCommandCompletionFailed: {}", completed.action);
            if session_response_succeeded.is_some() {
                tracing::warn!(
                    command_id = completed.command_id,
                    session_id = completed.session_id(),
                    error = %message,
                    "could not apply browser projection after the terminal renderer response was published"
                );
            } else {
                return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                    completed.command_id,
                    message,
                ));
            }
        }
        if completed.action == "discardConsoleEntries" {
            advance_runtime_observable_cursors_to_current_for_session_owner(
                conn,
                completed.session_id(),
            );
        }
        if completed.action == "disable" {
            if let Err(message) =
                apply_runtime_disable_projection_after_success(conn, completed.session_id())
            {
                if session_response_succeeded.is_some() {
                    tracing::warn!(
                        command_id = completed.command_id,
                        session_id = completed.session_id(),
                        error = %message,
                        "could not apply browser projection after the terminal renderer response was published"
                    );
                } else {
                    return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                        completed.command_id,
                        message,
                    ));
                }
            }
            if let Err(message) = conn
                .apply_runtime_binding_state_for_session_owner_async(completed.session_id())
                .await
            {
                if session_response_succeeded.is_some() {
                    tracing::warn!(
                        command_id = completed.command_id,
                        session_id = completed.session_id(),
                        error = %message,
                        "could not apply browser projection after the terminal renderer response was published"
                    );
                } else {
                    return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                        completed.command_id,
                        message,
                    ));
                }
            }
        }
        if !completed.release_object_ids.is_empty() {
            conn.unregister_runtime_remote_object_ids_for_session_owner(
                completed.session_id(),
                &completed.release_object_ids,
            );
        }
        if let Some(object_group) = completed.release_object_group.as_deref() {
            conn.unregister_runtime_remote_object_group_for_session_owner(
                completed.session_id(),
                object_group,
            );
        }
        if completed.action == "runIfWaitingForDebugger" {
            crate::domains::target::schedule_initial_document_target_url_navigation_after_debugger_resume(
                conn,
                completed.session_id(),
            );
        }
    }
    routed_output.push_ordered_into_plan(&mut plan, completed.command_id);
    if let Some(started) = timing_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            action = completed.action,
            stage = "runtime_inspector_plan_ready",
            elapsed_ms = started.elapsed().as_millis(),
        );
    }
    RuntimeCommandTaskStep::Complete(plan)
}

fn pending_runtime_deferred_inspector_reply_command(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    routed_output: RuntimeInspectorRoutedOutput,
    renderer_response_rx: Option<RuntimeInspectorAsyncCompletionReceiver>,
    page_owner_access_allowed: bool,
) -> RuntimeCommandTaskStep {
    let claimed_await = completed.command_id.and_then(|command_id| {
        conn.claim_pending_inspector_await_for_scheduler_deferred_reply(
            command_id,
            completed.session_id(),
        )
    });
    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: completed.command_id,
        action: completed.action,
        owner_scope: completed.owner_scope,
        object_group: completed.object_group,
        release_object_ids: completed.release_object_ids,
        release_object_group: completed.release_object_group,
        await_promise: completed.await_promise,
        wait_for_deferred_reply: completed.wait_for_deferred_reply,
        pending: PendingRuntimeCommandKind::InspectorDeferredReply {
            routed_output,
            renderer_response_rx,
            claimed_await,
            page_owner_access_allowed,
        },
    }))
}

fn complete_pending_runtime_deferred_inspector_reply_command(
    completed: RuntimeCommandCompletionMeta,
    routed_output: RuntimeInspectorRoutedOutput,
) -> CommandOutputPlan {
    let mut plan = CommandOutputPlan::default();
    routed_output.push_ordered_into_plan(&mut plan, completed.command_id);
    plan
}

fn push_runtime_protocol_event_or_background_event(
    plan: &mut CommandOutputPlan,
    command_id: Option<u64>,
    event: BackgroundProtocolEvent,
) {
    let Some(command_id) = command_id else {
        plan.push_background_event(event);
        return;
    };
    let is_command_response = event.protocol_message_id() == Some(command_id);
    if !is_command_response {
        plan.push_background_event(event);
        return;
    }
    let event = match event.into_command_response_payload() {
        Ok((_, _, payload)) => {
            push_command_response_payload_into_plan(plan, payload);
            return;
        }
        Err(event) => event,
    };
    let Some(message) = event.protocol_message().cloned() else {
        plan.push_background_event(event);
        return;
    };
    if !plan.push_runtime_inspector_protocol_response(message, Some(command_id)) {
        plan.push_background_event(event);
    }
}

fn push_command_response_payload_into_plan(
    plan: &mut CommandOutputPlan,
    payload: BackgroundCommandResponsePayload,
) {
    match payload {
        BackgroundCommandResponsePayload::Success { result } => plan.push_result(result),
        BackgroundCommandResponsePayload::Error {
            code,
            message,
            data,
        } => plan.push_error_with_data(code, message, data),
    }
}

fn complete_pending_runtime_enable_command(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    completed_enable: Result<CompletedRuntimeEnableEventsDispatch, String>,
) -> CommandOutputPlan {
    let replay = match completed_enable {
        Ok(completed_enable) => {
            match conn.complete_runtime_enable_events_for_session_owner(completed_enable) {
                Ok(replay) => replay,
                Err(message) => return CommandOutputPlan::error(-32000, message),
            }
        }
        Err(message) => return CommandOutputPlan::error(-32000, message),
    };
    if let Err(message) =
        apply_runtime_enable_projection_after_success(conn, completed.session_id())
    {
        return CommandOutputPlan::error(-32000, message);
    }
    let frame_id = conn.runtime_session_owner_frame_id(completed.session_id());

    let mut plan = CommandOutputPlan::success();
    for event in replay.into_events() {
        match event {
            RuntimeEnableReplayEvent::Context(event) => {
                if !should_emit_child_default_context_inventory_replay_once(
                    conn,
                    completed.session_id(),
                    frame_id.as_deref(),
                    &event,
                ) {
                    continue;
                }
                // Runtime.enable is a command-local replay, not a second live
                // context producer. Record delivery only after the replay
                // cursor accepts this exact event; marking it while merely
                // preparing the replay would suppress the first delivery.
                apply_runtime_context_protocol_event_side_effects_typed(
                    conn,
                    &event,
                    completed.session_id(),
                );
                let mut background_events = Vec::new();
                emit_runtime_context_protocol_background_event_typed(
                    &mut background_events,
                    event,
                    completed.session_id(),
                );
                for event in background_events {
                    plan.push_background_event(event);
                }
            }
            RuntimeEnableReplayEvent::Background(event) => {
                plan.push_background_event(event);
            }
        }
    }
    plan
}

fn apply_runtime_enable_projection_after_success(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> Result<(), String> {
    match conn.set_runtime_frontend_enabled_for_session_owner(session_id, true) {
        SessionOwnerRuntimeFrontendEnableResult::Handled => Ok(()),
        SessionOwnerRuntimeFrontendEnableResult::UnknownSession => {
            Err("Runtime.enable succeeded after session owner disappeared".to_owned())
        }
    }
}

fn complete_pending_runtime_binding_context_lookup_command(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    mut task: RuntimeBindingCommandTask,
    completed_lookup: Result<CompletedRuntimeChildDefaultContextLookupDispatch, String>,
) -> RuntimeCommandTaskStep {
    let is_child_default_context = match completed_lookup {
        Ok(completed_lookup) => match conn
            .complete_child_default_execution_context_lookup_for_session_owner(completed_lookup)
        {
            Ok(is_child_default_context) => is_child_default_context,
            Err(message) => {
                return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                    completed.command_id,
                    message,
                ));
            }
        },
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                completed.command_id,
                message,
            ));
        }
    };

    if !is_child_default_context {
        if matches!(task.action, RuntimeBindingCommand::Add)
            || matches!(task.action, RuntimeBindingCommand::Remove)
                && runtime_remove_binding_should_skip_live_page_update(conn, completed.session_id())
        {
            task.skip_live_page_update_after_inspector_success = true;
        }
        return start_pending_runtime_binding_inspector_phase(conn, &completed, task);
    }

    task.command_response = Some(RuntimeBindingCommandResponse::empty_success());
    match start_pending_runtime_binding_page_phase(
        conn,
        completed.command_id,
        completed.session_id(),
        task.clone(),
        completed.owner_scope.clone(),
    ) {
        Some(pending) => RuntimeCommandTaskStep::Pending(Box::new(pending)),
        None => complete_runtime_binding_after_live_update(conn, completed, task),
    }
}

async fn complete_pending_runtime_binding_inspector_command(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    mut task: RuntimeBindingCommandTask,
    completed_inspector: Result<CompletedRuntimeProtocolMessageDispatch, String>,
    response_flush: &crate::conn::CommandResponseFlushContext,
) -> RuntimeCommandTaskStep {
    let (messages, mut renderer_response_rx) = match completed_inspector {
        Ok(mut completed_protocol) => {
            let renderer_response_rx = completed_protocol.take_deferred_response_receiver();
            match conn
                .complete_runtime_protocol_message_for_session_owner_async(completed_protocol)
                .await
            {
                Ok(messages) => (messages, renderer_response_rx),
                Err(message) => {
                    return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                        completed.command_id,
                        message,
                    ));
                }
            }
        }
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                completed.command_id,
                message,
            ));
        }
    };
    let mut routed_output = RuntimeInspectorRoutedOutput::default();
    let saw_current_response = if let Some(messages) = messages {
        route_renderer_command_turn_output_into_routed_output(
            conn,
            messages,
            completed.command_id,
            completed.session_id(),
            response_flush,
            &mut routed_output,
        )
        .await
    } else {
        false
    };
    if saw_current_response {
        renderer_response_rx.take();
    }
    if let Some(renderer_response_rx) = renderer_response_rx {
        route_registered_runtime_response_receiver_into(
            conn,
            completed.command_id,
            completed.session_id(),
            renderer_response_rx,
            true,
            &mut routed_output,
        )
        .await;
    }
    record_runtime_binding_command_response_from_routed_events(
        &mut task,
        routed_output.events(),
        completed.command_id,
    );
    if routed_output.background_event_count() > 0 {
        tracing::debug!(
            events = routed_output.background_event_count(),
            "dropping unexpected Runtime binding background events during inspector phase"
        );
    }
    if !runtime_binding_command_response_succeeded(&task) {
        return RuntimeCommandTaskStep::Complete(runtime_binding_output_plan(task));
    }
    if task.skip_live_page_update_after_inspector_success {
        return complete_runtime_binding_after_live_update(conn, completed, task);
    }
    match start_pending_runtime_binding_page_phase(
        conn,
        completed.command_id,
        completed.session_id(),
        task.clone(),
        completed.owner_scope.clone(),
    ) {
        Some(pending) => RuntimeCommandTaskStep::Pending(Box::new(pending)),
        None => complete_runtime_binding_after_live_update(conn, completed, task),
    }
}

fn complete_pending_runtime_binding_page_command(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    task: RuntimeBindingCommandTask,
    completed_page: Result<CompletedRuntimeBindingPageCommandDispatch, String>,
) -> RuntimeCommandTaskStep {
    match completed_page {
        Ok(completed_page) => {
            if let Err(message) =
                conn.complete_runtime_binding_page_command_for_session_owner(completed_page)
                && message != "NoDocumentLoaded"
            {
                tracing::warn!(
                    binding = %task.name,
                    error = %message,
                    "Runtime binding page command succeeded before completion failed"
                );
            }
        }
        Err(message) => {
            tracing::warn!(
                binding = %task.name,
                error = %message,
                "Runtime binding page command failed"
            );
        }
    }
    match task.phase {
        RuntimeBindingPhase::LivePageUpdate => {
            complete_runtime_binding_after_live_update(conn, completed, task)
        }
        RuntimeBindingPhase::StoredBindingsApply => {
            RuntimeCommandTaskStep::Complete(runtime_binding_output_plan(task))
        }
    }
}

fn complete_runtime_binding_after_live_update(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    mut task: RuntimeBindingCommandTask,
) -> RuntimeCommandTaskStep {
    if task.should_persist {
        let persistence = match task.action {
            RuntimeBindingCommand::Add => persist_runtime_binding_definition_for_session_owner(
                conn,
                completed.session_id(),
                task.name.clone(),
                task.execution_context_name.clone(),
            ),
            RuntimeBindingCommand::Remove => remove_runtime_binding_definitions_for_session_owner(
                conn,
                completed.session_id(),
                &task.name,
            ),
        };
        if let Err(message) = persistence {
            tracing::warn!(
                binding = %task.name,
                error = %message,
                "Runtime binding command succeeded before owner binding persistence update failed"
            );
            return RuntimeCommandTaskStep::Complete(runtime_binding_output_plan(task));
        }
        task.phase = RuntimeBindingPhase::StoredBindingsApply;
        if let Some(pending) = start_pending_runtime_binding_page_phase(
            conn,
            completed.command_id,
            completed.session_id(),
            task.clone(),
            completed.owner_scope.clone(),
        ) {
            return RuntimeCommandTaskStep::Pending(Box::new(pending));
        }
    }
    RuntimeCommandTaskStep::Complete(runtime_binding_output_plan(task))
}

fn runtime_binding_output_plan(task: RuntimeBindingCommandTask) -> CommandOutputPlan {
    let mut plan = CommandOutputPlan::default();
    match task.command_response {
        Some(response) => response.push_into_plan(&mut plan),
        None => plan.push_error(-32000, "MissingRuntimeBindingCommandResponse"),
    }
    plan
}

fn record_runtime_binding_command_response_from_routed_events(
    task: &mut RuntimeBindingCommandTask,
    events: &[BackgroundProtocolEvent],
    command_id: Option<u64>,
) {
    let Some(command_id) = command_id else {
        return;
    };
    for event in events {
        let Some((event_command_id, _, payload)) = event.command_response_payload_ref() else {
            tracing::debug!(
                ?event,
                "dropping non-command Runtime binding inspector event after routing"
            );
            continue;
        };
        if event_command_id != Some(command_id) {
            tracing::debug!(
                ?event_command_id,
                command_id,
                "dropping non-matching Runtime binding inspector command response after routing"
            );
            continue;
        }
        let response = match payload {
            BackgroundCommandResponsePayloadRef::Success { .. } => {
                RuntimeBindingCommandResponse::Success
            }
            BackgroundCommandResponsePayloadRef::Error { code, message, .. } => {
                RuntimeBindingCommandResponse::Error {
                    code,
                    message: message.to_owned(),
                }
            }
        };
        if task.command_response.is_some() {
            tracing::warn!("overwriting Runtime binding command response");
        }
        task.command_response = Some(response);
    }
}

fn runtime_binding_command_response_succeeded(task: &RuntimeBindingCommandTask) -> bool {
    task.command_response
        .as_ref()
        .is_some_and(RuntimeBindingCommandResponse::succeeded)
}

fn runtime_inspector_error_plan(command_id: Option<u64>, message: String) -> CommandOutputPlan {
    if message == "Duplicate `id` in protocol request" {
        return CommandOutputPlan::error(-32600, message);
    }
    let error = match message.as_str() {
        "NoDocumentLoaded" => DevToolsError::new(DevToolsErrorKind::Internal, "NoDocumentLoaded"),
        "InvalidParams" => DevToolsError::new(DevToolsErrorKind::InvalidArgument, "InvalidParams"),
        _ => {
            if command_id.is_some() {
                DevToolsError::new(DevToolsErrorKind::Internal, message)
            } else {
                DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "Runtime inspector dispatch failed",
                )
            }
        }
    };
    CommandOutputPlan::from_devtools_error(error)
}

fn parse_synthesized_runtime_command(raw_json: String) -> Result<ParsedCdpCommand, String> {
    ParsedCdpCommand::parse_str(raw_json)
        .map_err(|error| format!("invalid synthesized Runtime Inspector command: {error}"))
}

fn worker_runtime_is_unavailable(message: &str) -> bool {
    matches!(
        message,
        "SharedWorkerRuntimeUnavailable" | "DedicatedWorkerRuntimeUnavailable"
    )
}

fn try_start_shared_worker_runtime_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: RuntimeAction,
) -> Option<RuntimeCommandTaskStep> {
    if conn
        .shared_worker_target_for_session(cmd.session_id)
        .is_none()
    {
        return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        )));
    }
    let command = WorkerRuntimeCommand::classify(action);

    if command.kind() == WorkerRuntimeCommandKind::RunIfWaitingForDebugger
        && matches!(
            conn.session_route(cmd.session_id),
            Some(CdpSessionRoute::DedicatedWorkerTarget { .. })
        )
    {
        let plan = match conn
            .run_dedicated_worker_if_waiting_for_debugger_for_session(cmd.session_id)
        {
            Ok(true) => CommandOutputPlan::success(),
            Ok(false) | Err(_) => {
                if let Some(events) =
                    crate::domains::target::release_failed_dedicated_worker_target_after_debugger_resume(
                        conn,
                        cmd.session_id,
                    )
                {
                    let mut plan = CommandOutputPlan::success();
                    plan.extend_background_events(events);
                    plan
                } else {
                    shared_worker_runtime_error_plan(
                        "DedicatedWorkerRuntimeUnavailable".to_owned(),
                    )
                }
            }
        };
        return Some(RuntimeCommandTaskStep::Complete(plan));
    }

    match command.kind() {
        WorkerRuntimeCommandKind::Enable => Some(
            match start_pending_shared_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message)
                    if message == "NoDocumentLoaded" || worker_runtime_is_unavailable(&message) =>
                {
                    RuntimeCommandTaskStep::Complete(
                        shared_worker_runtime_enable_command_output_plan_for_session(
                            conn,
                            cmd.session_id,
                        ),
                    )
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message))
                }
            },
        ),
        WorkerRuntimeCommandKind::Disable if is_bidi_runtime_listener_session(cmd.session_id) => {
            apply_shared_worker_runtime_disable_projection(conn, cmd.session_id);
            Some(RuntimeCommandTaskStep::Complete(
                CommandOutputPlan::success(),
            ))
        }
        WorkerRuntimeCommandKind::Disable => Some(
            match start_pending_shared_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message)
                    if message == "NoDocumentLoaded" || worker_runtime_is_unavailable(&message) =>
                {
                    apply_shared_worker_runtime_disable_projection(conn, cmd.session_id);
                    RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message))
                }
            },
        ),
        WorkerRuntimeCommandKind::DiscardConsoleEntries => Some(
            match start_pending_shared_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message)
                    if message == "NoDocumentLoaded" || worker_runtime_is_unavailable(&message) =>
                {
                    if let Some(session_id) = cmd.session_id
                        && let Some(target) =
                            conn.shared_worker_target_for_session_mut(Some(session_id))
                    {
                        target.discard_runtime_console_entries(session_id);
                    }
                    RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message))
                }
            },
        ),
        WorkerRuntimeCommandKind::RunIfWaitingForDebugger => Some(
            match start_pending_shared_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message)
                    if message == "NoDocumentLoaded" || worker_runtime_is_unavailable(&message) =>
                {
                    RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message))
                }
            },
        ),
        WorkerRuntimeCommandKind::Inspector => Some(
            match start_pending_shared_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message))
                }
            },
        ),
    }
}

fn release_service_worker_if_waiting_for_debugger(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> bool {
    let Some(session_id) = session_id else {
        return false;
    };
    let Some(CdpSessionRoute::ServiceWorkerTarget {
        browser_context_id,
        target_id,
    }) = conn.session_route(Some(session_id))
    else {
        return false;
    };
    let Some(browser_context) = conn.browser_context_by_id(&browser_context_id) else {
        return false;
    };
    let Some(version_id) = browser_context
        .service_worker_target(&target_id)
        .map(|target| target.renderer_version_id)
    else {
        return false;
    };
    browser_context
        .renderer_runtime()
        .run_service_worker_if_waiting_for_debugger_for_devtools(version_id)
}

fn is_bidi_runtime_listener_session(session_id: Option<&str>) -> bool {
    session_id.is_some_and(|session_id| session_id.starts_with("SID-bidi-runtime-listener-"))
}

fn try_start_service_worker_runtime_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: RuntimeAction,
) -> Option<RuntimeCommandTaskStep> {
    if conn
        .service_worker_target_for_session(cmd.session_id)
        .is_none()
    {
        return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32001,
            "Unknown sessionId",
        )));
    }
    let command = WorkerRuntimeCommand::classify(action);

    match command.kind() {
        WorkerRuntimeCommandKind::Enable => Some(
            match start_pending_service_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message)
                    if message == "NoDocumentLoaded"
                        || message == "ServiceWorkerRuntimeUnavailable" =>
                {
                    RuntimeCommandTaskStep::Complete(
                        service_worker_runtime_enable_command_output_plan_for_session(
                            conn,
                            cmd.session_id,
                        ),
                    )
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(message))
                }
            },
        ),
        WorkerRuntimeCommandKind::Disable => Some(
            match start_pending_service_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message)
                    if message == "NoDocumentLoaded"
                        || message == "ServiceWorkerRuntimeUnavailable" =>
                {
                    apply_service_worker_runtime_disable_projection(conn, cmd.session_id);
                    RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(message))
                }
            },
        ),
        WorkerRuntimeCommandKind::DiscardConsoleEntries => Some(
            match start_pending_service_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message)
                    if message == "NoDocumentLoaded"
                        || message == "ServiceWorkerRuntimeUnavailable" =>
                {
                    if let Some(session_id) = cmd.session_id
                        && let Some(target) =
                            conn.service_worker_target_for_session_mut(Some(session_id))
                    {
                        target.discard_runtime_console_entries(session_id);
                    }
                    RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(message))
                }
            },
        ),
        WorkerRuntimeCommandKind::RunIfWaitingForDebugger => {
            release_service_worker_if_waiting_for_debugger(conn, cmd.session_id);
            let dispatch =
                start_pending_service_worker_runtime_inspector_command(conn, cmd, command);
            Some(match dispatch {
                Ok(step) => step,
                Err(message)
                    if message == "NoDocumentLoaded"
                        || message == "ServiceWorkerRuntimeUnavailable" =>
                {
                    RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
                }
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(message))
                }
            })
        }
        WorkerRuntimeCommandKind::Inspector => Some(
            match start_pending_service_worker_runtime_inspector_command(conn, cmd, command) {
                Ok(step) => step,
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(message))
                }
            },
        ),
    }
}

fn start_pending_shared_worker_runtime_inspector_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command: WorkerRuntimeCommand,
) -> Result<RuntimeCommandTaskStep, String> {
    if !can_dispatch(cmd) {
        return Err("UnknownMethod".to_owned());
    }

    let action = command.action();
    let await_promise = runtime_command_awaits_promise(cmd, action);
    let inspector_json =
        prepare_runtime_inspector_payload(conn, cmd, command.payload_preparation())?;
    let object_group = runtime_object_group_for_command_result(conn, cmd, action);
    let release_object_ids = if action == RuntimeAction::ReleaseObject {
        cmd.params
            .map(runtime_remote_object_ids_in_map)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let release_object_group = if action == RuntimeAction::ReleaseObjectGroup {
        runtime_object_group_from_params(cmd.params).map(str::to_owned)
    } else {
        None
    };
    let session_owner_route = conn.session_route(cmd.session_id);
    let pre_registered_await = pre_register_runtime_await_if_needed(
        conn,
        await_promise,
        cmd.id,
        cmd.session_id,
        object_group.as_deref(),
        action.label(),
    )
    .map_err(|error| error.to_string())?;
    let pending = match start_shared_worker_frontend_inspector_dispatch(conn, cmd, inspector_json) {
        Ok(pending) => pending,
        Err(message) => {
            forget_pre_registered_runtime_await(conn, pre_registered_await, cmd.session_id);
            return Err(message);
        }
    };
    let binding_effect = shared_worker_runtime_binding_effect_from_command(cmd, command.binding())?;
    Ok(RuntimeCommandTaskStep::Pending(Box::new(
        PendingRuntimeCommandDispatch {
            command_id: cmd.id,
            action: action.label(),
            owner_scope: CommandOwnerScope::from_session_and_owner_route(
                cmd.session_id,
                session_owner_route,
            ),
            object_group,
            release_object_ids,
            release_object_group,
            await_promise,
            wait_for_deferred_reply: await_promise,
            pending: PendingRuntimeCommandKind::SharedWorkerInspector {
                pending,
                binding_effect,
            },
        },
    )))
}

fn start_pending_service_worker_runtime_inspector_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command: WorkerRuntimeCommand,
) -> Result<RuntimeCommandTaskStep, String> {
    if !can_dispatch(cmd) {
        return Err("UnknownMethod".to_owned());
    }

    let action = command.action();
    let await_promise = runtime_command_awaits_promise(cmd, action);
    let inspector_json =
        prepare_runtime_inspector_payload(conn, cmd, command.payload_preparation())?;
    let object_group = runtime_object_group_for_command_result(conn, cmd, action);
    let release_object_ids = if action == RuntimeAction::ReleaseObject {
        cmd.params
            .map(runtime_remote_object_ids_in_map)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let release_object_group = if action == RuntimeAction::ReleaseObjectGroup {
        runtime_object_group_from_params(cmd.params).map(str::to_owned)
    } else {
        None
    };
    let session_owner_route = conn.session_route(cmd.session_id);
    let pre_registered_await = pre_register_runtime_await_if_needed(
        conn,
        await_promise,
        cmd.id,
        cmd.session_id,
        object_group.as_deref(),
        action.label(),
    )
    .map_err(|error| error.to_string())?;
    let pending = match start_service_worker_frontend_inspector_dispatch(conn, cmd, inspector_json)
    {
        Ok(pending) => pending,
        Err(message) => {
            forget_pre_registered_runtime_await(conn, pre_registered_await, cmd.session_id);
            return Err(message);
        }
    };
    Ok(RuntimeCommandTaskStep::Pending(Box::new(
        PendingRuntimeCommandDispatch {
            command_id: cmd.id,
            action: action.label(),
            owner_scope: CommandOwnerScope::from_session_and_owner_route(
                cmd.session_id,
                session_owner_route,
            ),
            object_group,
            release_object_ids,
            release_object_group,
            await_promise,
            wait_for_deferred_reply: await_promise,
            pending: PendingRuntimeCommandKind::ServiceWorkerInspector { pending },
        },
    )))
}

pub(in crate::domains) async fn replay_shared_worker_runtime_bindings_for_session_async(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) {
    let Some(session_id) = session_id else {
        return;
    };
    let bindings = conn
        .shared_worker_target_for_session(Some(session_id))
        .map(|target| target.runtime_bindings_requiring_replay(session_id))
        .unwrap_or_default();
    for (index, binding) in bindings.into_iter().enumerate() {
        let command_id = SHARED_WORKER_RUNTIME_BINDING_REPLAY_COMMAND_ID_BASE
            .saturating_add(u64::try_from(index).unwrap_or(u64::MAX));
        let raw_json = shared_worker_runtime_binding_replay_json(command_id, &binding);
        match conn
            .dispatch_shared_worker_runtime_helper_protocol_message_for_session_async(
                Some(session_id),
                &raw_json,
                command_id,
            )
            .await
        {
            Ok(messages) if command_response_succeeded(&messages, Some(command_id)) => {
                if let Some(target) = conn.shared_worker_target_for_session_mut(Some(session_id)) {
                    target.mark_runtime_binding_replayed(session_id, &binding);
                }
            }
            Ok(messages) => {
                tracing::warn!(
                    binding = %binding.name,
                    messages = ?messages,
                    "shared worker Runtime binding replay did not receive a successful inspector response"
                );
            }
            Err(error) => {
                tracing::warn!(
                    binding = %binding.name,
                    error = %error,
                    "failed to replay shared worker Runtime binding"
                );
            }
        }
    }
}

fn shared_worker_runtime_binding_replay_json(
    command_id: u64,
    binding: &RuntimeBindingDefinition,
) -> String {
    let mut params = serde_json::Map::new();
    params.insert("name".to_owned(), json!(&binding.name));
    if let Some(execution_context_name) = &binding.execution_context_name {
        params.insert(
            "executionContextName".to_owned(),
            json!(execution_context_name),
        );
    }
    json!({
        "id": command_id,
        "method": "Runtime.addBinding",
        "params": params,
    })
    .to_string()
}

fn shared_worker_runtime_binding_effect_from_command(
    cmd: &Cmd<'_>,
    binding: Option<RuntimeBindingCommand>,
) -> Result<Option<SharedWorkerRuntimeBindingEffect>, String> {
    match binding {
        Some(RuntimeBindingCommand::Add) => {
            let params = cmd
                .get_params::<AddBindingParams>()
                .map_err(|_| "InvalidParams".to_owned())?
                .ok_or_else(|| "InvalidParams".to_owned())?;
            if params.execution_context_id.is_some() {
                return Ok(None);
            }
            Ok(Some(SharedWorkerRuntimeBindingEffect::Add {
                name: params.name,
                execution_context_name: params.execution_context_name,
            }))
        }
        Some(RuntimeBindingCommand::Remove) => {
            let params = cmd
                .get_params::<chromiumoxide_cdp::cdp::js_protocol::runtime::RemoveBindingParams>()
                .map_err(|_| "InvalidParams".to_owned())?
                .ok_or_else(|| "InvalidParams".to_owned())?;
            Ok(Some(SharedWorkerRuntimeBindingEffect::Remove {
                name: params.name,
            }))
        }
        None => Ok(None),
    }
}

fn apply_shared_worker_runtime_binding_effect_after_success(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    effect: SharedWorkerRuntimeBindingEffect,
) -> Result<(), String> {
    let session_id = session_id.ok_or_else(|| "UnknownSession".to_owned())?;
    match effect {
        SharedWorkerRuntimeBindingEffect::Add {
            name,
            execution_context_name,
        } => {
            let target = conn
                .shared_worker_target_for_session_mut(Some(session_id))
                .ok_or_else(|| "UnknownSession".to_owned())?;
            target.upsert_live_runtime_binding_definition(session_id, name, execution_context_name);
            Ok(())
        }
        SharedWorkerRuntimeBindingEffect::Remove { name } => {
            let target = conn
                .shared_worker_target_for_session_mut(Some(session_id))
                .ok_or_else(|| "UnknownSession".to_owned())?;
            target.remove_live_runtime_binding_definitions(session_id, &name);
            Ok(())
        }
    }
}

async fn complete_pending_shared_worker_runtime_inspector_command(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    completed_inspector: Result<CompletedSharedWorkerRuntimeProtocolMessageDispatch, String>,
    binding_effect: Option<SharedWorkerRuntimeBindingEffect>,
    timing_started: Option<std::time::Instant>,
) -> RuntimeCommandTaskStep {
    let (messages, mut renderer_response_rx) = match completed_inspector {
        Ok(mut completed_protocol) => {
            let renderer_response_rx = completed_protocol.take_deferred_response_receiver();
            match conn
                .complete_shared_worker_runtime_protocol_message_for_session(completed_protocol)
            {
                Ok(messages) => (messages, renderer_response_rx),
                Err(message) => {
                    return complete_shared_worker_runtime_inspector_error(
                        conn, completed, message,
                    );
                }
            }
        }
        Err(message) => {
            return complete_shared_worker_runtime_inspector_error(conn, completed, message);
        }
    };

    if let Some(started) = timing_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            action = completed.action,
            stage = "shared_worker_runtime_inspector_dispatch_returned",
            messages = messages.len(),
            elapsed_ms = started.elapsed().as_millis(),
        );
    }

    let mut plan = CommandOutputPlan::default();
    let mut routed_output = RuntimeInspectorRoutedOutput::default();
    let mut saw_current_response = route_inspector_messages_into_routed_output(
        conn,
        messages,
        completed.command_id,
        completed.session_id(),
        &mut routed_output,
    );
    if !completed.wait_for_deferred_reply
        && let Some(renderer_response_rx) = renderer_response_rx.take()
    {
        saw_current_response |= route_registered_runtime_response_receiver_into(
            conn,
            completed.command_id,
            completed.session_id(),
            renderer_response_rx,
            true,
            &mut routed_output,
        )
        .await;
    }
    if completed.wait_for_deferred_reply
        && (renderer_response_rx.is_some() || !saw_current_response)
        && completed.command_id.is_some()
    {
        return pending_runtime_deferred_inspector_reply_command(
            conn,
            completed,
            routed_output,
            renderer_response_rx,
            true,
        );
    }
    let succeeded = routed_output.command_response_succeeded(completed.command_id);
    apply_shared_worker_runtime_completion_projection(
        conn,
        completed.session_id(),
        completed.action,
        succeeded,
    );
    if succeeded {
        if let Some(console_action) = console_action_from_protocol_method(completed.action)
            && !apply_console_output_state_for_session(conn, completed.session_id(), console_action)
        {
            return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(format!(
                "ConsoleCommandCompletionFailed: {}",
                completed.action
            )));
        }
        if let Some(effect) = binding_effect {
            match apply_shared_worker_runtime_binding_effect_after_success(
                conn,
                completed.session_id(),
                effect,
            ) {
                Ok(()) => {}
                Err(message) => {
                    tracing::warn!(
                        action = completed.action,
                        error = %message,
                        "shared worker Runtime binding command succeeded before persistence update failed"
                    );
                }
            }
        }
        replay_shared_worker_runtime_bindings_for_session_async(conn, completed.session_id()).await;
    }
    if succeeded {
        routed_output.register_object_group_for_success(
            conn,
            completed.session_id(),
            completed.object_group.as_deref(),
        );
    }
    if succeeded {
        if !completed.release_object_ids.is_empty() {
            conn.unregister_runtime_remote_object_ids_for_session_owner(
                completed.session_id(),
                &completed.release_object_ids,
            );
        }
        if let Some(object_group) = completed.release_object_group.as_deref() {
            conn.unregister_runtime_remote_object_group_for_session_owner(
                completed.session_id(),
                object_group,
            );
        }
    }
    routed_output.push_background_events_before_response_events(&mut plan, completed.command_id);
    if completed.action == "enable" && succeeded {
        append_shared_worker_runtime_console_messages(conn, completed.session_id(), &mut plan);
    }
    if let Some(started) = timing_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            action = completed.action,
            stage = "shared_worker_runtime_inspector_plan_ready",
            elapsed_ms = started.elapsed().as_millis(),
        );
    }
    RuntimeCommandTaskStep::Complete(plan)
}

async fn complete_pending_service_worker_runtime_inspector_command(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    completed_inspector: Result<CompletedServiceWorkerRuntimeProtocolMessageDispatch, String>,
    timing_started: Option<std::time::Instant>,
) -> RuntimeCommandTaskStep {
    let (messages, mut renderer_response_rx) = match completed_inspector {
        Ok(mut completed_protocol) => {
            let renderer_response_rx = completed_protocol.take_deferred_response_receiver();
            match conn
                .complete_service_worker_runtime_protocol_message_for_session(completed_protocol)
            {
                Ok(messages) => (messages, renderer_response_rx),
                Err(message) => {
                    return complete_service_worker_runtime_inspector_error(
                        conn, completed, message,
                    );
                }
            }
        }
        Err(message) => {
            return complete_service_worker_runtime_inspector_error(conn, completed, message);
        }
    };

    if let Some(started) = timing_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            action = completed.action,
            stage = "service_worker_runtime_inspector_dispatch_returned",
            messages = messages.len(),
            elapsed_ms = started.elapsed().as_millis(),
        );
    }

    let mut plan = CommandOutputPlan::default();
    let mut routed_output = RuntimeInspectorRoutedOutput::default();
    let mut saw_current_response = route_inspector_messages_into_routed_output(
        conn,
        messages,
        completed.command_id,
        completed.session_id(),
        &mut routed_output,
    );
    if !completed.wait_for_deferred_reply
        && let Some(renderer_response_rx) = renderer_response_rx.take()
    {
        saw_current_response |= route_registered_runtime_response_receiver_into(
            conn,
            completed.command_id,
            completed.session_id(),
            renderer_response_rx,
            true,
            &mut routed_output,
        )
        .await;
    }
    if completed.wait_for_deferred_reply
        && (renderer_response_rx.is_some() || !saw_current_response)
        && completed.command_id.is_some()
    {
        return pending_runtime_deferred_inspector_reply_command(
            conn,
            completed,
            routed_output,
            renderer_response_rx,
            true,
        );
    }
    let succeeded = routed_output.command_response_succeeded(completed.command_id);
    apply_service_worker_runtime_completion_projection(
        conn,
        completed.session_id(),
        completed.action,
        succeeded,
    );
    if succeeded
        && let Some(console_action) = console_action_from_protocol_method(completed.action)
        && !apply_console_output_state_for_session(conn, completed.session_id(), console_action)
    {
        return RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(format!(
            "ConsoleCommandCompletionFailed: {}",
            completed.action
        )));
    }
    if succeeded {
        routed_output.register_object_group_for_success(
            conn,
            completed.session_id(),
            completed.object_group.as_deref(),
        );
    }
    if succeeded {
        if !completed.release_object_ids.is_empty() {
            conn.unregister_runtime_remote_object_ids_for_session_owner(
                completed.session_id(),
                &completed.release_object_ids,
            );
        }
        if let Some(object_group) = completed.release_object_group.as_deref() {
            conn.unregister_runtime_remote_object_group_for_session_owner(
                completed.session_id(),
                object_group,
            );
        }
    }
    routed_output.push_background_events_before_response_events(&mut plan, completed.command_id);
    if completed.action == "enable" && succeeded {
        append_service_worker_runtime_console_messages(conn, completed.session_id(), &mut plan);
        append_service_worker_runtime_exception_messages(conn, completed.session_id(), &mut plan);
    }
    if let Some(started) = timing_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            action = completed.action,
            stage = "service_worker_runtime_inspector_plan_ready",
            elapsed_ms = started.elapsed().as_millis(),
        );
    }
    RuntimeCommandTaskStep::Complete(plan)
}

fn apply_shared_worker_runtime_completion_projection(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    action: &str,
    succeeded: bool,
) {
    let Some(owner_session_id) = session_id else {
        return;
    };
    let Some(target) = conn.shared_worker_target_for_session_mut(Some(owner_session_id)) else {
        return;
    };

    match action {
        "enable" if succeeded => target.set_runtime_frontend_enabled(owner_session_id, true),
        "disable" if succeeded => {
            let was_enabled = target.runtime_frontend_enabled(owner_session_id);
            target.set_runtime_frontend_enabled(owner_session_id, false);
            if was_enabled {
                target.clear_runtime_binding_definitions(owner_session_id);
            }
        }
        "discardConsoleEntries" if succeeded => {
            target.discard_runtime_console_entries(owner_session_id)
        }
        _ => {}
    }
}

fn apply_shared_worker_runtime_disable_projection(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) {
    let Some(owner_session_id) = session_id else {
        return;
    };
    let Some(target) = conn.shared_worker_target_for_session_mut(Some(owner_session_id)) else {
        return;
    };
    let was_enabled = target.runtime_frontend_enabled(owner_session_id);
    target.set_runtime_frontend_enabled(owner_session_id, false);
    if was_enabled {
        target.clear_runtime_binding_definitions(owner_session_id);
    }
}

fn complete_shared_worker_runtime_inspector_error(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    message: String,
) -> RuntimeCommandTaskStep {
    if let Some(command_id) = completed.command_id {
        conn.forget_pending_inspector_await(command_id, completed.session_id());
    }
    match completed.action {
        "enable" if message == "NoDocumentLoaded" || worker_runtime_is_unavailable(&message) => {
            RuntimeCommandTaskStep::Complete(
                shared_worker_runtime_enable_command_output_plan_for_session(
                    conn,
                    completed.session_id(),
                ),
            )
        }
        "disable" if message == "NoDocumentLoaded" || worker_runtime_is_unavailable(&message) => {
            apply_shared_worker_runtime_disable_projection(conn, completed.session_id());
            RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
        }
        "discardConsoleEntries"
            if message == "NoDocumentLoaded" || worker_runtime_is_unavailable(&message) =>
        {
            if let Some(owner_session_id) = completed.session_id()
                && let Some(target) =
                    conn.shared_worker_target_for_session_mut(Some(owner_session_id))
            {
                target.discard_runtime_console_entries(owner_session_id);
            }
            RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
        }
        "Console.enable" | "Console.disable" | "Console.clearMessages"
            if worker_runtime_is_unavailable(&message) =>
        {
            if let Some(console_action) = console_action_from_protocol_method(completed.action)
                && !apply_console_output_state_for_session(
                    conn,
                    completed.session_id(),
                    console_action,
                )
            {
                return RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(
                    "UnknownSession".to_owned(),
                ));
            }
            RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
        }
        _ => RuntimeCommandTaskStep::Complete(shared_worker_runtime_error_plan(message)),
    }
}

fn apply_service_worker_runtime_completion_projection(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    action: &str,
    succeeded: bool,
) {
    let Some(owner_session_id) = session_id else {
        return;
    };
    let Some(target) = conn.service_worker_target_for_session_mut(Some(owner_session_id)) else {
        return;
    };

    match action {
        "enable" if succeeded => target.set_runtime_frontend_enabled(owner_session_id, true),
        "disable" if succeeded => {
            target.set_runtime_frontend_enabled(owner_session_id, false);
        }
        "discardConsoleEntries" if succeeded => {
            target.discard_runtime_console_entries(owner_session_id)
        }
        _ => {}
    }
}

fn apply_service_worker_runtime_disable_projection(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) {
    let Some(owner_session_id) = session_id else {
        return;
    };
    let Some(target) = conn.service_worker_target_for_session_mut(Some(owner_session_id)) else {
        return;
    };
    target.set_runtime_frontend_enabled(owner_session_id, false);
}

fn complete_service_worker_runtime_inspector_error(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    message: String,
) -> RuntimeCommandTaskStep {
    if let Some(command_id) = completed.command_id {
        conn.forget_pending_inspector_await(command_id, completed.session_id());
    }
    match completed.action {
        "enable"
            if message == "NoDocumentLoaded" || message == "ServiceWorkerRuntimeUnavailable" =>
        {
            RuntimeCommandTaskStep::Complete(
                service_worker_runtime_enable_command_output_plan_for_session(
                    conn,
                    completed.session_id(),
                ),
            )
        }
        "disable"
            if message == "NoDocumentLoaded" || message == "ServiceWorkerRuntimeUnavailable" =>
        {
            apply_service_worker_runtime_disable_projection(conn, completed.session_id());
            RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
        }
        "discardConsoleEntries"
            if message == "NoDocumentLoaded" || message == "ServiceWorkerRuntimeUnavailable" =>
        {
            if let Some(owner_session_id) = completed.session_id()
                && let Some(target) =
                    conn.service_worker_target_for_session_mut(Some(owner_session_id))
            {
                target.discard_runtime_console_entries(owner_session_id);
            }
            RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
        }
        "Console.enable" | "Console.disable" | "Console.clearMessages"
            if message == "ServiceWorkerRuntimeUnavailable" =>
        {
            if let Some(console_action) = console_action_from_protocol_method(completed.action)
                && !apply_console_output_state_for_session(
                    conn,
                    completed.session_id(),
                    console_action,
                )
            {
                return RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(
                    "UnknownSession".to_owned(),
                ));
            }
            RuntimeCommandTaskStep::Complete(CommandOutputPlan::success())
        }
        _ => RuntimeCommandTaskStep::Complete(service_worker_runtime_error_plan(message)),
    }
}

fn shared_worker_runtime_error_plan(message: String) -> CommandOutputPlan {
    match message.as_str() {
        "Duplicate `id` in protocol request" => CommandOutputPlan::error(-32600, message),
        "UnknownSession" => CommandOutputPlan::error(-32001, "Unknown sessionId"),
        "InvalidParams" => CommandOutputPlan::error(-32602, "InvalidParams"),
        "UnknownMethod" => CommandOutputPlan::error(-32601, "UnknownMethod"),
        "NoDocumentLoaded" => CommandOutputPlan::error(-32000, "NoDocumentLoaded"),
        _ => CommandOutputPlan::error(-32000, message),
    }
}

fn service_worker_runtime_error_plan(message: String) -> CommandOutputPlan {
    match message.as_str() {
        "Duplicate `id` in protocol request" => CommandOutputPlan::error(-32600, message),
        "UnknownSession" => CommandOutputPlan::error(-32001, "Unknown sessionId"),
        "InvalidParams" => CommandOutputPlan::error(-32602, "InvalidParams"),
        "UnknownMethod" => CommandOutputPlan::error(-32601, "UnknownMethod"),
        "NoDocumentLoaded" => CommandOutputPlan::error(-32000, "NoDocumentLoaded"),
        _ => CommandOutputPlan::error(-32000, message),
    }
}

fn append_shared_worker_runtime_console_messages(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    plan: &mut CommandOutputPlan,
) {
    let Some(target) = conn.shared_worker_target_for_session_mut(session_id) else {
        return;
    };
    let Some(session_id) = session_id else {
        return;
    };
    let has_runtime_context = target.real_runtime_execution_context_id().is_some();
    let runtime_messages = target.pending_runtime_console_messages(session_id).to_vec();
    let console_end = target.console_message_count();
    if has_runtime_context {
        target.mark_runtime_console_emitted(session_id, console_end);
    }

    let base_timestamp = monotonic_timestamp_seconds();
    for (index, message) in runtime_messages.iter().enumerate() {
        let (console_type, text) = runtime_console_message_type_and_text(&message.message);
        push_runtime_console_api_called_background_event(
            plan,
            Some(session_id),
            console_type,
            text,
            &message.args,
            message.stack.as_deref(),
            message.execution_context_id,
            base_timestamp + ((index + 1) as f64 * 0.000_001),
        );
    }
}

fn append_service_worker_runtime_console_messages(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    plan: &mut CommandOutputPlan,
) {
    let Some(target) = conn.service_worker_target_for_session_mut(session_id) else {
        return;
    };
    let Some(session_id) = session_id else {
        return;
    };
    let has_runtime_context = target.real_runtime_execution_context_id().is_some();
    let runtime_messages = target.pending_runtime_console_messages(session_id).to_vec();
    let console_end = target.console_message_count();
    if has_runtime_context {
        target.mark_runtime_console_emitted(session_id, console_end);
    }

    let base_timestamp = monotonic_timestamp_seconds();
    for (index, message) in runtime_messages.iter().enumerate() {
        let (console_type, text) = runtime_console_message_type_and_text(&message.message);
        push_runtime_console_api_called_background_event(
            plan,
            Some(session_id),
            console_type,
            text,
            &message.args,
            message.stack.as_deref(),
            message.execution_context_id,
            base_timestamp + ((index + 1) as f64 * 0.000_001),
        );
    }
}

fn append_service_worker_runtime_exception_messages(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    plan: &mut CommandOutputPlan,
) {
    let Some(target) = conn.service_worker_target_for_session_mut(session_id) else {
        return;
    };
    let Some(session_id) = session_id else {
        return;
    };
    let has_runtime_context = target.real_runtime_execution_context_id().is_some();
    let exception_messages = target
        .pending_runtime_exception_messages(session_id)
        .to_vec();
    let exception_start = target
        .exception_message_count()
        .saturating_sub(exception_messages.len());
    let exception_end = target.exception_message_count();
    if has_runtime_context {
        target.mark_runtime_exception_emitted(session_id, exception_end);
    }

    push_runtime_exception_thrown_protocol_messages(
        plan,
        session_id,
        &exception_messages,
        exception_start,
    );
}

fn shared_worker_runtime_enable_command_output_plan_for_session(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> CommandOutputPlan {
    let Some(target) = conn.shared_worker_target_for_session_mut(session_id) else {
        return CommandOutputPlan::error(-32001, "Unknown sessionId");
    };
    let Some(session_id) = session_id else {
        return CommandOutputPlan::error(-32001, "Unknown sessionId");
    };
    target.set_runtime_frontend_enabled(session_id, true);
    let execution_context_created = build_shared_worker_execution_context_created_event(target);
    let has_runtime_context = target.real_runtime_execution_context_id().is_some();
    let runtime_messages = target.pending_runtime_console_messages(session_id).to_vec();
    let console_end = target.console_message_count();
    if has_runtime_context {
        target.mark_runtime_console_emitted(session_id, console_end);
    }

    let mut plan = CommandOutputPlan::success();
    if let Some(execution_context_created) = execution_context_created {
        target.record_runtime_contexts_reported_to_frontend(session_id);
        push_execution_context_created_background_event(
            &mut plan,
            execution_context_created,
            session_id,
        );
    }
    let base_timestamp = monotonic_timestamp_seconds();
    for (index, message) in runtime_messages.iter().enumerate() {
        let (console_type, text) = runtime_console_message_type_and_text(&message.message);
        push_runtime_console_api_called_background_event(
            &mut plan,
            Some(session_id),
            console_type,
            text,
            &message.args,
            message.stack.as_deref(),
            message.execution_context_id,
            base_timestamp + ((index + 1) as f64 * 0.000_001),
        );
    }
    plan
}

fn service_worker_runtime_enable_command_output_plan_for_session(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> CommandOutputPlan {
    let Some(target) = conn.service_worker_target_for_session_mut(session_id) else {
        return CommandOutputPlan::error(-32001, "Unknown sessionId");
    };
    let Some(session_id) = session_id else {
        return CommandOutputPlan::error(-32001, "Unknown sessionId");
    };
    target.set_runtime_frontend_enabled(session_id, true);
    let execution_context_created = build_service_worker_execution_context_created_event(target);
    let has_runtime_context = target.real_runtime_execution_context_id().is_some();
    let runtime_messages = target.pending_runtime_console_messages(session_id).to_vec();
    let console_end = target.console_message_count();
    if has_runtime_context {
        target.mark_runtime_console_emitted(session_id, console_end);
    }
    let exception_messages = target
        .pending_runtime_exception_messages(session_id)
        .to_vec();
    let exception_start = target
        .exception_message_count()
        .saturating_sub(exception_messages.len());
    let exception_end = target.exception_message_count();
    if has_runtime_context {
        target.mark_runtime_exception_emitted(session_id, exception_end);
    }

    let mut plan = CommandOutputPlan::success();
    if let Some(execution_context_created) = execution_context_created {
        target.record_runtime_contexts_reported_to_frontend(session_id);
        push_execution_context_created_background_event(
            &mut plan,
            execution_context_created,
            session_id,
        );
    }
    let base_timestamp = monotonic_timestamp_seconds();
    for (index, message) in runtime_messages.iter().enumerate() {
        let (console_type, text) = runtime_console_message_type_and_text(&message.message);
        push_runtime_console_api_called_background_event(
            &mut plan,
            Some(session_id),
            console_type,
            text,
            &message.args,
            message.stack.as_deref(),
            message.execution_context_id,
            base_timestamp + ((index + 1) as f64 * 0.000_001),
        );
    }
    push_runtime_exception_thrown_protocol_messages(
        &mut plan,
        session_id,
        &exception_messages,
        exception_start,
    );
    plan
}

fn push_execution_context_created_background_event(
    plan: &mut CommandOutputPlan,
    event: RuntimeContextProtocolEvent,
    session_id: &str,
) {
    let mut events = Vec::new();
    emit_runtime_context_protocol_background_event_typed(&mut events, event, Some(session_id));
    plan.extend_background_events(events);
}

fn push_runtime_console_api_called_background_event(
    plan: &mut CommandOutputPlan,
    session_id: Option<&str>,
    console_type: &str,
    text: &str,
    args: &[Value],
    stack: Option<&str>,
    execution_context_id: i64,
    timestamp: f64,
) {
    plan.push_background_event(runtime_console_api_called_background_event(
        session_id,
        None,
        console_type,
        text,
        args,
        stack,
        execution_context_id,
        timestamp,
    ));
}

fn push_runtime_exception_thrown_protocol_messages(
    plan: &mut CommandOutputPlan,
    session_id: &str,
    messages: &[ServiceWorkerRuntimeExceptionSnapshot],
    exception_start: usize,
) {
    let base_timestamp = monotonic_timestamp_seconds();
    for (offset, message) in messages.iter().enumerate() {
        let exception_index = exception_start + offset;
        plan.push_background_event(runtime_exception_thrown_background_event(
            Some(session_id),
            None,
            &message.message.message,
            &message.message.filename,
            message.execution_context_id,
            exception_index,
            base_timestamp + ((offset + 1) as f64 * 0.000_001),
            Some(u64::from(message.message.lineno.saturating_sub(1))),
            Some(u64::from(message.message.colno.saturating_sub(1))),
        ));
    }
}

fn build_shared_worker_execution_context_created_event(
    target: &crate::conn::SharedWorkerTargetState,
) -> Option<RuntimeContextProtocolEvent> {
    let context_id = target.real_runtime_execution_context_id()?;
    let realm_id = format!("shared-worker-{}", target.target_id);
    let origin = url::Url::parse(&target.url)
        .ok()
        .map(|url| moli_url::origin_ascii_serialization(&url))
        .unwrap_or_else(|| "null".to_owned());
    Some(RuntimeContextProtocolEvent::Created(
        RuntimeExecutionContextEvent {
            target_id: None,
            context_id: Some(context_id),
            realm_id: Some(DevToolsRealmId::from(realm_id)),
            frame_id: None,
            origin: Some(origin),
            name: Some(target.name.clone()),
            is_default: Some(true),
            context_type: Some("worker".to_owned()),
            grant_universal_access: None,
        },
    ))
}

fn build_service_worker_execution_context_created_event(
    target: &crate::conn::ServiceWorkerTargetState,
) -> Option<RuntimeContextProtocolEvent> {
    let context_id = target.real_runtime_execution_context_id()?;
    let realm_id = format!("service-worker-{}", target.target_id);
    let origin = url::Url::parse(&target.script_url)
        .ok()
        .map(|url| moli_url::origin_ascii_serialization(&url))
        .unwrap_or_else(|| "null".to_owned());
    Some(RuntimeContextProtocolEvent::Created(
        RuntimeExecutionContextEvent {
            target_id: None,
            context_id: Some(context_id),
            realm_id: Some(DevToolsRealmId::from(realm_id)),
            frame_id: None,
            origin: Some(origin),
            name: Some(String::new()),
            is_default: Some(true),
            context_type: Some("service-worker".to_owned()),
            grant_universal_access: None,
        },
    ))
}

fn disable_command_output_plan_sync(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    match apply_runtime_disable_projection_after_success(conn, cmd.session_id) {
        Ok(()) => CommandOutputPlan::success(),
        Err(_) => CommandOutputPlan::error(-32001, "Unknown sessionId"),
    }
}

fn start_pending_runtime_disable_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> RuntimeCommandTaskStep {
    if !conn
        .runtime_session_owner_slot(cmd.session_id)
        .is_ok_and(|slot| slot.has_loaded_page())
    {
        return RuntimeCommandTaskStep::Complete(disable_command_output_plan_sync(conn, cmd));
    }

    let pending = match start_pending_runtime_inspector_dispatch_with_delivery(
        conn,
        cmd,
        cmd.json.to_owned(),
        cmd.terminal_response_delivery(RendererInspectorResponseDelivery::DevToolsSession),
    ) {
        Ok(pending) => pending,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: "disable",
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::Inspector { pending },
    }))
}

fn start_runtime_run_if_waiting_for_debugger_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> RuntimeCommandTaskStep {
    if !conn
        .runtime_session_owner_slot(cmd.session_id)
        .is_ok_and(|slot| slot.has_loaded_page())
    {
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::success());
    }

    let pending = match start_pending_runtime_inspector_dispatch_with_delivery(
        conn,
        cmd,
        cmd.json.to_owned(),
        cmd.terminal_response_delivery(RendererInspectorResponseDelivery::DevToolsSession),
    ) {
        Ok(pending) => pending,
        Err(message) if message == "NoDocumentLoaded" => {
            return RuntimeCommandTaskStep::Complete(CommandOutputPlan::success());
        }
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: "runIfWaitingForDebugger",
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::Inspector { pending },
    }))
}

fn apply_runtime_disable_projection_after_success(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> Result<(), String> {
    let was_enabled = conn
        .target_runtime_session_state_for_session(session_id)
        .is_some_and(|state| state.runtime_frontend_enabled);
    match conn.set_runtime_frontend_enabled_for_session_owner(session_id, false) {
        SessionOwnerRuntimeFrontendEnableResult::Handled => {
            advance_runtime_observable_cursors_to_current_for_session_owner(conn, session_id);
            if was_enabled {
                clear_runtime_binding_definitions_for_session_owner(conn, session_id)?;
            }
            Ok(())
        }
        SessionOwnerRuntimeFrontendEnableResult::UnknownSession => {
            Err("Runtime.disable succeeded after session owner disappeared".to_owned())
        }
    }
}

fn start_runtime_discard_console_entries_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> RuntimeCommandTaskStep {
    if !conn
        .runtime_session_owner_slot(cmd.session_id)
        .is_ok_and(|slot| slot.has_loaded_page())
    {
        advance_runtime_observable_cursors_to_current_for_session_owner(conn, cmd.session_id);
        return RuntimeCommandTaskStep::Complete(CommandOutputPlan::success());
    }
    let pending = match start_pending_runtime_inspector_dispatch_with_delivery(
        conn,
        cmd,
        cmd.json.to_owned(),
        cmd.terminal_response_delivery(RendererInspectorResponseDelivery::DevToolsSession),
    ) {
        Ok(pending) => pending,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };
    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: "discardConsoleEntries",
        owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::Inspector { pending },
    }))
}

#[cfg(test)]
mod protocol_neutral_tests {
    use crate::devtools_runtime::{
        DevToolsCallFunctionCommand, DevToolsCommand, DevToolsCommandContext, DevToolsProtocol,
        DevToolsResultOwnership, RuntimeExecutionContextEvent,
    };
    use moli_core::RendererOwnerLocalHostId;
    use moli_core::page::{MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH, RendererSharedWorkerConsoleMessage};
    use moli_page_types::RendererInspectorResponseDelivery;
    use moli_shared_worker::SharedWorkerInstanceId;
    use serde_json::{Value, json};

    use crate::conn::{
        BrowserContext, CdpConnection, CdpSessionRoute, Cmd, SharedWorkerTargetState,
    };
    use crate::domains::actions::ConsoleAction;
    use crate::testing::TestContext;

    use super::super::bidi_nodes::{
        BidiIncludeShadowTree, BidiNodeSerializationOptions,
        devtools_serialization_options_for_node_probe,
    };
    use super::{
        DevToolsRuntimeTarget, RuntimeCommandTaskStep, RuntimeProbeCompletionScope,
        apply_shared_worker_runtime_completion_projection, build_cdp_call_function_command,
        build_cdp_evaluate_script_command, cdp_call_argument_from_devtools_argument,
        devtools_call_function_cdp_arguments, devtools_call_function_declaration,
        devtools_call_function_deserializes_bidi_local_values,
        devtools_command_has_bidi_script_channel_arguments, locate_nodes_error_from_exception,
        materialize_devtools_script_window_remote_value, start_console_inspector_command_dispatch,
        start_devtools_runtime_command,
    };
    use crate::devtools_runtime::{
        DevToolsCommandResult, DevToolsErrorKind, DevToolsLocateNodesLocator,
        DevToolsRemoteHandleId, DevToolsRemoteValue, DevToolsScriptException, DevToolsScriptResult,
        DevToolsTargetId,
    };

    fn worker_context_created_event(context_id: i64) -> RuntimeExecutionContextEvent {
        RuntimeExecutionContextEvent {
            target_id: None,
            context_id: Some(context_id),
            realm_id: None,
            frame_id: None,
            origin: None,
            name: None,
            is_default: None,
            context_type: Some("worker".to_owned()),
            grant_universal_access: None,
        }
    }

    fn deeply_nested_plain_value(mut value: Value, depth: usize) -> Value {
        for _ in 0..depth {
            value = json!({ "child": [value] });
        }
        value
    }

    fn deeply_nested_deep_serialized_array(mut value: Value, depth: usize) -> Value {
        for _ in 0..depth {
            value = json!({
                "type": "array",
                "value": [value],
            });
        }
        value
    }

    fn run_deep_protocol_value_test(name: &'static str, test: impl FnOnce() + Send + 'static) {
        let result = std::thread::Builder::new()
            .name(name.to_owned())
            .stack_size(32 * 1024 * 1024)
            .spawn(test)
            .expect("large-stack protocol value test thread should spawn")
            .join();
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }

    #[test]
    fn bidi_shared_reference_collection_uses_protocol_depth_cap() {
        run_deep_protocol_value_test("bidi-shared-reference-depth-cap", || {
            let value = json!({
                "outer": [
                    { "type": "node", "sharedId": "SHARED-1" },
                ],
            });
            let mut references = Vec::new();
            super::collect_bidi_node_shared_reference_paths(
                &value,
                super::BidiCallFunctionValueRoot::Argument(0),
                &mut references,
            );

            assert_eq!(references.len(), 1);
            assert_eq!(references[0].shared_id, "SHARED-1");
            assert_eq!(
                references[0].path,
                vec![
                    super::BidiValuePathSegment::Key("outer".to_owned()),
                    super::BidiValuePathSegment::Index(0),
                ]
            );

            let deep_value = deeply_nested_plain_value(
                json!({ "type": "node", "sharedId": "TOO-DEEP" }),
                MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH + 8,
            );
            references.clear();
            super::collect_bidi_node_shared_reference_paths(
                &deep_value,
                super::BidiCallFunctionValueRoot::Argument(0),
                &mut references,
            );
            assert!(references.is_empty());
        });
    }

    #[test]
    fn devtools_remote_reference_collection_uses_protocol_depth_cap() {
        run_deep_protocol_value_test("devtools-remote-reference-depth-cap", || {
            let mut object_ids = Vec::new();
            super::collect_devtools_remote_object_ids(
                &json!({ "outer": [{ "objectId": "OBJECT-1" }] }),
                &mut object_ids,
            );
            assert_eq!(object_ids, vec!["OBJECT-1"]);

            let deep_value = deeply_nested_plain_value(
                json!({ "objectId": "TOO-DEEP" }),
                MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH + 8,
            );
            object_ids.clear();
            super::collect_devtools_remote_object_ids(&deep_value, &mut object_ids);
            assert!(object_ids.is_empty());

            let mut references = Vec::new();
            super::collect_devtools_remote_references(
                &json!({ "outer": [{ "type": "node", "sharedId": "NODE-1" }] }),
                &mut references,
            );
            assert_eq!(references.len(), 1);
            assert_eq!(references[0].object_id, "NODE-1");
            assert_eq!(references[0].kind, super::RuntimeRemoteReferenceKind::Node);
        });
    }

    #[test]
    fn deep_serialized_candidate_paths_use_protocol_depth_cap() {
        run_deep_protocol_value_test("deep-serialized-candidate-depth-cap", || {
            let value = json!({
                "type": "array",
                "value": [
                    {
                        "type": "object",
                        "value": [],
                    },
                ],
            });
            let paths = super::collect_deep_serialized_node_candidate_paths(&value);
            assert_eq!(paths.len(), 1);
            assert_eq!(paths[0].json_pointer, "/value/0");
            assert_eq!(
                paths[0].js_path,
                vec![json!({
                    "kind": "index",
                    "index": 0,
                })]
            );

            let deep_value = deeply_nested_deep_serialized_array(
                json!({
                    "type": "object",
                    "value": [],
                }),
                MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH + 8,
            );
            let paths = super::collect_deep_serialized_node_candidate_paths(&deep_value);
            assert!(paths.is_empty());
        });
    }

    #[test]
    fn cdp_evaluate_builds_protocol_neutral_script_command() {
        let params = json!({
            "expression": "document.title",
            "returnByValue": true,
            "userGesture": true
        });
        let cmd = Cmd::for_test(
            Some(12),
            "Runtime.evaluate",
            &params,
            Some("SID-1"),
            r#"{"id":12,"method":"Runtime.evaluate"}"#,
        );

        let command = build_cdp_evaluate_script_command(&cmd, Some("TID-1"), Some("BID-1"), true);

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-1")
        );
        assert_eq!(
            command.context.target_id.as_ref().map(|id| id.as_str()),
            Some("TID-1")
        );
        assert_eq!(
            command
                .context
                .browser_context_id
                .as_ref()
                .map(|id| id.as_str()),
            Some("BID-1")
        );
        assert_eq!(command.realm_id, None);
        assert_eq!(command.expression, "document.title");
        assert!(command.await_promise);
        assert!(command.user_gesture);
        assert_eq!(command.result_ownership, DevToolsResultOwnership::ByValue);
    }

    #[test]
    fn bidi_window_context_requires_explicit_remote_metadata() {
        let target = DevToolsRuntimeTarget {
            route: CdpSessionRoute::Browser,
            execution_context_id: None,
            window_context_id: Some(DevToolsTargetId::from("TOP")),
        };
        let mut shape_only = DevToolsRemoteValue::from_json_value(Value::Null);
        shape_only.deep_serialized_value = Some(json!({
            "type": "object",
            "internalId": "WINDOW-1",
            "value": [
                ["window", {"type": "object", "internalId": "WINDOW-1"}],
                ["self", {"type": "object", "internalId": "WINDOW-1"}],
                ["parent", {"type": "object", "internalId": "WINDOW-1"}],
                ["top", {"type": "object", "internalId": "WINDOW-1"}],
                ["frames", {"type": "object", "internalId": "WINDOW-1"}],
                ["document", {"type": "node"}]
            ]
        }));
        let mut shape_only_result =
            DevToolsCommandResult::Script(Box::new(DevToolsScriptResult::Value(shape_only)));

        materialize_devtools_script_window_remote_value(&mut shape_only_result, &target);

        let DevToolsCommandResult::Script(result) = shape_only_result else {
            panic!("expected script result");
        };
        let DevToolsScriptResult::Value(value) = *result else {
            panic!("expected value result");
        };
        assert_eq!(
            value.window_context, None,
            "a Window-shaped deep serialization must not be guessed into the current context without the renderer BiDi window marker"
        );

        let mut marked_window = DevToolsRemoteValue::from_json_value(Value::Null);
        marked_window.deep_serialized_value = Some(json!({
            "type": "object",
            "value": [
                ["__moliBidiRemoteValue", {"type": "boolean", "value": true}],
                ["type", {"type": "string", "value": "window"}],
                ["context", {"type": "string", "value": "CHILD"}]
            ]
        }));
        let mut marked_result =
            DevToolsCommandResult::Script(Box::new(DevToolsScriptResult::Value(marked_window)));

        materialize_devtools_script_window_remote_value(&mut marked_result, &target);

        let DevToolsCommandResult::Script(result) = marked_result else {
            panic!("expected script result");
        };
        let DevToolsScriptResult::Value(value) = *result else {
            panic!("expected value result");
        };
        assert_eq!(
            value
                .window_context
                .as_deref()
                .map(DevToolsTargetId::as_str),
            Some("CHILD")
        );
    }

    #[test]
    fn node_probe_serialization_options_keep_unbounded_dom_depth_unbounded() {
        let options =
            devtools_serialization_options_for_node_probe(&BidiNodeSerializationOptions {
                value_depth: -1,
                snapshot_depth: -1,
                include_shadow_tree: BidiIncludeShadowTree::All,
            });

        assert_eq!(options.max_object_depth, Some(0));
        assert_eq!(options.max_dom_depth, None);
        assert_eq!(options.include_shadow_tree.as_deref(), Some("all"));
    }

    #[test]
    fn locate_nodes_error_classification_uses_engine_selector_errors_only() {
        let invalid_css = locate_nodes_error_from_exception(DevToolsScriptException {
            exception_id: None,
            script_id: None,
            text:
                "SyntaxError: Failed to execute 'querySelectorAll' on 'Document': '>' is not a valid selector."
                    .to_owned(),
            value: None,
            realm: None,
            line_number: None,
            column_number: None,
            stack_trace: None,
        }, &DevToolsLocateNodesLocator::Css(">".to_owned()));
        assert_eq!(invalid_css.kind, DevToolsErrorKind::InvalidSelector);

        let invalid_xpath = locate_nodes_error_from_exception(
            DevToolsScriptException {
                exception_id: None,
                script_id: None,
                text:
                    "DOMException: The string 'this][isnot][valid' is not a valid XPath expression."
                        .to_owned(),
                value: None,
                realm: None,
                line_number: None,
                column_number: None,
                stack_trace: None,
            },
            &DevToolsLocateNodesLocator::XPath("this][isnot][valid".to_owned()),
        );
        assert_eq!(invalid_xpath.kind, DevToolsErrorKind::InvalidSelector);

        let bare_xpath_dom_exception = locate_nodes_error_from_exception(
            DevToolsScriptException {
                exception_id: None,
                script_id: None,
                text: "DOMException".to_owned(),
                value: None,
                realm: None,
                line_number: None,
                column_number: None,
                stack_trace: None,
            },
            &DevToolsLocateNodesLocator::XPath("this][isnot][valid".to_owned()),
        );
        assert_eq!(
            bare_xpath_dom_exception.kind,
            DevToolsErrorKind::InvalidSelector
        );

        let application_error = locate_nodes_error_from_exception(
            DevToolsScriptException {
                exception_id: None,
                script_id: None,
                text: "Error: application says this is not a valid selector".to_owned(),
                value: None,
                realm: None,
                line_number: None,
                column_number: None,
                stack_trace: None,
            },
            &DevToolsLocateNodesLocator::Css("*".to_owned()),
        );
        assert_eq!(application_error.kind, DevToolsErrorKind::Internal);
    }

    #[test]
    fn locate_nodes_deep_serialized_records_use_backend_node_id() {
        let backend_node_id = moli_core::page::RENDERER_BACKEND_NODE_ID_START + 42;
        let value = json!({
            "type": "array",
            "value": [
                {
                    "type": "object",
                    "value": [
                        ["backendNodeId", { "type": "number", "value": backend_node_id }],
                        ["node", {
                            "type": "node",
                            "sharedId": "NODE-1",
                            "value": {
                                "nodeType": 1,
                                "localName": "button"
                            }
                        }]
                    ]
                }
            ]
        });

        let nodes = super::locate_nodes_remote_values_from_deep_serialized_array(&value)
            .expect("locateNodes deepSerializedValue should parse");

        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].backend_node_id, Some(backend_node_id));
        assert_eq!(nodes[0].node_id, None);
        assert_eq!(
            nodes[0].shared_id,
            Some(DevToolsRemoteHandleId::from("NODE-1"))
        );
    }

    #[test]
    fn cdp_evaluate_without_return_by_value_uses_root_ownership() {
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(13),
            "Runtime.evaluate",
            &params,
            None,
            r#"{"id":13,"method":"Runtime.evaluate"}"#,
        );

        let command = build_cdp_evaluate_script_command(&cmd, None, None, false);

        assert_eq!(command.expression, "");
        assert!(!command.await_promise);
        assert_eq!(command.result_ownership, DevToolsResultOwnership::Root);
    }

    #[test]
    fn cdp_call_function_builds_protocol_neutral_script_command() {
        let params = json!({
            "executionContextId": 7,
            "objectId": "remote-object-1",
            "functionDeclaration": "function(arg) { return this.value + arg; }",
            "arguments": [
                { "value": 2 },
                { "objectId": "remote-object-2" }
            ],
            "awaitPromise": true,
            "returnByValue": true,
            "userGesture": true,
            "objectGroup": "grp"
        });
        let cmd = Cmd::for_test(
            Some(14),
            "Runtime.callFunctionOn",
            &params,
            Some("SID-call"),
            r#"{"id":14,"method":"Runtime.callFunctionOn"}"#,
        );

        let command = build_cdp_call_function_command(&cmd, Some("TID-call"), Some("BID-call"));

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-call")
        );
        assert_eq!(
            command.context.target_id.as_ref().map(|id| id.as_str()),
            Some("TID-call")
        );
        assert_eq!(
            command
                .context
                .browser_context_id
                .as_ref()
                .map(|id| id.as_str()),
            Some("BID-call")
        );
        assert_eq!(command.realm_id.as_ref().map(|id| id.as_str()), Some("7"));
        assert_eq!(
            command.object_id.as_ref().map(|id| id.as_str()),
            Some("remote-object-1")
        );
        assert_eq!(
            command.function_declaration,
            "function(arg) { return this.value + arg; }"
        );
        assert_eq!(command.arguments.len(), 2);
        assert!(command.await_promise);
        assert!(command.user_gesture);
        assert_eq!(command.result_ownership, DevToolsResultOwnership::ByValue);
        assert_eq!(command.object_group.as_deref(), Some("grp"));
    }

    #[test]
    fn cdp_call_argument_recursively_converts_bidi_array_and_object_values() {
        let argument = json!({
            "type": "array",
            "value": [
                {"type": "string", "value": "outer"},
                {
                    "type": "object",
                    "value": [
                        ["inner", {"type": "number", "value": 7}],
                        ["flag", {"type": "boolean", "value": true}]
                    ]
                }
            ]
        });

        assert_eq!(
            cdp_call_argument_from_devtools_argument(argument),
            json!({
                "value": [
                    "outer",
                    {
                        "inner": 7,
                        "flag": true
                    }
                ]
            })
        );
    }

    #[test]
    fn cdp_call_argument_converts_bidi_bigint_to_unserializable_value() {
        assert_eq!(
            cdp_call_argument_from_devtools_argument(json!({
                "type": "bigint",
                "value": "17",
            })),
            json!({ "unserializableValue": "17n" })
        );
        assert_eq!(
            cdp_call_argument_from_devtools_argument(json!({
                "type": "bigint",
                "value": "19n",
            })),
            json!({ "unserializableValue": "19n" })
        );
    }

    #[test]
    fn bidi_call_function_deserializes_js_backed_local_values() {
        let argument = json!({
            "type": "map",
            "value": [
                [
                    "created",
                    {"type": "date", "value": "2022-05-31T13:47:29.000Z"}
                ],
                [
                    {"type": "regexp", "value": {"pattern": "foo", "flags": "g"}},
                    {
                        "type": "set",
                        "value": [
                            {"type": "string", "value": "bar"}
                        ]
                    }
                ]
            ]
        });
        let command = DevToolsCallFunctionCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: None,
                target_id: None,
                browser_context_id: None,
            },
            realm_id: None,
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: "(arg) => arg".to_owned(),
            arguments: vec![argument.clone()],
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::ByValue,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        };

        assert!(devtools_call_function_deserializes_bidi_local_values(
            &command
        ));
        let declaration = devtools_call_function_declaration(&command, true, 1);
        assert!(declaration.contains("new Date(value.value)"));
        assert!(declaration.contains("new Map"));
        assert!(declaration.contains("new RegExp"));
        assert!(declaration.contains("new Set"));
        assert!(declaration.contains("value.value === 'NaN'"));
        assert!(declaration.contains("f(...deserializedArgs)"));

        assert_eq!(
            devtools_call_function_cdp_arguments(&command, true),
            vec![json!({
                    "value": {
                        "__moliBidiLocalValue": true,
                        "type": "map",
                        "value": [
                            [
                                "created",
                                {
                                    "__moliBidiLocalValue": true,
                                    "type": "date",
                                    "value": "2022-05-31T13:47:29.000Z"
                                }
                            ],
                            [
                                {
                                    "__moliBidiLocalValue": true,
                                    "type": "regexp",
                                    "value": {
                                        "pattern": "foo",
                                        "flags": "g"
                                    }
                                },
                                {
                                    "__moliBidiLocalValue": true,
                                    "type": "set",
                                    "value": [
                                        {
                                            "__moliBidiLocalValue": true,
                                            "type": "string",
                                            "value": "bar"
                                        }
                                    ]
                                }
                            ]
                        ]
                    }
            })]
        );
    }

    #[test]
    fn bidi_call_function_deserializes_nested_unserializable_numbers_and_bigints() {
        let command = DevToolsCallFunctionCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: None,
                target_id: None,
                browser_context_id: None,
            },
            realm_id: None,
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: "(arg) => arg".to_owned(),
            arguments: vec![json!({
                "type": "object",
                "value": [
                    ["nan", {"type": "number", "value": "NaN"}],
                    ["negativeZero", {"type": "number", "value": "-0"}],
                    ["big", {"type": "bigint", "value": "23"}]
                ]
            })],
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::ByValue,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        };

        assert!(devtools_call_function_deserializes_bidi_local_values(
            &command
        ));
        let declaration = devtools_call_function_declaration(&command, true, 1);
        assert!(declaration.contains("value.value === 'NaN'"));
        assert!(declaration.contains("BigInt(value.value)"));

        assert_eq!(
            devtools_call_function_cdp_arguments(&command, true),
            vec![json!({
                "value": {
                    "__moliBidiLocalValue": true,
                    "type": "object",
                    "value": [
                        [
                            "nan",
                            {
                                "__moliBidiLocalValue": true,
                                "type": "number",
                                "value": "NaN"
                            }
                        ],
                        [
                            "negativeZero",
                            {
                                "__moliBidiLocalValue": true,
                                "type": "number",
                                "value": "-0"
                            }
                        ],
                        [
                            "big",
                            {
                                "__moliBidiLocalValue": true,
                                "type": "bigint",
                                "value": "23"
                            }
                        ]
                    ]
                }
            })]
        );
    }

    #[test]
    fn bidi_call_function_deserializes_channel_arguments() {
        let command = DevToolsCallFunctionCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: None,
                target_id: None,
                browser_context_id: None,
            },
            realm_id: None,
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: "(channel) => channel('foo')".to_owned(),
            arguments: vec![json!({
                "type": "channel",
                "value": {
                    "channel": "channel_name",
                    "ownership": "root",
                    "serializationOptions": {
                        "maxObjectDepth": 0
                    }
                }
            })],
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::ByValue,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        };

        assert!(devtools_call_function_deserializes_bidi_local_values(
            &command
        ));
        assert!(devtools_command_has_bidi_script_channel_arguments(
            &DevToolsCommand::CallFunction(command.clone())
        ));
        let declaration = devtools_call_function_declaration(&command, true, 1);
        assert!(!declaration.contains("__moliBidiScriptMessageQueue"));
        assert!(!declaration.contains("__moliBidiScriptMessageValues"));
        assert!(!declaration.contains("__moliBidiEmitScriptMessage"));
        assert!(declaration.contains("__moliCreateBidiChannelDelegate"));
        assert!(!declaration.contains("__moliBidiPreloadChannelRegistry"));
        assert!(!declaration.contains("Object.create(null)"));

        assert_eq!(
            devtools_call_function_cdp_arguments(&command, true),
            vec![json!({
                "value": {
                    "__moliBidiLocalValue": true,
                    "type": "channel",
                    "value": {
                        "channel": "channel_name",
                        "ownership": "root",
                        "serializationOptions": {
                            "maxObjectDepth": 0
                        }
                    }
                }
            })]
        );
    }

    #[test]
    fn bidi_call_function_deserializes_nested_remote_references() {
        let command = DevToolsCallFunctionCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: None,
                target_id: None,
                browser_context_id: None,
            },
            realm_id: None,
            world_name: None,
            object_id: None,
            this_parameter: Some(json!({
                "type": "object",
                "value": [
                    ["nested", {"handle": "THIS-HANDLE"}]
                ]
            })),
            function_declaration: "function(arg) { return this.nested === arg[0]; }".to_owned(),
            arguments: vec![json!({
                "type": "array",
                "value": [
                    {
                        "handle": "ARG-HANDLE",
                        "sharedId": "ARG-SHARED"
                    }
                ]
            })],
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::ByValue,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        };

        assert!(devtools_call_function_deserializes_bidi_local_values(
            &command
        ));
        let declaration = devtools_call_function_declaration(&command, true, 2);
        assert!(declaration.contains("const __primaryArgs = args.slice(0, 2);"));
        assert!(declaration.contains("const __remoteReferences = args.slice(2);"));
        assert!(declaration.contains("__moliBidiRemoteReference"));
        assert!(declaration.contains("f.apply(deserializedThis, deserializedArgs)"));
        assert!(
            declaration
                .find("const __remoteReferences = args.slice(2);")
                .unwrap()
                < declaration.find("const __deserialize =").unwrap()
        );

        assert_eq!(
            devtools_call_function_cdp_arguments(&command, true),
            vec![
                json!({
                    "value": {
                        "__moliBidiLocalValue": true,
                        "type": "object",
                        "value": [
                            [
                                "nested",
                                {
                                    "__moliBidiRemoteReference": true,
                                    "index": 0
                                }
                            ]
                        ]
                    }
                }),
                json!({
                    "value": {
                        "__moliBidiLocalValue": true,
                        "type": "array",
                        "value": [
                            {
                                "__moliBidiRemoteReference": true,
                                "index": 1
                            }
                        ]
                    }
                }),
                json!({ "objectId": "THIS-HANDLE" }),
                json!({ "objectId": "ARG-SHARED" }),
            ]
        );
    }

    #[test]
    fn bidi_call_function_treats_remote_collection_preview_nodes_as_inert() {
        let command = DevToolsCallFunctionCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: None,
                target_id: None,
                browser_context_id: None,
            },
            realm_id: None,
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: "(collection) => collection.item(0)".to_owned(),
            arguments: vec![json!({
                "type": "htmlcollection",
                "handle": "COLLECTION-HANDLE",
                "value": [
                    {
                        "type": "node",
                        "sharedId": "PREVIEW-NODE",
                        "value": {
                            "nodeType": 1,
                            "localName": "span"
                        }
                    }
                ]
            })],
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::ByValue,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        };

        assert!(!devtools_call_function_deserializes_bidi_local_values(
            &command
        ));
        assert_eq!(
            devtools_call_function_cdp_arguments(&command, false),
            vec![json!({ "objectId": "COLLECTION-HANDLE" })]
        );
    }

    #[test]
    fn bidi_call_function_passes_remote_node_reference_as_remote_handle() {
        let command = DevToolsCallFunctionCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: None,
                target_id: None,
                browser_context_id: None,
            },
            realm_id: None,
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: "(node) => node.nodeType".to_owned(),
            arguments: vec![json!({
                "type": "node",
                "sharedId": "REMOTE-NODE",
                "value": {
                    "nodeType": 10
                }
            })],
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::ByValue,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        };

        assert!(!devtools_call_function_deserializes_bidi_local_values(
            &command
        ));
        assert_eq!(
            devtools_call_function_cdp_arguments(&command, false),
            vec![json!({ "objectId": "REMOTE-NODE" })]
        );
    }

    #[test]
    fn cdp_call_argument_prefers_shared_id_over_handle() {
        assert_eq!(
            cdp_call_argument_from_devtools_argument(json!({
                "handle": "HANDLE-1",
                "sharedId": "SHARED-1",
            })),
            json!({ "objectId": "SHARED-1" })
        );
    }

    #[test]
    fn runtime_probe_completion_scope_restores_on_drop() {
        let mut conn = CdpConnection::new();
        let previous_route = Some(CdpSessionRoute::Browser);
        conn.replace_none_session_owner_route_override(previous_route.clone());

        let target_route = CdpSessionRoute::ActiveTarget {
            browser_context_id: "BID-active".to_owned(),
            target_id: Some("TID-active".to_owned()),
        };
        {
            let mut scope = RuntimeProbeCompletionScope::enter(&mut conn, target_route.clone());
            assert_eq!(
                scope.conn_mut().none_session_owner_route_override(),
                Some(target_route)
            );
        }

        assert_eq!(conn.none_session_owner_route_override(), previous_route);
    }

    #[test]
    fn none_session_owner_route_override_scope_restores_on_drop() {
        let mut conn = CdpConnection::new();
        let previous_route = Some(CdpSessionRoute::Browser);
        conn.replace_none_session_owner_route_override(previous_route.clone());

        let target_route = CdpSessionRoute::ActiveTarget {
            browser_context_id: "BID-active".to_owned(),
            target_id: Some("TID-active".to_owned()),
        };
        {
            let mut scope = conn.scoped_none_session_owner_route_override(target_route.clone());
            assert_eq!(
                scope.conn_mut().none_session_owner_route_override(),
                Some(target_route)
            );
        }

        assert_eq!(conn.none_session_owner_route_override(), previous_route);
    }

    #[test]
    fn devtools_runtime_entry_routes_evaluate_command_to_inspector_error_plan() {
        let mut conn = CdpConnection::new();
        let params = json!({"expression": "1 + 1"});
        let cmd = Cmd::for_test(
            Some(15),
            "Runtime.evaluate",
            &params,
            Some("SID-eval"),
            r#"{"id":15,"method":"Runtime.evaluate"}"#,
        );
        let command = build_cdp_evaluate_script_command(&cmd, None, None, false);
        let step = start_devtools_runtime_command(
            &mut conn,
            &cmd,
            DevToolsCommand::EvaluateScript(command),
            cmd.json.to_owned(),
            false,
            RendererInspectorResponseDelivery::CommandReply,
        );

        let RuntimeCommandTaskStep::Complete(plan) = step else {
            panic!("Runtime.evaluate without a loaded target should complete with an error plan");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(15));
        assert!(out[0]["error"].is_object());
    }

    #[test]
    fn duplicate_pending_runtime_id_returns_chromium_error_without_replacing_owner() {
        let mut conn = CdpConnection::new();
        let mut browser_context = BrowserContext::new("BID-duplicate".to_owned());
        browser_context.set_active_target_id("TID-duplicate".to_owned());
        browser_context.attach_active_session("SID-duplicate".to_owned());
        conn.browser_context = Some(browser_context);
        conn.try_register_pending_inspector_await_with_object_group(
            77,
            Some("SID-duplicate"),
            Some("original-group"),
        )
        .unwrap();

        let params = json!({
            "expression": "new Promise(() => {})",
            "awaitPromise": true,
        });
        let cmd = Cmd::for_test(
            Some(77),
            "Runtime.evaluate",
            &params,
            Some("SID-duplicate"),
            r#"{"id":77,"method":"Runtime.evaluate","params":{"expression":"new Promise(() => {})","awaitPromise":true},"sessionId":"SID-duplicate"}"#,
        );
        let command = build_cdp_evaluate_script_command(&cmd, None, None, true);
        let step = start_devtools_runtime_command(
            &mut conn,
            &cmd,
            DevToolsCommand::EvaluateScript(command),
            cmd.json.to_owned(),
            true,
            RendererInspectorResponseDelivery::CommandReply,
        );

        let RuntimeCommandTaskStep::Complete(plan) = step else {
            panic!("duplicate pending frontend id must fail before renderer dispatch");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(
            out,
            vec![json!({
                "id": 77,
                "error": {
                    "code": -32600,
                    "message": "Duplicate `id` in protocol request",
                },
                "sessionId": "SID-duplicate",
            })]
        );
        assert!(
            conn.has_pending_inspector_awaits_for_session_owner(Some("SID-duplicate")),
            "duplicate registration must leave the original completion owner intact"
        );
    }

    #[tokio::test]
    async fn duplicate_non_await_v8_command_preserves_original_completion_owner() {
        let mut ctx = TestContext::new();
        let mut browser_context = BrowserContext::new("BID-console-duplicate".to_owned());
        browser_context.set_active_target_id("TID-console-duplicate".to_owned());
        browser_context.attach_active_session("SID-console-duplicate".to_owned());
        ctx.conn.browser_context = Some(browser_context);
        let page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<p>console duplicate</p>")
            .await
            .expect("page should load");
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .active_target
            .runtime_slot
            .set_loaded_page_for_test(page);

        let params = json!({});
        let enable = Cmd::for_test(
            Some(78),
            "Console.enable",
            &params,
            Some("SID-console-duplicate"),
            r#"{"id":78,"method":"Console.enable","sessionId":"SID-console-duplicate"}"#,
        );
        let RuntimeCommandTaskStep::Pending(original) =
            start_console_inspector_command_dispatch(&mut ctx.conn, &enable, ConsoleAction::Enable)
        else {
            panic!("first Console command should own a renderer correlation");
        };

        let disable = Cmd::for_test(
            Some(78),
            "Console.disable",
            &params,
            Some("SID-console-duplicate"),
            r#"{"id":78,"method":"Console.disable","sessionId":"SID-console-duplicate"}"#,
        );
        let RuntimeCommandTaskStep::Complete(duplicate_plan) =
            start_console_inspector_command_dispatch(
                &mut ctx.conn,
                &disable,
                ConsoleAction::Disable,
            )
        else {
            panic!("duplicate Console command must fail before renderer dispatch");
        };
        let mut duplicate_output = Vec::new();
        duplicate_plan.emit_into(&mut duplicate_output, disable.id, disable.session_id);
        assert_eq!(
            duplicate_output,
            vec![json!({
                "id": 78,
                "error": {
                    "code": -32600,
                    "message": "Duplicate `id` in protocol request",
                },
                "sessionId": "SID-console-duplicate",
            })]
        );

        let completed = original.wait().await;
        let RuntimeCommandTaskStep::Complete(mut original_plan) =
            super::complete_pending_runtime_command(&mut ctx.conn, completed).await
        else {
            panic!("original Console command should still complete");
        };
        let predecessor = original_plan
            .take_renderer_output_predecessor()
            .expect("original Console command should publish a session response");
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
        let mut original_output = Vec::new();
        original_plan.emit_into(&mut original_output, enable.id, enable.session_id);
        original_output.push(ctx.take_response_by_id(78));
        assert_eq!(original_output.len(), 1);
        assert_eq!(original_output[0]["id"], json!(78));
        assert_eq!(
            original_output[0]["sessionId"],
            json!("SID-console-duplicate")
        );
        assert_eq!(original_output[0]["result"], json!({}));
    }

    #[test]
    fn shared_worker_runtime_disable_projection_waits_for_success() {
        let mut conn = CdpConnection::new();
        let mut browser_context = BrowserContext::new("BID-shared".to_owned());
        let mut target = SharedWorkerTargetState::new(
            RendererOwnerLocalHostId::new_for_testing(1),
            SharedWorkerInstanceId::from_u64(91),
            "TID-shared-worker".to_owned(),
            None,
            "https://example.test/shared-worker.js".to_owned(),
            "shared-worker".to_owned(),
        );
        target.attach_session("SID-shared-worker".to_owned());
        target.set_runtime_frontend_enabled("SID-shared-worker", true);
        target
            .record_runtime_execution_context_created_event(&worker_context_created_event(81_081));
        target.upsert_live_runtime_binding_definition(
            "SID-shared-worker",
            "workerBindingClearedOnDisable".to_owned(),
            None,
        );
        target.record_console_message(RendererSharedWorkerConsoleMessage {
            message: "warn: retained".to_owned(),
            args: Vec::new(),
            stack: None,
        });
        browser_context.insert_shared_worker_target(target);
        conn.browser_context = Some(browser_context);

        apply_shared_worker_runtime_completion_projection(
            &mut conn,
            Some("SID-shared-worker"),
            "disable",
            false,
        );

        assert_eq!(
            conn.shared_worker_target_for_session(Some("SID-shared-worker"))
                .expect("shared worker target should remain attached")
                .pending_runtime_console_messages("SID-shared-worker")
                .len(),
            1,
            "failed V8 Runtime.disable must not clear target-local Runtime replay state"
        );
        assert_eq!(
            conn.shared_worker_target_for_session(Some("SID-shared-worker"))
                .expect("shared worker target should remain attached")
                .runtime_bindings("SID-shared-worker")
                .len(),
            1,
            "failed V8 Runtime.disable must not clear target-local Runtime binding state"
        );

        apply_shared_worker_runtime_completion_projection(
            &mut conn,
            Some("SID-shared-worker"),
            "disable",
            true,
        );

        assert!(
            conn.shared_worker_target_for_session(Some("SID-shared-worker"))
                .expect("shared worker target should remain attached")
                .pending_runtime_console_messages("SID-shared-worker")
                .is_empty(),
            "successful V8 Runtime.disable should clear target-local Runtime replay state"
        );
        assert!(
            conn.shared_worker_target_for_session(Some("SID-shared-worker"))
                .expect("shared worker target should remain attached")
                .runtime_bindings("SID-shared-worker")
                .is_empty(),
            "successful V8 Runtime.disable should clear target-local Runtime binding state"
        );
    }
}
