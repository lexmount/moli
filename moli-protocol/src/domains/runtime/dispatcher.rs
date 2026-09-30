use std::collections::HashSet;

use serde_json::{Map, Value, json};

use crate::automation::{
    AutomationCommand, AutomationContext, AutomationResult, DevToolsBidiChannelProperties,
    DevToolsCallFunctionCommand, DevToolsDomNodeReference, DevToolsError, DevToolsErrorKind,
    DevToolsEvaluateScriptCommand, DevToolsGetFrameOwnerCommand, DevToolsGetRealmsCommand,
    DevToolsGetRealmsResult, DevToolsLocateNodesCommand, DevToolsLocateNodesLocator,
    DevToolsLocateNodesResult, DevToolsLocateNodesTextMatch, DevToolsRealmId,
    DevToolsReleaseObjectsCommand, DevToolsRemoteHandleId, DevToolsRemoteValue,
    DevToolsResolveNodeCommand, DevToolsResultOwnership, DevToolsScriptException,
    DevToolsScriptResult, DevToolsSerializationOptions, DevToolsTargetId, FrontendProtocol,
    RuntimeExecutionContextEvent, is_webdriver_bidi_node_shared_id,
    webdriver_bidi_node_shared_id_for_backend_node_id,
};
use moli_core::page::{
    BidiPreloadChannelHandoff, DocumentNodeSnapshot, MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH,
    RendererCommandTurnOutput, RendererDomBidiNodeBindingResolution, RendererRuntimeCommandOutput,
    RendererRuntimeInspectorMessage,
};
use moli_page_types::RendererInspectorResponseDelivery;

use crate::conn::{
    AutomationCommandDispatchOutcome, AutomationExecutionOutput, BackgroundCommandResponsePayload,
    BackgroundCommandResponsePayloadRef, BackgroundProtocolEvent, BidiChannelListenerResidence,
    BidiChannelOwnerAction, BidiChannelPageOwner, CdpConnection, CdpRendererCommandAccess,
    CdpRendererCommandPolicy, CdpSchedulerEvent, CdpSessionRoute, ClaimedPendingInspectorAwait,
    Cmd, CommandOwnerScope, CompletedMoliDiagnosticsDispatch,
    CompletedRuntimeBindingPageCommandDispatch, CompletedRuntimeChildDefaultContextLookupDispatch,
    CompletedRuntimeEnableEventsDispatch, CompletedRuntimeProtocolMessageDispatch,
    CompletedServiceWorkerRuntimeProtocolMessageDispatch,
    CompletedSharedWorkerRuntimeProtocolMessageDispatch, DuplicatePendingRendererCommand,
    InspectorCommandDispatch, ParsedCdpCommand, PendingBidiChannelListener,
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
use crate::domains::console::{
    apply_console_output_state_for_owner, apply_console_output_state_for_session,
};
use crate::domains::observable_output::{
    advance_runtime_observable_cursors_to_current_for_owner,
    advance_runtime_observable_cursors_to_current_for_session_owner,
    runtime_console_api_called_background_event, runtime_console_message_type_and_text,
    runtime_exception_thrown_background_event,
};
use crate::domains::runtime_context_events::{
    RuntimeContextProtocolEvent, apply_runtime_context_protocol_event_side_effects_for_owner_typed,
    emit_runtime_context_protocol_background_event_typed,
    should_emit_child_default_context_inventory_replay_once_for_owner,
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
        AddBindingParams, clear_runtime_binding_definitions_for_owner,
        persist_runtime_binding_definition_for_owner, remove_runtime_binding_definitions_for_owner,
    },
    command_classification::{
        MainRuntimeCommand, MainRuntimeInspectorCommand, RuntimeBindingCommand,
        RuntimeDevToolsScriptCommand, RuntimeInspectorPayloadPreparation, WorkerRuntimeCommand,
        WorkerRuntimeCommandKind,
    },
    evaluate::can_dispatch,
};

mod bidi_channels;
mod bidi_script_values;
mod dispatch;
mod locate_nodes;
mod node_values;
mod remote_values;
mod runtime_command_flow;
mod runtime_lifecycle;
mod runtime_realms;
mod runtime_targets;
mod worker_runtime;

use self::bidi_channels::*;
use self::bidi_script_values::*;
use self::dispatch::*;
use self::locate_nodes::*;
use self::node_values::*;
use self::remote_values::*;
use self::runtime_command_flow::*;
use self::runtime_lifecycle::*;
use self::runtime_realms::*;
use self::runtime_targets::*;
use self::worker_runtime::*;

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
    command_context: AutomationContext,
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
    Complete(Box<AutomationCommandDispatchOutcome>),
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

enum PendingRuntimeCommandKind {
    Inspector {
        pending: PendingRuntimeProtocolMessageDispatch,
    },
    InspectorDeferredReply {
        routed_output: RuntimeInspectorRoutedOutput,
        renderer_response_rx: Option<RuntimeInspectorAsyncCompletionReceiver>,
        claimed_await: Option<ClaimedPendingInspectorAwait>,
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
        owner: &CommandOwnerScope,
        object_group: Option<&str>,
    ) {
        let Some(object_group) = object_group else {
            return;
        };
        for event in &self.events {
            if let Some((_, _, BackgroundCommandResponsePayloadRef::Success { result })) =
                event.command_response_payload_ref()
            {
                conn.register_runtime_remote_object_ids_from_value_for_owner_with_group(
                    owner,
                    result,
                    object_group,
                );
            } else if let Some(message) = event.protocol_message() {
                conn.register_runtime_remote_object_ids_from_value_for_owner_with_group(
                    owner,
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
        if let Some(predecessor) = self.renderer_output_predecessor {
            plan.set_renderer_output_predecessor(predecessor);
        }
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
    response_delivery: RendererInspectorResponseDelivery,
    session_response_predecessor: Option<moli_core::RendererOutputFence>,
    session_response_succeeded: Option<bool>,
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

    #[cfg(test)]
    pub(crate) fn owner_scope(&self) -> &CommandOwnerScope {
        &self.owner_scope
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

    pub(crate) fn append_scheduler_deferred_inspector_reply_output(
        &mut self,
        response_events: Vec<BackgroundProtocolEvent>,
        events: Vec<BackgroundProtocolEvent>,
    ) {
        let PendingRuntimeCommandKind::InspectorDeferredReply {
            routed_output,
            renderer_response_rx: _,
            claimed_await: _,
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
        let mut response_events = Vec::new();
        let mut background_events = Vec::new();
        let (routed, renderer_output_predecessor) = conn
            .route_scheduler_deferred_runtime_inspector_response_into(
                response,
                &owner_scope,
                &mut response_events,
                &mut background_events,
            )
            .await;
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
        let routed = self
            .route_scheduler_deferred_inspector_response(
                conn,
                crate::conn::RuntimeInspectorResponseReady::for_owner(
                    command_id,
                    &self.owner_scope,
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
            } => {
                conn.complete_claimed_pending_inspector_await_for_scheduler_deferred_reply(
                    claimed_await,
                    routed_output.events(),
                );
                CompletedRuntimeCommandKind::InspectorDeferredReplyReady { routed_output }
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
        let owner_scope = self.owner_scope.clone();
        match self.pending {
            PendingRuntimeCommandKind::InspectorDeferredReply { claimed_await, .. } => {
                conn.cancel_claimed_pending_inspector_await_for_scheduler_deferred_reply(
                    claimed_await,
                    "forgotten",
                );
            }
            _ => {
                if let Some(command_id) = self.command_id {
                    conn.forget_pending_inspector_await_for_owner(command_id, &owner_scope);
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
                } => CompletedRuntimeCommandKind::InspectorDeferredReplyReady { routed_output },
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
}

/// Worker Emulation methods are implemented beside the native Navigator, not
/// by V8's Inspector. Use the same session transport and completion routing.
pub(crate) fn try_start_worker_emulation_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<RuntimeCommandTaskStep> {
    let action = match cmd.method {
        "Emulation.setHardwareConcurrencyOverride" => "Emulation.setHardwareConcurrencyOverride",
        "Emulation.setDataSaverOverride" => "Emulation.setDataSaverOverride",
        "Emulation.setAutomationOverride" => "Emulation.setAutomationOverride",
        "Emulation.setUserAgentOverride" | "Network.setUserAgentOverride" => {
            "Emulation.setUserAgentOverride"
        }
        _ => return None,
    };
    let pending = match conn.session_route(cmd.session_id) {
        Some(
            CdpSessionRoute::DedicatedWorkerTarget { .. }
            | CdpSessionRoute::SharedWorkerTarget { .. },
        ) => start_shared_worker_frontend_inspector_dispatch(
            conn,
            cmd,
            cmd.json.to_owned(),
            cmd.terminal_response_delivery(),
        )
        .map(|pending| PendingRuntimeCommandKind::SharedWorkerInspector {
            pending,
            binding_effect: None,
        }),
        Some(CdpSessionRoute::ServiceWorkerTarget { .. }) => {
            start_service_worker_frontend_inspector_dispatch(
                conn,
                cmd,
                cmd.json.to_owned(),
                cmd.terminal_response_delivery(),
            )
            .map(|pending| PendingRuntimeCommandKind::ServiceWorkerInspector { pending })
        }
        _ => return None,
    };
    Some(match pending {
        Ok(pending) => RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
            command_id: cmd.id,
            action,
            owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
            object_group: None,
            release_object_ids: Vec::new(),
            release_object_group: None,
            await_promise: false,
            wait_for_deferred_reply: false,
            pending,
        })),
        Err(message) => RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message)),
    })
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
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let pending = match start_pending_runtime_inspector_dispatch_with_delivery(
        conn,
        cmd,
        &owner_scope,
        dispatch.into_inspector_json(),
        cmd.terminal_response_delivery(),
    ) {
        Ok(pending) => pending,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action,
        owner_scope,
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
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let pending = match start_pending_runtime_inspector_dispatch_with_delivery(
        conn,
        cmd,
        &owner_scope,
        cmd.json.to_owned(),
        cmd.terminal_response_delivery(),
    ) {
        Ok(pending) => pending,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: method,
        owner_scope,
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

    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let pending = match cmd.renderer_policy().access() {
        CdpRendererCommandAccess::MainThread => {
            start_pending_runtime_inspector_dispatch_with_delivery(
                conn,
                cmd,
                &owner_scope,
                inspector_json,
                cmd.terminal_response_delivery(),
            )
        }
        CdpRendererCommandAccess::Io => start_pending_runtime_io_inspector_dispatch(
            conn,
            cmd,
            &owner_scope,
            inspector_json,
            cmd.terminal_response_delivery(),
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
        owner_scope,
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::Inspector { pending },
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

    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let pending = match start_pending_runtime_inspector_dispatch_with_delivery(
        conn,
        cmd,
        &owner_scope,
        cmd.json.to_owned(),
        cmd.terminal_response_delivery(),
    ) {
        Ok(pending) => pending,
        Err(message) => {
            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message));
        }
    };

    RuntimeCommandTaskStep::Pending(Box::new(PendingRuntimeCommandDispatch {
        command_id: cmd.id,
        action: console_action_protocol_method(action),
        owner_scope,
        object_group: None,
        release_object_ids: Vec::new(),
        release_object_group: None,
        await_promise: false,
        wait_for_deferred_reply: false,
        pending: PendingRuntimeCommandKind::Inspector { pending },
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

pub(crate) async fn execute_devtools_runtime_command_async_with_protocol_events(
    conn: &mut CdpConnection,
    mut command: AutomationCommand,
) -> AutomationExecutionOutput {
    if let AutomationCommand::GetRealms(command) = command {
        return AutomationExecutionOutput::new(
            execute_devtools_get_realms_command_async(conn, command).await,
        );
    }
    if let AutomationCommand::ReleaseObjects(command) = command {
        return AutomationExecutionOutput::new(
            execute_devtools_release_objects_command_async(conn, command).await,
        );
    }
    if let AutomationCommand::LocateNodes(command) = command {
        return execute_devtools_locate_nodes_command_async(conn, command).await;
    }
    let result_kind = devtools_runtime_command_result_kind(&command);
    let result_ownership = devtools_runtime_result_ownership(&command);
    let serialization_options = devtools_runtime_serialization_options(&command);
    let target = match devtools_runtime_target_async(conn, &command).await {
        Ok(target) => target,
        Err(error) => return AutomationExecutionOutput::new(Err(error)),
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
    let target_owner = CommandOwnerScope::for_route(target.route.clone());
    if let AutomationCommand::CallFunction(call_function) = &mut command
        && matches!(
            call_function.context.protocol,
            FrontendProtocol::WebDriverBidi
        )
        && let Err(error) = remap_bidi_node_shared_references_for_target_async(
            conn,
            &target,
            call_function,
            target_realm.as_ref(),
        )
        .await
    {
        return AutomationExecutionOutput::new(Err(error));
    }
    let validation_result = validate_protocol_neutral_runtime_handle_realms(
        conn,
        &target_owner,
        &command,
        target_realm.as_ref(),
    );
    if let Err(error) = validation_result {
        return AutomationExecutionOutput::new(Err(error));
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
                    return AutomationExecutionOutput::from_parts(
                        Err(DevToolsError::new(
                            DevToolsErrorKind::Internal,
                            "MissingDevToolsCommandResult",
                        )),
                        protocol_events,
                        renderer_output_predecessor,
                    );
                };
                if result_kind == DevToolsRuntimeCommandResultKind::Empty {
                    return AutomationExecutionOutput::from_parts(
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
                        return AutomationExecutionOutput::from_parts(
                            Err(error),
                            protocol_events,
                            renderer_output_predecessor,
                        );
                    }
                };
                register_devtools_script_result_remote_object(conn, &target_owner, &result);
                materialize_devtools_script_dom_collection_remote_value_async(
                    conn,
                    &mut result,
                    serialization_options.as_ref(),
                    &target,
                    target_realm.as_ref(),
                )
                .await;
                materialize_devtools_script_deep_serialized_root_value_async(
                    conn,
                    &mut result,
                    serialization_options.as_ref(),
                    &target,
                )
                .await;
                materialize_devtools_script_node_remote_value_async(
                    conn,
                    &mut result,
                    serialization_options.as_ref(),
                    &target,
                    target_realm.as_ref(),
                )
                .await;
                materialize_devtools_script_deep_serialized_node_remote_values_async(
                    conn,
                    &mut result,
                    serialization_options.as_ref(),
                    &target,
                    target_realm.as_ref(),
                )
                .await;
                materialize_devtools_script_window_remote_value(&mut result, &target);
                register_devtools_script_result_remote_object_realm(
                    conn,
                    &target_owner,
                    &result,
                    target_realm.as_ref(),
                );
                return AutomationExecutionOutput::from_parts(
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
                        return AutomationExecutionOutput::from_parts(
                            Err(DevToolsError::new(DevToolsErrorKind::Internal, message)),
                            Vec::new(),
                            None,
                        );
                    }
                    pending.complete_scheduler_deferred_inspector_reply(conn)
                } else {
                    pending.wait().await
                };
                step = complete_pending_runtime_command(conn, completed).await;
            }
        }
    }
}

impl CdpConnection {
    pub async fn start_devtools_runtime_command_dispatch(
        &mut self,
        mut command: AutomationCommand,
    ) -> DevToolsRuntimeCommandTaskStep {
        let command_context = command.context().clone();
        if let AutomationCommand::GetRealms(command) = command {
            let result = execute_devtools_get_realms_command_async(self, command).await;
            return self
                .complete_devtools_runtime_direct_result(command_context, result, Vec::new(), None)
                .await;
        }
        if let AutomationCommand::ReleaseObjects(command) = command {
            let result = execute_devtools_release_objects_command_async(self, command).await;
            return self
                .complete_devtools_runtime_direct_result(command_context, result, Vec::new(), None)
                .await;
        }
        if let AutomationCommand::LocateNodes(command) = command {
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
        let target_owner = CommandOwnerScope::for_route(target.route.clone());
        if let AutomationCommand::CallFunction(call_function) = &mut command
            && matches!(
                call_function.context.protocol,
                FrontendProtocol::WebDriverBidi
            )
            && let Err(error) = remap_bidi_node_shared_references_for_target_async(
                self,
                &target,
                call_function,
                target_realm.as_ref(),
            )
            .await
        {
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
            self,
            &target_owner,
            &command,
            target_realm.as_ref(),
        );
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
        let step = complete_pending_runtime_command(self, completed.completed).await;
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
        let target_owner = CommandOwnerScope::for_route(state.target.route.clone());
        register_devtools_script_result_remote_object(self, &target_owner, &result);
        materialize_devtools_script_dom_collection_remote_value_async(
            self,
            &mut result,
            state.serialization_options.as_ref(),
            &state.target,
            state.target_realm.as_ref(),
        )
        .await;
        materialize_devtools_script_deep_serialized_root_value_async(
            self,
            &mut result,
            state.serialization_options.as_ref(),
            &state.target,
        )
        .await;
        materialize_devtools_script_node_remote_value_async(
            self,
            &mut result,
            state.serialization_options.as_ref(),
            &state.target,
            state.target_realm.as_ref(),
        )
        .await;
        materialize_devtools_script_deep_serialized_node_remote_values_async(
            self,
            &mut result,
            state.serialization_options.as_ref(),
            &state.target,
            state.target_realm.as_ref(),
        )
        .await;
        materialize_devtools_script_window_remote_value(&mut result, &state.target);
        register_devtools_script_result_remote_object_realm(
            self,
            &target_owner,
            &result,
            state.target_realm.as_ref(),
        );
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
        command_context: AutomationContext,
        result: Result<AutomationResult, DevToolsError>,
        protocol_events: Vec<BackgroundProtocolEvent>,
        renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
    ) -> DevToolsRuntimeCommandTaskStep {
        DevToolsRuntimeCommandTaskStep::Complete(Box::new(
            self.finish_automation_command_dispatch(
                command_context,
                result,
                protocol_events,
                renderer_output_predecessor,
            )
            .await,
        ))
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

pub(crate) async fn start_bidi_preload_channel_listeners_for_execution_context_background_events_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    execution_context_id: i64,
    out: &mut Vec<BackgroundProtocolEvent>,
) {
    let session_id = owner.session_id();
    let handoff_owners = conn.target_owner_bidi_channel_preload_handoffs_for_owner(owner);
    if handoff_owners.is_empty() {
        return;
    }
    let target_id = conn
        .target_owner_identity_for_owner(owner)
        .and_then(|(_, target_id)| target_id)
        .map(DevToolsTargetId::from);
    let route = owner.resolve_route(conn).or_else(|| {
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
    let Some(listener_owner) = bidi_channel_page_owner_for_runtime_target(conn, &target, owner)
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

struct BidiLocalValueDescriptorContext<'a> {
    remote_object_ids: &'a mut Vec<String>,
    preload_channel_handoffs: Option<&'a mut Vec<BidiPreloadChannelHandoff>>,
    preload_channel_error: Option<String>,
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

enum BidiWindowRemoteResult {
    TargetWindow,
    Context(DevToolsTargetId),
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

pub(crate) async fn execute_runtime_listener_command_for_owner(
    conn: &mut CdpConnection,
    owner: CommandOwnerScope,
    enabled: bool,
) -> CommandOutputPlan {
    let mut step = if enabled {
        start_runtime_enable_command_for_owner(conn, Some(0), owner)
    } else {
        let raw = r#"{"id":0,"method":"Runtime.disable","params":{}}"#;
        let parsed = ParsedCdpCommand::parse_str(raw.to_owned())
            .expect("internal Runtime.disable command must be valid CDP");
        let cmd = Cmd::from_parsed(&parsed)
            .expect("internal Runtime.disable command must produce a command view");
        start_runtime_disable_command_for_owner(
            conn,
            &cmd,
            owner,
            RendererInspectorResponseDelivery::AdapterReply,
        )
    };
    loop {
        match step {
            RuntimeCommandTaskStep::Complete(plan) => return plan,
            RuntimeCommandTaskStep::Pending(pending) => {
                step = complete_pending_runtime_command(conn, pending.wait().await).await;
            }
        }
    }
}

#[cfg(test)]
mod protocol_neutral_tests {
    use crate::automation::{
        AutomationCommand, AutomationContext, DevToolsCallFunctionCommand, DevToolsResultOwnership,
        FrontendProtocol, RuntimeExecutionContextEvent,
    };
    use moli_core::RendererOwnerLocalHostId;
    use moli_core::page::{MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH, RendererSharedWorkerConsoleMessage};
    use moli_page_types::RendererInspectorResponseDelivery;
    use moli_shared_worker::SharedWorkerInstanceId;
    use serde_json::{Value, json};

    use crate::conn::{
        BrowserContext, CdpConnection, CdpSessionRoute, Cmd, CommandOwnerScope,
        SharedWorkerTargetState,
    };
    use crate::domains::actions::ConsoleAction;
    use crate::testing::TestContext;

    use super::super::bidi_nodes::{
        BidiIncludeShadowTree, BidiNodeSerializationOptions,
        devtools_serialization_options_for_node_probe,
    };
    use super::{
        DevToolsRuntimeTarget, RuntimeCommandTaskStep,
        apply_shared_worker_runtime_completion_projection,
        automation_command_has_bidi_script_channel_arguments, build_cdp_call_function_command,
        build_cdp_evaluate_script_command, cdp_call_argument_from_devtools_argument,
        devtools_call_function_cdp_arguments, devtools_call_function_declaration,
        devtools_call_function_deserializes_bidi_local_values, locate_nodes_error_from_exception,
        materialize_devtools_script_window_remote_value, start_console_inspector_command_dispatch,
        start_devtools_runtime_command,
    };
    use crate::automation::{
        AutomationResult, DevToolsErrorKind, DevToolsLocateNodesLocator, DevToolsRemoteHandleId,
        DevToolsRemoteValue, DevToolsScriptException, DevToolsScriptResult, DevToolsTargetId,
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

        assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
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
            AutomationResult::Script(Box::new(DevToolsScriptResult::Value(shape_only)));

        materialize_devtools_script_window_remote_value(&mut shape_only_result, &target);

        let AutomationResult::Script(result) = shape_only_result else {
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
            AutomationResult::Script(Box::new(DevToolsScriptResult::Value(marked_window)));

        materialize_devtools_script_window_remote_value(&mut marked_result, &target);

        let AutomationResult::Script(result) = marked_result else {
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

        assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
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
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
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
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
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
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
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
        assert!(automation_command_has_bidi_script_channel_arguments(
            &AutomationCommand::CallFunction(command.clone())
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
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
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
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
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
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
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
            AutomationCommand::EvaluateScript(command),
            cmd.json.to_owned(),
            false,
            RendererInspectorResponseDelivery::AdapterReply,
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
        conn.install_browser_context_fixture_for_test(browser_context);
        conn.try_register_pending_inspector_await_with_object_group_for_owner(
            77,
            &CommandOwnerScope::for_session("SID-duplicate"),
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
            AutomationCommand::EvaluateScript(command),
            cmd.json.to_owned(),
            true,
            RendererInspectorResponseDelivery::AdapterReply,
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
        ctx.conn
            .install_browser_context_fixture_for_test(browser_context);
        let page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<p>console duplicate</p>")
            .await
            .expect("page should load");
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .active_page_target_mut()
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
        conn.install_browser_context_fixture_for_test(browser_context);

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
