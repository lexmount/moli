use std::{collections::hash_map::Entry, future::Future, pin::Pin};

use moli_page_types::{FrontendCommandId, RendererCallId, RendererInspectorResponseDelivery};
use moli_protocol_cdp::CdpRendererCommandReplayDispatch;
use moli_shared_worker::SharedWorkerInstanceId;
use serde_json::{Map, Value, json};

use crate::devtools_runtime::{
    AutomationEvent, DevToolsFrameId, DevToolsRealmId, DevToolsRemoteValue,
    DevToolsResultOwnership, DevToolsTargetId, RuntimeExecutionContextEvent, ScriptMessageEvent,
};
use moli_core::{
    RendererOutputFence, RendererRuntimeCommandCausalIdentity,
    RendererRuntimeInspectorResponseSender,
    page::{
        DocumentNodeObjectSnapshot, DocumentNodeRuntimeObjectResolution,
        MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH, RendererAgentAttachmentId, RendererCommandTurnOutput,
        RendererDomBidiNodeBindingResolution, RendererDomBidiNodeSharedIdResolution,
        RendererInspectorCommandRoute, RendererRuntimeCommandOutput,
        RendererRuntimeInspectorMessage, RendererRuntimeRealmInfo,
    },
};

use crate::conn::state::{
    DevToolsSessionState, PreparedRendererCallTermination, SessionRendererCallReplay,
    SessionRendererCallTermination,
};
use crate::domains::command_output::protocol_message_background_event;
use crate::domains::runtime_context_events::{
    RuntimeContextProtocolEvent, apply_runtime_context_protocol_event_side_effects_for_owner_typed,
    emit_runtime_context_protocol_background_event_typed,
    qualify_runtime_context_protocol_event_for_owner_typed,
};

use super::*;

type RuntimeInspectorResponseReceiver = RuntimeInspectorAsyncCompletionReceiver;

const SHARED_WORKER_RUNTIME_REMOTE_OBJECT_CLEANUP_COMMAND_ID_BASE: u64 = 900_600_000;
const BIDI_SCRIPT_RESULT_OBJECT_GROUP: &str = "webdriver-bidi";
const BIDI_CHANNEL_OBJECT_GROUP_PREFIX: &str = "webdriver-bidi-channel-";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeBindingCallEvent {
    source: moli_core::page::RuntimeBindingCallSourceIdentity,
    name: String,
    payload: String,
    execution_context_id: i64,
}

impl RuntimeBindingCallEvent {
    pub(crate) fn from_renderer_call(call: moli_core::page::PendingRuntimeBindingCall) -> Self {
        Self {
            source: call.source,
            name: call.name,
            payload: call.payload,
            execution_context_id: call.execution_context_id,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        local_window_id: u64,
        realm_generation: u64,
        name: impl Into<String>,
        payload: impl Into<String>,
        execution_context_id: i64,
    ) -> Self {
        Self {
            source: moli_core::page::RuntimeBindingCallSourceIdentity::new(
                local_window_id,
                realm_generation,
            ),
            name: name.into(),
            payload: payload.into(),
            execution_context_id,
        }
    }

    #[cfg(test)]
    pub(crate) fn source(&self) -> moli_core::page::RuntimeBindingCallSourceIdentity {
        self.source
    }

    pub(crate) fn into_background_protocol_event(
        self,
        session_id: Option<&str>,
    ) -> BackgroundProtocolEvent {
        BackgroundProtocolEvent::runtime_binding_called(
            session_id,
            self.name,
            self.payload,
            self.execution_context_id,
        )
    }
}

#[cfg(test)]
fn runtime_protocol_message_id(raw_json: &str) -> Option<u64> {
    let message = serde_json::from_str::<Value>(raw_json).ok()?;
    match message.get("id")? {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().and_then(|id| u64::try_from(id).ok())),
        _ => None,
    }
}

fn rewrite_runtime_inspector_command_for_renderer(
    raw_json: &str,
    command_id_rewrite: Option<(FrontendCommandId, RendererCallId)>,
    owner_target_id: Option<&str>,
) -> Result<String, String> {
    let mut message = serde_json::from_str::<Value>(raw_json)
        .map_err(|error| format!("invalid runtime Inspector command JSON: {error}"))?;
    let Some(object) = message.as_object_mut() else {
        return Err("runtime Inspector command must be a JSON object".to_owned());
    };

    if let Some((frontend_command_id, renderer_call_id)) = command_id_rewrite {
        let wire_command_id = object.get("id").and_then(Value::as_u64);
        if wire_command_id != Some(frontend_command_id.get()) {
            return Err(format!(
                "runtime Inspector command id mismatch: expected {}, got {}",
                frontend_command_id.get(),
                object
                    .get("id")
                    .map(Value::to_string)
                    .unwrap_or_else(|| "missing".to_owned())
            ));
        }
        object.insert("id".to_owned(), json!(renderer_call_id.get()));
    }

    let targets_realm_by_unique_id = matches!(
        object.get("method").and_then(Value::as_str),
        Some("Runtime.evaluate" | "Runtime.callFunctionOn")
    );
    // External realm ids are target-qualified because native V8 unique ids are
    // only unique inside one renderer runtime. V8 Inspector accepts only the
    // native suffix, so undo the qualification at the owning renderer boundary.
    // An id owned by another target intentionally remains unmodified and V8
    // rejects it as an invalid uniqueContextId.
    if targets_realm_by_unique_id
        && let Some(owner_target_id) = owner_target_id
        && let Some(Value::String(unique_context_id)) = object
            .get_mut("params")
            .and_then(Value::as_object_mut)
            .and_then(|params| params.get_mut("uniqueContextId"))
    {
        let owner_prefix = format!("{owner_target_id}:");
        if let Some(native_realm_id) = unique_context_id.strip_prefix(&owner_prefix)
            && !native_realm_id.is_empty()
        {
            *unique_context_id = native_realm_id.to_owned();
        }
    }

    serde_json::to_string(&message)
        .map_err(|error| format!("failed to encode runtime Inspector command: {error}"))
}

pub(crate) fn renderer_command_turn_frontend_protocol_response(
    output: &RendererCommandTurnOutput,
    frontend_command_id: u64,
) -> Option<&Value> {
    output.runtime_inspector_output().and_then(|output| {
        runtime_inspector_frontend_response(output.messages(), frontend_command_id)
    })
}

fn runtime_inspector_frontend_response(
    messages: &[RendererRuntimeInspectorMessage],
    command_id: u64,
) -> Option<&Value> {
    messages.iter().find_map(|message| {
        let RendererRuntimeInspectorMessage::Protocol(message) = message else {
            return None;
        };
        (message.get("id").and_then(Value::as_u64) == Some(command_id)).then(|| message.value())
    })
}

#[derive(Debug)]
enum RuntimeRemoteObjectOwnerIdentity {
    Page {
        browser_context_id: String,
        target_id: Option<String>,
        devtools_session_id: Option<String>,
    },
    SharedWorker {
        browser_context_id: String,
        instance_id: SharedWorkerInstanceId,
        session_id: String,
    },
    DedicatedWorker {
        browser_context_id: String,
        instance_id: u64,
        session_id: String,
    },
    ServiceWorker {
        browser_context_id: String,
        version_id: u64,
        session_id: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SharedWorkerRuntimeTargetRoute {
    browser_context_id: String,
    worker: WorkerRuntimeTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum WorkerRuntimeTarget {
    Shared(SharedWorkerInstanceId),
    Dedicated(u64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ServiceWorkerRuntimeTargetRoute {
    browser_context_id: String,
    version_id: u64,
}

enum BidiChannelListenerRoute {
    NotListener,
    Consumed,
    Event(BackgroundProtocolEvent),
}

#[derive(Debug)]
pub(crate) struct OwnerRuntimeResponse {
    command_id: u64,
    session_id: Option<String>,
    owner_route: Option<CdpSessionRoute>,
    object_group: Option<String>,
    message: Value,
    bidi_channel_listener: Option<BidiChannelListenerResidence>,
}

#[derive(Debug)]
pub(crate) struct ClaimedPendingInspectorAwait {
    command_id: u64,
    owner: CommandOwnerScope,
    entry: PendingInspectorAwait,
}

#[derive(Debug)]
pub(crate) struct ClaimedPendingInspectorAwaitOwner {
    command_id: u64,
    owner: CommandOwnerScope,
    bidi_channel_object_group: Option<String>,
    renderer_correlation: Option<RendererCommandCorrelation>,
}

impl ClaimedPendingInspectorAwaitOwner {
    fn from_claimed(claimed: &ClaimedPendingInspectorAwait) -> Self {
        Self {
            command_id: claimed.command_id,
            owner: claimed.owner.clone(),
            bidi_channel_object_group: claimed
                .entry
                .bidi_channel_listener()
                .map(|listener| listener.channel_object_group().to_owned()),
            renderer_correlation: claimed.entry.renderer_correlation(),
        }
    }

    fn session_id(&self) -> Option<&str> {
        self.owner.session_id()
    }

    fn matches_session_owner(&self, session_id: Option<&str>) -> bool {
        self.session_id() == session_id
    }

    fn matches_owner(&self, owner: &CommandOwnerScope) -> bool {
        &self.owner == owner
    }
}

impl OwnerRuntimeResponse {
    fn from_pending_inspector_await(
        command_id: u64,
        entry: PendingInspectorAwait,
        owner_route: Option<CdpSessionRoute>,
        message: Value,
    ) -> Self {
        Self {
            command_id,
            session_id: entry.session_id().map(str::to_owned),
            owner_route,
            object_group: entry.object_group().map(str::to_owned),
            message,
            bidi_channel_listener: entry.bidi_channel_listener().cloned(),
        }
    }

    fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    fn owner_route(&self) -> Option<&CdpSessionRoute> {
        self.owner_route.as_ref()
    }

    fn object_group(&self) -> Option<&str> {
        self.object_group.as_deref()
    }

    fn bidi_channel_listener(&self) -> Option<&BidiChannelListenerResidence> {
        self.bidi_channel_listener.as_ref()
    }

    fn into_protocol_message(self) -> Value {
        self.message
    }
}

pub struct PendingRuntimeProtocolMessageDispatch {
    owner: CommandOwnerScope,
    route: RuntimeProtocolMessagePageRoute,
    pending: PendingRuntimeProtocolMessageDispatchKind,
    response_route: RuntimeProtocolResponseRoute,
}

enum RuntimeProtocolResponseRoute {
    // The receiver is consumed before renderer completion is projected. It is
    // absent for fire-and-forget commands and for replay, where the original
    // dispatch still owns the command-reply waiter.
    CommandReply(Option<RuntimeInspectorResponseReceiver>),
    DevToolsSession,
}

impl RuntimeProtocolResponseRoute {
    fn for_registered_delivery(
        delivery: RendererInspectorResponseDelivery,
        receiver: Option<RuntimeInspectorResponseReceiver>,
    ) -> Self {
        match delivery {
            RendererInspectorResponseDelivery::CommandReply => Self::CommandReply(Some(
                receiver.expect("a registered command-reply route must allocate its receiver"),
            )),
            RendererInspectorResponseDelivery::DevToolsSession => {
                assert!(
                    receiver.is_none(),
                    "a DevTools session response cannot retain a command-reply receiver"
                );
                Self::DevToolsSession
            }
        }
    }

    const fn without_local_receiver_for_delivery(
        delivery: RendererInspectorResponseDelivery,
    ) -> Self {
        match delivery {
            RendererInspectorResponseDelivery::CommandReply => Self::CommandReply(None),
            RendererInspectorResponseDelivery::DevToolsSession => Self::DevToolsSession,
        }
    }

    const fn command_reply_without_receiver() -> Self {
        Self::CommandReply(None)
    }

    const fn delivery(&self) -> RendererInspectorResponseDelivery {
        match self {
            Self::CommandReply(_) => RendererInspectorResponseDelivery::CommandReply,
            Self::DevToolsSession => RendererInspectorResponseDelivery::DevToolsSession,
        }
    }

    fn take_command_reply_receiver(&mut self) -> Option<RuntimeInspectorResponseReceiver> {
        match self {
            Self::CommandReply(receiver) => receiver.take(),
            Self::DevToolsSession => None,
        }
    }
}

enum PendingRuntimeProtocolMessageDispatchKind {
    Page(moli_core::page::PendingPageCommand),
    Routable(moli_core::page::PendingRuntimeInspectorCommandDispatch),
}

pub struct PendingSharedWorkerRuntimeProtocolMessageDispatch {
    session_id: Option<String>,
    pending: SharedWorkerRuntimeProtocolDispatchFuture,
    response_route: RuntimeProtocolResponseRoute,
}

pub struct PendingServiceWorkerRuntimeProtocolMessageDispatch {
    session_id: Option<String>,
    pending: ServiceWorkerRuntimeProtocolDispatchFuture,
    response_route: RuntimeProtocolResponseRoute,
}

pub struct PendingMoliDiagnosticsDispatch {
    pending: Vec<PendingMoliDiagnosticsPageSnapshot>,
}

struct PendingMoliDiagnosticsPageSnapshot {
    browser_context_id: String,
    target_id: Option<String>,
    pending: moli_core::page::PendingPageCommand,
}

pub struct PendingRuntimeEnableEventsDispatch {
    owner: CommandOwnerScope,
    route: RuntimeProtocolMessagePageRoute,
    pending: moli_core::page::PendingPageCommand,
}

pub struct PendingRuntimeBindingPageCommandDispatch {
    owner: CommandOwnerScope,
    operation: &'static str,
    pending: moli_core::page::PendingPageCommand,
}

pub struct PendingRuntimeChildDefaultContextLookupDispatch {
    owner: CommandOwnerScope,
    pending: moli_core::page::PendingPageCommand,
}

pub struct CompletedRuntimeProtocolMessageDispatch {
    owner: CommandOwnerScope,
    route: RuntimeProtocolMessagePageRoute,
    completion: moli_core::page::CompletedRuntimeInspectorCommandDispatch,
    response_route: RuntimeProtocolResponseRoute,
}

pub struct CompletedSharedWorkerRuntimeProtocolMessageDispatch {
    session_id: Option<String>,
    dispatch: CompletedWorkerRuntimeProtocolDispatch,
    response_route: RuntimeProtocolResponseRoute,
}

pub struct CompletedServiceWorkerRuntimeProtocolMessageDispatch {
    session_id: Option<String>,
    dispatch: CompletedWorkerRuntimeProtocolDispatch,
    response_route: RuntimeProtocolResponseRoute,
}

struct CompletedWorkerRuntimeProtocolDispatch {
    messages: Vec<RendererRuntimeInspectorMessage>,
    session_response_predecessor: Option<RendererOutputFence>,
    session_response_succeeded: Option<bool>,
}

impl CompletedWorkerRuntimeProtocolDispatch {
    fn command_reply(messages: Vec<RendererRuntimeInspectorMessage>) -> Self {
        Self {
            messages,
            session_response_predecessor: None,
            session_response_succeeded: None,
        }
    }

    fn devtools_session(
        completed: moli_core::page::CompletedWorkerRuntimeInspectorCommandDispatch,
    ) -> Self {
        let (messages, predecessor, response_succeeded) = completed.into_parts();
        Self {
            messages,
            session_response_predecessor: Some(predecessor),
            session_response_succeeded: Some(response_succeeded),
        }
    }
}

pub struct CompletedMoliDiagnosticsDispatch {
    completed: Vec<CompletedMoliDiagnosticsPageSnapshot>,
}

struct CompletedMoliDiagnosticsPageSnapshot {
    browser_context_id: String,
    target_id: Option<String>,
    completion: Result<moli_core::page::CompletedPageCommand, String>,
}

pub struct CompletedRuntimeEnableEventsDispatch {
    owner: CommandOwnerScope,
    route: RuntimeProtocolMessagePageRoute,
    completion: moli_core::page::CompletedPageCommand,
}

pub(crate) struct RuntimeEnableEventsReplay {
    events: Vec<RuntimeEnableReplayEvent>,
}

pub(crate) enum RuntimeEnableReplayEvent {
    Context(RuntimeContextProtocolEvent),
    Background(BackgroundProtocolEvent),
}

impl RuntimeEnableEventsReplay {
    fn from_renderer_messages(messages: Vec<RendererRuntimeInspectorMessage>) -> Self {
        Self {
            events: messages
                .into_iter()
                .map(RuntimeEnableReplayEvent::from_renderer_message)
                .collect(),
        }
    }

    pub(crate) fn into_events(self) -> Vec<RuntimeEnableReplayEvent> {
        self.events
    }

    fn events_mut(&mut self) -> &mut [RuntimeEnableReplayEvent] {
        &mut self.events
    }
}

impl RuntimeEnableReplayEvent {
    fn from_renderer_message(message: RendererRuntimeInspectorMessage) -> Self {
        match message {
            RendererRuntimeInspectorMessage::RuntimeContext(event) => {
                Self::Context(RuntimeContextProtocolEvent::from_restore_event(event))
            }
            RendererRuntimeInspectorMessage::Protocol(message) => {
                Self::Background(protocol_message_background_event(message.into_value()))
            }
        }
    }
}

pub struct CompletedRuntimeBindingPageCommandDispatch {
    owner: CommandOwnerScope,
    operation: &'static str,
    completion: moli_core::page::CompletedPageCommand,
}

pub struct CompletedRuntimeChildDefaultContextLookupDispatch {
    owner: CommandOwnerScope,
    completion: moli_core::page::CompletedPageCommand,
}

type SharedWorkerRuntimeProtocolDispatchFuture =
    Pin<Box<dyn Future<Output = Result<CompletedWorkerRuntimeProtocolDispatch, String>>>>;
type ServiceWorkerRuntimeProtocolDispatchFuture =
    Pin<Box<dyn Future<Output = Result<CompletedWorkerRuntimeProtocolDispatch, String>>>>;

#[derive(Clone, Debug)]
struct RuntimeProtocolMessagePageRoute {
    browser_context_id: String,
    target_id: Option<String>,
    renderer_agent_attachment_id: RendererAgentAttachmentId,
}

fn collect_moli_diagnostics_pending_snapshots(
    browser_context: &mut BrowserContext,
    pending: &mut Vec<PendingMoliDiagnosticsPageSnapshot>,
) -> Result<(), String> {
    if browser_context
        .active_page_target()
        .runtime_slot
        .has_loaded_page()
    {
        let pending_snapshot = browser_context
            .active_page_target()
            .runtime_slot
            .loaded_page()
            .expect("active target loaded page should exist")
            .start_page_diagnostics_snapshot()
            .map_err(|error| error.to_string())?;
        pending.push(PendingMoliDiagnosticsPageSnapshot {
            browser_context_id: browser_context.id.clone(),
            target_id: None,
            pending: pending_snapshot,
        });
    }

    let browser_context_id = browser_context.id.clone();
    for target in browser_context.background_targets_mut() {
        if !target.has_loaded_page() {
            continue;
        }
        let pending_snapshot = target
            .loaded_page()
            .expect("background target loaded page should exist")
            .start_page_diagnostics_snapshot()
            .map_err(|error| error.to_string())?;
        pending.push(PendingMoliDiagnosticsPageSnapshot {
            browser_context_id: browser_context_id.clone(),
            target_id: Some(target.target_id().to_owned()),
            pending: pending_snapshot,
        });
    }

    Ok(())
}

impl PendingRuntimeProtocolMessageDispatch {
    pub async fn wait(self) -> Result<CompletedRuntimeProtocolMessageDispatch, String> {
        let completion = match self.pending {
            PendingRuntimeProtocolMessageDispatchKind::Page(pending) => {
                moli_core::page::CompletedRuntimeInspectorCommandDispatch::Owner(Box::new(
                    pending
                        .wait()
                        .await
                        .map_err(|error| format!("runtime inspector dispatch failed: {error}"))?,
                ))
            }
            PendingRuntimeProtocolMessageDispatchKind::Routable(pending) => pending
                .wait()
                .await
                .map_err(|error| format!("runtime inspector dispatch failed: {error}"))?,
        };
        Ok(CompletedRuntimeProtocolMessageDispatch {
            owner: self.owner,
            route: self.route,
            completion,
            response_route: self.response_route,
        })
    }
}

impl PendingSharedWorkerRuntimeProtocolMessageDispatch {
    pub async fn wait(self) -> Result<CompletedSharedWorkerRuntimeProtocolMessageDispatch, String> {
        let dispatch = self.pending.await?;
        Ok(CompletedSharedWorkerRuntimeProtocolMessageDispatch {
            session_id: self.session_id,
            dispatch,
            response_route: self.response_route,
        })
    }
}

impl PendingServiceWorkerRuntimeProtocolMessageDispatch {
    pub async fn wait(
        self,
    ) -> Result<CompletedServiceWorkerRuntimeProtocolMessageDispatch, String> {
        let dispatch = self.pending.await?;
        Ok(CompletedServiceWorkerRuntimeProtocolMessageDispatch {
            session_id: self.session_id,
            dispatch,
            response_route: self.response_route,
        })
    }
}

impl CompletedRuntimeProtocolMessageDispatch {
    pub(crate) fn owner(&self) -> &CommandOwnerScope {
        &self.owner
    }

    pub(crate) fn session_response_succeeded(&self) -> Option<bool> {
        match &self.completion {
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::OwnerSessionResponse {
                response_succeeded,
                ..
            }
            | moli_core::page::CompletedRuntimeInspectorCommandDispatch::InspectorSessionResponse {
                response_succeeded,
                ..
            } => Some(*response_succeeded),
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::OwnerSessionErrorSettled(
                _,
            ) => Some(false),
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::Owner(_)
            | moli_core::page::CompletedRuntimeInspectorCommandDispatch::Inspector => None,
        }
    }

    pub(crate) fn session_response_predecessor(&self) -> Option<RendererOutputFence> {
        match &self.completion {
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::OwnerSessionResponse {
                completion,
                ..
            } => completion.renderer_output_predecessor(),
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::OwnerSessionErrorSettled(
                predecessor,
            ) => Some(predecessor.clone()),
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::InspectorSessionResponse {
                predecessor,
                ..
            } => Some(predecessor.clone()),
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::Owner(_)
            | moli_core::page::CompletedRuntimeInspectorCommandDispatch::Inspector => None,
        }
    }

    pub(crate) fn take_deferred_response_receiver(
        &mut self,
    ) -> Option<RuntimeInspectorResponseReceiver> {
        self.response_route.take_command_reply_receiver()
    }

    pub(crate) const fn response_delivery(&self) -> RendererInspectorResponseDelivery {
        self.response_route.delivery()
    }
}

impl CompletedSharedWorkerRuntimeProtocolMessageDispatch {
    pub(crate) fn take_deferred_response_receiver(
        &mut self,
    ) -> Option<RuntimeInspectorResponseReceiver> {
        self.response_route.take_command_reply_receiver()
    }

    pub(crate) const fn response_delivery(&self) -> RendererInspectorResponseDelivery {
        self.response_route.delivery()
    }

    pub(crate) fn session_response_predecessor(&self) -> Option<RendererOutputFence> {
        self.dispatch.session_response_predecessor.clone()
    }

    pub(crate) fn session_response_succeeded(&self) -> Option<bool> {
        self.dispatch.session_response_succeeded
    }
}

impl CompletedServiceWorkerRuntimeProtocolMessageDispatch {
    pub(crate) fn take_deferred_response_receiver(
        &mut self,
    ) -> Option<RuntimeInspectorResponseReceiver> {
        self.response_route.take_command_reply_receiver()
    }

    pub(crate) const fn response_delivery(&self) -> RendererInspectorResponseDelivery {
        self.response_route.delivery()
    }

    pub(crate) fn session_response_predecessor(&self) -> Option<RendererOutputFence> {
        self.dispatch.session_response_predecessor.clone()
    }

    pub(crate) fn session_response_succeeded(&self) -> Option<bool> {
        self.dispatch.session_response_succeeded
    }
}

impl PendingMoliDiagnosticsDispatch {
    pub async fn wait(self) -> Result<CompletedMoliDiagnosticsDispatch, String> {
        let mut completed = Vec::with_capacity(self.pending.len());
        for pending in self.pending {
            completed.push(CompletedMoliDiagnosticsPageSnapshot {
                browser_context_id: pending.browser_context_id,
                target_id: pending.target_id,
                completion: pending
                    .pending
                    .wait()
                    .await
                    .map_err(|error| format!("moli diagnostics snapshot failed: {error}")),
            });
        }
        Ok(CompletedMoliDiagnosticsDispatch { completed })
    }
}

impl PendingRuntimeEnableEventsDispatch {
    pub async fn wait(self) -> Result<CompletedRuntimeEnableEventsDispatch, String> {
        let completion = self
            .pending
            .wait()
            .await
            .map_err(|error| format!("runtime enable event replay failed: {error}"))?;
        Ok(CompletedRuntimeEnableEventsDispatch {
            owner: self.owner,
            route: self.route,
            completion,
        })
    }
}

impl PendingRuntimeBindingPageCommandDispatch {
    pub async fn wait(self) -> Result<CompletedRuntimeBindingPageCommandDispatch, String> {
        let completion = self
            .pending
            .wait()
            .await
            .map_err(|error| format!("{} failed: {error}", self.operation))?;
        Ok(CompletedRuntimeBindingPageCommandDispatch {
            owner: self.owner,
            operation: self.operation,
            completion,
        })
    }
}

impl PendingRuntimeChildDefaultContextLookupDispatch {
    pub async fn wait(self) -> Result<CompletedRuntimeChildDefaultContextLookupDispatch, String> {
        let completion = self
            .pending
            .wait()
            .await
            .map_err(|error| format!("runtime child default context lookup failed: {error}"))?;
        Ok(CompletedRuntimeChildDefaultContextLookupDispatch {
            owner: self.owner,
            completion,
        })
    }
}

fn push_pending_inspector_await_error_background_event(
    out: &mut Vec<BackgroundProtocolEvent>,
    cdp_id: u64,
    session_id: Option<&str>,
    reason: &'static str,
) {
    out.push(BackgroundProtocolEvent::command_error(
        Some(cdp_id),
        session_id,
        -32000,
        reason.to_owned(),
        None,
    ));
}

fn push_terminated_renderer_call_error_background_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    terminated: Vec<RendererCommandCorrelation>,
    session_id: Option<&str>,
    reason: &'static str,
) {
    out.extend(terminated.into_iter().map(|correlation| {
        BackgroundProtocolEvent::command_error(
            Some(correlation.frontend_command_id().get()),
            session_id,
            -32000,
            reason.to_owned(),
            None,
        )
    }));
}

fn bidi_channel_listener_call_function_json(
    command_id: u64,
    listener: &PendingBidiChannelListener,
) -> String {
    let serialization_options = listener
        .properties()
        .serialization_options
        .as_ref()
        .map(crate::domains::runtime::devtools_deep_serialization_options_json)
        .unwrap_or_else(|| {
            json!({
                "serialization": "deep",
            })
        });
    let params = json!({
        "functionDeclaration": "(async function() { return await this.getMessage(); })",
        "objectId": listener.channel_handle().as_str(),
        "awaitPromise": true,
        "returnByValue": !matches!(listener.properties().ownership, DevToolsResultOwnership::Root),
        "objectGroup": BIDI_SCRIPT_RESULT_OBJECT_GROUP,
        "serializationOptions": serialization_options,
    });
    json!({
        "id": command_id,
        "method": "Runtime.callFunctionOn",
        "params": params,
    })
    .to_string()
}

impl CdpConnection {
    /// Registers a CDP request as awaiting a deferred V8 inspector reply.
    /// Used by `Runtime.evaluate`/`Runtime.callFunctionOn` when `awaitPromise=true`
    /// is dispatched directly to V8 inspector. V8 calls back after the promise
    /// settles, and the reply is routed via [`Self::route_inspector_messages_into`].
    #[cfg(test)]
    pub(crate) fn register_pending_inspector_await(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.try_register_pending_inspector_await_with_object_group_for_owner(
            cdp_request_id,
            &owner,
            None,
        )
        .expect("pending Inspector await frontend command id must be unique per session");
    }

    pub(crate) fn try_register_pending_inspector_await_with_object_group_for_owner(
        &mut self,
        cdp_request_id: u64,
        owner: &CommandOwnerScope,
        object_group: Option<&str>,
    ) -> Result<(), DuplicatePendingRendererCommand> {
        let session_id = owner.session_id();
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            return target.try_register_pending_inspector_await(
                owner_session_id,
                cdp_request_id,
                session_id,
                object_group,
            );
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            return target.try_register_pending_inspector_await(
                owner_session_id,
                cdp_request_id,
                session_id,
                object_group,
            );
        }
        self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            |state| {
                state.try_register_pending_inspector_await(cdp_request_id, session_id, object_group)
            },
        )
        .unwrap_or(Ok(()))
    }

    pub(crate) fn try_register_renderer_call_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        cdp_request_id: u64,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
        descriptor: RendererCommandDescriptor,
    ) -> Result<PreparedRendererCallDispatch, String> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            return target
                .try_register_renderer_call(
                    owner_session_id,
                    cdp_request_id,
                    dispatched_attachment_id,
                    descriptor,
                )
                .ok_or_else(|| "UnknownSession".to_owned())?
                .map_err(|error| error.to_string());
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            return target
                .try_register_renderer_call(
                    owner_session_id,
                    cdp_request_id,
                    dispatched_attachment_id,
                    descriptor,
                )
                .ok_or_else(|| "UnknownSession".to_owned())?
                .map_err(|error| error.to_string());
        }
        self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.try_register_renderer_call(cdp_request_id, dispatched_attachment_id, descriptor)
        })
        .ok_or_else(|| "UnknownSession".to_owned())?
        .map_err(|error| error.to_string())
    }

    fn try_register_renderer_call_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        cdp_request_id: u64,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
        descriptor: RendererCommandDescriptor,
    ) -> Result<PreparedRendererCallDispatch, String> {
        if owner.session_id().is_some() {
            return self.try_register_renderer_call_for_session_owner(
                owner.session_id(),
                cdp_request_id,
                dispatched_attachment_id,
                descriptor,
            );
        }
        self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            |state| {
                state.try_register_renderer_call(
                    cdp_request_id,
                    dispatched_attachment_id,
                    descriptor,
                )
            },
        )
        .ok_or_else(|| "UnknownSession".to_owned())?
        .map_err(|error| error.to_string())
    }

    pub(crate) fn take_renderer_call_for_frontend_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            return target.take_renderer_call_for_frontend(owner_session_id, cdp_request_id);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            return target.take_renderer_call_for_frontend(owner_session_id, cdp_request_id);
        }
        self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.take_renderer_call_for_frontend(cdp_request_id)
        })
        .flatten()
    }

    pub(crate) fn take_renderer_call_for_frontend_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        if owner.session_id().is_some() {
            return self.take_renderer_call_for_frontend_for_session_owner(
                owner.session_id(),
                cdp_request_id,
            );
        }
        self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            |state| state.take_renderer_call_for_frontend(cdp_request_id),
        )
        .flatten()
    }

    fn renderer_call_for_frontend_for_session_owner(
        &self,
        session_id: Option<&str>,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target.renderer_call_for_frontend(owner_session_id, cdp_request_id);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target.renderer_call_for_frontend(owner_session_id, cdp_request_id);
        }
        self.target_devtools_session_state_for_session(session_id)?
            .renderer_call_for_frontend(cdp_request_id)
    }

    fn renderer_command_descriptor_for_renderer_if_attachment_matches_for_session_owner(
        &self,
        session_id: Option<&str>,
        renderer_call_id: RendererCallId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererCommandDescriptor> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target.renderer_command_descriptor_for_renderer_if_attachment_matches(
                owner_session_id,
                renderer_call_id,
                dispatched_attachment_id,
            );
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target.renderer_command_descriptor_for_renderer_if_attachment_matches(
                owner_session_id,
                renderer_call_id,
                dispatched_attachment_id,
            );
        }
        self.target_devtools_session_state_for_session(session_id)?
            .renderer_command_descriptor_for_renderer_if_attachment_matches(
                renderer_call_id,
                dispatched_attachment_id,
            )
    }

    pub(crate) fn renderer_runtime_command_cause_for_frontend(
        &self,
        session_id: Option<&str>,
        cdp_request_id: u64,
    ) -> Option<RendererRuntimeCommandCausalIdentity> {
        self.renderer_runtime_command_cause_for_route(session_id, None, cdp_request_id)
    }

    pub(crate) fn renderer_runtime_command_cause_for_route(
        &self,
        session_id: Option<&str>,
        owner_route: Option<&CdpSessionRoute>,
        cdp_request_id: u64,
    ) -> Option<RendererRuntimeCommandCausalIdentity> {
        let correlation = if session_id.is_some() {
            self.renderer_call_for_frontend_for_session_owner(session_id, cdp_request_id)
        } else {
            self.target_devtools_session_state_for_route(session_id, owner_route)?
                .renderer_call_for_frontend(cdp_request_id)
        }?;
        Some(RendererRuntimeCommandCausalIdentity::new(
            self.target_renderer_runtime_inspector_session_id_for_route(session_id, owner_route),
            correlation.renderer_call_id().get(),
        ))
    }

    fn take_renderer_call_for_frontend_if_matches_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        cdp_request_id: u64,
        renderer_call_id: RendererCallId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            return target.take_renderer_call_for_frontend_if_matches(
                owner_session_id,
                cdp_request_id,
                renderer_call_id,
                dispatched_attachment_id,
            );
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            return target.take_renderer_call_for_frontend_if_matches(
                owner_session_id,
                cdp_request_id,
                renderer_call_id,
                dispatched_attachment_id,
            );
        }
        self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.take_renderer_call_for_frontend_if_matches(
                cdp_request_id,
                renderer_call_id,
                dispatched_attachment_id,
            )
        })
        .flatten()
    }

    fn take_renderer_call_for_frontend_if_matches_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        cdp_request_id: u64,
        renderer_call_id: RendererCallId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        if owner.session_id().is_some() {
            return self.take_renderer_call_for_frontend_if_matches_for_session_owner(
                owner.session_id(),
                cdp_request_id,
                renderer_call_id,
                dispatched_attachment_id,
            );
        }
        self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            |state| {
                state.take_renderer_call_for_frontend_if_matches(
                    cdp_request_id,
                    renderer_call_id,
                    dispatched_attachment_id,
                )
            },
        )
        .flatten()
    }

    pub(crate) fn take_renderer_call_if_correlation_matches_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        correlation: RendererCommandCorrelation,
    ) -> bool {
        self.take_renderer_call_for_frontend_if_matches_for_session_owner(
            session_id,
            correlation.frontend_command_id().get(),
            correlation.renderer_call_id(),
            correlation.dispatched_attachment_id(),
        ) == Some(correlation)
    }

    pub(crate) fn take_renderer_call_if_correlation_matches_for_route(
        &mut self,
        session_id: Option<&str>,
        owner_route: Option<&CdpSessionRoute>,
        correlation: RendererCommandCorrelation,
    ) -> bool {
        if session_id.is_some() {
            return self.take_renderer_call_if_correlation_matches_for_session_owner(
                session_id,
                correlation,
            );
        }
        self.with_target_devtools_session_state_for_route_mut(session_id, owner_route, |state| {
            state.take_renderer_call_for_frontend_if_matches(
                correlation.frontend_command_id().get(),
                correlation.renderer_call_id(),
                correlation.dispatched_attachment_id(),
            )
        })
        .flatten()
            == Some(correlation)
    }

    fn take_frontend_command_for_renderer_if_attachment_matches_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        renderer_call_id: RendererCallId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            return target.take_frontend_command_for_renderer_if_attachment_matches(
                owner_session_id,
                renderer_call_id,
                dispatched_attachment_id,
            );
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            return target.take_frontend_command_for_renderer_if_attachment_matches(
                owner_session_id,
                renderer_call_id,
                dispatched_attachment_id,
            );
        }
        self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.take_frontend_command_for_renderer_if_attachment_matches(
                renderer_call_id,
                dispatched_attachment_id,
            )
        })
        .flatten()
    }

    fn take_frontend_command_for_renderer_if_attachment_matches_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        renderer_call_id: RendererCallId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        if owner.session_id().is_some() {
            return self
                .take_frontend_command_for_renderer_if_attachment_matches_for_session_owner(
                    owner.session_id(),
                    renderer_call_id,
                    dispatched_attachment_id,
                );
        }
        self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            |state| {
                state.take_frontend_command_for_renderer_if_attachment_matches(
                    renderer_call_id,
                    dispatched_attachment_id,
                )
            },
        )
        .flatten()
    }

    fn prepare_renderer_call_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        descriptor: RendererCommandDescriptor,
        cdp_request_id: u64,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Result<
        (
            RendererCommandCorrelation,
            String,
            RendererRuntimeInspectorResponseSender,
            RuntimeProtocolResponseRoute,
        ),
        String,
    > {
        let raw_json = descriptor.frontend_payload().to_owned();
        let response_delivery = descriptor.response_delivery();
        let prepared = self.try_register_renderer_call_for_session_owner(
            session_id,
            cdp_request_id,
            dispatched_attachment_id,
            descriptor,
        )?;
        let correlation = prepared.correlation();
        match self.rewrite_runtime_inspector_command_for_session_owner(
            session_id,
            &raw_json,
            Some((
                FrontendCommandId::new(cdp_request_id),
                correlation.renderer_call_id(),
            )),
        ) {
            Ok(raw_json) => {
                let (correlation, response_sender, response_receiver) = prepared.into_parts();
                Ok((
                    correlation,
                    raw_json,
                    response_sender,
                    RuntimeProtocolResponseRoute::for_registered_delivery(
                        response_delivery,
                        response_receiver,
                    ),
                ))
            }
            Err(error) => {
                let removed = self
                    .take_renderer_call_for_frontend_for_session_owner(session_id, cdp_request_id);
                debug_assert_eq!(removed, Some(correlation));
                Err(error)
            }
        }
    }

    fn prepare_renderer_call_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        descriptor: RendererCommandDescriptor,
        cdp_request_id: u64,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Result<
        (
            RendererCommandCorrelation,
            String,
            RendererRuntimeInspectorResponseSender,
            RuntimeProtocolResponseRoute,
        ),
        String,
    > {
        let raw_json = descriptor.frontend_payload().to_owned();
        let response_delivery = descriptor.response_delivery();
        let prepared = self.try_register_renderer_call_for_owner(
            owner,
            cdp_request_id,
            dispatched_attachment_id,
            descriptor,
        )?;
        let correlation = prepared.correlation();
        match self.rewrite_runtime_inspector_command_for_owner(
            owner,
            &raw_json,
            Some((
                FrontendCommandId::new(cdp_request_id),
                correlation.renderer_call_id(),
            )),
        ) {
            Ok(raw_json) => {
                let (correlation, response_sender, response_receiver) = prepared.into_parts();
                Ok((
                    correlation,
                    raw_json,
                    response_sender,
                    RuntimeProtocolResponseRoute::for_registered_delivery(
                        response_delivery,
                        response_receiver,
                    ),
                ))
            }
            Err(error) => {
                let removed = self.take_renderer_call_for_frontend_for_owner(owner, cdp_request_id);
                debug_assert_eq!(removed, Some(correlation));
                Err(error)
            }
        }
    }

    fn rewrite_runtime_inspector_command_for_session_owner(
        &self,
        session_id: Option<&str>,
        raw_json: &str,
        command_id_rewrite: Option<(FrontendCommandId, RendererCallId)>,
    ) -> Result<String, String> {
        let owner_target_id = self
            .runtime_context_owner_identity_for_session(session_id)
            .and_then(|(_, target_id)| target_id);
        rewrite_runtime_inspector_command_for_renderer(
            raw_json,
            command_id_rewrite,
            owner_target_id.as_deref(),
        )
    }

    fn rewrite_runtime_inspector_command_for_owner(
        &self,
        owner: &CommandOwnerScope,
        raw_json: &str,
        command_id_rewrite: Option<(FrontendCommandId, RendererCallId)>,
    ) -> Result<String, String> {
        if owner.session_id().is_some() {
            return self.rewrite_runtime_inspector_command_for_session_owner(
                owner.session_id(),
                raw_json,
                command_id_rewrite,
            );
        }
        let owner_target_id = self
            .target_owner_identity_for_route(owner.session_id(), owner.session_owner_route())
            .and_then(|(_, target_id)| target_id);
        rewrite_runtime_inspector_command_for_renderer(
            raw_json,
            command_id_rewrite,
            owner_target_id.as_deref(),
        )
    }

    pub(crate) fn register_runtime_await_job_for_owner(
        &mut self,
        cdp_request_id: u64,
        owner: &CommandOwnerScope,
        object_group: Option<&str>,
        action: &'static str,
    ) {
        let session_id = owner.session_id();
        let owner_route = owner
            .session_owner_route()
            .cloned()
            .or_else(|| self.session_route(session_id));
        let job = RuntimeAwaitJob::new(
            cdp_request_id,
            session_id,
            owner_route,
            object_group,
            action,
        );
        let trace_fields = job.trace_fields();
        let key = PendingRendererCommandKey::new(session_id, cdp_request_id);
        match self.pending_runtime_await_jobs.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(job);
            }
            Entry::Occupied(_) => {
                tracing::error!(
                    cdp_request_id,
                    session_id,
                    "runtime await trace job already exists for frontend command"
                );
                return;
            }
        }
        self.record_runtime_await_trace(
            "runtime_await_job_start",
            Some(cdp_request_id),
            session_id,
            trace_fields,
        );
    }

    pub(crate) fn trace_runtime_await_pending_registered(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) {
        self.record_runtime_await_trace(
            "runtime_await_pending_registered",
            Some(cdp_request_id),
            session_id,
            self.runtime_await_job_trace_fields(cdp_request_id, session_id),
        );
    }

    pub(crate) fn trace_runtime_await_initial_dispatch_done(
        &mut self,
        cdp_request_id: Option<u64>,
        session_id: Option<&str>,
        messages: usize,
        saw_current_response: bool,
    ) {
        self.record_runtime_await_trace(
            "runtime_await_initial_dispatch_done",
            cdp_request_id,
            session_id,
            json!({
                "messages": messages,
                "matchingResponseSeen": saw_current_response,
                "job": cdp_request_id
                    .map(|id| self.runtime_await_job_trace_fields(id, session_id)),
            }),
        );
    }

    pub(crate) fn complete_runtime_await_job(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) {
        let key = PendingRendererCommandKey::new(session_id, cdp_request_id);
        let Some(job) = self.pending_runtime_await_jobs.remove(&key) else {
            return;
        };
        let session_id = job.session_id();
        let fields = job.trace_fields();
        self.record_runtime_await_trace(
            "runtime_await_completed",
            Some(cdp_request_id),
            session_id.as_deref(),
            fields,
        );
    }

    pub(crate) fn cancel_runtime_await_job(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
        reason: &'static str,
    ) {
        let key = PendingRendererCommandKey::new(session_id, cdp_request_id);
        let Some(job) = self.pending_runtime_await_jobs.remove(&key) else {
            return;
        };
        let session_id = job.session_id();
        let mut fields = job.trace_fields();
        if let Some(object) = fields.as_object_mut() {
            object.insert("reason".to_owned(), json!(reason));
        }
        self.record_runtime_await_trace(
            "runtime_await_cancelled",
            Some(cdp_request_id),
            session_id.as_deref(),
            fields,
        );
    }

    fn runtime_await_job_trace_fields(
        &self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) -> Value {
        let key = PendingRendererCommandKey::new(session_id, cdp_request_id);
        self.pending_runtime_await_jobs
            .get(&key)
            .map(RuntimeAwaitJob::trace_fields)
            .unwrap_or_else(|| json!({}))
    }

    pub(crate) fn runtime_await_owner_route_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<CdpSessionRoute> {
        if let Some(route) = self.session_route(session_id) {
            return Some(route);
        }
        self.target_owner_identity_for_session(session_id).and_then(
            |(browser_context_id, target_id)| {
                target_id.map(|target_id| CdpSessionRoute::PageTarget {
                    browser_context_id,
                    target_id,
                    session_key: moli_page_types::DevToolsSessionKey::Primary,
                })
            },
        )
    }

    pub(crate) fn next_internal_runtime_command_id(&mut self) -> u64 {
        let id = self.next_internal_runtime_command_id;
        self.next_internal_runtime_command_id = self
            .next_internal_runtime_command_id
            .checked_add(1)
            .expect("internal Runtime command id space exhausted");
        id
    }

    pub(crate) fn next_bidi_channel_object_group(&mut self) -> String {
        format!(
            "{BIDI_CHANNEL_OBJECT_GROUP_PREFIX}{}",
            self.next_internal_runtime_command_id()
        )
    }

    #[cfg(test)]
    pub(crate) fn register_pending_bidi_channel_listener(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
        listener: BidiChannelListenerResidence,
    ) {
        assert_eq!(
            listener.owner().session_id(),
            session_id,
            "BiDi listener residence must be registered under its exact Page attachment"
        );
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.register_pending_bidi_channel_listener(
                cdp_request_id,
                session_id,
                Some(BIDI_SCRIPT_RESULT_OBJECT_GROUP),
                listener,
            );
        });
    }

    fn register_pending_bidi_channel_listener_for_owner(
        &mut self,
        cdp_request_id: u64,
        owner: &CommandOwnerScope,
        listener: BidiChannelListenerResidence,
    ) {
        assert_eq!(
            listener.owner().command_owner(),
            owner,
            "BiDi listener residence must be registered under its exact Page owner"
        );
        let _ = self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            |state| {
                state.register_pending_bidi_channel_listener(
                    cdp_request_id,
                    owner.session_id(),
                    Some(BIDI_SCRIPT_RESULT_OBJECT_GROUP),
                    listener,
                );
            },
        );
    }

    pub(crate) fn publish_bidi_channel_listener_start(
        &mut self,
        listener: BidiChannelListenerResidence,
    ) {
        self.publish_bidi_channel_owner_action(BidiChannelOwnerAction::start_listener(listener));
    }

    fn publish_bidi_channel_object_group_release(
        &mut self,
        owner: BidiChannelPageOwner,
        object_group: impl Into<String>,
    ) {
        self.publish_bidi_channel_owner_action(BidiChannelOwnerAction::release_object_group(
            owner,
            object_group,
        ));
    }

    fn publish_bidi_channel_owner_action(&mut self, action: BidiChannelOwnerAction) {
        let publish_sequence = self
            .scheduler_state
            .allocate_protocol_work_publish_sequence();
        let work = crate::domains::activity::ProtocolSchedulerWork::bidi_channel_owner_action(
            publish_sequence,
            action,
        );
        self.scheduler_state
            .push_scheduler_event(CdpSchedulerEvent::ProtocolWorkPublished { work });
    }

    pub(crate) fn forget_pending_inspector_await(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) {
        self.cancel_runtime_await_job(cdp_request_id, session_id, "forgotten");
        let _ = self.remove_pending_inspector_await_for_cancellation(cdp_request_id, session_id);
    }

    pub(crate) fn forget_pending_inspector_await_for_owner(
        &mut self,
        cdp_request_id: u64,
        owner: &CommandOwnerScope,
    ) {
        self.cancel_runtime_await_job(cdp_request_id, owner.session_id(), "forgotten");
        let _ =
            self.remove_pending_inspector_await_for_cancellation_for_owner(cdp_request_id, owner);
    }

    pub(crate) fn claim_pending_inspector_await_for_scheduler_deferred_reply(
        &mut self,
        cdp_request_id: u64,
        owner: &CommandOwnerScope,
    ) -> Option<ClaimedPendingInspectorAwait> {
        let claimed = self
            .remove_pending_inspector_await_for_owner(cdp_request_id, owner)
            .map(|entry| ClaimedPendingInspectorAwait {
                command_id: cdp_request_id,
                owner: owner.clone(),
                entry,
            })?;
        let key = PendingRendererCommandKey::new(owner.session_id(), cdp_request_id);
        match self.claimed_pending_inspector_await_owners.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(ClaimedPendingInspectorAwaitOwner::from_claimed(&claimed));
            }
            Entry::Occupied(_) => {
                panic!("claimed pending Inspector await owner must be unique per session");
            }
        }
        Some(claimed)
    }

    fn remove_claimed_pending_inspector_await_owner(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) -> Option<ClaimedPendingInspectorAwaitOwner> {
        let key = PendingRendererCommandKey::new(session_id, cdp_request_id);
        self.claimed_pending_inspector_await_owners.remove(&key)
    }

    fn drain_claimed_pending_inspector_await_owners_for_session(
        &mut self,
        session_id: Option<&str>,
    ) -> Vec<ClaimedPendingInspectorAwaitOwner> {
        let to_remove = self
            .claimed_pending_inspector_await_owners
            .iter()
            .filter_map(|(key, owner)| {
                owner
                    .matches_session_owner(session_id)
                    .then_some(key.clone())
            })
            .collect::<Vec<_>>();
        to_remove
            .into_iter()
            .filter_map(|key| self.claimed_pending_inspector_await_owners.remove(&key))
            .collect()
    }

    fn drain_claimed_pending_inspector_await_owners_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Vec<ClaimedPendingInspectorAwaitOwner> {
        let to_remove = self
            .claimed_pending_inspector_await_owners
            .iter()
            .filter_map(|(key, claimed)| claimed.matches_owner(owner).then_some(key.clone()))
            .collect::<Vec<_>>();
        to_remove
            .into_iter()
            .filter_map(|key| self.claimed_pending_inspector_await_owners.remove(&key))
            .collect()
    }

    fn push_claimed_pending_inspector_await_owner_errors(
        &mut self,
        background_events: &mut Vec<BackgroundProtocolEvent>,
        owners: Vec<ClaimedPendingInspectorAwaitOwner>,
        reason: &'static str,
    ) {
        for owner in owners {
            self.cancel_runtime_await_job(owner.command_id, owner.session_id(), reason);
            if let Some(correlation) = owner.renderer_correlation {
                let _ = self.take_renderer_call_for_frontend_if_matches_for_owner(
                    &owner.owner,
                    correlation.frontend_command_id().get(),
                    correlation.renderer_call_id(),
                    correlation.dispatched_attachment_id(),
                );
            }
            if let Some(object_group) = owner.bidi_channel_object_group.as_deref() {
                self.unregister_runtime_remote_object_group_for_owner(&owner.owner, object_group);
                continue;
            }
            let mut response = RuntimeInspectorResponseReady::for_owner(
                owner.command_id,
                &owner.owner,
                Err(reason.to_owned()),
            );
            if let Some(correlation) = owner.renderer_correlation {
                response.bind_renderer_call_id(correlation.renderer_call_id());
            }
            background_events.push(BackgroundProtocolEvent::runtime_inspector_response_ready(
                response,
            ));
        }
    }

    #[cfg(test)]
    pub(crate) fn has_claimed_pending_inspector_awaits_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        self.claimed_pending_inspector_await_owners
            .values()
            .any(|owner| owner.matches_session_owner(session_id))
    }

    #[cfg(test)]
    pub(crate) fn has_unclaimed_pending_inspector_awaits_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target.has_pending_inspector_awaits_for_session(owner_session_id);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target.has_pending_inspector_awaits_for_session(owner_session_id);
        }
        self.target_devtools_session_state_for_session(session_id)
            .is_some_and(DevToolsSessionState::has_pending_inspector_awaits)
    }

    pub(crate) fn complete_claimed_pending_inspector_await_for_scheduler_deferred_reply(
        &mut self,
        claimed: Option<ClaimedPendingInspectorAwait>,
        protocol_events: &[BackgroundProtocolEvent],
    ) {
        let Some(claimed) = claimed else {
            return;
        };
        let ClaimedPendingInspectorAwait {
            command_id,
            owner,
            entry,
        } = claimed;
        let session_id = owner.session_id().map(str::to_owned);
        self.remove_claimed_pending_inspector_await_owner(command_id, session_id.as_deref());
        self.complete_runtime_await_job(command_id, session_id.as_deref());
        self.apply_completed_pending_inspector_await_entry(&owner, entry, protocol_events);
    }

    pub(crate) fn cancel_claimed_pending_inspector_await_for_scheduler_deferred_reply(
        &mut self,
        claimed: Option<ClaimedPendingInspectorAwait>,
        reason: &'static str,
    ) {
        let Some(claimed) = claimed else {
            return;
        };
        let ClaimedPendingInspectorAwait {
            command_id,
            owner,
            entry,
        } = claimed;
        let session_id = owner.session_id().map(str::to_owned);
        self.remove_claimed_pending_inspector_await_owner(command_id, session_id.as_deref());
        self.cancel_runtime_await_job(command_id, session_id.as_deref(), reason);
        if let Some(correlation) = entry.renderer_correlation() {
            let _ = self.take_renderer_call_for_frontend_if_matches_for_owner(
                &owner,
                correlation.frontend_command_id().get(),
                correlation.renderer_call_id(),
                correlation.dispatched_attachment_id(),
            );
        }
        if let Some(listener) = entry.bidi_channel_listener() {
            self.unregister_runtime_remote_object_group_for_owner(
                &owner,
                listener.channel_object_group(),
            );
        }
    }

    fn apply_completed_pending_inspector_await_entry(
        &mut self,
        owner: &CommandOwnerScope,
        entry: PendingInspectorAwait,
        protocol_events: &[BackgroundProtocolEvent],
    ) {
        if let Some(object_group) = entry.object_group() {
            for event in protocol_events {
                if let Some((_, _, BackgroundCommandResponsePayloadRef::Success { result })) =
                    event.command_response_payload_ref()
                {
                    self.register_runtime_remote_object_ids_from_value_for_owner_with_group(
                        owner,
                        result,
                        object_group,
                    );
                } else if let Some(message) = event.protocol_message() {
                    self.register_runtime_remote_object_ids_from_value_for_owner_with_group(
                        owner,
                        message,
                        object_group,
                    );
                }
            }
        } else {
            for event in protocol_events {
                if let Some((_, _, BackgroundCommandResponsePayloadRef::Success { result })) =
                    event.command_response_payload_ref()
                {
                    self.register_runtime_remote_object_ids_from_value_for_owner(owner, result);
                } else if let Some(message) = event.protocol_message() {
                    self.register_runtime_remote_object_ids_from_value_for_owner(owner, message);
                }
            }
        }
        if let Some(listener) = entry.bidi_channel_listener() {
            self.unregister_runtime_remote_object_group_for_owner(
                owner,
                listener.channel_object_group(),
            );
        }
    }

    fn remove_pending_inspector_await(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) -> Option<PendingInspectorAwait> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            return target.remove_pending_inspector_await(owner_session_id, cdp_request_id);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            return target.remove_pending_inspector_await(owner_session_id, cdp_request_id);
        }
        self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.remove_pending_inspector_await(cdp_request_id)
        })
        .flatten()
    }

    fn remove_pending_inspector_await_for_owner(
        &mut self,
        cdp_request_id: u64,
        owner: &CommandOwnerScope,
    ) -> Option<PendingInspectorAwait> {
        if owner.session_id().is_some() {
            return self.remove_pending_inspector_await(cdp_request_id, owner.session_id());
        }
        self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            |state| state.remove_pending_inspector_await(cdp_request_id),
        )
        .flatten()
    }

    fn remove_pending_inspector_await_for_cancellation(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) -> Option<PendingInspectorAwait> {
        let entry = self.remove_pending_inspector_await(cdp_request_id, session_id);
        if let Some(correlation) = entry
            .as_ref()
            .and_then(PendingInspectorAwait::renderer_correlation)
        {
            self.discard_renderer_call_for_session_owner_if_matches(session_id, correlation);
        } else if entry.is_none() {
            let _ =
                self.take_renderer_call_for_frontend_for_session_owner(session_id, cdp_request_id);
        }
        entry
    }

    fn remove_pending_inspector_await_for_cancellation_for_owner(
        &mut self,
        cdp_request_id: u64,
        owner: &CommandOwnerScope,
    ) -> Option<PendingInspectorAwait> {
        let entry = self.remove_pending_inspector_await_for_owner(cdp_request_id, owner);
        if let Some(correlation) = entry
            .as_ref()
            .and_then(PendingInspectorAwait::renderer_correlation)
        {
            let _ = self.take_renderer_call_for_frontend_if_matches_for_owner(
                owner,
                correlation.frontend_command_id().get(),
                correlation.renderer_call_id(),
                correlation.dispatched_attachment_id(),
            );
        } else if entry.is_none() {
            let _ = self.take_renderer_call_for_frontend_for_owner(owner, cdp_request_id);
        }
        entry
    }

    fn discard_renderer_call_for_session_owner_if_matches(
        &mut self,
        session_id: Option<&str>,
        correlation: RendererCommandCorrelation,
    ) {
        let _ = self.take_renderer_call_for_frontend_if_matches_for_session_owner(
            session_id,
            correlation.frontend_command_id().get(),
            correlation.renderer_call_id(),
            correlation.dispatched_attachment_id(),
        );
    }

    pub fn has_pending_inspector_awaits(&self) -> bool {
        if !self.claimed_pending_inspector_await_owners.is_empty() {
            return true;
        }
        self.browser_contexts().any(|browser_context| {
            browser_context
                .page_targets
                .iter()
                .any(|target| target.has_pending_inspector_awaits())
                || browser_context
                    .shared_worker_targets
                    .values()
                    .any(SharedWorkerTargetState::has_pending_inspector_awaits)
                || browser_context
                    .dedicated_worker_targets
                    .values()
                    .any(|target| target.has_pending_inspector_awaits())
                || browser_context
                    .service_worker_targets
                    .values()
                    .any(ServiceWorkerTargetState::has_pending_inspector_awaits)
        })
    }

    pub fn has_pending_inspector_awaits_for_session_owner(&self, session_id: Option<&str>) -> bool {
        if self
            .claimed_pending_inspector_await_owners
            .values()
            .any(|owner| owner.matches_session_owner(session_id))
        {
            return true;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target.has_pending_inspector_awaits_for_session(owner_session_id);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target.has_pending_inspector_awaits_for_session(owner_session_id);
        }
        self.target_devtools_session_state_for_session(session_id)
            .is_some_and(DevToolsSessionState::has_pending_inspector_awaits)
    }

    pub(crate) fn fail_pending_inspector_awaits_from_shared_worker_target_session_background_events_into(
        out: &mut Vec<BackgroundProtocolEvent>,
        target: &mut SharedWorkerTargetState,
        owner_session_id: &str,
        reason: &'static str,
    ) {
        for (cdp_id, entry) in target.drain_pending_inspector_awaits_for_session(owner_session_id) {
            if let Some(listener) = entry.bidi_channel_listener() {
                let object_owner_session_id = entry.session_id().unwrap_or(owner_session_id);
                target.unregister_runtime_remote_object_group(
                    object_owner_session_id,
                    listener.channel_object_group(),
                );
                continue;
            }
            push_pending_inspector_await_error_background_event(
                out,
                cdp_id,
                entry.session_id(),
                reason,
            );
        }
        for correlation in target.terminate_renderer_calls_for_session(owner_session_id, reason) {
            push_terminated_renderer_call_error_background_events(
                out,
                vec![correlation],
                Some(owner_session_id),
                reason,
            );
        }
    }

    pub(crate) fn fail_pending_inspector_awaits_from_service_worker_target_state_background_events_into(
        out: &mut Vec<BackgroundProtocolEvent>,
        target: &mut ServiceWorkerTargetState,
        reason: &'static str,
    ) {
        for (cdp_id, entry) in target.drain_pending_inspector_awaits() {
            if let Some(listener) = entry.bidi_channel_listener() {
                if let Some(session_id) = entry.session_id() {
                    target.unregister_runtime_remote_object_group(
                        session_id,
                        listener.channel_object_group(),
                    );
                }
                continue;
            }
            push_pending_inspector_await_error_background_event(
                out,
                cdp_id,
                entry.session_id(),
                reason,
            );
        }
        for (session_id, correlation) in target.terminate_renderer_calls(reason) {
            push_terminated_renderer_call_error_background_events(
                out,
                vec![correlation],
                Some(&session_id),
                reason,
            );
        }
    }

    pub(crate) fn fail_pending_inspector_awaits_for_session_owner_background_events_into(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
        claimed_background_events: &mut Vec<BackgroundProtocolEvent>,
        session_id: Option<&str>,
        reason: &'static str,
    ) {
        let claimed = self.drain_claimed_pending_inspector_await_owners_for_session(session_id);
        self.push_claimed_pending_inspector_await_owner_errors(
            claimed_background_events,
            claimed,
            reason,
        );
        if let Some(owner_session_id) = session_id
            && self.shared_worker_target_for_session(session_id).is_some()
        {
            let drained = self
                .shared_worker_target_for_session_mut(session_id)
                .map(|target| target.drain_pending_inspector_awaits_for_session(owner_session_id))
                .unwrap_or_default();
            let mut listener_groups_to_unregister = Vec::new();
            for (cdp_id, entry) in drained {
                self.cancel_runtime_await_job(cdp_id, entry.session_id(), reason);
                if let Some(listener) = entry.bidi_channel_listener() {
                    listener_groups_to_unregister.push((
                        entry.session_id().map(str::to_owned),
                        listener.channel_object_group().to_owned(),
                    ));
                    continue;
                }
                push_pending_inspector_await_error_background_event(
                    out,
                    cdp_id,
                    entry.session_id(),
                    reason,
                );
            }
            if let Some(target) = self.shared_worker_target_for_session_mut(session_id) {
                for (entry_session_id, object_group) in listener_groups_to_unregister {
                    let object_owner_session_id =
                        entry_session_id.as_deref().unwrap_or(owner_session_id);
                    target.unregister_runtime_remote_object_group(
                        object_owner_session_id,
                        &object_group,
                    );
                }
                let terminated =
                    target.terminate_renderer_calls_for_session(owner_session_id, reason);
                push_terminated_renderer_call_error_background_events(
                    out,
                    terminated,
                    Some(owner_session_id),
                    reason,
                );
            }
            return;
        }
        if let Some(owner_session_id) = session_id
            && self.service_worker_target_for_session(session_id).is_some()
        {
            let drained = self
                .service_worker_target_for_session_mut(session_id)
                .map(|target| target.drain_pending_inspector_awaits_for_session(owner_session_id))
                .unwrap_or_default();
            let mut listener_groups_to_unregister = Vec::new();
            for (cdp_id, entry) in drained {
                self.cancel_runtime_await_job(cdp_id, entry.session_id(), reason);
                if let Some(listener) = entry.bidi_channel_listener() {
                    listener_groups_to_unregister.push((
                        entry.session_id().map(str::to_owned),
                        listener.channel_object_group().to_owned(),
                    ));
                    continue;
                }
                push_pending_inspector_await_error_background_event(
                    out,
                    cdp_id,
                    entry.session_id(),
                    reason,
                );
            }
            if let Some(target) = self.service_worker_target_for_session_mut(session_id) {
                for (entry_session_id, object_group) in listener_groups_to_unregister {
                    let object_owner_session_id =
                        entry_session_id.as_deref().unwrap_or(owner_session_id);
                    target.unregister_runtime_remote_object_group(
                        object_owner_session_id,
                        &object_group,
                    );
                }
                let terminated =
                    target.terminate_renderer_calls_for_session(owner_session_id, reason);
                push_terminated_renderer_call_error_background_events(
                    out,
                    terminated,
                    Some(owner_session_id),
                    reason,
                );
            }
            return;
        }
        let drained = self
            .with_target_devtools_session_state_for_session_mut(session_id, |state| {
                state.drain_pending_inspector_awaits()
            })
            .unwrap_or_default();
        for (cdp_id, entry) in drained {
            self.cancel_runtime_await_job(cdp_id, entry.session_id(), reason);
            if let Some(listener) = entry.bidi_channel_listener() {
                self.unregister_runtime_remote_object_group_for_session_owner(
                    entry.session_id(),
                    listener.channel_object_group(),
                );
                continue;
            }
            push_pending_inspector_await_error_background_event(
                out,
                cdp_id,
                entry.session_id(),
                reason,
            );
        }
        let terminated = self
            .with_target_devtools_session_state_for_session_mut(session_id, |state| {
                state.terminate_all_renderer_calls(reason)
            })
            .unwrap_or_default();
        push_terminated_renderer_call_error_background_events(out, terminated, session_id, reason);
    }

    pub(crate) fn fail_pending_inspector_awaits_for_owner_background_events_into(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
        claimed_background_events: &mut Vec<BackgroundProtocolEvent>,
        owner: &CommandOwnerScope,
        reason: &'static str,
    ) {
        if owner.session_id().is_some() {
            self.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out,
                claimed_background_events,
                owner.session_id(),
                reason,
            );
            return;
        }

        let claimed = self.drain_claimed_pending_inspector_await_owners_for_owner(owner);
        self.push_claimed_pending_inspector_await_owner_errors(
            claimed_background_events,
            claimed,
            reason,
        );
        let drained = self
            .with_target_devtools_session_state_for_route_mut(
                None,
                owner.session_owner_route(),
                |state| state.drain_pending_inspector_awaits(),
            )
            .unwrap_or_default();
        for (cdp_id, entry) in drained {
            self.cancel_runtime_await_job(cdp_id, entry.session_id(), reason);
            if let Some(listener) = entry.bidi_channel_listener() {
                self.unregister_runtime_remote_object_group_for_owner(
                    owner,
                    listener.channel_object_group(),
                );
                continue;
            }
            push_pending_inspector_await_error_background_event(
                out,
                cdp_id,
                entry.session_id(),
                reason,
            );
        }
        let terminated = self
            .with_target_devtools_session_state_for_route_mut(
                None,
                owner.session_owner_route(),
                |state| state.terminate_all_renderer_calls(reason),
            )
            .unwrap_or_default();
        push_terminated_renderer_call_error_background_events(out, terminated, None, reason);
    }

    pub(crate) fn validate_runtime_remote_object_ids_for_session_owner(
        &self,
        session_id: Option<&str>,
        object_ids: &[String],
    ) -> Result<(), String> {
        if object_ids.is_empty() {
            return Ok(());
        }
        let Some(owner) = self.runtime_remote_object_owner_identity_for_session(session_id) else {
            return Ok(());
        };
        for object_id in object_ids {
            // V8 remote object ids are scoped to an Inspector session. Two
            // sessions connected to the same context can therefore emit the
            // same wire id for different objects. Prefer the current
            // session's registration before using the cross-owner check to
            // reject a handle borrowed from another session.
            if self.runtime_remote_object_id_known_for_session_owner(session_id, object_id) {
                continue;
            }
            if self.runtime_remote_object_id_known_for_different_owner(&owner, object_id) {
                return Err("Cannot find object with given id".to_owned());
            }
        }
        Ok(())
    }

    pub(crate) fn validate_runtime_remote_object_ids_for_owner(
        &self,
        owner: &CommandOwnerScope,
        object_ids: &[String],
    ) -> Result<(), String> {
        if owner.session_id().is_some() {
            return self.validate_runtime_remote_object_ids_for_session_owner(
                owner.session_id(),
                object_ids,
            );
        }
        if object_ids.is_empty() {
            return Ok(());
        }
        let Some(owner_identity) = self.runtime_remote_object_owner_identity_for_owner(owner)
        else {
            return Ok(());
        };
        for object_id in object_ids {
            if self.runtime_remote_object_id_known_for_owner(owner, object_id) {
                continue;
            }
            if self.runtime_remote_object_id_known_for_different_owner(&owner_identity, object_id) {
                return Err("Cannot find object with given id".to_owned());
            }
        }
        Ok(())
    }

    pub(crate) fn runtime_remote_object_id_known_for_session_owner(
        &self,
        session_id: Option<&str>,
        object_id: &str,
    ) -> bool {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target.has_runtime_remote_object_id(owner_session_id, object_id);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target.has_runtime_remote_object_id(owner_session_id, object_id);
        }
        self.target_devtools_session_state_for_session(session_id)
            .is_some_and(|state| state.has_runtime_remote_object_id(object_id))
    }

    pub(crate) fn runtime_remote_object_id_known_for_owner(
        &self,
        owner: &CommandOwnerScope,
        object_id: &str,
    ) -> bool {
        if owner.session_id().is_some() {
            return self
                .runtime_remote_object_id_known_for_session_owner(owner.session_id(), object_id);
        }
        self.target_devtools_session_state_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        )
        .is_some_and(|state| state.has_runtime_remote_object_id(object_id))
    }

    pub(crate) fn register_runtime_remote_object_ids_from_value_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        value: &Value,
    ) {
        let object_ids = runtime_remote_object_ids_in_value(value);
        self.register_runtime_remote_object_ids_for_session_owner(session_id, object_ids);
    }

    pub(crate) fn register_runtime_remote_object_ids_from_value_for_route(
        &mut self,
        session_id: Option<&str>,
        owner_route: Option<&CdpSessionRoute>,
        value: &Value,
    ) {
        let object_ids = runtime_remote_object_ids_in_value(value);
        let _ = self.with_target_devtools_session_state_for_route_mut(
            session_id,
            owner_route,
            |state| state.register_runtime_remote_object_ids(object_ids),
        );
    }

    pub(crate) fn register_runtime_remote_object_ids_from_value_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        value: &Value,
    ) {
        if owner.session_id().is_some() {
            self.register_runtime_remote_object_ids_from_value_for_session_owner(
                owner.session_id(),
                value,
            );
            return;
        }
        self.register_runtime_remote_object_ids_from_value_for_route(
            owner.session_id(),
            owner.session_owner_route(),
            value,
        );
    }

    pub(crate) fn register_runtime_remote_object_ids_from_value_for_session_owner_with_group(
        &mut self,
        session_id: Option<&str>,
        value: &Value,
        object_group: &str,
    ) {
        let object_ids = runtime_remote_object_ids_in_value(value);
        self.register_runtime_remote_object_ids_for_session_owner_with_group(
            session_id,
            object_ids,
            object_group,
        );
    }

    pub(crate) fn register_runtime_remote_object_ids_from_value_for_route_with_group(
        &mut self,
        session_id: Option<&str>,
        owner_route: Option<&CdpSessionRoute>,
        value: &Value,
        object_group: &str,
    ) {
        let object_ids = runtime_remote_object_ids_in_value(value);
        let _ = self.with_target_devtools_session_state_for_route_mut(
            session_id,
            owner_route,
            |state| state.register_runtime_remote_object_ids_with_group(object_ids, object_group),
        );
    }

    pub(crate) fn register_runtime_remote_object_ids_from_value_for_owner_with_group(
        &mut self,
        owner: &CommandOwnerScope,
        value: &Value,
        object_group: &str,
    ) {
        if owner.session_id().is_some() {
            self.register_runtime_remote_object_ids_from_value_for_session_owner_with_group(
                owner.session_id(),
                value,
                object_group,
            );
            return;
        }
        self.register_runtime_remote_object_ids_from_value_for_route_with_group(
            owner.session_id(),
            owner.session_owner_route(),
            value,
            object_group,
        );
    }

    pub(crate) fn runtime_remote_object_group_for_session_owner(
        &self,
        session_id: Option<&str>,
        object_id: &str,
    ) -> Option<String> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target
                .runtime_remote_object_group(owner_session_id, object_id)
                .map(str::to_owned);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target
                .runtime_remote_object_group(owner_session_id, object_id)
                .map(str::to_owned);
        }
        self.target_devtools_session_state_for_session(session_id)?
            .runtime_remote_object_group(object_id)
            .map(str::to_owned)
    }

    pub(crate) fn runtime_remote_object_group_for_owner(
        &self,
        owner: &CommandOwnerScope,
        object_id: &str,
    ) -> Option<String> {
        if owner.session_id().is_some() {
            return self
                .runtime_remote_object_group_for_session_owner(owner.session_id(), object_id);
        }
        self.target_devtools_session_state_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        )?
        .runtime_remote_object_group(object_id)
        .map(str::to_owned)
    }

    pub(crate) fn runtime_remote_object_realm_for_session_owner(
        &self,
        session_id: Option<&str>,
        object_id: &str,
    ) -> Option<String> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target
                .runtime_remote_object_realm(owner_session_id, object_id)
                .map(str::to_owned);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target
                .runtime_remote_object_realm(owner_session_id, object_id)
                .map(str::to_owned);
        }
        self.target_devtools_session_state_for_session(session_id)?
            .runtime_remote_object_realm(object_id)
            .map(str::to_owned)
    }

    pub(crate) fn runtime_remote_object_realm_for_owner(
        &self,
        owner: &CommandOwnerScope,
        object_id: &str,
    ) -> Option<String> {
        if owner.session_id().is_some() {
            return self
                .runtime_remote_object_realm_for_session_owner(owner.session_id(), object_id);
        }
        self.target_devtools_session_state_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        )?
        .runtime_remote_object_realm(object_id)
        .map(str::to_owned)
    }

    pub(crate) fn runtime_remote_object_alias_for_session_owner(
        &self,
        session_id: Option<&str>,
        object_id: &str,
    ) -> Option<String> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target
                .runtime_remote_object_alias(owner_session_id, object_id)
                .map(str::to_owned);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target
                .runtime_remote_object_alias(owner_session_id, object_id)
                .map(str::to_owned);
        }
        self.target_devtools_session_state_for_session(session_id)?
            .runtime_remote_object_alias(object_id)
            .map(str::to_owned)
    }

    pub(crate) fn runtime_remote_object_alias_for_owner(
        &self,
        owner: &CommandOwnerScope,
        object_id: &str,
    ) -> Option<String> {
        if owner.session_id().is_some() {
            return self
                .runtime_remote_object_alias_for_session_owner(owner.session_id(), object_id);
        }
        self.target_devtools_session_state_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        )?
        .runtime_remote_object_alias(object_id)
        .map(str::to_owned)
    }

    pub(crate) fn register_runtime_remote_object_alias_for_owner_with_realm(
        &mut self,
        owner: &CommandOwnerScope,
        alias_id: String,
        object_id: String,
        realm_id: &str,
    ) {
        if owner.session_id().is_some() {
            self.register_runtime_remote_object_alias_for_session_owner_with_realm(
                owner.session_id(),
                alias_id,
                object_id,
                realm_id,
            );
            return;
        }
        let _ = self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            |state| {
                state
                    .register_runtime_remote_object_alias_with_realm(alias_id, object_id, realm_id);
            },
        );
    }

    pub(crate) fn unregister_runtime_remote_object_ids_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        object_ids: &[String],
    ) {
        if object_ids.is_empty() {
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.unregister_runtime_remote_object_ids(owner_session_id, object_ids);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.unregister_runtime_remote_object_ids(owner_session_id, object_ids);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.unregister_runtime_remote_object_ids(object_ids);
        });
    }

    pub(crate) fn unregister_runtime_remote_object_ids_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        object_ids: &[String],
    ) {
        if owner.session_id().is_some() {
            self.unregister_runtime_remote_object_ids_for_session_owner(
                owner.session_id(),
                object_ids,
            );
            return;
        }
        let _ = self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            |state| state.unregister_runtime_remote_object_ids(object_ids),
        );
    }

    pub(crate) fn unregister_runtime_remote_object_group_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        object_group: &str,
    ) {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.unregister_runtime_remote_object_group(owner_session_id, object_group);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.unregister_runtime_remote_object_group(owner_session_id, object_group);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.unregister_runtime_remote_object_group(object_group);
        });
    }

    pub(crate) fn unregister_runtime_remote_object_group_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        object_group: &str,
    ) {
        if owner.session_id().is_some() {
            self.unregister_runtime_remote_object_group_for_session_owner(
                owner.session_id(),
                object_group,
            );
            return;
        }
        let _ = self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            |state| state.unregister_runtime_remote_object_group(object_group),
        );
    }

    pub(crate) fn clear_runtime_remote_object_tracking_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.clear_runtime_remote_object_tracking(owner_session_id);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.clear_runtime_remote_object_tracking(owner_session_id);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.clear_runtime_remote_object_tracking();
        });
    }

    pub(crate) fn clear_runtime_remote_object_tracking_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) {
        if owner.session_id().is_some() {
            self.clear_runtime_remote_object_tracking_for_session_owner(owner.session_id());
            return;
        }
        let _ = self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            DevToolsSessionState::clear_runtime_remote_object_tracking,
        );
    }

    pub(crate) fn record_runtime_contexts_reported_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.record_runtime_contexts_reported_to_frontend(owner_session_id);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.record_runtime_contexts_reported_to_frontend(owner_session_id);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.record_runtime_contexts_reported_to_frontend();
        });
    }

    pub(crate) fn record_runtime_contexts_reported_for_owner(&mut self, owner: &CommandOwnerScope) {
        if owner.session_id().is_some() {
            self.record_runtime_contexts_reported_for_session_owner(owner.session_id());
            return;
        }
        let _ = self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            DevToolsSessionState::record_runtime_contexts_reported_to_frontend,
        );
    }

    pub(crate) fn record_runtime_contexts_cleared_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.record_runtime_contexts_cleared_for_frontend(owner_session_id);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.record_runtime_contexts_cleared_for_frontend(owner_session_id);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.record_runtime_contexts_cleared_for_frontend();
        });
    }

    pub(crate) fn record_runtime_contexts_cleared_for_owner(&mut self, owner: &CommandOwnerScope) {
        if owner.session_id().is_some() {
            self.record_runtime_contexts_cleared_for_session_owner(owner.session_id());
            return;
        }
        let _ = self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            DevToolsSessionState::record_runtime_contexts_cleared_for_frontend,
        );
    }

    pub(crate) fn record_runtime_context_protocol_event_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        event: &RuntimeContextProtocolEvent,
    ) {
        if session_id.is_none() {
            return;
        }
        if let Some(target) = self.shared_worker_target_for_session_mut(session_id) {
            match event {
                RuntimeContextProtocolEvent::Created(event) => {
                    target.record_runtime_execution_context_created_event(event);
                }
                RuntimeContextProtocolEvent::Destroyed(event) => {
                    target.record_runtime_execution_context_destroyed_event(event);
                }
                RuntimeContextProtocolEvent::Cleared(_) => {
                    target.record_runtime_execution_contexts_cleared_event();
                }
            }
            return;
        }
        if let Some(target) = self.service_worker_target_for_session_mut(session_id) {
            match event {
                RuntimeContextProtocolEvent::Created(event) => {
                    target.record_runtime_execution_context_created_event(event);
                }
                RuntimeContextProtocolEvent::Destroyed(event) => {
                    target.record_runtime_execution_context_destroyed_event(event);
                }
                RuntimeContextProtocolEvent::Cleared(_) => {
                    target.record_runtime_execution_contexts_cleared_event();
                }
            }
        }
    }

    pub(crate) fn record_runtime_context_protocol_event_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        event: &RuntimeContextProtocolEvent,
    ) {
        if owner.session_id().is_some() {
            self.record_runtime_context_protocol_event_for_session_owner(owner.session_id(), event);
        }
    }

    pub(crate) fn clear_runtime_remote_objects_for_realm_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        realm_id: &str,
    ) {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.clear_runtime_remote_objects_for_realm(owner_session_id, realm_id);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.clear_runtime_remote_objects_for_realm(owner_session_id, realm_id);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.clear_runtime_remote_objects_for_realm(realm_id);
        });
    }

    pub(crate) fn clear_runtime_remote_objects_for_realm_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        realm_id: &str,
    ) {
        if owner.session_id().is_some() {
            self.clear_runtime_remote_objects_for_realm_for_session_owner(
                owner.session_id(),
                realm_id,
            );
            return;
        }
        let _ = self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            |state| state.clear_runtime_remote_objects_for_realm(realm_id),
        );
    }

    pub(crate) fn register_runtime_remote_object_ids_for_session_owner_with_realm(
        &mut self,
        session_id: Option<&str>,
        object_ids: Vec<String>,
        realm_id: &str,
    ) {
        if object_ids.is_empty() {
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_ids_with_realm(
                owner_session_id,
                object_ids,
                realm_id,
            );
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_ids_with_realm(
                owner_session_id,
                object_ids,
                realm_id,
            );
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.register_runtime_remote_object_ids_with_realm(object_ids, realm_id);
        });
    }

    pub(crate) fn register_runtime_remote_object_ids_for_owner_with_realm(
        &mut self,
        owner: &CommandOwnerScope,
        object_ids: Vec<String>,
        realm_id: &str,
    ) {
        if owner.session_id().is_some() {
            self.register_runtime_remote_object_ids_for_session_owner_with_realm(
                owner.session_id(),
                object_ids,
                realm_id,
            );
            return;
        }
        if object_ids.is_empty() {
            return;
        }
        let _ = self.with_target_devtools_session_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            |state| state.register_runtime_remote_object_ids_with_realm(object_ids, realm_id),
        );
    }

    pub(crate) fn register_runtime_remote_object_alias_for_session_owner_with_realm(
        &mut self,
        session_id: Option<&str>,
        alias_id: String,
        object_id: String,
        realm_id: &str,
    ) {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_alias_with_realm(
                owner_session_id,
                alias_id,
                object_id,
                realm_id,
            );
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_alias_with_realm(
                owner_session_id,
                alias_id,
                object_id,
                realm_id,
            );
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.register_runtime_remote_object_alias_with_realm(alias_id, object_id, realm_id);
        });
    }

    fn register_runtime_remote_object_ids_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        object_ids: Vec<String>,
    ) {
        if object_ids.is_empty() {
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_ids_for_session(owner_session_id, object_ids);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_ids_for_session(owner_session_id, object_ids);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.register_runtime_remote_object_ids(object_ids);
        });
    }

    fn register_runtime_remote_object_ids_for_session_owner_with_group(
        &mut self,
        session_id: Option<&str>,
        object_ids: Vec<String>,
        object_group: &str,
    ) {
        if object_ids.is_empty() {
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_ids_with_group(
                owner_session_id,
                object_ids,
                object_group,
            );
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_ids_with_group(
                owner_session_id,
                object_ids,
                object_group,
            );
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.register_runtime_remote_object_ids_with_group(object_ids, object_group);
        });
    }

    fn runtime_remote_object_owner_identity_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<RuntimeRemoteObjectOwnerIdentity> {
        if let Some(CdpSessionRoute::SharedWorkerTarget {
            browser_context_id,
            target_id,
        }) = self.session_route(session_id)
        {
            let target = self
                .browser_context_by_id(&browser_context_id)?
                .shared_worker_target(&target_id)?;
            return Some(RuntimeRemoteObjectOwnerIdentity::SharedWorker {
                browser_context_id,
                instance_id: target.renderer_instance_id,
                session_id: session_id?.to_owned(),
            });
        }
        if let Some(CdpSessionRoute::DedicatedWorkerTarget {
            browser_context_id,
            target_id,
        }) = self.session_route(session_id)
        {
            let target = self
                .browser_context_by_id(&browser_context_id)?
                .dedicated_worker_target(&target_id)?;
            return Some(RuntimeRemoteObjectOwnerIdentity::DedicatedWorker {
                browser_context_id,
                instance_id: target.renderer_instance_id,
                session_id: session_id?.to_owned(),
            });
        }
        if let Some(CdpSessionRoute::ServiceWorkerTarget {
            browser_context_id,
            target_id,
        }) = self.session_route(session_id)
        {
            let target = self
                .browser_context_by_id(&browser_context_id)?
                .service_worker_target(&target_id)?;
            return Some(RuntimeRemoteObjectOwnerIdentity::ServiceWorker {
                browser_context_id,
                version_id: target.renderer_version_id,
                session_id: session_id?.to_owned(),
            });
        }
        let (browser_context_id, target_id) = self.target_owner_identity_for_session(session_id)?;
        let devtools_session_id =
            self.target_devtools_auxiliary_session_id_for_session(session_id)?;
        Some(RuntimeRemoteObjectOwnerIdentity::Page {
            browser_context_id,
            target_id,
            devtools_session_id,
        })
    }

    fn runtime_remote_object_owner_identity_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<RuntimeRemoteObjectOwnerIdentity> {
        if owner.session_id().is_some() {
            return self.runtime_remote_object_owner_identity_for_session(owner.session_id());
        }
        let (browser_context_id, target_id) =
            self.target_owner_identity_for_route(owner.session_id(), owner.session_owner_route())?;
        Some(RuntimeRemoteObjectOwnerIdentity::Page {
            browser_context_id,
            target_id,
            devtools_session_id: None,
        })
    }

    fn runtime_remote_object_id_known_for_different_owner(
        &self,
        owner: &RuntimeRemoteObjectOwnerIdentity,
        object_id: &str,
    ) -> bool {
        for browser_context in self.browser_contexts() {
            let current_page_owner = match owner {
                RuntimeRemoteObjectOwnerIdentity::Page {
                    browser_context_id,
                    target_id,
                    devtools_session_id,
                } if browser_context_id == &browser_context.id => Some((
                    target_id
                        .as_deref()
                        .or_else(|| browser_context.active_target_id()),
                    devtools_session_id.as_deref(),
                )),
                _ => None,
            };
            if browser_context.page_targets.iter().any(|target| {
                if current_page_owner
                    .is_some_and(|(target_id, _)| target_id == Some(target.target_id()))
                {
                    target.has_runtime_remote_object_id_for_different_session(
                        current_page_owner.and_then(|(_, session_id)| session_id),
                        object_id,
                    )
                } else {
                    target.has_runtime_remote_object_id(object_id)
                }
            }) {
                return true;
            }

            for target in browser_context.shared_worker_targets.values() {
                let shared_worker_is_current_owner = matches!(
                    owner,
                    RuntimeRemoteObjectOwnerIdentity::SharedWorker {
                        browser_context_id,
                        instance_id,
                        session_id,
                    } if browser_context_id == &browser_context.id
                        && instance_id == &target.renderer_instance_id
                        && target.is_session(session_id)
                );
                if !shared_worker_is_current_owner
                    && target.any_session_has_runtime_remote_object_id(object_id)
                {
                    return true;
                }
            }
            for target in browser_context.dedicated_worker_targets.values() {
                let dedicated_worker_is_current_owner = matches!(
                    owner,
                    RuntimeRemoteObjectOwnerIdentity::DedicatedWorker {
                        browser_context_id,
                        instance_id,
                        session_id,
                    } if browser_context_id == &browser_context.id
                        && *instance_id == target.renderer_instance_id
                        && target.is_session(session_id)
                );
                if !dedicated_worker_is_current_owner
                    && target.any_session_has_runtime_remote_object_id(object_id)
                {
                    return true;
                }
            }
            for target in browser_context.service_worker_targets.values() {
                let service_worker_is_current_owner = matches!(
                    owner,
                    RuntimeRemoteObjectOwnerIdentity::ServiceWorker {
                        browser_context_id,
                        version_id,
                        session_id,
                    } if browser_context_id == &browser_context.id
                        && *version_id == target.renderer_version_id
                        && target.is_session(session_id)
                );
                if !service_worker_is_current_owner
                    && target.any_session_has_runtime_remote_object_id(object_id)
                {
                    return true;
                }
            }
        }
        false
    }

    pub(crate) async fn release_shared_worker_runtime_remote_objects_for_session_best_effort_async(
        &mut self,
        session_id: &str,
    ) {
        let Some((object_groups, object_ids)) = self
            .shared_worker_target_for_session_mut(Some(session_id))
            .map(|target| target.take_runtime_remote_object_cleanup_plan(session_id))
        else {
            return;
        };
        if object_groups.is_empty() && object_ids.is_empty() {
            return;
        }

        let mut command_id = SHARED_WORKER_RUNTIME_REMOTE_OBJECT_CLEANUP_COMMAND_ID_BASE;
        for object_group in object_groups {
            let raw_json = json!({
                "id": command_id,
                "method": "Runtime.releaseObjectGroup",
                "params": { "objectGroup": object_group }
            })
            .to_string();
            if let Err(error) = self
                .dispatch_shared_worker_runtime_helper_protocol_message_for_session_async(
                    Some(session_id),
                    &raw_json,
                    command_id,
                )
                .await
            {
                tracing::warn!(
                    object_group = %object_group,
                    error = %error,
                    "failed to release shared worker Runtime object group during target detach"
                );
            }
            command_id = command_id.saturating_add(1);
        }
        for object_id in object_ids {
            let raw_json = json!({
                "id": command_id,
                "method": "Runtime.releaseObject",
                "params": { "objectId": object_id }
            })
            .to_string();
            if let Err(error) = self
                .dispatch_shared_worker_runtime_helper_protocol_message_for_session_async(
                    Some(session_id),
                    &raw_json,
                    command_id,
                )
                .await
            {
                tracing::warn!(
                    object_id = %object_id,
                    error = %error,
                    "failed to release shared worker Runtime object during target detach"
                );
            }
            command_id = command_id.saturating_add(1);
        }
    }

    /// Routes a batch of inspector messages into `out`, demultiplexing by id.
    ///
    /// For each message:
    /// - if it carries an `id` matching a pending inspector await registry entry,
    ///   the entry is consumed and the message is sent with that entry's
    ///   `session_id` (regardless of `current_session_id`);
    /// - otherwise if its `id` matches `current_cmd_id`, the message is sent
    ///   with `current_session_id`;
    /// - otherwise the message is dropped (orphan id; logs at warn);
    /// - notifications (no `id`) are routed as background events with
    ///   `current_session_id`.
    ///
    /// Returns true if a message matching `current_cmd_id` was produced (either
    /// via a pending entry or directly).
    #[cfg(test)]
    pub(crate) fn route_inspector_messages_into(
        &mut self,
        messages: Vec<Value>,
        current_cmd_id: Option<u64>,
        current_session_id: Option<&str>,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> bool {
        self.route_inspector_messages_with_background_events_into(
            messages,
            current_cmd_id,
            current_session_id,
            response_events,
            background_events,
        )
    }

    #[cfg(test)]
    pub(crate) fn route_inspector_messages_with_background_events_into(
        &mut self,
        messages: Vec<Value>,
        current_cmd_id: Option<u64>,
        current_session_id: Option<&str>,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> bool {
        let owner = CommandOwnerScope::capture(self, current_session_id);
        let mut current_seen = false;
        for message in messages {
            current_seen |= self
                .route_runtime_inspector_protocol_message_for_owner_with_background_events_into(
                    message,
                    current_cmd_id,
                    &owner,
                    response_events,
                    background_events,
                );
        }
        current_seen
    }

    pub(crate) fn route_renderer_runtime_inspector_messages_for_owner_with_background_events_into(
        &mut self,
        messages: Vec<RendererRuntimeInspectorMessage>,
        current_cmd_id: Option<u64>,
        owner: &CommandOwnerScope,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> bool {
        let current_session_id = owner.session_id();
        let mut current_seen = false;
        for message in messages {
            match message {
                RendererRuntimeInspectorMessage::RuntimeContext(event) => {
                    let mut event = RuntimeContextProtocolEvent::from_restore_event(event);
                    qualify_runtime_context_protocol_event_for_owner_typed(self, &mut event, owner);
                    apply_runtime_context_protocol_event_side_effects_for_owner_typed(
                        self, &event, owner,
                    );
                    let mut runtime_context_events = Vec::new();
                    emit_runtime_context_protocol_background_event_typed(
                        &mut runtime_context_events,
                        event,
                        current_session_id,
                    );
                    if current_cmd_id.is_some() {
                        response_events.extend(runtime_context_events);
                    } else {
                        background_events.extend(runtime_context_events);
                    }
                }
                RendererRuntimeInspectorMessage::Protocol(message) => {
                    current_seen |= self
                        .route_runtime_inspector_protocol_message_for_owner_with_background_events_into(
                            message.into_value(),
                            current_cmd_id,
                            owner,
                            response_events,
                            background_events,
                        );
                }
            }
        }
        current_seen
    }

    pub(crate) fn route_renderer_runtime_command_output_for_owner_into(
        &mut self,
        output: RendererRuntimeCommandOutput,
        current_cmd_id: Option<u64>,
        owner: &CommandOwnerScope,
        ordered_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> bool {
        let current_session_id = owner.session_id();
        let attachment_is_current =
            output
                .renderer_agent_attachment_id()
                .is_none_or(|attachment_id| {
                    self.current_renderer_agent_attachment_id_for_owner(owner)
                        == Some(attachment_id)
                });
        if !attachment_is_current {
            tracing::debug!(
                attachment_id = ?output.renderer_agent_attachment_id(),
                session_id = current_session_id,
                "dropping renderer command output from a stale attachment"
            );
            return false;
        }
        if output.renderer_agent_attachment_id().is_some()
            && let Some(state) = output.v8_state_update().cloned()
        {
            let _ = self.merge_v8_inspector_session_state_for_route(
                owner.session_id(),
                owner.session_owner_route(),
                state,
            );
        }
        let mut current_seen = false;
        for message in output.into_messages() {
            let mut response_events = Vec::new();
            let mut background_events = Vec::new();
            current_seen |= self
                .route_renderer_runtime_inspector_messages_for_owner_with_background_events_into(
                    vec![message],
                    current_cmd_id,
                    owner,
                    &mut response_events,
                    &mut background_events,
                );
            ordered_events.extend(response_events);
            ordered_events.extend(background_events);
        }
        current_seen
    }

    pub(crate) fn route_renderer_command_turn_output_for_owner_into(
        &mut self,
        output: RendererCommandTurnOutput,
        current_cmd_id: Option<u64>,
        owner: &CommandOwnerScope,
        response_flush: &CommandResponseFlushContext,
        ordered_events: &mut Vec<BackgroundProtocolEvent>,
        post_response_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> (bool, Option<moli_core::RendererOutputFence>) {
        let mut command = CommandDispatchContext::new(response_flush.clone());
        let completion = command.consume_renderer_command_turn_output(output);
        ordered_events.extend(command.take_protocol_events());
        post_response_events.extend(command.take_post_response_events());
        let renderer_output_predecessor = command.take_renderer_output_predecessor();
        let Some(output) = completion.into_runtime_inspector_output() else {
            tracing::error!("Runtime command turn completed with a non-Runtime reply");
            return (false, renderer_output_predecessor);
        };
        let response_seen = self.route_renderer_runtime_command_output_for_owner_into(
            output,
            current_cmd_id,
            owner,
            ordered_events,
        );
        (response_seen, renderer_output_predecessor)
    }

    fn route_runtime_inspector_protocol_message_for_owner_with_background_events_into(
        &mut self,
        message: Value,
        current_cmd_id: Option<u64>,
        owner: &CommandOwnerScope,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> bool {
        let current_session_id = owner.session_id();
        let mut message = message;
        let id = message.get("id").and_then(Value::as_u64);
        match id {
            Some(id) => {
                let entry = self.remove_pending_inspector_await_for_owner(id, owner);
                if let Some(entry) = entry {
                    let owner_route = owner
                        .session_owner_route()
                        .cloned()
                        .or_else(|| self.runtime_await_owner_route_for_session(entry.session_id()));
                    let response = OwnerRuntimeResponse::from_pending_inspector_await(
                        id,
                        entry,
                        owner_route,
                        message,
                    );
                    return self.route_owner_runtime_response_into(
                        response,
                        current_cmd_id,
                        response_events,
                        background_events,
                    );
                }
                if Some(id) == current_cmd_id {
                    self.register_runtime_remote_object_ids_from_value_for_owner(owner, &message);
                    let response =
                        BackgroundCommandResponsePayload::from_owned_runtime_inspector_message(
                            message,
                        );
                    response_events.push(BackgroundProtocolEvent::command_response(
                        Some(id),
                        current_session_id,
                        response,
                    ));
                    return true;
                }
                tracing::warn!(
                    id,
                    "dropping inspector reply with no matching pending await"
                );
            }
            None => {
                if message.get("method").and_then(Value::as_str) == Some("Page.windowOpen") {
                    let params = message.get("params").unwrap_or(&Value::Null);
                    let Some(url) = params.get("url").and_then(Value::as_str) else {
                        tracing::warn!("dropping Page.windowOpen without a string url");
                        return false;
                    };
                    let Some(window_name) = params.get("windowName").and_then(Value::as_str) else {
                        tracing::warn!("dropping Page.windowOpen without a string windowName");
                        return false;
                    };
                    let Some(window_features) =
                        params.get("windowFeatures").and_then(Value::as_array)
                    else {
                        tracing::warn!("dropping Page.windowOpen without windowFeatures");
                        return false;
                    };
                    let Some(user_gesture) = params.get("userGesture").and_then(Value::as_bool)
                    else {
                        tracing::warn!("dropping Page.windowOpen without userGesture");
                        return false;
                    };
                    let Some(window_features) = window_features
                        .iter()
                        .map(Value::as_str)
                        .collect::<Option<Vec<_>>>()
                    else {
                        tracing::warn!("dropping Page.windowOpen with a non-string window feature");
                        return false;
                    };
                    let window_features = window_features
                        .into_iter()
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    crate::domains::page::emit_page_window_open_background_events_for_route(
                        self,
                        background_events,
                        owner.session_id(),
                        owner.session_owner_route(),
                        url,
                        window_name,
                        &window_features,
                        user_gesture,
                    );
                    return false;
                }
                self.register_runtime_remote_object_ids_from_value_for_owner(owner, &message);
                if let Some(mut event) =
                    RuntimeContextProtocolEvent::from_context_protocol_message(message.clone())
                {
                    qualify_runtime_context_protocol_event_for_owner_typed(self, &mut event, owner);
                    apply_runtime_context_protocol_event_side_effects_for_owner_typed(
                        self, &event, owner,
                    );
                    let mut runtime_context_events = Vec::new();
                    emit_runtime_context_protocol_background_event_typed(
                        &mut runtime_context_events,
                        event,
                        current_session_id,
                    );
                    if current_cmd_id.is_some() {
                        response_events.extend(runtime_context_events);
                    } else {
                        background_events.extend(runtime_context_events);
                    }
                    return false;
                }
                if let Some(session_id) = current_session_id {
                    message["sessionId"] = json!(session_id);
                } else if let Some(map) = message.as_object_mut() {
                    map.remove("sessionId");
                }
                background_events.push(protocol_message_background_event(message));
            }
        }
        false
    }

    pub(crate) async fn route_scheduler_deferred_runtime_inspector_response_into(
        &mut self,
        mut response: RuntimeInspectorResponseReady,
        owner: &CommandOwnerScope,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> (bool, Option<moli_core::RendererOutputFence>) {
        response.bind_owner(owner);
        let Some(response) = self.resolve_runtime_inspector_response_ready(response) else {
            return (false, None);
        };
        // The V8 response and the renderer turn publication travel over
        // separate channels. Preserve the exact Page-stream cursor while
        // consuming the response so the command completion cannot overtake
        // owner actions (for example popup target creation) produced by the
        // same turn.
        let (current_cmd_id, output, renderer_output_predecessor) =
            response.into_renderer_command_output();
        let (renderer_agent_attachment_id, v8_state_update, messages) = output.into_parts();
        let mut ordered_events = Vec::new();
        let output = RendererRuntimeCommandOutput::from_parts(
            renderer_agent_attachment_id,
            v8_state_update,
            messages,
        );
        let current_seen = self.route_renderer_runtime_command_output_for_owner_into(
            output,
            Some(current_cmd_id),
            owner,
            &mut ordered_events,
        );
        response_events.extend(ordered_events);
        let _ = background_events;
        (current_seen, renderer_output_predecessor)
    }

    pub fn route_registered_runtime_inspector_response_into(
        &mut self,
        mut response: RuntimeInspectorResponseReady,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) {
        let owner = response
            .owner()
            .cloned()
            .or_else(|| response.session_id().map(CommandOwnerScope::for_session));
        let Some(owner) = owner else {
            tracing::debug!(
                command_id = response.command_id(),
                "dropping implicit runtime Inspector response without an exact owner"
            );
            return;
        };
        response.bind_owner(&owner);
        let Some(response) = self.resolve_runtime_inspector_response_ready(response) else {
            return;
        };
        let message = response.into_protocol_message_for_typed_runtime_route();
        self.route_runtime_inspector_protocol_message_for_owner_with_background_events_into(
            message,
            None,
            &owner,
            response_events,
            background_events,
        );
    }

    pub(crate) fn resolve_runtime_inspector_response_ready(
        &mut self,
        mut response: RuntimeInspectorResponseReady,
    ) -> Option<RuntimeInspectorResponseReady> {
        if response.has_bound_renderer_call_id() {
            return Some(response);
        }
        let command_id = response.command_id();
        let session_id = response.session_id().map(str::to_owned);
        let owner = response.owner().cloned();
        let correlation = if let Some(renderer_call_id) = response.renderer_call_id() {
            let dispatched_attachment_id = response.renderer_agent_attachment_id();
            // A lease can complete immediately before attachment cutover while
            // its response-ready event is still queued. The registry mapping
            // proves that this exact old lease won before rotation; requiring
            // the attachment to remain current here would lose that response.
            match owner.as_ref() {
                Some(owner) => self.take_renderer_call_for_frontend_if_matches_for_owner(
                    owner,
                    command_id,
                    renderer_call_id,
                    dispatched_attachment_id,
                ),
                None => self.take_renderer_call_for_frontend_if_matches_for_session_owner(
                    session_id.as_deref(),
                    command_id,
                    renderer_call_id,
                    dispatched_attachment_id,
                ),
            }
        } else {
            match owner.as_ref() {
                Some(owner) => self.take_renderer_call_for_frontend_for_owner(owner, command_id),
                None => self.take_renderer_call_for_frontend_for_session_owner(
                    session_id.as_deref(),
                    command_id,
                ),
            }
        };
        let Some(correlation) = correlation else {
            tracing::debug!(
                command_id,
                session_id,
                "dropping runtime Inspector response without a pending renderer correlation"
            );
            return None;
        };
        debug_assert!(
            response.renderer_call_id().is_none()
                || correlation.dispatched_attachment_id()
                    == response.renderer_agent_attachment_id()
        );
        response.bind_renderer_call_id(correlation.renderer_call_id());
        Some(response)
    }

    fn restore_frontend_command_ids_in_runtime_messages(
        &mut self,
        session_id: Option<&str>,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
        messages: &mut [RendererRuntimeInspectorMessage],
    ) {
        if dispatched_attachment_id.is_some_and(|attachment_id| {
            !self.renderer_agent_attachment_is_current_for_session_owner(session_id, attachment_id)
        }) {
            return;
        }
        for message in messages {
            let RendererRuntimeInspectorMessage::Protocol(message) = message else {
                continue;
            };
            let Some(renderer_call_id) = message.renderer_call_id() else {
                continue;
            };
            let Some(correlation) = self
                .take_frontend_command_for_renderer_if_attachment_matches_for_session_owner(
                    session_id,
                    renderer_call_id,
                    dispatched_attachment_id,
                )
            else {
                continue;
            };
            debug_assert_eq!(
                correlation.dispatched_attachment_id(),
                dispatched_attachment_id
            );
            message.value_mut()["id"] = json!(correlation.frontend_command_id().get());
        }
    }

    fn restore_frontend_command_ids_in_runtime_messages_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
        messages: &mut [RendererRuntimeInspectorMessage],
    ) {
        if dispatched_attachment_id.is_some_and(|attachment_id| {
            self.current_renderer_agent_attachment_id_for_owner(owner) != Some(attachment_id)
        }) {
            return;
        }
        for message in messages {
            let RendererRuntimeInspectorMessage::Protocol(message) = message else {
                continue;
            };
            let Some(renderer_call_id) = message.renderer_call_id() else {
                continue;
            };
            let Some(correlation) = self
                .take_frontend_command_for_renderer_if_attachment_matches_for_owner(
                    owner,
                    renderer_call_id,
                    dispatched_attachment_id,
                )
            else {
                continue;
            };
            debug_assert_eq!(
                correlation.dispatched_attachment_id(),
                dispatched_attachment_id
            );
            message.value_mut()["id"] = json!(correlation.frontend_command_id().get());
        }
    }

    /// Resolves terminal responses carried by the renderer's concrete
    /// DevTools session output stream.
    ///
    /// Notifications have no renderer call id and remain in place. A response
    /// without an exact `(session, attachment, renderer call)` registration is
    /// stale and must not expose the renderer-private id on the wire.
    pub(crate) fn restore_frontend_command_ids_in_devtools_session_output(
        &mut self,
        session_id: Option<&str>,
        dispatched_attachment_id: RendererAgentAttachmentId,
        messages: &mut Vec<RendererRuntimeInspectorMessage>,
    ) {
        self.restore_frontend_command_ids_in_devtools_session_output_with_projection(
            session_id,
            Some(dispatched_attachment_id),
            messages,
            true,
        );
    }

    /// Resolves terminal responses carried by a Worker target's concrete
    /// renderer output stream. Worker Inspector sessions do not use a Page
    /// attachment id; the exact worker session lifetime is validated by the
    /// prepared-output authority at the same ingress boundary.
    pub(crate) fn restore_frontend_command_ids_in_worker_devtools_session_output(
        &mut self,
        session_id: &str,
        messages: &mut Vec<RendererRuntimeInspectorMessage>,
    ) {
        self.restore_frontend_command_ids_in_devtools_session_output_with_projection(
            Some(session_id),
            None,
            messages,
            true,
        );
    }

    /// Resolves a terminal response which won its renderer response lease
    /// before navigation, but whose old Page journal reached protocol ingress
    /// after the replacement attachment committed.
    ///
    /// The exact `(session, renderer call, attachment)` correlation remains
    /// the terminal authority in this race.  Notifications and V8 state from
    /// the retired attachment are discarded by the caller, and remote-object
    /// ownership is intentionally not projected onto the replacement
    /// document.
    pub(crate) fn restore_frontend_command_ids_in_retired_devtools_session_output(
        &mut self,
        session_id: Option<&str>,
        dispatched_attachment_id: RendererAgentAttachmentId,
        messages: &mut Vec<RendererRuntimeInspectorMessage>,
    ) {
        self.restore_frontend_command_ids_in_devtools_session_output_with_projection(
            session_id,
            Some(dispatched_attachment_id),
            messages,
            false,
        );
    }

    fn restore_frontend_command_ids_in_devtools_session_output_with_projection(
        &mut self,
        session_id: Option<&str>,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
        messages: &mut Vec<RendererRuntimeInspectorMessage>,
        project_runtime_object_ownership: bool,
    ) {
        messages.retain_mut(|message| {
            let RendererRuntimeInspectorMessage::Protocol(message) = message else {
                return true;
            };
            let Some(renderer_call_id) = message.renderer_call_id() else {
                return true;
            };
            let descriptor = self
                .renderer_command_descriptor_for_renderer_if_attachment_matches_for_session_owner(
                    session_id,
                    renderer_call_id,
                    dispatched_attachment_id,
                );
            let result_object_group = descriptor.as_ref().and_then(|descriptor| {
                self.runtime_result_object_group_for_renderer_command_descriptor(
                    session_id, descriptor,
                )
            });
            let Some(correlation) = self
                .take_frontend_command_for_renderer_if_attachment_matches_for_session_owner(
                    session_id,
                    renderer_call_id,
                    dispatched_attachment_id,
                )
            else {
                tracing::debug!(
                    session_id,
                    renderer_call_id = renderer_call_id.get(),
                    attachment_id = ?dispatched_attachment_id.map(RendererAgentAttachmentId::get),
                    "dropping DevTools session response without a live renderer correlation"
                );
                return false;
            };
            message.value_mut()["id"] = json!(correlation.frontend_command_id().get());
            if project_runtime_object_ownership && message.value().get("result").is_some() {
                if let Some(object_group) = result_object_group.as_deref() {
                    self.register_runtime_remote_object_ids_from_value_for_session_owner_with_group(
                        session_id,
                        message.value(),
                        object_group,
                    );
                } else {
                    self.register_runtime_remote_object_ids_from_value_for_session_owner(
                        session_id,
                        message.value(),
                    );
                }
            }
            true
        });
    }

    fn runtime_result_object_group_for_renderer_command_descriptor(
        &self,
        session_id: Option<&str>,
        descriptor: &RendererCommandDescriptor,
    ) -> Option<String> {
        let command = serde_json::from_str::<Value>(descriptor.frontend_payload()).ok()?;
        let method = command.get("method")?.as_str()?;
        let params = command.get("params")?.as_object()?;
        match method {
            "Runtime.evaluate" | "Runtime.runScript" => params
                .get("objectGroup")
                .and_then(Value::as_str)
                .map(str::to_owned),
            "Runtime.callFunctionOn" => params
                .get("objectGroup")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    self.runtime_remote_object_group_for_session_owner(
                        session_id,
                        params.get("objectId")?.as_str()?,
                    )
                }),
            "Runtime.getProperties" => self.runtime_remote_object_group_for_session_owner(
                session_id,
                params.get("objectId")?.as_str()?,
            ),
            "Runtime.awaitPromise" => self.runtime_remote_object_group_for_session_owner(
                session_id,
                params.get("promiseObjectId")?.as_str()?,
            ),
            "Runtime.queryObjects" => params
                .get("objectGroup")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    self.runtime_remote_object_group_for_session_owner(
                        session_id,
                        params.get("prototypeObjectId")?.as_str()?,
                    )
                }),
            _ => None,
        }
    }

    fn start_or_enqueue_registered_runtime_inspector_response_ready(
        &self,
        command_id: u64,
        owner: &CommandOwnerScope,
        mut response_rx: RuntimeInspectorResponseReceiver,
    ) -> bool {
        let Some(response_tx) = self.runtime_inspector_response_ready_sender() else {
            return false;
        };
        let owner = owner.clone();
        // Keep both completion timings on the same response-ready lane. If the
        // renderer callback has already completed, enqueue it immediately; if
        // not, spawn a waiter that will enqueue the same event later.
        match response_rx.try_recv() {
            Ok(completion) => {
                let _ = response_tx.send(crate::conn::RuntimeInspectorResponseReady::for_owner(
                    command_id,
                    &owner,
                    Ok(completion),
                ));
                return true;
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                let _ = response_tx.send(crate::conn::RuntimeInspectorResponseReady::for_owner(
                    command_id,
                    &owner,
                    Err("RuntimeInspectorResponseCanceled".to_owned()),
                ));
                return true;
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
        }
        tokio::task::spawn_local(async move {
            let response = response_rx
                .await
                .map_err(|_| "RuntimeInspectorResponseCanceled".to_owned());
            let _ = response_tx.send(crate::conn::RuntimeInspectorResponseReady::for_owner(
                command_id, &owner, response,
            ));
        });
        true
    }

    fn route_owner_runtime_response_into(
        &mut self,
        response: OwnerRuntimeResponse,
        current_cmd_id: Option<u64>,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> bool {
        let command_id = response.command_id;
        self.complete_runtime_await_job(command_id, response.session_id());
        self.trace_owner_runtime_response_route(&response);
        let current_seen = Some(command_id) == current_cmd_id;
        match self.route_bidi_channel_listener_owner_runtime_response(&response) {
            BidiChannelListenerRoute::NotListener => {}
            BidiChannelListenerRoute::Consumed => return current_seen,
            BidiChannelListenerRoute::Event(event) => {
                background_events.push(event);
                return current_seen;
            }
        }
        let routed_session_id = response.session_id().map(str::to_owned);
        if let Some(object_group) = response.object_group() {
            self.register_runtime_remote_object_ids_from_value_for_route_with_group(
                routed_session_id.as_deref(),
                response.owner_route(),
                &response.message,
                object_group,
            );
        } else {
            self.register_runtime_remote_object_ids_from_value_for_route(
                routed_session_id.as_deref(),
                response.owner_route(),
                &response.message,
            );
        }
        let mut message = response.into_protocol_message();
        if let Some(session_id) = routed_session_id.as_deref() {
            message["sessionId"] = json!(session_id);
        } else if let Some(map) = message.as_object_mut() {
            map.remove("sessionId");
        }
        response_events.push(protocol_message_background_event(message));
        current_seen
    }

    fn trace_owner_runtime_response_route(&mut self, response: &OwnerRuntimeResponse) {
        let current_route = self.runtime_await_owner_route_for_session(response.session_id());
        if current_route != response.owner_route {
            tracing::debug!(
                command_id = response.command_id,
                session_id = response.session_id(),
                response_owner_route = ?response.owner_route(),
                current_owner_route = ?current_route,
                "owner runtime response route no longer matches current session owner"
            );
        }
        self.record_runtime_await_trace(
            "owner_runtime_response_route",
            Some(response.command_id),
            response.session_id(),
            json!({
                "ownerRoute": response.owner_route().map(|route| format!("{route:?}")),
                "currentOwnerRoute": current_route.as_ref().map(|route| format!("{route:?}")),
            }),
        );
    }

    fn route_bidi_channel_listener_owner_runtime_response(
        &mut self,
        response: &OwnerRuntimeResponse,
    ) -> BidiChannelListenerRoute {
        let Some(residence) = response.bidi_channel_listener().cloned() else {
            return BidiChannelListenerRoute::NotListener;
        };
        let owner = residence.owner().clone();
        if !owner.is_current(self) {
            tracing::debug!(
                command_id = response.command_id,
                session_id = owner.session_id(),
                "discarding BiDi channel listener reply for a stale Page attachment"
            );
            return BidiChannelListenerRoute::Consumed;
        }
        let listener = residence.listener();
        let message = &response.message;
        if let Some(error) = message.get("error") {
            tracing::debug!(
                ?error,
                channel = %listener.properties().channel,
                "BiDi channel listener stopped after inspector error"
            );
            self.publish_bidi_channel_object_group_release(
                owner,
                listener.channel_object_group().to_owned(),
            );
            return BidiChannelListenerRoute::Consumed;
        }
        let result = message.get("result").unwrap_or(&Value::Null);
        if let Some(exception_details) = result.get("exceptionDetails") {
            tracing::debug!(
                ?exception_details,
                channel = %listener.properties().channel,
                "BiDi channel listener stopped after JavaScript exception"
            );
            self.publish_bidi_channel_object_group_release(
                owner,
                listener.channel_object_group().to_owned(),
            );
            return BidiChannelListenerRoute::Consumed;
        }
        let remote = result.get("result").unwrap_or(&Value::Null);
        let properties = listener.properties().clone();
        let realm_id = listener.realm_id().clone();
        let data = DevToolsRemoteValue::from_cdp_remote_object(
            remote,
            matches!(properties.ownership, DevToolsResultOwnership::Root),
            Some(realm_id.clone()),
        );
        if let Some(remote_object_id) = data.handle.as_ref().or(data.shared_id.as_ref()) {
            self.register_runtime_remote_object_ids_for_owner_with_realm(
                owner.command_owner(),
                vec![remote_object_id.as_str().to_owned()],
                realm_id.as_str(),
            );
        }
        let event = BackgroundProtocolEvent::immediate_automation_event(
            json!({
                "method": "Moli.scriptMessage",
                "params": {}
            }),
            AutomationEvent::ScriptMessage(ScriptMessageEvent {
                target_id: Some(listener.target_id().clone()),
                realm_id: Some(realm_id),
                channel: properties.channel.clone(),
                data,
            }),
        );
        self.publish_bidi_channel_listener_start(residence);
        BidiChannelListenerRoute::Event(event)
    }

    pub(crate) async fn document_node_snapshot_for_runtime_remote_object_id_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        object_id: &str,
        depth: i32,
        pierce: bool,
    ) -> Result<Option<DocumentNodeObjectSnapshot>, String> {
        let include_whitespace = crate::domains::dom::dom_agent_includes_whitespace_for_route(
            self,
            owner.session_id(),
            owner.session_owner_route(),
        );
        let inspector_session_id = self.target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let pending = {
            let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
            page.start_document_node_snapshot_for_object_id_in_inspector_session(
                inspector_session_id,
                include_whitespace,
                object_id,
                depth,
                pierce,
            )
            .map_err(|error| format!("resolve runtime node snapshot failed: {error}"))?
        };
        let completion = pending
            .wait()
            .await
            .map_err(|error| format!("resolve runtime node snapshot failed: {error}"))?;
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        page.finish_document_node_snapshot_for_object_id(completion)
            .map_err(|error| format!("resolve runtime node snapshot failed: {error}"))
    }

    pub(crate) async fn document_node_snapshot_for_backend_node_id_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<Option<DocumentNodeObjectSnapshot>, String> {
        let pending = {
            let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
            page.start_document_node_snapshot_for_backend_node_id(backend_node_id, depth, pierce)
                .map_err(|error| format!("resolve backend node snapshot failed: {error}"))?
        };
        let completion = pending
            .wait()
            .await
            .map_err(|error| format!("resolve backend node snapshot failed: {error}"))?;
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        page.finish_document_node_snapshot_for_backend_node_id(completion)
            .map_err(|error| format!("resolve backend node snapshot failed: {error}"))
    }

    pub(crate) async fn register_document_bidi_node_binding_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        shared_id: &str,
        backend_node_id: u32,
    ) -> Result<(), String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.register_document_bidi_node_binding_for_owner_async(&owner, shared_id, backend_node_id)
            .await
    }

    pub(crate) async fn register_document_bidi_node_binding_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        shared_id: &str,
        backend_node_id: u32,
    ) -> Result<(), String> {
        let inspector_session_id = self.target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let pending = {
            let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
            page.start_register_document_bidi_node_binding(
                inspector_session_id,
                shared_id.to_owned(),
                backend_node_id,
            )
            .map_err(|error| format!("register BiDi node binding failed: {error}"))?
        };
        let completion = pending
            .wait()
            .await
            .map_err(|error| format!("register BiDi node binding failed: {error}"))?;
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        page.finish_register_document_bidi_node_binding(completion)
            .map_err(|error| format!("register BiDi node binding failed: {error}"))
    }

    pub(crate) async fn document_bidi_node_binding_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        shared_id: &str,
    ) -> Result<RendererDomBidiNodeBindingResolution, String> {
        let inspector_session_id = self.target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let pending = {
            let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
            page.start_document_bidi_node_binding(inspector_session_id, shared_id.to_owned())
                .map_err(|error| format!("resolve BiDi node binding failed: {error}"))?
        };
        let completion = pending
            .wait()
            .await
            .map_err(|error| format!("resolve BiDi node binding failed: {error}"))?;
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        page.finish_document_bidi_node_binding(completion)
            .map_err(|error| format!("resolve BiDi node binding failed: {error}"))
    }

    pub(crate) async fn document_bidi_node_shared_id_for_backend_node_id_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        backend_node_id: u32,
    ) -> Result<RendererDomBidiNodeSharedIdResolution, String> {
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_session(session_id);
        let pending = {
            let page = self.runtime_session_owner_page_mut(session_id)?;
            page.start_document_bidi_node_shared_id_for_backend_node_id(
                inspector_session_id,
                backend_node_id,
            )
            .map_err(|error| format!("resolve BiDi node shared id failed: {error}"))?
        };
        let completion = pending
            .wait()
            .await
            .map_err(|error| format!("resolve BiDi node shared id failed: {error}"))?;
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.finish_document_bidi_node_shared_id_for_backend_node_id(completion)
            .map_err(|error| format!("resolve BiDi node shared id failed: {error}"))
    }

    pub(crate) async fn runtime_remote_object_for_backend_node_id_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        backend_node_id: u32,
        execution_context_id: Option<i64>,
        object_group: Option<&str>,
    ) -> Result<Option<Value>, String> {
        let inspector_session_id = self.target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let pending = {
            let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
            page.start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
                inspector_session_id,
                backend_node_id,
                execution_context_id,
                object_group,
            )
            .map_err(|error| format!("resolve runtime object for backend node failed: {error}"))?
        };
        let completion = pending
            .wait()
            .await
            .map_err(|error| format!("resolve runtime object for backend node failed: {error}"))?;
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let result = page
            .finish_resolve_runtime_object_for_backend_node_id(completion)
            .map_err(|error| format!("resolve runtime object for backend node failed: {error}"))?;

        match result {
            DocumentNodeRuntimeObjectResolution::Found(remote_object) => {
                Ok(Some(remote_object.into_protocol_value()))
            }
            DocumentNodeRuntimeObjectResolution::MissingNode => Ok(None),
            DocumentNodeRuntimeObjectResolution::MissingContext => Err(
                "resolve runtime object for backend node failed: missing execution context"
                    .to_owned(),
            ),
        }
    }

    pub async fn evaluate_runtime_expression_with_await_async(
        &mut self,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value, String> {
        self.evaluate_runtime_expression_with_await_for_session_owner_async(
            None,
            expression,
            await_promise,
        )
        .await
    }

    #[cfg(test)]
    pub(crate) async fn evaluate_runtime_expression_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        expression: &str,
    ) -> Result<Value, String> {
        self.evaluate_runtime_expression_with_await_for_session_owner_async(
            session_id, expression, false,
        )
        .await
    }

    pub(crate) async fn evaluate_runtime_expression_with_await_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value, String> {
        self.evaluate_runtime_expression_for_session_owner_once_async(
            session_id,
            expression,
            await_promise,
        )
        .await
    }

    async fn evaluate_runtime_expression_for_session_owner_once_async(
        &mut self,
        session_id: Option<&str>,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value, String> {
        let payload = {
            let page = self.runtime_session_owner_page_mut(session_id)?;
            page.evaluate_runtime_expression_without_navigation_follow_with_await_async(
                expression,
                await_promise,
            )
            .await
            .map_err(|error| format!("runtime evaluation failed: {error}"))?
        };
        self.ingest_runtime_session_owner_output_updates(session_id);
        Ok(payload)
    }

    #[cfg(test)]
    pub async fn dispatch_runtime_protocol_message_async(
        &mut self,
        raw_json: &str,
    ) -> Result<Vec<Value>, String> {
        self.dispatch_runtime_protocol_message_for_session_owner_async(None, raw_json)
            .await
    }

    pub(crate) fn start_runtime_enable_events_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<PendingRuntimeEnableEventsDispatch, String> {
        let route = self.runtime_protocol_message_page_route_for_owner(owner)?;
        let inspector_session_id = self.target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let pending = page
            .start_runtime_enable_events_for_inspector_session(inspector_session_id.as_deref())
            .map_err(|error| format!("runtime enable event replay failed: {error}"))?;
        Ok(PendingRuntimeEnableEventsDispatch {
            owner: owner.clone(),
            route,
            pending,
        })
    }

    pub(crate) fn complete_runtime_enable_events(
        &mut self,
        completed: CompletedRuntimeEnableEventsDispatch,
    ) -> Result<RuntimeEnableEventsReplay, String> {
        let owner = completed.owner;
        let session_id = owner.session_id();
        let page = self.runtime_protocol_message_started_page_mut(&completed.route)?;
        let output = page
            .finish_runtime_enable_output(completed.completion)
            .map_err(|error| format!("runtime enable event replay failed: {error}"))?;
        let (attachment_id, v8_state_update, messages) = output.into_parts();
        if attachment_id != Some(completed.route.renderer_agent_attachment_id) {
            return Err(
                "Runtime.enable completed from an unexpected renderer attachment".to_owned(),
            );
        }
        if let Some(state) = v8_state_update
            && !self.merge_v8_inspector_session_state_for_route(
                owner.session_id(),
                owner.session_owner_route(),
                state,
            )
        {
            return Err("Runtime.enable completed after session owner disappeared".to_owned());
        }
        let mut replay = RuntimeEnableEventsReplay::from_renderer_messages(messages);
        let _ =
            self.set_renderer_runtime_agent_owns_page_console_api_events_for_owner(&owner, true);
        self.ingest_runtime_session_owner_output_updates_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        for event in replay.events_mut() {
            match event {
                RuntimeEnableReplayEvent::Context(event) => {
                    qualify_runtime_context_protocol_event_for_owner_typed(self, event, &owner);
                }
                RuntimeEnableReplayEvent::Background(event) => {
                    event.ensure_protocol_session_id(session_id);
                }
            }
        }
        Ok(replay)
    }

    fn runtime_session_owner_page_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<&mut Page, String> {
        self.loaded_page_mut_for_protocol_access(session_id)
    }

    fn runtime_session_owner_page_mut_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<&mut Page, String> {
        self.loaded_page_mut_for_protocol_access_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        )
    }

    fn runtime_session_owner_page_mut_for_interruptible_control_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<&mut Page, String> {
        self.loaded_page_mut_for_interruptible_protocol_access_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        )
    }

    fn runtime_protocol_message_page_route_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Result<RuntimeProtocolMessagePageRoute, String> {
        let (browser_context_id, target_id) = self
            .target_owner_identity_for_session(session_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        let slot = self.runtime_session_owner_slot(session_id)?;
        let renderer_agent_attachment_id = slot
            .current_renderer_attachment()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .id();
        Ok(RuntimeProtocolMessagePageRoute {
            browser_context_id,
            target_id,
            renderer_agent_attachment_id,
        })
    }

    fn runtime_protocol_message_page_route_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Result<RuntimeProtocolMessagePageRoute, String> {
        let (browser_context_id, target_id) = self
            .target_owner_identity_for_route(owner.session_id(), owner.session_owner_route())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        let slot = self.runtime_session_owner_slot_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        )?;
        let renderer_agent_attachment_id = slot
            .current_renderer_attachment()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .id();
        Ok(RuntimeProtocolMessagePageRoute {
            browser_context_id,
            target_id,
            renderer_agent_attachment_id,
        })
    }

    fn runtime_protocol_message_started_slot_mut(
        &mut self,
        route: &RuntimeProtocolMessagePageRoute,
    ) -> Result<&mut TargetRuntimeSlot, String> {
        let browser_context = self
            .browser_context_by_id_mut(&route.browser_context_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        let slot = if browser_context.active_target_id() == route.target_id.as_deref() {
            &mut browser_context.active_page_target_mut().runtime_slot
        } else {
            let target_id = route
                .target_id
                .as_deref()
                .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
            browser_context
                .background_target_mut(target_id)
                .map(|target| &mut target.runtime_slot)
                .ok_or_else(|| "NoDocumentLoaded".to_owned())?
        };
        if slot
            .current_renderer_attachment()
            .map(|attachment| attachment.id())
            != Some(route.renderer_agent_attachment_id)
        {
            return Err("Renderer attachment changed".to_owned());
        }
        Ok(slot)
    }

    fn runtime_protocol_message_started_page_mut(
        &mut self,
        route: &RuntimeProtocolMessagePageRoute,
    ) -> Result<&mut Page, String> {
        self.runtime_protocol_message_started_slot_mut(route)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())
    }

    fn consume_runtime_protocol_message_completion(
        &mut self,
        route: &RuntimeProtocolMessagePageRoute,
        completion: moli_core::page::CompletedPageCommand,
    ) -> Result<RendererCommandTurnOutput, String> {
        let output = if let Ok(page) = self.runtime_protocol_message_started_page_mut(route) {
            page.finish_runtime_protocol_message_command_turn(completion)
        } else {
            // Completion means the renderer owner has already committed the
            // command's Page state and concrete protocol publication. The
            // target can install a successor attachment before this protocol
            // task resumes (for example, form.submit() followed by a normal
            // command response). Preserve that immutable result; there is
            // simply no current Page cache belonging to this route to update.
            completion.into_runtime_protocol_message_command_turn()
        };
        output.map_err(|error| format!("runtime inspector dispatch failed: {error}"))
    }

    fn ingest_runtime_protocol_message_started_route_output_updates(
        &mut self,
        route: &RuntimeProtocolMessagePageRoute,
    ) {
        if let Ok(slot) = self.runtime_protocol_message_started_slot_mut(route) {
            let _ = slot.ingest_owner_page_observable_output_updates();
        }
    }

    fn shared_worker_runtime_target_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Result<SharedWorkerRuntimeTargetRoute, String> {
        let session_id = session_id.ok_or_else(|| "UnknownSession".to_owned())?;
        let route = self
            .session_route(Some(session_id))
            .ok_or_else(|| "UnknownSession".to_owned())?;
        match route {
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let target = self
                    .browser_context_by_id(&browser_context_id)
                    .and_then(|context| context.shared_worker_target(&target_id))
                    .ok_or_else(|| "UnknownSession".to_owned())?;
                Ok(SharedWorkerRuntimeTargetRoute {
                    browser_context_id,
                    worker: WorkerRuntimeTarget::Shared(target.renderer_instance_id),
                })
            }
            CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let target = self
                    .browser_context_by_id(&browser_context_id)
                    .and_then(|context| context.dedicated_worker_target(&target_id))
                    .ok_or_else(|| "UnknownSession".to_owned())?;
                Ok(SharedWorkerRuntimeTargetRoute {
                    browser_context_id,
                    worker: WorkerRuntimeTarget::Dedicated(target.renderer_instance_id),
                })
            }
            _ => Err("UnknownSession".to_owned()),
        }
    }

    pub(crate) fn run_dedicated_worker_if_waiting_for_debugger_for_session(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<bool, String> {
        let session_id = session_id.ok_or_else(|| "UnknownSession".to_owned())?;
        let route = self.shared_worker_runtime_target_for_session(Some(session_id))?;
        let WorkerRuntimeTarget::Dedicated(instance_id) = route.worker else {
            return Ok(false);
        };
        if let Some(target) = self.dedicated_worker_target_for_session_mut(Some(session_id)) {
            target.discard_main_script_network_replay_for(session_id);
        }
        let renderer_runtime = self
            .browser_context_by_id(&route.browser_context_id)
            .map(|context| context.renderer_runtime())
            .ok_or_else(|| "UnknownSession".to_owned())?;
        Ok(renderer_runtime.run_dedicated_worker_if_waiting_for_debugger_for_devtools(instance_id))
    }

    fn service_worker_runtime_target_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Result<ServiceWorkerRuntimeTargetRoute, String> {
        let session_id = session_id.ok_or_else(|| "UnknownSession".to_owned())?;
        let CdpSessionRoute::ServiceWorkerTarget {
            browser_context_id,
            target_id,
        } = self
            .session_route(Some(session_id))
            .ok_or_else(|| "UnknownSession".to_owned())?
        else {
            return Err("UnknownSession".to_owned());
        };
        let target = self
            .browser_context_by_id(&browser_context_id)
            .and_then(|context| context.service_worker_target(&target_id))
            .ok_or_else(|| "UnknownSession".to_owned())?;
        Ok(ServiceWorkerRuntimeTargetRoute {
            browser_context_id,
            version_id: target.renderer_version_id,
        })
    }

    pub(crate) fn start_shared_worker_runtime_protocol_message_for_session(
        &mut self,
        session_id: Option<&str>,
        raw_json: String,
    ) -> Result<PendingSharedWorkerRuntimeProtocolMessageDispatch, String> {
        let raw_json =
            self.rewrite_runtime_inspector_command_for_session_owner(session_id, &raw_json, None)?;
        self.start_shared_worker_runtime_protocol_message_for_session_with_optional_deferred_response(
            session_id,
            raw_json,
            None,
            RuntimeProtocolResponseRoute::command_reply_without_receiver(),
        )
    }

    pub(crate) fn start_shared_worker_runtime_protocol_message_for_session_with_deferred_response(
        &mut self,
        session_id: Option<&str>,
        descriptor: RendererCommandDescriptor,
        command_id: u64,
    ) -> Result<PendingSharedWorkerRuntimeProtocolMessageDispatch, String> {
        self.shared_worker_runtime_target_for_session(session_id)?;
        let (_correlation, raw_json, response_sender, response_route) =
            self.prepare_renderer_call_for_session_owner(session_id, descriptor, command_id, None)?;
        self.start_shared_worker_runtime_protocol_message_for_session_with_optional_deferred_response(
            session_id,
            raw_json,
            Some(response_sender),
            response_route,
        )
    }

    fn start_shared_worker_runtime_protocol_message_for_session_with_optional_deferred_response(
        &mut self,
        session_id: Option<&str>,
        raw_json: String,
        response_sender: Option<RendererRuntimeInspectorResponseSender>,
        response_route: RuntimeProtocolResponseRoute,
    ) -> Result<PendingSharedWorkerRuntimeProtocolMessageDispatch, String> {
        let route = self.shared_worker_runtime_target_for_session(session_id)?;
        let renderer_runtime = self
            .browser_context_by_id(&route.browser_context_id)
            .map(|context| context.renderer_runtime())
            .ok_or_else(|| "UnknownSession".to_owned())?;
        let worker = route.worker;
        let inspector_session_id = session_id.map(str::to_owned);
        let response_delivery = response_route.delivery();
        let pending: SharedWorkerRuntimeProtocolDispatchFuture = match (
            worker,
            response_sender,
            response_delivery,
        ) {
            (
                WorkerRuntimeTarget::Shared(instance_id),
                Some(response),
                RendererInspectorResponseDelivery::CommandReply,
            ) => Box::pin(async move {
                renderer_runtime
                    .dispatch_shared_worker_runtime_protocol_message_with_deferred_response(
                        instance_id,
                        inspector_session_id,
                        raw_json,
                        response,
                    )
                    .await
                    .map(CompletedWorkerRuntimeProtocolDispatch::command_reply)
            }),
            (
                WorkerRuntimeTarget::Shared(instance_id),
                Some(response),
                RendererInspectorResponseDelivery::DevToolsSession,
            ) => {
                let inspector_session_id =
                    inspector_session_id.ok_or_else(|| "UnknownSession".to_owned())?;
                Box::pin(async move {
                    renderer_runtime
                        .dispatch_shared_worker_runtime_protocol_message_with_devtools_session_response(
                            instance_id,
                            inspector_session_id,
                            raw_json,
                            response,
                        )
                        .await
                        .map(CompletedWorkerRuntimeProtocolDispatch::devtools_session)
                })
            }
            (
                WorkerRuntimeTarget::Shared(instance_id),
                None,
                RendererInspectorResponseDelivery::CommandReply,
            ) => Box::pin(async move {
                renderer_runtime
                    .dispatch_shared_worker_runtime_protocol_message(
                        instance_id,
                        inspector_session_id,
                        raw_json,
                    )
                    .await
                    .map(CompletedWorkerRuntimeProtocolDispatch::command_reply)
            }),
            (
                WorkerRuntimeTarget::Dedicated(instance_id),
                Some(response),
                RendererInspectorResponseDelivery::CommandReply,
            ) => Box::pin(async move {
                renderer_runtime
                    .dispatch_dedicated_worker_runtime_protocol_message_with_deferred_response(
                        instance_id,
                        inspector_session_id,
                        raw_json,
                        response,
                    )
                    .await
                    .map(CompletedWorkerRuntimeProtocolDispatch::command_reply)
            }),
            (
                WorkerRuntimeTarget::Dedicated(instance_id),
                Some(response),
                RendererInspectorResponseDelivery::DevToolsSession,
            ) => {
                let inspector_session_id =
                    inspector_session_id.ok_or_else(|| "UnknownSession".to_owned())?;
                Box::pin(async move {
                    renderer_runtime
                        .dispatch_dedicated_worker_runtime_protocol_message_with_devtools_session_response(
                            instance_id,
                            inspector_session_id,
                            raw_json,
                            response,
                        )
                        .await
                        .map(CompletedWorkerRuntimeProtocolDispatch::devtools_session)
                })
            }
            (
                WorkerRuntimeTarget::Dedicated(instance_id),
                None,
                RendererInspectorResponseDelivery::CommandReply,
            ) => Box::pin(async move {
                renderer_runtime
                    .dispatch_dedicated_worker_runtime_protocol_message(
                        instance_id,
                        inspector_session_id,
                        raw_json,
                    )
                    .await
                    .map(CompletedWorkerRuntimeProtocolDispatch::command_reply)
            }),
            (_, None, RendererInspectorResponseDelivery::DevToolsSession) => {
                return Err("DevToolsSessionResponseSenderMissing".to_owned());
            }
        };
        Ok(PendingSharedWorkerRuntimeProtocolMessageDispatch {
            session_id: session_id.map(str::to_owned),
            pending,
            response_route,
        })
    }

    pub(crate) async fn dispatch_shared_worker_runtime_helper_protocol_message_for_session_async(
        &mut self,
        session_id: Option<&str>,
        raw_json: &str,
        command_id: u64,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        let descriptor = RendererCommandDescriptor::from_synthesized_payload(raw_json.to_owned())?;
        let pending = self
            .start_shared_worker_runtime_protocol_message_for_session_with_deferred_response(
                session_id, descriptor, command_id,
            )?;
        let mut completed = pending.wait().await?;
        let response_rx = completed.take_deferred_response_receiver();
        let mut messages =
            self.complete_shared_worker_runtime_protocol_message_for_session(completed)?;
        if let Some(response_rx) = response_rx
            && let Some(message) = self
                .await_registered_runtime_inspector_response_for_session_owner_async(
                    session_id,
                    command_id,
                    response_rx,
                )
                .await
        {
            messages.push(message);
        }
        Ok(messages)
    }

    pub(crate) fn complete_shared_worker_runtime_protocol_message_for_session(
        &mut self,
        mut completed: CompletedSharedWorkerRuntimeProtocolMessageDispatch,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        self.restore_frontend_command_ids_in_runtime_messages(
            completed.session_id.as_deref(),
            None,
            &mut completed.dispatch.messages,
        );
        Ok(completed.dispatch.messages)
    }

    pub(crate) fn start_service_worker_runtime_protocol_message_for_session(
        &mut self,
        session_id: Option<&str>,
        raw_json: String,
    ) -> Result<PendingServiceWorkerRuntimeProtocolMessageDispatch, String> {
        let raw_json =
            self.rewrite_runtime_inspector_command_for_session_owner(session_id, &raw_json, None)?;
        self.start_service_worker_runtime_protocol_message_for_session_with_optional_deferred_response(
            session_id,
            raw_json,
            None,
            RuntimeProtocolResponseRoute::command_reply_without_receiver(),
        )
    }

    pub(crate) fn start_service_worker_runtime_protocol_message_for_session_with_deferred_response(
        &mut self,
        session_id: Option<&str>,
        descriptor: RendererCommandDescriptor,
        command_id: u64,
    ) -> Result<PendingServiceWorkerRuntimeProtocolMessageDispatch, String> {
        self.service_worker_runtime_target_for_session(session_id)?;
        let (_correlation, raw_json, response_sender, response_route) =
            self.prepare_renderer_call_for_session_owner(session_id, descriptor, command_id, None)?;
        self.start_service_worker_runtime_protocol_message_for_session_with_optional_deferred_response(
            session_id,
            raw_json,
            Some(response_sender),
            response_route,
        )
    }

    fn start_service_worker_runtime_protocol_message_for_session_with_optional_deferred_response(
        &mut self,
        session_id: Option<&str>,
        raw_json: String,
        response_sender: Option<RendererRuntimeInspectorResponseSender>,
        response_route: RuntimeProtocolResponseRoute,
    ) -> Result<PendingServiceWorkerRuntimeProtocolMessageDispatch, String> {
        let route = self.service_worker_runtime_target_for_session(session_id)?;
        let renderer_runtime = self
            .browser_context_by_id(&route.browser_context_id)
            .map(|context| context.renderer_runtime())
            .ok_or_else(|| "UnknownSession".to_owned())?;
        let version_id = route.version_id;
        let inspector_session_id = session_id.map(str::to_owned);
        let response_delivery = response_route.delivery();
        let pending: ServiceWorkerRuntimeProtocolDispatchFuture = Box::pin(async move {
            match (response_sender, response_delivery) {
                (Some(response), RendererInspectorResponseDelivery::CommandReply) => {
                    renderer_runtime
                        .dispatch_service_worker_runtime_protocol_message_with_deferred_response(
                            version_id,
                            inspector_session_id,
                            raw_json,
                            response,
                        )
                        .await
                        .map(CompletedWorkerRuntimeProtocolDispatch::command_reply)
                }
                (Some(response), RendererInspectorResponseDelivery::DevToolsSession) => {
                    let inspector_session_id =
                        inspector_session_id.ok_or_else(|| "UnknownSession".to_owned())?;
                    renderer_runtime
                            .dispatch_service_worker_runtime_protocol_message_with_devtools_session_response(
                                version_id,
                                inspector_session_id,
                                raw_json,
                                response,
                            )
                            .await
                            .map(CompletedWorkerRuntimeProtocolDispatch::devtools_session)
                }
                (None, RendererInspectorResponseDelivery::CommandReply) => renderer_runtime
                    .dispatch_service_worker_runtime_protocol_message(
                        version_id,
                        inspector_session_id,
                        raw_json,
                    )
                    .await
                    .map(CompletedWorkerRuntimeProtocolDispatch::command_reply),
                (None, RendererInspectorResponseDelivery::DevToolsSession) => {
                    Err("DevToolsSessionResponseSenderMissing".to_owned())
                }
            }
        });
        Ok(PendingServiceWorkerRuntimeProtocolMessageDispatch {
            session_id: session_id.map(str::to_owned),
            pending,
            response_route,
        })
    }

    pub(crate) fn complete_service_worker_runtime_protocol_message_for_session(
        &mut self,
        mut completed: CompletedServiceWorkerRuntimeProtocolMessageDispatch,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        self.restore_frontend_command_ids_in_runtime_messages(
            completed.session_id.as_deref(),
            None,
            &mut completed.dispatch.messages,
        );
        Ok(completed.dispatch.messages)
    }

    pub(crate) fn start_moli_diagnostics(
        &mut self,
    ) -> Result<PendingMoliDiagnosticsDispatch, String> {
        let mut pending = Vec::new();

        if let Some(browser_context) = self.browser_context.as_mut() {
            collect_moli_diagnostics_pending_snapshots(browser_context, &mut pending)?;
        }
        for browser_context in &mut self.inactive_browser_contexts {
            collect_moli_diagnostics_pending_snapshots(browser_context, &mut pending)?;
        }

        Ok(PendingMoliDiagnosticsDispatch { pending })
    }

    pub(crate) fn complete_moli_diagnostics(
        &mut self,
        completed: CompletedMoliDiagnosticsDispatch,
    ) -> Value {
        let mut dedicated_worker_loading_count = 0;
        let mut dedicated_worker_running_worker_isolate_count = 0;
        let mut document_context_count = 0;
        let mut isolated_world_context_count = 0;
        let mut child_default_context_count = 0;
        let mut failed_page_snapshot_count = 0;

        for completed in completed.completed {
            let Some(page) = self
                .browser_context_by_id_mut(&completed.browser_context_id)
                .and_then(|browser_context| {
                    if let Some(target_id) = completed.target_id.as_deref() {
                        browser_context
                            .background_target_mut(target_id)
                            .and_then(|target| target.loaded_page_mut())
                    } else {
                        browser_context
                            .active_page_target_mut()
                            .runtime_slot
                            .loaded_page_mut()
                    }
                })
            else {
                failed_page_snapshot_count += 1;
                continue;
            };
            let Ok(completion) = completed.completion else {
                failed_page_snapshot_count += 1;
                continue;
            };
            let Ok(snapshot) = page.finish_page_diagnostics_snapshot(completion) else {
                failed_page_snapshot_count += 1;
                continue;
            };
            document_context_count += snapshot.diagnostics.document_context_count;
            isolated_world_context_count += snapshot.diagnostics.isolated_world_context_count;
            child_default_context_count += snapshot.diagnostics.child_default_context_count;
            dedicated_worker_loading_count += snapshot.diagnostics.dedicated_worker_loading_count;
            dedicated_worker_running_worker_isolate_count += snapshot
                .diagnostics
                .dedicated_worker_running_worker_isolate_count;
        }

        let estimated_document_isolate_count = self
            .browser_contexts()
            .map(|browser_context| {
                browser_context.loaded_document_page_count()
                    + browser_context.pending_document_page_build_count()
            })
            .sum::<usize>();
        let shared_worker_running_worker_isolate_count = self
            .browser_contexts()
            .map(|browser_context| {
                browser_context
                    .shared_worker_runtime_diagnostics_for_diagnostics()
                    .running_worker_isolate_count
            })
            .sum::<usize>();
        let estimated_worker_isolate_count = dedicated_worker_running_worker_isolate_count
            + shared_worker_running_worker_isolate_count;
        let estimated_live_v8_isolate_count =
            estimated_document_isolate_count + estimated_worker_isolate_count;

        let mut diagnostics = self.moli_memory_diagnostics();
        diagnostics["isolateScope"]["documentContextCount"] = json!(document_context_count);
        diagnostics["isolateScope"]["isolatedWorldContextCount"] =
            json!(isolated_world_context_count);
        diagnostics["isolateScope"]["childDefaultContextCount"] =
            json!(child_default_context_count);
        diagnostics["isolateScope"]["dedicatedWorkerLoadingCount"] =
            json!(dedicated_worker_loading_count);
        diagnostics["isolateScope"]["dedicatedWorkerRunningWorkerIsolateCount"] =
            json!(dedicated_worker_running_worker_isolate_count);
        diagnostics["isolateScope"]["dedicatedWorkerDiagnosticsFailedPageSnapshotCount"] =
            json!(failed_page_snapshot_count);
        diagnostics["isolateScope"]["estimatedWorkerIsolateCount"] =
            json!(estimated_worker_isolate_count);
        diagnostics["isolateScope"]["estimatedLiveV8IsolateCount"] =
            json!(estimated_live_v8_isolate_count);
        diagnostics
    }

    pub(crate) fn ingest_runtime_session_owner_output_updates(&mut self, session_id: Option<&str>) {
        self.ingest_runtime_session_owner_output_updates_for_route(session_id, None)
    }

    pub(crate) fn ingest_runtime_session_owner_output_updates_for_route(
        &mut self,
        session_id: Option<&str>,
        owner_route: Option<&CdpSessionRoute>,
    ) {
        if let Ok(slot) = self.runtime_session_owner_slot_mut_for_route(session_id, owner_route) {
            let _ = slot.ingest_owner_page_observable_output_updates();
        }
    }

    pub(crate) fn runtime_session_owner_frame_id(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        self.runtime_session_owner_frame_id_for_route(session_id, None)
    }

    pub(crate) fn runtime_session_owner_frame_id_for_route(
        &self,
        session_id: Option<&str>,
        owner_route: Option<&CdpSessionRoute>,
    ) -> Option<String> {
        if let Some((_, target_id)) = self.target_owner_identity_for_route(session_id, owner_route)
            && target_id.is_some()
        {
            return target_id;
        }
        match session_id {
            None => self
                .browser_context
                .as_ref()
                .and_then(|bc| bc.active_target_id_owned()),
            Some(session_id) => match self.session_route(Some(session_id))? {
                CdpSessionRoute::Browser => self
                    .browser_context
                    .as_ref()
                    .and_then(|bc| bc.active_target_id_owned()),
                CdpSessionRoute::BrowserContext { browser_context_id } => self
                    .browser_context_by_id(&browser_context_id)
                    .and_then(|bc| bc.active_target_id_owned()),
                CdpSessionRoute::PageTarget { target_id, .. } => Some(target_id),
                CdpSessionRoute::TabTarget { .. }
                | CdpSessionRoute::SharedWorkerTarget { .. }
                | CdpSessionRoute::DedicatedWorkerTarget { .. }
                | CdpSessionRoute::ServiceWorkerTarget { .. } => None,
            },
        }
    }

    #[cfg(test)]
    async fn dispatch_runtime_protocol_message_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        raw_json: &str,
    ) -> Result<Vec<Value>, String> {
        // Direct raw protocol compatibility still accepts id-bearing messages.
        // Internal helpers should call the explicit helper variant so the
        // callback owner is visible at the call site.
        if let Some(command_id) = runtime_protocol_message_id(raw_json) {
            return Ok(self
                .dispatch_runtime_helper_protocol_message_for_session_owner_async(
                    session_id, raw_json, command_id,
                )
                .await?
                .into_iter()
                .map(RendererRuntimeInspectorMessage::into_v8_inspector_message)
                .collect());
        }
        let timing_started = moli_trace::cdp_nav_timing_enabled().then(std::time::Instant::now);
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_session(session_id);
        let messages = {
            let page = self.runtime_session_owner_page_mut(session_id)?;
            page.dispatch_runtime_protocol_message_for_inspector_session_async(
                inspector_session_id,
                raw_json,
            )
            .await
            .map_err(|error| format!("runtime inspector dispatch failed: {error}"))?
            .into_iter()
            .map(RendererRuntimeInspectorMessage::into_v8_inspector_message)
            .collect::<Vec<_>>()
        };
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "runtime_inspector_page_dispatch_done",
                messages = messages.len(),
                elapsed_ms = started.elapsed().as_millis(),
            );
        }
        self.ingest_runtime_session_owner_output_updates(session_id);
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "runtime_inspector_output_ingested",
                elapsed_ms = started.elapsed().as_millis(),
            );
        }
        Ok(messages)
    }

    #[cfg(test)]
    pub(crate) async fn dispatch_runtime_helper_protocol_message_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        raw_json: &str,
        command_id: u64,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        let descriptor = RendererCommandDescriptor::from_synthesized_payload(raw_json.to_owned())?;
        let owner = CommandOwnerScope::capture(self, session_id);
        let pending = self.start_runtime_protocol_message_for_owner_with_deferred_response(
            &owner, descriptor, command_id,
        )?;
        let completed = pending.wait().await?;
        self.complete_runtime_helper_protocol_message_for_session_owner_async(completed, command_id)
            .await
    }

    pub(crate) async fn await_registered_runtime_inspector_response_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        command_id: u64,
        response_rx: RuntimeInspectorResponseReceiver,
    ) -> Option<RendererRuntimeInspectorMessage> {
        let response = crate::conn::RuntimeInspectorResponseReady::new(
            command_id,
            session_id,
            response_rx
                .await
                .map_err(|_| "RuntimeInspectorResponseCanceled".to_owned()),
        );
        let mut response = self.resolve_runtime_inspector_response_ready(response)?;
        if response
            .renderer_agent_attachment_id()
            .is_some_and(|attachment_id| {
                !self.renderer_agent_attachment_is_current_for_session_owner(
                    session_id,
                    attachment_id,
                )
            })
        {
            response.replace_with_error("Execution context was destroyed by navigation");
        }
        let message = response.into_protocol_message_for_typed_runtime_route();
        Some(RendererRuntimeInspectorMessage::protocol(message))
    }

    pub(crate) fn start_runtime_protocol_message_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        raw_json: String,
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
        self.start_runtime_protocol_message_for_owner_with_access(
            owner,
            raw_json,
            RendererInspectorCommandRoute::MainThread,
        )
    }

    pub(crate) fn start_runtime_io_protocol_message_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        raw_json: String,
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
        self.start_runtime_protocol_message_for_owner_with_access(
            owner,
            raw_json,
            RendererInspectorCommandRoute::Io,
        )
    }

    fn start_runtime_protocol_message_for_owner_with_access(
        &mut self,
        owner: &CommandOwnerScope,
        raw_json: String,
        inspector_route: RendererInspectorCommandRoute,
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
        let route = self.runtime_protocol_message_page_route_for_owner(owner)?;
        let raw_json = self.rewrite_runtime_inspector_command_for_owner(owner, &raw_json, None)?;
        let inspector_session_id = self.target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let page = match inspector_route {
            RendererInspectorCommandRoute::MainThread => {
                self.runtime_session_owner_page_mut_for_owner(owner)?
            }
            RendererInspectorCommandRoute::Io => {
                self.runtime_session_owner_page_mut_for_interruptible_control_for_owner(owner)?
            }
        };
        let pending = match inspector_route {
            RendererInspectorCommandRoute::MainThread => page
                .start_runtime_protocol_message_for_inspector_session(
                    inspector_session_id,
                    raw_json,
                )
                .map(PendingRuntimeProtocolMessageDispatchKind::Page),
            RendererInspectorCommandRoute::Io => page
                .start_runtime_inspector_io_message_without_response(inspector_session_id, raw_json)
                .map(PendingRuntimeProtocolMessageDispatchKind::Routable),
        }
        .map_err(|error| format!("runtime inspector dispatch failed: {error}"))?;
        Ok(PendingRuntimeProtocolMessageDispatch {
            owner: owner.clone(),
            route,
            pending,
            response_route: RuntimeProtocolResponseRoute::command_reply_without_receiver(),
        })
    }

    pub(crate) fn start_runtime_protocol_message_for_owner_with_deferred_response(
        &mut self,
        owner: &CommandOwnerScope,
        descriptor: RendererCommandDescriptor,
        command_id: u64,
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
        self.start_runtime_protocol_message_for_owner_with_deferred_response_and_access(
            owner,
            descriptor,
            command_id,
            RendererInspectorCommandRoute::MainThread,
        )
    }

    pub(crate) fn start_runtime_io_protocol_message_for_owner_with_deferred_response(
        &mut self,
        owner: &CommandOwnerScope,
        descriptor: RendererCommandDescriptor,
        command_id: u64,
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
        self.start_runtime_protocol_message_for_owner_with_deferred_response_and_access(
            owner,
            descriptor,
            command_id,
            RendererInspectorCommandRoute::Io,
        )
    }

    fn start_runtime_protocol_message_for_owner_with_deferred_response_and_access(
        &mut self,
        owner: &CommandOwnerScope,
        descriptor: RendererCommandDescriptor,
        command_id: u64,
        inspector_route: RendererInspectorCommandRoute,
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
        let route = self.runtime_protocol_message_page_route_for_owner(owner)?;
        let (correlation, raw_json, response_sender, response_route) = self
            .prepare_renderer_call_for_owner(
                owner,
                descriptor,
                command_id,
                Some(route.renderer_agent_attachment_id),
            )?;
        let inspector_session_id = self.target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let page_result = match inspector_route {
            RendererInspectorCommandRoute::MainThread => {
                self.runtime_session_owner_page_mut_for_owner(owner)
            }
            RendererInspectorCommandRoute::Io => {
                self.runtime_session_owner_page_mut_for_interruptible_control_for_owner(owner)
            }
        };
        let page = match page_result {
            Ok(page) => page,
            Err(error) => {
                let removed = self.take_renderer_call_for_frontend_for_owner(owner, command_id);
                debug_assert_eq!(removed, Some(correlation));
                return Err(error);
            }
        };
        let pending = match page.start_routable_runtime_protocol_message_for_inspector_session(
            inspector_session_id,
            inspector_route,
            None,
            raw_json,
            response_sender,
        ) {
            Ok(pending) => pending,
            Err(error) => {
                let removed = self.take_renderer_call_for_frontend_for_owner(owner, command_id);
                debug_assert_eq!(removed, Some(correlation));
                return Err(format!("runtime inspector dispatch failed: {error}"));
            }
        };
        Ok(PendingRuntimeProtocolMessageDispatch {
            owner: owner.clone(),
            route,
            pending: PendingRuntimeProtocolMessageDispatchKind::Routable(pending),
            response_route,
        })
    }

    pub(crate) fn start_runtime_protocol_message_with_context_resolution_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        action: &str,
        raw_json: String,
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
        let route = self.runtime_protocol_message_page_route_for_owner(owner)?;
        let raw_json = self.rewrite_runtime_inspector_command_for_owner(owner, &raw_json, None)?;
        let inspector_session_id = self.target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let pending = page
            .start_runtime_protocol_message_for_inspector_session_with_context_resolution(
                inspector_session_id,
                action.to_owned(),
                raw_json,
            )
            .map_err(|error| format!("runtime inspector dispatch failed: {error}"))?;
        Ok(PendingRuntimeProtocolMessageDispatch {
            owner: owner.clone(),
            route,
            pending: PendingRuntimeProtocolMessageDispatchKind::Page(pending),
            response_route: RuntimeProtocolResponseRoute::command_reply_without_receiver(),
        })
    }

    pub(crate) fn start_runtime_protocol_message_with_context_resolution_for_owner_with_deferred_response(
        &mut self,
        owner: &CommandOwnerScope,
        action: &str,
        descriptor: RendererCommandDescriptor,
        command_id: u64,
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
        let route = self.runtime_protocol_message_page_route_for_owner(owner)?;
        let (correlation, raw_json, response_sender, response_route) = self
            .prepare_renderer_call_for_owner(
                owner,
                descriptor,
                command_id,
                Some(route.renderer_agent_attachment_id),
            )?;
        let inspector_session_id = self.target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let page = match self.runtime_session_owner_page_mut_for_owner(owner) {
            Ok(page) => page,
            Err(error) => {
                let removed = self.take_renderer_call_for_frontend_for_owner(owner, command_id);
                debug_assert_eq!(removed, Some(correlation));
                return Err(error);
            }
        };
        let pending = match page.start_routable_runtime_protocol_message_for_inspector_session(
            inspector_session_id,
            RendererInspectorCommandRoute::MainThread,
            Some(action.to_owned()),
            raw_json,
            response_sender,
        ) {
            Ok(pending) => pending,
            Err(error) => {
                let removed = self.take_renderer_call_for_frontend_for_owner(owner, command_id);
                debug_assert_eq!(removed, Some(correlation));
                return Err(format!("runtime inspector dispatch failed: {error}"));
            }
        };
        Ok(PendingRuntimeProtocolMessageDispatch {
            owner: owner.clone(),
            route,
            pending: PendingRuntimeProtocolMessageDispatchKind::Routable(pending),
            response_route,
        })
    }

    pub(crate) async fn complete_runtime_protocol_message_async(
        &mut self,
        completed: CompletedRuntimeProtocolMessageDispatch,
    ) -> Result<Option<RendererCommandTurnOutput>, String> {
        let timing_started = moli_trace::cdp_nav_timing_enabled().then(std::time::Instant::now);
        let owner = completed.owner().clone();
        let completion = match completed.completion {
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::Owner(completion) => {
                *completion
            }
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::OwnerSessionResponse {
                completion,
                ..
            } => *completion,
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::Inspector
            | moli_core::page::CompletedRuntimeInspectorCommandDispatch::InspectorSessionResponse {
                ..
            }
            | moli_core::page::CompletedRuntimeInspectorCommandDispatch::OwnerSessionErrorSettled(
                _,
            ) => {
                return Ok(None);
            }
        };
        let mut output =
            self.consume_runtime_protocol_message_completion(&completed.route, completion)?;
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "runtime_inspector_page_dispatch_done",
                output_messages = output
                    .runtime_inspector_output()
                    .map_or(0, |messages| messages.len()),
                elapsed_ms = started.elapsed().as_millis(),
            );
        }
        self.ingest_runtime_protocol_message_started_route_output_updates(&completed.route);
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "runtime_inspector_output_ingested",
                elapsed_ms = started.elapsed().as_millis(),
            );
        }
        let runtime_messages = output.runtime_inspector_output_mut().ok_or_else(|| {
            "runtime inspector dispatch completed with a non-Runtime renderer reply".to_owned()
        })?;
        runtime_messages
            .bind_renderer_agent_attachment(completed.route.renderer_agent_attachment_id);
        let runtime_messages = runtime_messages.messages_mut();
        self.restore_frontend_command_ids_in_runtime_messages_for_owner(
            &owner,
            Some(completed.route.renderer_agent_attachment_id),
            runtime_messages,
        );
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "runtime_inspector_command_output_ready",
                output_messages = output
                    .runtime_inspector_output()
                    .map_or(0, |messages| messages.len()),
                elapsed_ms = started.elapsed().as_millis(),
            );
        }
        Ok(Some(output))
    }

    pub(crate) async fn replay_prepared_renderer_calls_after_navigation_async(
        &mut self,
        replays: Vec<SessionRendererCallReplay>,
        new_attachment_id: RendererAgentAttachmentId,
    ) -> Result<Vec<BackgroundProtocolEvent>, String> {
        let mut events = Vec::new();
        for replay in replays {
            let frontend_session_id = replay.frontend_session_id().map(str::to_owned);
            let owner = CommandOwnerScope::capture(self, frontend_session_id.as_deref());
            let renderer_inspector_session_id =
                replay.renderer_inspector_session_id().map(str::to_owned);
            let (correlation, replay, response_delivery, frontend_payload, response_sender) =
                replay.into_replay().into_parts();
            let route = match self.runtime_protocol_message_page_route_for_session_owner(
                frontend_session_id.as_deref(),
            ) {
                Ok(route) => route,
                Err(error) => {
                    self.settle_renderer_replacement_error(
                        &mut events,
                        frontend_session_id.as_deref(),
                        response_delivery,
                        &response_sender,
                        correlation,
                        &error,
                    );
                    continue;
                }
            };
            if route.renderer_agent_attachment_id != new_attachment_id {
                self.settle_renderer_replacement_error(
                    &mut events,
                    frontend_session_id.as_deref(),
                    response_delivery,
                    &response_sender,
                    correlation,
                    "renderer replay attachment is no longer current",
                );
                continue;
            }
            let dispatch = match replay {
                RendererCommandReplay::Inspector(dispatch) => dispatch,
                RendererCommandReplay::PerformanceGetMetrics => {
                    debug_assert_eq!(
                        response_delivery,
                        RendererInspectorResponseDelivery::DevToolsSession
                    );
                    let pending = self
                        .runtime_session_owner_page_mut(frontend_session_id.as_deref())
                        .map_err(|error| error.to_string())
                        .and_then(|page| {
                            let result = crate::domains::performance::performance_metrics_result(
                                &page.cached_performance_metric_snapshot(),
                            );
                            page.start_performance_get_metrics_from_io_with_response(
                                renderer_inspector_session_id.clone(),
                                result,
                                response_sender.clone(),
                            )
                            .map_err(|error| error.to_string())
                        });
                    let completion = match pending {
                        Ok(pending) => pending.wait().await.map_err(|error| error.to_string()),
                        Err(error) => Err(error),
                    };
                    match completion {
                        Ok(
                            moli_core::page::CompletedDevToolsIoCommandDispatch::SessionResponse {
                                ..
                            },
                        )
                        | Ok(moli_core::page::CompletedDevToolsIoCommandDispatch::Dispatched) => {}
                        Err(error) => {
                            self.settle_renderer_replacement_error(
                                &mut events,
                                frontend_session_id.as_deref(),
                                response_delivery,
                                &response_sender,
                                correlation,
                                &format!("Performance replay dispatch failed: {error}"),
                            );
                        }
                    }
                    continue;
                }
                RendererCommandReplay::SetScriptExecutionDisabled { disabled } => {
                    debug_assert_eq!(
                        response_delivery,
                        RendererInspectorResponseDelivery::DevToolsSession
                    );
                    let pending = self
                        .runtime_session_owner_page_mut(frontend_session_id.as_deref())
                        .map_err(|error| error.to_string())
                        .and_then(|page| {
                            page.start_set_script_execution_disabled_from_io_with_response(
                                renderer_inspector_session_id.clone(),
                                disabled,
                                response_sender.clone(),
                            )
                            .map_err(|error| error.to_string())
                        });
                    let completion = match pending {
                        Ok(pending) => pending.wait().await.map_err(|error| error.to_string()),
                        Err(error) => Err(error),
                    };
                    match completion {
                        Ok(
                            moli_core::page::CompletedDevToolsIoCommandDispatch::SessionResponse {
                                ..
                            },
                        )
                        | Ok(moli_core::page::CompletedDevToolsIoCommandDispatch::Dispatched) => {}
                        Err(error) => {
                            self.settle_renderer_replacement_error(
                                &mut events,
                                frontend_session_id.as_deref(),
                                response_delivery,
                                &response_sender,
                                correlation,
                                &format!("Emulation replay dispatch failed: {error}"),
                            );
                        }
                    }
                    continue;
                }
            };
            let raw_json = match self.rewrite_runtime_inspector_command_for_session_owner(
                frontend_session_id.as_deref(),
                &frontend_payload,
                Some((
                    correlation.frontend_command_id(),
                    correlation.renderer_call_id(),
                )),
            ) {
                Ok(raw_json) => raw_json,
                Err(error) => {
                    self.settle_renderer_replacement_error(
                        &mut events,
                        frontend_session_id.as_deref(),
                        response_delivery,
                        &response_sender,
                        correlation,
                        &error,
                    );
                    continue;
                }
            };
            let pending = {
                let page = match self.runtime_session_owner_page_mut(frontend_session_id.as_deref())
                {
                    Ok(page) => page,
                    Err(error) => {
                        self.settle_renderer_replacement_error(
                            &mut events,
                            frontend_session_id.as_deref(),
                            response_delivery,
                            &response_sender,
                            correlation,
                            &error,
                        );
                        continue;
                    }
                };
                let dispatch_sender = response_sender.clone();
                match response_delivery {
                    RendererInspectorResponseDelivery::CommandReply => match dispatch {
                        CdpRendererCommandReplayDispatch::ResolveRuntimeContext => page
                            .start_runtime_protocol_message_for_inspector_session_with_context_resolution_and_deferred_response(
                                renderer_inspector_session_id,
                                "addBinding".to_owned(),
                                raw_json,
                                dispatch_sender,
                            )
                            .map(PendingRuntimeProtocolMessageDispatchKind::Page),
                        CdpRendererCommandReplayDispatch::Direct => page
                            .start_runtime_protocol_message_for_inspector_session_with_deferred_response(
                                renderer_inspector_session_id,
                                raw_json,
                                dispatch_sender,
                            )
                            .map(PendingRuntimeProtocolMessageDispatchKind::Page),
                    },
                    RendererInspectorResponseDelivery::DevToolsSession => {
                        debug_assert_eq!(
                            dispatch,
                            CdpRendererCommandReplayDispatch::Direct,
                            "the migrated synchronous IO family must replay directly"
                        );
                        page.start_routable_runtime_protocol_message_for_inspector_session(
                            renderer_inspector_session_id,
                            RendererInspectorCommandRoute::Io,
                            None,
                            raw_json,
                            dispatch_sender,
                        )
                        .map(PendingRuntimeProtocolMessageDispatchKind::Routable)
                    }
                }
            };
            let pending = match pending {
                Ok(pending) => PendingRuntimeProtocolMessageDispatch {
                    owner,
                    route,
                    pending,
                    response_route:
                        RuntimeProtocolResponseRoute::without_local_receiver_for_delivery(
                            response_delivery,
                        ),
                },
                Err(error) => {
                    self.settle_renderer_replacement_error(
                        &mut events,
                        frontend_session_id.as_deref(),
                        response_delivery,
                        &response_sender,
                        correlation,
                        &format!("runtime inspector replay dispatch failed: {error}"),
                    );
                    continue;
                }
            };
            let completed = match pending.wait().await {
                Ok(completed) => completed,
                Err(error) => {
                    self.settle_renderer_replacement_error(
                        &mut events,
                        frontend_session_id.as_deref(),
                        response_delivery,
                        &response_sender,
                        correlation,
                        &error,
                    );
                    continue;
                }
            };
            let completed_owner = completed.owner().clone();
            let completion = match completed.completion {
                moli_core::page::CompletedRuntimeInspectorCommandDispatch::Owner(completion) => {
                    *completion
                }
                moli_core::page::CompletedRuntimeInspectorCommandDispatch::Inspector => {
                    continue;
                }
                moli_core::page::CompletedRuntimeInspectorCommandDispatch::InspectorSessionResponse {
                    ..
                } => {
                    continue;
                }
                moli_core::page::CompletedRuntimeInspectorCommandDispatch::OwnerSessionErrorSettled(
                    _,
                ) => {
                    continue;
                }
                moli_core::page::CompletedRuntimeInspectorCommandDispatch::OwnerSessionResponse {
                    completion,
                    ..
                } => *completion,
            };
            let mut command_turn_output = match self
                .consume_runtime_protocol_message_completion(&completed.route, completion)
            {
                Ok(output) => output,
                Err(error) => {
                    send_renderer_replacement_error(
                        &response_sender,
                        correlation,
                        &format!("runtime inspector replay dispatch failed: {error}"),
                    );
                    continue;
                }
            };
            command_turn_output.bind_renderer_agent_attachment(new_attachment_id);
            self.ingest_runtime_protocol_message_started_route_output_updates(&completed.route);
            let mut command = CommandDispatchContext::default();
            let completion = command.consume_renderer_command_turn_output(command_turn_output);
            events.extend(command.take_protocol_events());
            events.extend(command.take_post_response_events());
            let Some(output) = completion.into_runtime_inspector_output() else {
                send_renderer_replacement_error(
                    &response_sender,
                    correlation,
                    "runtime inspector replay completed with a non-Runtime renderer reply",
                );
                continue;
            };
            if output
                .protocol_response(correlation.renderer_call_id().get())
                .is_some()
            {
                let _ = response_sender.send_output(output);
                continue;
            }
            let _ = self.route_renderer_runtime_command_output_for_owner_into(
                output,
                None,
                &completed_owner,
                &mut events,
            );
        }
        Ok(events)
    }

    fn settle_renderer_replacement_error(
        &mut self,
        events: &mut Vec<BackgroundProtocolEvent>,
        frontend_session_id: Option<&str>,
        response_delivery: RendererInspectorResponseDelivery,
        response_sender: &RendererRuntimeInspectorResponseSender,
        correlation: RendererCommandCorrelation,
        message: &str,
    ) {
        if response_delivery == RendererInspectorResponseDelivery::CommandReply {
            send_renderer_replacement_error(response_sender, correlation, message);
            return;
        }

        self.settle_devtools_session_renderer_error(
            events,
            frontend_session_id,
            correlation,
            message,
        );
    }

    fn settle_devtools_session_renderer_error(
        &mut self,
        events: &mut Vec<BackgroundProtocolEvent>,
        frontend_session_id: Option<&str>,
        correlation: RendererCommandCorrelation,
        message: &str,
    ) {
        let Some(resolved) = self
            .take_frontend_command_for_renderer_if_attachment_matches_for_session_owner(
                frontend_session_id,
                correlation.renderer_call_id(),
                correlation.dispatched_attachment_id(),
            )
        else {
            return;
        };
        debug_assert_eq!(resolved, correlation);
        let mut response = json!({
            "id": resolved.frontend_command_id().get(),
            "error": {
                "code": -32000,
                "message": message,
            },
        });
        if let Some(session_id) = frontend_session_id {
            response["sessionId"] = json!(session_id);
        }
        events.push(protocol_message_background_event(response));
    }

    pub(crate) fn terminate_prepared_renderer_calls_after_navigation(
        &mut self,
        terminations: Vec<SessionRendererCallTermination>,
        reason: &str,
    ) -> Vec<BackgroundProtocolEvent> {
        let mut events = Vec::new();
        for termination in terminations {
            let (frontend_session_id, termination) = termination.into_parts();
            match termination {
                PreparedRendererCallTermination::CommandReply {
                    correlation,
                    response_sender,
                } => send_renderer_replacement_error(&response_sender, correlation, reason),
                PreparedRendererCallTermination::DevToolsSession { correlation } => self
                    .settle_devtools_session_renderer_error(
                        &mut events,
                        frontend_session_id.as_deref(),
                        correlation,
                        reason,
                    ),
            }
        }
        events
    }

    #[cfg(test)]
    pub(crate) async fn complete_runtime_helper_protocol_message_for_session_owner_async(
        &mut self,
        mut completed: CompletedRuntimeProtocolMessageDispatch,
        command_id: u64,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        let response_rx = completed.take_deferred_response_receiver();
        let session_id = completed.owner().session_id().map(str::to_owned);
        let output = self
            .complete_runtime_protocol_message_async(completed)
            .await?;
        let response_in_output = output.as_ref().is_some_and(|output| {
            renderer_command_turn_frontend_protocol_response(output, command_id).is_some()
        });
        let mut messages = Vec::new();
        let mut runtime_messages = Vec::new();
        if let Some(output) = output {
            let (completion, _) = output.into_completion_and_predecessor();
            let Some(runtime_output) = completion.into_runtime_inspector_output() else {
                return Err(
                    "runtime inspector dispatch completed with a non-Runtime renderer reply"
                        .to_owned(),
                );
            };
            runtime_messages = runtime_output.into_messages();
        }
        if !response_in_output && let Some(response_rx) = response_rx {
            let response = RuntimeInspectorResponseReady::new(
                command_id,
                session_id.as_deref(),
                response_rx
                    .await
                    .map_err(|_| "RuntimeInspectorResponseCanceled".to_owned()),
            );
            if let Some(response) = self.resolve_runtime_inspector_response_ready(response) {
                let (_, output, renderer_output_predecessor) =
                    response.into_renderer_command_output();
                assert!(
                    renderer_output_predecessor.is_none(),
                    "message-only Runtime test helper cannot discard a concrete output cursor"
                );
                runtime_messages.extend(output.into_messages());
            }
        }
        messages.extend(runtime_messages);
        Ok(messages)
    }

    /// Completes one concrete BiDi channel owner action under its frozen Page
    /// route.
    ///
    /// Stale work is consumed without entering a replacement runtime. The old
    /// Page or detached session owns any renderer-side cleanup; applying an
    /// object-group release to the new attachment would be the more dangerous
    /// outcome because group names belong to the producing runtime.
    pub(crate) async fn complete_bidi_channel_owner_action_with_background_events_async(
        &mut self,
        action: BidiChannelOwnerAction,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) {
        let (owner, body) = action.into_parts();
        if !owner.is_current(self) {
            tracing::debug!(
                session_id = owner.session_id(),
                action = ?body,
                "discarding stale BiDi channel owner action"
            );
            return;
        }
        let command_owner = owner.command_owner().clone();
        match body {
            BidiChannelOwnerActionBody::StartListener(listener) => {
                self.start_bidi_channel_listener_once_for_owner_with_background_events_async(
                    &command_owner,
                    BidiChannelListenerResidence::from_boxed(owner, listener),
                    background_events,
                )
                .await;
            }
            BidiChannelOwnerActionBody::ReleaseObjectGroup(object_group) => {
                self.release_bidi_channel_object_group_for_owner_best_effort_async(
                    &command_owner,
                    &object_group,
                )
                .await;
            }
        }
    }

    pub(crate) async fn release_bidi_channel_object_group_for_owner_best_effort_async(
        &mut self,
        owner: &CommandOwnerScope,
        object_group: &str,
    ) {
        let command_id = self.next_internal_runtime_command_id();
        let raw_json = json!({
            "id": command_id,
            "method": "Runtime.releaseObjectGroup",
            "params": { "objectGroup": object_group }
        })
        .to_string();
        let descriptor = match RendererCommandDescriptor::from_synthesized_payload(raw_json) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                tracing::debug!(%error, object_group, "failed to prepare BiDi object group release");
                self.unregister_runtime_remote_object_group_for_owner(owner, object_group);
                return;
            }
        };
        let pending = match self
            .start_runtime_protocol_message_with_context_resolution_for_owner_with_deferred_response(
                owner,
                "releaseObjectGroup",
                descriptor,
                command_id,
            ) {
            Ok(pending) => pending,
            Err(error) => {
                tracing::debug!(
                    %error,
                    object_group,
                    "failed to start BiDi channel object group release"
                );
                self.unregister_runtime_remote_object_group_for_owner(owner, object_group);
                return;
            }
        };
        let mut completed = match pending.wait().await {
            Ok(completed) => completed,
            Err(error) => {
                self.forget_pending_inspector_await_for_owner(command_id, owner);
                tracing::debug!(
                    %error,
                    object_group,
                    "BiDi channel object group release dispatch failed"
                );
                self.unregister_runtime_remote_object_group_for_owner(owner, object_group);
                return;
            }
        };
        let mut renderer_response_rx = completed.take_deferred_response_receiver();
        let messages = match self
            .complete_runtime_protocol_message_async(completed)
            .await
        {
            Ok(messages) => messages,
            Err(error) => {
                self.forget_pending_inspector_await_for_owner(command_id, owner);
                tracing::debug!(
                    %error,
                    object_group,
                    "BiDi channel object group release completion failed"
                );
                self.unregister_runtime_remote_object_group_for_owner(owner, object_group);
                return;
            }
        };
        let mut release_events = Vec::new();
        let mut release_post_response_events = Vec::new();
        let response_flush = CommandResponseFlushContext::default();
        let release_response_seen = if let Some(messages) = messages {
            self.route_renderer_command_turn_output_for_owner_into(
                messages,
                Some(command_id),
                owner,
                &response_flush,
                &mut release_events,
                &mut release_post_response_events,
            )
            .0
        } else {
            false
        };
        if !release_response_seen {
            tracing::debug!(
                command_id,
                "internal object group release inspector response was not routed as current command"
            );
        }
        if release_response_seen {
            renderer_response_rx.take();
        }
        release_events.extend(release_post_response_events);
        if let Some(renderer_response_rx) = renderer_response_rx {
            let response = RuntimeInspectorResponseReady::for_owner(
                command_id,
                owner,
                renderer_response_rx
                    .await
                    .map_err(|_| "RuntimeInspectorResponseCanceled".to_owned()),
            );
            if let Some(response) = self.resolve_runtime_inspector_response_ready(response) {
                let (_, output, renderer_output_predecessor) =
                    response.into_renderer_command_output();
                assert!(
                    renderer_output_predecessor.is_none(),
                    "internal object-group cleanup cannot discard a concrete output cursor"
                );
                let release_response_seen = self
                    .route_renderer_runtime_command_output_for_owner_into(
                        output,
                        Some(command_id),
                        owner,
                        &mut release_events,
                    );
                if !release_response_seen {
                    tracing::debug!(
                        command_id,
                        "internal object group release deferred inspector response was not routed as current command"
                    );
                }
            }
        }
        self.unregister_runtime_remote_object_group_for_owner(owner, object_group);
    }

    async fn start_bidi_channel_listener_once_for_owner_with_background_events_async(
        &mut self,
        owner: &CommandOwnerScope,
        residence: BidiChannelListenerResidence,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) {
        let listener = residence.listener();
        if self.runtime_inspector_response_ready_sender().is_none() {
            self.release_bidi_channel_object_group_for_owner_best_effort_async(
                owner,
                listener.channel_object_group(),
            )
            .await;
            return;
        }
        let command_id = self.next_internal_runtime_command_id();
        let raw_json = bidi_channel_listener_call_function_json(command_id, listener);
        let descriptor = match RendererCommandDescriptor::from_synthesized_payload(raw_json) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                tracing::debug!(%error, "failed to prepare BiDi channel listener command");
                self.release_bidi_channel_object_group_for_owner_best_effort_async(
                    owner,
                    listener.channel_object_group(),
                )
                .await;
                return;
            }
        };
        let pending = match self
            .start_runtime_protocol_message_with_context_resolution_for_owner_with_deferred_response(
                owner,
                "callFunctionOn",
                descriptor,
                command_id,
            ) {
            Ok(pending) => pending,
            Err(error) => {
                tracing::debug!(
                    %error,
                    channel = %listener.properties().channel,
                    "failed to start BiDi channel listener"
                );
                self.release_bidi_channel_object_group_for_owner_best_effort_async(
                    owner,
                    listener.channel_object_group(),
                )
                .await;
                return;
            }
        };
        self.register_pending_bidi_channel_listener_for_owner(command_id, owner, residence);
        let mut completed = match pending.wait().await {
            Ok(completed) => completed,
            Err(error) => {
                let object_group = self
                    .remove_pending_inspector_await_for_cancellation_for_owner(command_id, owner)
                    .and_then(|entry| {
                        entry
                            .bidi_channel_listener()
                            .map(|listener| listener.channel_object_group().to_owned())
                    });
                tracing::debug!(%error, "BiDi channel listener dispatch failed");
                if let Some(object_group) = object_group {
                    self.release_bidi_channel_object_group_for_owner_best_effort_async(
                        owner,
                        &object_group,
                    )
                    .await;
                }
                return;
            }
        };
        let mut renderer_response_rx = completed.take_deferred_response_receiver();
        let messages = match self
            .complete_runtime_protocol_message_async(completed)
            .await
        {
            Ok(messages) => messages,
            Err(error) => {
                let object_group = self
                    .remove_pending_inspector_await_for_cancellation_for_owner(command_id, owner)
                    .and_then(|entry| {
                        entry
                            .bidi_channel_listener()
                            .map(|listener| listener.channel_object_group().to_owned())
                    });
                tracing::debug!(%error, "BiDi channel listener completion failed");
                if let Some(object_group) = object_group {
                    self.release_bidi_channel_object_group_for_owner_best_effort_async(
                        owner,
                        &object_group,
                    )
                    .await;
                }
                return;
            }
        };
        let mut listener_events = Vec::new();
        let mut listener_post_response_events = Vec::new();
        let response_flush = CommandResponseFlushContext::default();
        let listener_response_seen = if let Some(messages) = messages {
            self.route_renderer_command_turn_output_for_owner_into(
                messages,
                Some(command_id),
                owner,
                &response_flush,
                &mut listener_events,
                &mut listener_post_response_events,
            )
            .0
        } else {
            false
        };
        if !listener_response_seen {
            tracing::debug!(
                command_id,
                "BiDi channel listener inspector response was consumed before command response routing"
            );
        }
        if listener_response_seen {
            renderer_response_rx.take();
        }
        listener_events.extend(listener_post_response_events);
        let non_listener_response_count = listener_events
            .iter()
            .filter(|event| event.protocol_message_id().is_some())
            .count();
        if non_listener_response_count > 0 {
            tracing::debug!(
                messages = non_listener_response_count,
                "BiDi channel listener produced non-listener protocol messages on background route"
            );
        }
        background_events.extend(
            listener_events
                .into_iter()
                .filter(|event| event.protocol_message_id().is_none()),
        );
        if let Some(renderer_response_rx) = renderer_response_rx {
            // Listener responses use the same response-ready lane whether the
            // oneshot is already completed or still pending.
            if self.start_or_enqueue_registered_runtime_inspector_response_ready(
                command_id,
                owner,
                renderer_response_rx,
            ) {
                return;
            }
            let object_group = self
                .remove_pending_inspector_await_for_cancellation_for_owner(command_id, owner)
                .and_then(|entry| {
                    entry
                        .bidi_channel_listener()
                        .map(|listener| listener.channel_object_group().to_owned())
                });
            tracing::debug!(
                channel_object_group = object_group.as_deref(),
                "BiDi channel listener started without scheduler runtime response hook"
            );
            if let Some(object_group) = object_group {
                self.release_bidi_channel_object_group_for_owner_best_effort_async(
                    owner,
                    &object_group,
                )
                .await;
            }
        }
    }

    pub(crate) async fn runtime_realm_inventory_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<Vec<RuntimeExecutionContextEvent>, String> {
        let target_id = self
            .target_owner_identity_for_route(owner.session_id(), owner.session_owner_route())
            .and_then(|(_, target_id)| target_id);
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let target_id = target_id.as_deref();
        let devtools_target_id = target_id.map(DevToolsTargetId::from);
        let realms = page
            .runtime_realm_inventory_async()
            .await
            .map_err(|error| format!("runtime realm inventory failed: {error}"))?;
        realms
            .into_iter()
            .map(|realm| {
                runtime_realm_info_to_execution_context_event(
                    realm,
                    target_id,
                    devtools_target_id.clone(),
                )
            })
            .collect()
    }

    pub async fn runtime_default_execution_context_id_async(
        &mut self,
    ) -> Result<Option<i64>, String> {
        self.runtime_default_execution_context_id_for_session_owner_async(None)
            .await
    }

    pub async fn runtime_default_execution_context_id_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<Option<i64>, String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.default_execution_context_id_async()
            .await
            .map_err(|error| format!("runtime default execution context lookup failed: {error}"))
    }

    pub(crate) async fn runtime_default_or_initial_execution_context_id_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<Option<i64>, String> {
        let page = self
            .runtime_session_owner_slot_mut_for_route(
                owner.session_id(),
                owner.session_owner_route(),
            )?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.default_or_initial_execution_context_id_async()
            .await
            .map_err(|error| format!("runtime default execution context lookup failed: {error}"))
    }

    pub(crate) async fn runtime_ensure_isolated_world_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        frame_id: Option<&str>,
        world_name: &str,
    ) -> Result<i64, String> {
        let owner_target_id = self
            .target_owner_identity_for_route(owner.session_id(), owner.session_owner_route())
            .and_then(|(_, target_id)| target_id);
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let result = if let Some(frame_id) = frame_id
            && owner_target_id.as_deref() != Some(frame_id)
        {
            page.create_isolated_world_for_frame_async(frame_id, world_name, false)
                .await
        } else {
            page.create_isolated_world_async(world_name, false).await
        };
        result.map_err(|error| format!("runtime isolated world creation failed: {error}"))
    }

    pub async fn has_isolated_execution_context_id_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<bool, String> {
        self.has_isolated_execution_context_id_for_session_owner_async(None, execution_context_id)
            .await
    }

    pub async fn has_isolated_execution_context_id_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        execution_context_id: i64,
    ) -> Result<bool, String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.has_isolated_execution_context_id_async(execution_context_id)
            .await
            .map_err(|error| format!("runtime isolated context lookup failed: {error}"))
    }

    pub async fn has_child_default_execution_context_id_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        execution_context_id: i64,
    ) -> Result<bool, String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.child_frame_id_for_default_execution_context_id_async(execution_context_id)
            .await
            .map(|frame_id| frame_id.is_some())
            .map_err(|error| format!("runtime child default context lookup failed: {error}"))
    }

    pub async fn child_default_execution_context_id_for_frame_id_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        frame_id: &str,
    ) -> Result<Option<i64>, String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.child_default_execution_context_id_for_frame_id_for_owner_async(&owner, frame_id)
            .await
    }

    pub(crate) async fn child_default_execution_context_id_for_frame_id_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        frame_id: &str,
    ) -> Result<Option<i64>, String> {
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        page.child_default_execution_context_id_for_frame_id_async(frame_id)
            .await
            .map_err(|error| format!("runtime child default context lookup failed: {error}"))
    }

    pub(crate) fn start_child_default_execution_context_lookup_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        execution_context_id: i64,
    ) -> Result<PendingRuntimeChildDefaultContextLookupDispatch, String> {
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let pending = page
            .start_child_frame_id_for_default_execution_context_id(execution_context_id)
            .map_err(|error| format!("runtime child default context lookup failed: {error}"))?;
        Ok(PendingRuntimeChildDefaultContextLookupDispatch {
            owner: owner.clone(),
            pending,
        })
    }

    pub(crate) fn complete_child_default_execution_context_lookup(
        &mut self,
        completed: CompletedRuntimeChildDefaultContextLookupDispatch,
    ) -> Result<bool, String> {
        let page = self.runtime_session_owner_page_mut_for_owner(&completed.owner)?;
        page.finish_child_frame_id_for_default_execution_context_id(completed.completion)
            .map(|frame_id| frame_id.is_some())
            .map_err(|error| format!("runtime child default context lookup failed: {error}"))
    }

    pub async fn inspector_execution_context_id_for_isolated_context_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<Option<i64>, String> {
        self.inspector_execution_context_id_for_isolated_context_for_session_owner_async(
            None,
            execution_context_id,
        )
        .await
    }

    pub async fn inspector_execution_context_id_for_isolated_context_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        execution_context_id: i64,
    ) -> Result<Option<i64>, String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.ensure_isolated_worlds_attached_to_inspector_async()
            .await
            .map_err(|error| {
                format!("runtime isolated inspector context attachment failed: {error}")
            })?;
        page.inspector_execution_context_id_for_isolated_context_async(execution_context_id)
            .await
            .map_err(|error| format!("runtime isolated inspector context lookup failed: {error}"))
    }

    pub async fn isolated_execution_context_id_for_inspector_context_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<Option<i64>, String> {
        self.isolated_execution_context_id_for_inspector_context_for_session_owner_async(
            None,
            execution_context_id,
        )
        .await
    }

    pub async fn isolated_execution_context_id_for_inspector_context_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        execution_context_id: i64,
    ) -> Result<Option<i64>, String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.ensure_isolated_worlds_attached_to_inspector_async()
            .await
            .map_err(|error| {
                format!("runtime isolated compatibility context attachment failed: {error}")
            })?;
        page.isolated_execution_context_id_for_inspector_context_async(execution_context_id)
            .await
            .map_err(|error| {
                format!("runtime isolated compatibility context lookup failed: {error}")
            })
    }

    pub async fn evaluate_runtime_expression_in_execution_context_with_await_async(
        &mut self,
        execution_context_id: i64,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value, String> {
        self.evaluate_runtime_expression_in_execution_context_for_session_owner_async(
            None,
            execution_context_id,
            expression,
            await_promise,
        )
        .await
    }

    pub async fn evaluate_runtime_expression_in_execution_context_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        execution_context_id: i64,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value, String> {
        self.evaluate_runtime_expression_in_execution_context_for_session_owner_once_async(
            session_id,
            execution_context_id,
            expression,
            await_promise,
        )
        .await
    }

    async fn evaluate_runtime_expression_in_execution_context_for_session_owner_once_async(
        &mut self,
        session_id: Option<&str>,
        execution_context_id: i64,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value, String> {
        let payload = {
            let page = self.runtime_session_owner_page_mut(session_id)?;
            page.evaluate_runtime_expression_in_execution_context_without_navigation_follow_with_await_async(
                execution_context_id,
                expression,
                await_promise,
            )
            .await
            .map_err(|error| format!("runtime evaluation failed: {error}"))?
        };
        self.ingest_runtime_session_owner_output_updates(session_id);
        Ok(payload)
    }

    pub async fn install_runtime_binding_async(
        &mut self,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<(), String> {
        self.install_runtime_binding_for_session_owner_async(
            None,
            name,
            execution_context_name,
            execution_context_id,
        )
        .await
    }

    pub async fn install_runtime_binding_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<(), String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.install_runtime_binding_async(name, execution_context_name, execution_context_id)
            .await
            .map_err(|error| format!("runtime binding install failed: {error}"))
    }

    pub(crate) fn start_install_runtime_binding_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<PendingRuntimeBindingPageCommandDispatch, String> {
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let pending = page
            .start_install_runtime_binding(name, execution_context_name, execution_context_id)
            .map_err(|error| format!("runtime binding install failed: {error}"))?;
        Ok(PendingRuntimeBindingPageCommandDispatch {
            owner: owner.clone(),
            operation: "runtime binding install",
            pending,
        })
    }

    pub(crate) fn start_apply_stored_runtime_bindings_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<PendingRuntimeBindingPageCommandDispatch, String> {
        let stored_runtime_bindings = self.target_runtime_bindings_for_renderer_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let session_runtime_bindings = self.target_runtime_bindings_for_current_inspector_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let inspector_session_id = self.target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let pending = page
            .start_set_runtime_binding_state(
                inspector_session_id,
                &stored_runtime_bindings,
                &session_runtime_bindings,
            )
            .map_err(|error| format!("runtime binding state update failed: {error}"))?;
        Ok(PendingRuntimeBindingPageCommandDispatch {
            owner: owner.clone(),
            operation: "runtime binding state update",
            pending,
        })
    }

    pub(crate) async fn apply_runtime_binding_state_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<(), String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.apply_runtime_binding_state_for_owner_async(&owner)
            .await
    }

    pub(crate) async fn apply_runtime_binding_state_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<(), String> {
        let stored_runtime_bindings = self.target_runtime_bindings_for_renderer_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let session_runtime_bindings = self.target_runtime_bindings_for_current_inspector_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let inspector_session_id = self.target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        page.set_runtime_binding_state_async(
            inspector_session_id,
            &stored_runtime_bindings,
            &session_runtime_bindings,
        )
        .await
        .map_err(|error| format!("runtime binding state update failed: {error}"))
    }

    pub async fn remove_runtime_binding_async(&mut self, name: &str) -> Result<(), String> {
        self.remove_runtime_binding_for_session_owner_async(None, name)
            .await
    }

    pub async fn remove_runtime_binding_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        name: &str,
    ) -> Result<(), String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.remove_runtime_binding_async(name)
            .await
            .map_err(|error| format!("runtime binding removal failed: {error}"))
    }

    pub(crate) fn start_remove_runtime_binding_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        name: &str,
    ) -> Result<PendingRuntimeBindingPageCommandDispatch, String> {
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let pending = page
            .start_remove_runtime_binding(name)
            .map_err(|error| format!("runtime binding removal failed: {error}"))?;
        Ok(PendingRuntimeBindingPageCommandDispatch {
            owner: owner.clone(),
            operation: "runtime binding removal",
            pending,
        })
    }

    pub(crate) fn complete_runtime_binding_page_command(
        &mut self,
        completed: CompletedRuntimeBindingPageCommandDispatch,
    ) -> Result<(), String> {
        let page = self.runtime_session_owner_page_mut_for_owner(&completed.owner)?;
        page.finish_unit_runtime_page_command(completed.completion, completed.operation)
            .map_err(|error| format!("{} failed: {error}", completed.operation))
    }

    pub async fn remove_default_runtime_binding_async(&mut self, name: &str) -> Result<(), String> {
        self.remove_default_runtime_binding_for_session_owner_async(None, name)
            .await
    }

    pub async fn remove_default_runtime_binding_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        name: &str,
    ) -> Result<(), String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.remove_default_runtime_binding_async(name)
            .await
            .map_err(|error| format!("runtime default binding removal failed: {error}"))
    }

    pub(crate) async fn detach_runtime_inspector_session_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<bool, String> {
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_session(session_id);
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.detach_runtime_inspector_session_async(inspector_session_id.as_deref())
            .await
            .map_err(|error| format!("runtime inspector session detach failed: {error}"))
    }
}

fn send_renderer_replacement_error(
    response_sender: &RendererRuntimeInspectorResponseSender,
    correlation: RendererCommandCorrelation,
    message: &str,
) {
    let _ = response_sender.clone().send(json!({
        "id": correlation.renderer_call_id().get(),
        "error": {
            "code": -32000,
            "message": message,
        },
    }));
}

fn runtime_realm_info_to_execution_context_event(
    realm: RendererRuntimeRealmInfo,
    owner_frame_id: Option<&str>,
    target_id: Option<DevToolsTargetId>,
) -> Result<RuntimeExecutionContextEvent, String> {
    let realm_frame_id = realm
        .frame_id
        .as_deref()
        .filter(|frame_id| !frame_id.is_empty())
        .or(owner_frame_id);
    let realm_id = realm
        .realm_id
        .filter(|realm_id| !realm_id.is_empty())
        .map(|realm_id| protocol_global_realm_id(realm_id, target_id.as_ref(), realm_frame_id))
        .map(DevToolsRealmId::from);
    Ok(RuntimeExecutionContextEvent {
        target_id,
        context_id: Some(realm.context_id),
        realm_id,
        frame_id: realm_frame_id.map(DevToolsFrameId::from),
        origin: Some(realm.origin),
        name: Some(realm.name),
        is_default: Some(realm.is_default),
        context_type: Some(realm.context_type),
        grant_universal_access: None,
    })
}

fn protocol_global_realm_id(
    native_realm_id: String,
    target_id: Option<&DevToolsTargetId>,
    frame_id: Option<&str>,
) -> String {
    let Some(owner_id) = target_id.map(DevToolsTargetId::as_str).or(frame_id) else {
        return native_realm_id;
    };
    format!("{owner_id}:{native_realm_id}")
}

fn collect_runtime_remote_object_ids(
    mut object_ids: Vec<String>,
    mut stack: Vec<(&Value, usize)>,
) -> Vec<String> {
    while let Some((value, remaining_tree_depth)) = stack.pop() {
        let Some(next_tree_depth) = remaining_tree_depth.checked_sub(1) else {
            continue;
        };
        match value {
            Value::Object(map) => {
                for key in ["objectId", "promiseObjectId", "errorObjectId"] {
                    if let Some(object_id) = map.get(key).and_then(Value::as_str) {
                        object_ids.push(object_id.to_owned());
                    }
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
    object_ids.sort();
    object_ids.dedup();
    object_ids
}

pub(crate) fn runtime_remote_object_ids_in_value(value: &Value) -> Vec<String> {
    collect_runtime_remote_object_ids(
        Vec::new(),
        vec![(value, MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH)],
    )
}

pub(crate) fn runtime_remote_object_ids_in_map(map: &Map<String, Value>) -> Vec<String> {
    let mut object_ids = Vec::new();
    for key in ["objectId", "promiseObjectId", "errorObjectId"] {
        if let Some(object_id) = map.get(key).and_then(Value::as_str) {
            object_ids.push(object_id.to_owned());
        }
    }
    let Some(next_tree_depth) = MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH.checked_sub(1) else {
        return object_ids;
    };
    collect_runtime_remote_object_ids(
        object_ids,
        map.values().map(|value| (value, next_tree_depth)).collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestContext;
    use moli_core::page::{MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH, is_renderer_backend_node_id};

    #[test]
    fn registered_command_reply_route_owns_exactly_one_receiver() {
        let (_response_tx, response_rx) = tokio::sync::oneshot::channel();
        let mut route = RuntimeProtocolResponseRoute::for_registered_delivery(
            RendererInspectorResponseDelivery::CommandReply,
            Some(response_rx),
        );

        assert_eq!(
            route.delivery(),
            RendererInspectorResponseDelivery::CommandReply
        );
        assert!(route.take_command_reply_receiver().is_some());
        assert!(route.take_command_reply_receiver().is_none());
    }

    #[test]
    #[should_panic(expected = "registered command-reply route must allocate its receiver")]
    fn registered_command_reply_route_rejects_a_missing_receiver() {
        let _ = RuntimeProtocolResponseRoute::for_registered_delivery(
            RendererInspectorResponseDelivery::CommandReply,
            None,
        );
    }

    #[test]
    fn registered_devtools_session_route_has_no_command_reply_receiver() {
        let mut route = RuntimeProtocolResponseRoute::for_registered_delivery(
            RendererInspectorResponseDelivery::DevToolsSession,
            None,
        );

        assert_eq!(
            route.delivery(),
            RendererInspectorResponseDelivery::DevToolsSession
        );
        assert!(route.take_command_reply_receiver().is_none());
    }

    #[test]
    #[should_panic(expected = "DevTools session response cannot retain a command-reply receiver")]
    fn devtools_session_route_rejects_command_reply_receiver() {
        let (_response_tx, response_rx) = tokio::sync::oneshot::channel();
        let _ = RuntimeProtocolResponseRoute::for_registered_delivery(
            RendererInspectorResponseDelivery::DevToolsSession,
            Some(response_rx),
        );
    }

    #[test]
    fn replay_response_routes_never_claim_a_second_local_receiver() {
        let mut command_reply = RuntimeProtocolResponseRoute::without_local_receiver_for_delivery(
            RendererInspectorResponseDelivery::CommandReply,
        );
        let mut devtools_session =
            RuntimeProtocolResponseRoute::without_local_receiver_for_delivery(
                RendererInspectorResponseDelivery::DevToolsSession,
            );

        assert_eq!(
            command_reply.delivery(),
            RendererInspectorResponseDelivery::CommandReply
        );
        assert_eq!(
            devtools_session.delivery(),
            RendererInspectorResponseDelivery::DevToolsSession
        );
        assert!(command_reply.take_command_reply_receiver().is_none());
        assert!(devtools_session.take_command_reply_receiver().is_none());
    }

    #[test]
    fn runtime_inspector_command_rewrites_large_frontend_id_to_renderer_call_id() {
        let frontend_command_id = FrontendCommandId::new(i32::MAX as u64 + 73);
        let raw_json = json!({
            "id": frontend_command_id.get(),
            "method": "Runtime.evaluate",
            "params": { "expression": "42" },
            "sessionId": "SID-large-id",
        })
        .to_string();

        let rewritten = rewrite_runtime_inspector_command_for_renderer(
            &raw_json,
            Some((frontend_command_id, RendererCallId::new(11))),
            None,
        )
        .unwrap();
        let rewritten: Value = serde_json::from_str(&rewritten).unwrap();

        assert_eq!(rewritten["id"], json!(11));
        assert_eq!(rewritten["method"], json!("Runtime.evaluate"));
        assert_eq!(rewritten["params"]["expression"], json!("42"));
        assert_eq!(rewritten["sessionId"], json!("SID-large-id"));
    }

    #[test]
    fn runtime_inspector_command_rewrite_rejects_mismatched_wire_id() {
        let error = rewrite_runtime_inspector_command_for_renderer(
            r#"{"id":8,"method":"Runtime.evaluate","params":{}}"#,
            Some((FrontendCommandId::new(9), RendererCallId::new(1))),
            None,
        )
        .unwrap_err();

        assert_eq!(
            error,
            "runtime Inspector command id mismatch: expected 9, got 8"
        );
    }

    #[test]
    fn runtime_inspector_command_dequalifies_current_owner_unique_context_id() {
        let raw_json = json!({
            "id": 8,
            "method": "Runtime.callFunctionOn",
            "params": {
                "functionDeclaration": "function() { return 42; }",
                "uniqueContextId": "TID-current:17.23"
            }
        })
        .to_string();

        let rewritten =
            rewrite_runtime_inspector_command_for_renderer(&raw_json, None, Some("TID-current"))
                .unwrap();
        let rewritten: Value = serde_json::from_str(&rewritten).unwrap();

        assert_eq!(rewritten["params"]["uniqueContextId"], json!("17.23"));
    }

    #[test]
    fn runtime_inspector_command_does_not_dequalify_another_owners_realm() {
        let raw_json = json!({
            "id": 8,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "42",
                "uniqueContextId": "TID-stale:17.23"
            }
        })
        .to_string();

        let rewritten =
            rewrite_runtime_inspector_command_for_renderer(&raw_json, None, Some("TID-current"))
                .unwrap();
        let rewritten: Value = serde_json::from_str(&rewritten).unwrap();

        assert_eq!(
            rewritten["params"]["uniqueContextId"],
            json!("TID-stale:17.23")
        );
    }

    fn connection_with_bidi_page_session() -> CdpConnection {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-owner".to_owned());
        browser_context.set_active_target_id("TID-active");
        browser_context.attach_active_session("SID-active".to_owned());
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        conn.browser_context = Some(browser_context);
        conn
    }

    #[test]
    fn runtime_remote_object_validation_allows_session_local_id_collisions() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-owner".to_owned());
        browser_context.set_active_target_id("TID-active");
        browser_context.attach_active_session("SID-active".to_owned());
        assert!(
            browser_context
                .assign_auxiliary_session_to_target("TID-active", "SID-auxiliary".to_owned(),)
        );
        conn.browser_context = Some(browser_context);

        conn.register_runtime_remote_object_ids_for_session_owner(
            Some("SID-active"),
            vec!["same-wire-id".to_owned()],
        );
        conn.register_runtime_remote_object_ids_for_session_owner(
            Some("SID-auxiliary"),
            vec!["same-wire-id".to_owned(), "auxiliary-only".to_owned()],
        );

        assert!(
            conn.validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-active"),
                &["same-wire-id".to_owned()],
            )
            .is_ok(),
            "a current-session handle must win over an identical wire id in another session"
        );
        assert!(
            conn.validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-auxiliary"),
                &["same-wire-id".to_owned()],
            )
            .is_ok(),
            "the same V8 wire id can independently belong to the auxiliary session"
        );
        assert_eq!(
            conn.validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-active"),
                &["auxiliary-only".to_owned()],
            ),
            Err("Cannot find object with given id".to_owned()),
            "an id known only to another session must remain inaccessible"
        );
    }

    #[test]
    fn runtime_remote_object_validation_tolerates_an_empty_browser_context() {
        let mut conn = connection_with_bidi_page_session();
        conn.insert_browser_context(BrowserContext::new("BID-empty".to_owned()));

        assert!(
            conn.validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-active"),
                &["unregistered-wire-id".to_owned()],
            )
            .is_ok(),
            "an unrelated BrowserContext without a Page target must not be dereferenced as active"
        );
    }

    fn bidi_channel_listener_for_test(channel: &str) -> PendingBidiChannelListener {
        PendingBidiChannelListener::new(
            Some(DevToolsTargetId::from("TID-active")),
            Some(crate::devtools_runtime::DevToolsRealmId::from(
                "realm-active",
            )),
            crate::devtools_runtime::DevToolsRemoteHandleId::from(format!(
                "channel-proxy-{channel}"
            )),
            format!("webdriver-bidi-channel-{channel}"),
            crate::devtools_runtime::DevToolsBidiChannelProperties {
                channel: channel.to_owned(),
                ownership: DevToolsResultOwnership::None,
                serialization_options: None,
            },
        )
        .expect("test listener should include target and realm")
    }

    fn renderer_command_descriptor_for_test(command_id: u64) -> RendererCommandDescriptor {
        RendererCommandDescriptor::from_synthesized_payload(
            json!({
                "id": command_id,
                "method": "Runtime.evaluate",
                "params": { "expression": "1" },
            })
            .to_string(),
        )
        .unwrap()
    }

    fn devtools_session_renderer_command_descriptor_for_test(
        command_id: u64,
    ) -> RendererCommandDescriptor {
        let frontend_payload = json!({
            "id": command_id,
            "method": "Runtime.evaluate",
            "params": { "expression": command_id.to_string() },
        })
        .to_string();
        let frontend =
            ParsedCdpCommand::parse_str(&frontend_payload).expect("frontend command should parse");
        RendererCommandDescriptor::from_frontend_policy(
            frontend.json().to_owned(),
            frontend.renderer_policy(),
            RendererInspectorResponseDelivery::DevToolsSession,
        )
    }

    fn register_devtools_session_response_for_test(
        conn: &mut CdpConnection,
        session_id: &str,
        frontend_command_id: u64,
        attachment_id: RendererAgentAttachmentId,
        frontend_payload: &str,
    ) -> RendererCommandCorrelation {
        let frontend =
            ParsedCdpCommand::parse_str(frontend_payload).expect("frontend command should parse");
        let prepared = conn
            .try_register_renderer_call_for_session_owner(
                Some(session_id),
                frontend_command_id,
                Some(attachment_id),
                RendererCommandDescriptor::from_frontend_policy(
                    frontend.json().to_owned(),
                    frontend.renderer_policy(),
                    RendererInspectorResponseDelivery::DevToolsSession,
                ),
            )
            .expect("frontend response correlation should register");
        let (correlation, response_sender, response_receiver) = prepared.into_parts();
        assert!(
            response_receiver.is_none(),
            "DevToolsSession delivery must not allocate a command-reply receiver"
        );
        drop(response_sender);
        correlation
    }

    #[test]
    fn navigation_termination_consumes_a_devtools_session_frontend_call() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-navigation-termination".to_owned());
        browser_context.set_active_target_id("TID-navigation-termination".to_owned());
        browser_context.attach_active_session("SID-navigation-termination".to_owned());
        conn.browser_context = Some(browser_context);

        let old_attachment = RendererAgentAttachmentId::allocate();
        let terminal_attachment = RendererAgentAttachmentId::allocate();
        let prepared = conn
            .try_register_renderer_call_for_session_owner(
                Some("SID-navigation-termination"),
                69,
                Some(old_attachment),
                devtools_session_renderer_command_descriptor_for_test(69),
            )
            .expect("frontend response correlation should register");
        let (old_correlation, old_sender, response_receiver) = prepared.into_parts();
        assert!(response_receiver.is_none());

        let replacements = {
            let browser_context = conn
                .browser_context
                .as_mut()
                .expect("test browser context should remain loaded");
            let page_state = browser_context.active_page_target_mut();
            page_state
                .devtools_sessions
                .prepare_renderer_call_replacements(
                    Some("SID-navigation-termination"),
                    old_attachment,
                    terminal_attachment,
                )
                .expect("navigation replacement should prepare")
        };
        let (replacement_attachment, terminations, replays) = replacements.into_parts();
        assert_eq!(replacement_attachment, terminal_attachment);
        assert_eq!(terminations.len(), 1);
        assert!(replays.is_empty());
        assert!(
            old_sender
                .send(json!({
                    "id": old_correlation.renderer_call_id().get(),
                    "result": {},
                }))
                .is_err(),
            "navigation termination must invalidate the old renderer lease"
        );
        assert_eq!(
            conn.renderer_call_for_frontend_for_session_owner(
                Some("SID-navigation-termination"),
                69,
            ),
            Some(old_correlation),
            "direct session termination must retain the original correlation until settlement"
        );

        let termination_events = conn.terminate_prepared_renderer_calls_after_navigation(
            terminations,
            "Inspected target navigated or closed",
        );
        assert_eq!(termination_events.len(), 1);
        let response = termination_events[0]
            .protocol_message()
            .expect("DevToolsSession termination should emit a frontend response");
        assert_eq!(response["id"], json!(69));
        assert_eq!(response["sessionId"], json!("SID-navigation-termination"));
        assert_eq!(response["error"]["code"], json!(-32000));
        assert_eq!(
            response["error"]["message"],
            json!("Inspected target navigated or closed")
        );

        assert!(
            conn.renderer_runtime_command_cause_for_frontend(
                Some("SID-navigation-termination"),
                69,
            )
            .is_none(),
            "navigation termination must consume the frontend correlation"
        );
    }

    #[test]
    fn navigation_termination_isolated_same_frontend_id_by_devtools_session() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-navigation-sessions".to_owned());
        browser_context.set_active_target_id("TID-navigation-sessions".to_owned());
        browser_context.attach_active_session("SID-navigation-primary".to_owned());
        assert!(browser_context.assign_auxiliary_session_to_target(
            "TID-navigation-sessions",
            "SID-navigation-auxiliary".to_owned(),
        ));
        conn.browser_context = Some(browser_context);

        let old_attachment = RendererAgentAttachmentId::allocate();
        let terminal_attachment = RendererAgentAttachmentId::allocate();
        let primary = conn
            .try_register_renderer_call_for_session_owner(
                Some("SID-navigation-primary"),
                71,
                Some(old_attachment),
                devtools_session_renderer_command_descriptor_for_test(71),
            )
            .expect("primary frontend response correlation should register");
        let auxiliary = conn
            .try_register_renderer_call_for_session_owner(
                Some("SID-navigation-auxiliary"),
                71,
                Some(old_attachment),
                devtools_session_renderer_command_descriptor_for_test(71),
            )
            .expect("auxiliary frontend response correlation should register");
        let (primary_correlation, primary_sender, primary_receiver) = primary.into_parts();
        let (auxiliary_correlation, auxiliary_sender, auxiliary_receiver) = auxiliary.into_parts();
        assert!(primary_receiver.is_none());
        assert!(auxiliary_receiver.is_none());

        let replacements = {
            let browser_context = conn
                .browser_context
                .as_mut()
                .expect("test browser context should remain loaded");
            let page_state = browser_context.active_page_target_mut();
            page_state
                .devtools_sessions
                .prepare_renderer_call_replacements(
                    Some("SID-navigation-primary"),
                    old_attachment,
                    terminal_attachment,
                )
                .expect("navigation replacement should prepare for every session")
        };
        let (_, terminations, replays) = replacements.into_parts();
        assert_eq!(terminations.len(), 2);
        assert!(replays.is_empty());
        for (correlation, sender) in [
            (primary_correlation, primary_sender),
            (auxiliary_correlation, auxiliary_sender),
        ] {
            assert!(
                sender
                    .send(json!({
                        "id": correlation.renderer_call_id().get(),
                        "result": {},
                    }))
                    .is_err(),
                "navigation replacement must invalidate every old session lease"
            );
        }
        assert_eq!(
            conn.renderer_call_for_frontend_for_session_owner(Some("SID-navigation-primary"), 71,),
            Some(primary_correlation)
        );
        assert_eq!(
            conn.renderer_call_for_frontend_for_session_owner(
                Some("SID-navigation-auxiliary"),
                71,
            ),
            Some(auxiliary_correlation)
        );

        let termination_events = conn.terminate_prepared_renderer_calls_after_navigation(
            terminations,
            "Inspected target navigated or closed",
        );
        assert_eq!(termination_events.len(), 2);
        let session_ids = termination_events
            .iter()
            .map(|event| {
                let response = event
                    .protocol_message()
                    .expect("each session termination should emit a frontend response");
                assert_eq!(response["id"], json!(71));
                assert_eq!(response["error"]["code"], json!(-32000));
                response["sessionId"]
                    .as_str()
                    .expect("attached sessions must retain sessionId")
                    .to_owned()
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            session_ids,
            std::collections::BTreeSet::from([
                "SID-navigation-primary".to_owned(),
                "SID-navigation-auxiliary".to_owned(),
            ])
        );
        assert!(
            conn.renderer_runtime_command_cause_for_frontend(Some("SID-navigation-primary"), 71,)
                .is_none()
        );
        assert!(
            conn.renderer_runtime_command_cause_for_frontend(Some("SID-navigation-auxiliary"), 71,)
                .is_none()
        );
    }

    #[test]
    fn navigation_termination_preserves_sessionless_page_response_shape() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-navigation-sessionless".to_owned());
        browser_context.set_active_target_id("TID-navigation-sessionless".to_owned());
        conn.browser_context = Some(browser_context);

        let old_attachment = RendererAgentAttachmentId::allocate();
        let terminal_attachment = RendererAgentAttachmentId::allocate();
        let prepared = conn
            .try_register_renderer_call_for_session_owner(
                None,
                72,
                Some(old_attachment),
                devtools_session_renderer_command_descriptor_for_test(72),
            )
            .expect("sessionless frontend response correlation should register");
        let (old_correlation, old_sender, response_receiver) = prepared.into_parts();
        assert!(response_receiver.is_none());

        let replacements = {
            let browser_context = conn
                .browser_context
                .as_mut()
                .expect("test browser context should remain loaded");
            let page_state = browser_context.active_page_target_mut();
            page_state
                .devtools_sessions
                .prepare_renderer_call_replacements(None, old_attachment, terminal_attachment)
                .expect("sessionless navigation replacement should prepare")
        };
        let (_, terminations, replays) = replacements.into_parts();
        assert_eq!(terminations.len(), 1);
        assert!(replays.is_empty());
        drop(old_sender);
        assert_eq!(
            conn.renderer_call_for_frontend_for_session_owner(None, 72),
            Some(old_correlation)
        );

        let termination_events = conn.terminate_prepared_renderer_calls_after_navigation(
            terminations,
            "Inspected target navigated or closed",
        );
        assert_eq!(termination_events.len(), 1);
        let response = termination_events[0]
            .protocol_message()
            .expect("sessionless termination should emit a frontend response");
        assert_eq!(response["id"], json!(72));
        assert!(response.get("sessionId").is_none());
        assert_eq!(response["error"]["code"], json!(-32000));
        assert!(
            conn.renderer_runtime_command_cause_for_frontend(None, 72)
                .is_none()
        );
    }

    #[test]
    fn devtools_session_output_wrong_attachment_does_not_consume_live_correlation() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-attachment-race".to_owned());
        browser_context.set_active_target_id("TID-attachment-race".to_owned());
        browser_context.attach_active_session("SID-attachment-race".to_owned());
        conn.browser_context = Some(browser_context);

        let live_attachment = RendererAgentAttachmentId::allocate();
        let stale_attachment = RendererAgentAttachmentId::allocate();
        let correlation = register_devtools_session_response_for_test(
            &mut conn,
            "SID-attachment-race",
            70,
            live_attachment,
            r#"{"id":70,"method":"Runtime.evaluate","params":{"expression":"70"}}"#,
        );
        let response = || {
            RendererRuntimeInspectorMessage::protocol(json!({
                "id": correlation.renderer_call_id().get(),
                "result": { "result": { "type": "number", "value": 70 } },
            }))
        };

        let mut stale_messages = vec![response()];
        conn.restore_frontend_command_ids_in_devtools_session_output(
            Some("SID-attachment-race"),
            stale_attachment,
            &mut stale_messages,
        );
        assert!(
            stale_messages.is_empty(),
            "a response from the retired attachment must be dropped"
        );
        assert!(
            conn.renderer_runtime_command_cause_for_frontend(Some("SID-attachment-race"), 70,)
                .is_some(),
            "the retired attachment must not consume the live correlation"
        );

        let mut live_messages = vec![response()];
        conn.restore_frontend_command_ids_in_devtools_session_output(
            Some("SID-attachment-race"),
            live_attachment,
            &mut live_messages,
        );
        let [RendererRuntimeInspectorMessage::Protocol(message)] = live_messages.as_slice() else {
            panic!("the live attachment must publish exactly one response");
        };
        assert_eq!(message.value()["id"], json!(70));
        assert!(
            conn.renderer_runtime_command_cause_for_frontend(Some("SID-attachment-race"), 70,)
                .is_none(),
            "the live attachment must consume the correlation exactly once"
        );
    }

    #[test]
    fn devtools_session_output_keeps_first_of_duplicate_terminal_responses() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-duplicate-response".to_owned());
        browser_context.set_active_target_id("TID-duplicate-response".to_owned());
        browser_context.attach_active_session("SID-duplicate-response".to_owned());
        conn.browser_context = Some(browser_context);

        let attachment_id = RendererAgentAttachmentId::allocate();
        let correlation = register_devtools_session_response_for_test(
            &mut conn,
            "SID-duplicate-response",
            71,
            attachment_id,
            r#"{"id":71,"method":"Runtime.evaluate","params":{"expression":"71"}}"#,
        );
        let mut messages = vec![
            RendererRuntimeInspectorMessage::protocol(json!({
                "id": correlation.renderer_call_id().get(),
                "result": { "result": { "type": "string", "value": "first" } },
            })),
            RendererRuntimeInspectorMessage::protocol(json!({
                "id": correlation.renderer_call_id().get(),
                "result": { "result": { "type": "string", "value": "duplicate" } },
            })),
        ];

        conn.restore_frontend_command_ids_in_devtools_session_output(
            Some("SID-duplicate-response"),
            attachment_id,
            &mut messages,
        );

        let [RendererRuntimeInspectorMessage::Protocol(message)] = messages.as_slice() else {
            panic!("duplicate terminal responses must collapse to one frontend response");
        };
        assert_eq!(message.value()["id"], json!(71));
        assert_eq!(message.value()["result"]["result"]["value"], json!("first"));
    }

    #[test]
    fn devtools_session_output_restores_interleaved_calls_without_reordering() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-interleaved-response".to_owned());
        browser_context.set_active_target_id("TID-interleaved-response".to_owned());
        browser_context.attach_active_session("SID-interleaved-response".to_owned());
        conn.browser_context = Some(browser_context);

        let attachment_id = RendererAgentAttachmentId::allocate();
        let first = register_devtools_session_response_for_test(
            &mut conn,
            "SID-interleaved-response",
            80,
            attachment_id,
            r#"{"id":80,"method":"Runtime.evaluate","params":{"expression":"80"}}"#,
        );
        let second = register_devtools_session_response_for_test(
            &mut conn,
            "SID-interleaved-response",
            81,
            attachment_id,
            r#"{"id":81,"method":"Runtime.evaluate","params":{"expression":"81"}}"#,
        );
        let mut messages = vec![
            RendererRuntimeInspectorMessage::protocol(json!({
                "id": second.renderer_call_id().get(),
                "result": { "result": { "type": "number", "value": 81 } },
            })),
            RendererRuntimeInspectorMessage::protocol(json!({
                "method": "Debugger.scriptParsed",
                "params": { "scriptId": "interleaved" },
            })),
            RendererRuntimeInspectorMessage::protocol(json!({
                "id": first.renderer_call_id().get(),
                "result": { "result": { "type": "number", "value": 80 } },
            })),
        ];

        conn.restore_frontend_command_ids_in_devtools_session_output(
            Some("SID-interleaved-response"),
            attachment_id,
            &mut messages,
        );

        assert_eq!(messages.len(), 3);
        let RendererRuntimeInspectorMessage::Protocol(second_response) = &messages[0] else {
            panic!("the second call response must remain first");
        };
        let RendererRuntimeInspectorMessage::Protocol(notification) = &messages[1] else {
            panic!("the notification must remain between the responses");
        };
        let RendererRuntimeInspectorMessage::Protocol(first_response) = &messages[2] else {
            panic!("the first call response must remain last");
        };
        assert_eq!(second_response.value()["id"], json!(81));
        assert_eq!(
            notification.value()["method"],
            json!("Debugger.scriptParsed")
        );
        assert_eq!(first_response.value()["id"], json!(80));
    }

    #[tokio::test]
    async fn devtools_session_output_restores_only_the_exact_registered_frontend_response() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-session-output".to_owned());
        browser_context.set_active_target_id("TID-session-output".to_owned());
        browser_context.attach_active_session("SID-session-output".to_owned());
        conn.browser_context = Some(browser_context);

        let attachment_id = RendererAgentAttachmentId::allocate();
        let frontend = ParsedCdpCommand::parse_str(
            r#"{"id":44,"method":"Runtime.evaluate","params":{"expression":"({ answer: 42 })","objectGroup":"nested-main"}}"#,
        )
        .expect("frontend command should parse");
        let prepared = conn
            .try_register_renderer_call_for_session_owner(
                Some("SID-session-output"),
                44,
                Some(attachment_id),
                RendererCommandDescriptor::from_frontend_policy(
                    frontend.json().to_owned(),
                    frontend.renderer_policy(),
                    RendererInspectorResponseDelivery::DevToolsSession,
                ),
            )
            .expect("frontend response correlation should register");
        let (correlation, response_sender, response_receiver) = prepared.into_parts();
        assert!(
            response_receiver.is_none(),
            "DevToolsSession delivery must not allocate a legacy command-reply receiver"
        );
        drop(response_sender);
        let mut messages = vec![
            RendererRuntimeInspectorMessage::protocol(json!({
                "method": "Debugger.scriptParsed",
                "params": { "scriptId": "7" },
            })),
            RendererRuntimeInspectorMessage::protocol(json!({
                "id": correlation.renderer_call_id().get(),
                "result": {
                    "result": {
                        "type": "object",
                        "objectId": "nested-main-object"
                    }
                },
            })),
            RendererRuntimeInspectorMessage::protocol(json!({
                "id": correlation.renderer_call_id().get() + 1,
                "result": { "scriptSource": "stale" },
            })),
        ];

        conn.restore_frontend_command_ids_in_devtools_session_output(
            Some("SID-session-output"),
            attachment_id,
            &mut messages,
        );

        assert_eq!(
            messages.len(),
            2,
            "stale renderer responses must be dropped"
        );
        let RendererRuntimeInspectorMessage::Protocol(response) = &messages[1] else {
            panic!("expected a protocol response");
        };
        assert_eq!(response.value()["id"], json!(44));
        assert_eq!(
            conn.runtime_remote_object_group_for_session_owner(
                Some("SID-session-output"),
                "nested-main-object",
            ),
            Some("nested-main".to_owned()),
            "session output must retain Runtime object ownership metadata",
        );
        assert!(
            conn.renderer_runtime_command_cause_for_frontend(Some("SID-session-output"), 44,)
                .is_none(),
            "publishing the session response must consume its exact correlation"
        );
    }

    #[test]
    fn devtools_session_output_preserves_remote_object_group_projection() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-session-projection".to_owned());
        browser_context.set_active_target_id("TID-session-projection".to_owned());
        browser_context.attach_active_session("SID-session-projection".to_owned());
        conn.browser_context = Some(browser_context);
        let session_id = Some("SID-session-projection");
        let attachment_id = RendererAgentAttachmentId::allocate();

        conn.register_runtime_remote_object_ids_from_value_for_session_owner_with_group(
            session_id,
            &json!({ "objectId": "nested-parent-object" }),
            "nested-object-group",
        );

        let get_properties = ParsedCdpCommand::parse_str(
            r#"{"id":45,"method":"Runtime.getProperties","params":{"objectId":"nested-parent-object","ownProperties":true}}"#,
        )
        .expect("getProperties command should parse");
        let prepared = conn
            .try_register_renderer_call_for_session_owner(
                session_id,
                45,
                Some(attachment_id),
                RendererCommandDescriptor::from_frontend_policy(
                    get_properties.json().to_owned(),
                    get_properties.renderer_policy(),
                    RendererInspectorResponseDelivery::DevToolsSession,
                ),
            )
            .expect("getProperties response correlation should register");
        let (correlation, response_sender, response_receiver) = prepared.into_parts();
        assert!(response_receiver.is_none());
        drop(response_sender);
        let mut messages = vec![RendererRuntimeInspectorMessage::protocol(json!({
            "id": correlation.renderer_call_id().get(),
            "result": {
                "result": [{
                    "name": "child",
                    "value": {
                        "type": "object",
                        "objectId": "nested-child-object"
                    }
                }]
            },
        }))];
        conn.restore_frontend_command_ids_in_devtools_session_output(
            session_id,
            attachment_id,
            &mut messages,
        );
        assert_eq!(
            conn.runtime_remote_object_group_for_session_owner(session_id, "nested-child-object",),
            Some("nested-object-group".to_owned()),
            "getProperties results must inherit the receiver object's group",
        );
    }

    fn bidi_channel_listener_residence_for_test(
        conn: &CdpConnection,
        session_id: &str,
        channel: &str,
    ) -> BidiChannelListenerResidence {
        BidiChannelListenerResidence::new(
            BidiChannelPageOwner::capture_for_owner(
                conn,
                CommandOwnerScope::for_session(session_id),
            )
            .expect("test Page attachment"),
            bidi_channel_listener_for_test(channel),
        )
    }

    fn deeply_nested_plain_value(mut value: Value, depth: usize) -> Value {
        for _ in 0..depth {
            value = json!({ "child": [value] });
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

    fn take_only_protocol_work(
        conn: &mut CdpConnection,
    ) -> crate::domains::activity::ProtocolSchedulerWork {
        let scheduler_events = conn.take_scheduler_events();
        let [CdpSchedulerEvent::ProtocolWorkPublished { work }] =
            <[_; 1]>::try_from(scheduler_events)
                .expect("test action must publish exactly one protocol work")
        else {
            unreachable!("array pattern fixes the only event kind")
        };
        work
    }

    #[test]
    fn runtime_remote_object_ids_include_await_promise_handles() {
        let value = json!({
            "params": {
                "errorObjectId": "error-1",
                "promiseObjectId": "promise-1",
                "arguments": [{ "objectId": "arg-1" }]
            }
        });
        let object_ids = runtime_remote_object_ids_in_value(&value);

        assert_eq!(
            object_ids,
            vec![
                "arg-1".to_owned(),
                "error-1".to_owned(),
                "promise-1".to_owned()
            ],
            "Runtime object-owner validation must include objectId, promiseObjectId, and errorObjectId handles"
        );
        assert_eq!(
            runtime_remote_object_ids_in_map(
                value
                    .as_object()
                    .expect("the test protocol payload must be an object")
            ),
            object_ids,
            "validated object params must preserve the existing recursive handle scan"
        );
    }

    #[test]
    fn runtime_remote_object_ids_respect_protocol_depth_cap() {
        run_deep_protocol_value_test("runtime-remote-object-ids-depth-cap", || {
            let object_ids = runtime_remote_object_ids_in_value(&deeply_nested_plain_value(
                json!({ "objectId": "too-deep" }),
                MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH + 8,
            ));

            assert!(object_ids.is_empty());
        });
    }

    #[test]
    fn bidi_channel_listener_owner_work_publishes_concrete_scheduler_work() {
        let mut conn = connection_with_bidi_page_session();
        let listener = bidi_channel_listener_residence_for_test(&conn, "SID-active", "wake");
        conn.publish_bidi_channel_listener_start(listener);

        let scheduler_events = conn.take_scheduler_events();
        let [CdpSchedulerEvent::ProtocolWorkPublished { work }] = scheduler_events.as_slice()
        else {
            panic!("listener start must publish one concrete protocol work: {scheduler_events:?}");
        };
        assert_eq!(
            work.kind(),
            crate::domains::activity::ProtocolSchedulerWorkKind::BidiChannelOwnerAction
        );
        assert_eq!(
            work.bidi_channel_owner_action_kind(),
            Some(BidiChannelOwnerActionKind::StartListener)
        );
        assert_eq!(work.publish_sequence().get(), 1);
    }

    #[test]
    fn bidi_channel_actions_keep_causal_publication_order() {
        let mut conn = connection_with_bidi_page_session();
        let listener = bidi_channel_listener_residence_for_test(&conn, "SID-active", "ordered");
        let owner = listener.owner().clone();
        conn.publish_bidi_channel_listener_start(listener);
        conn.publish_bidi_channel_object_group_release(owner, "webdriver-bidi-channel-ordered");
        let scheduler_events = conn.take_scheduler_events();
        let works = scheduler_events
            .iter()
            .map(|event| {
                let CdpSchedulerEvent::ProtocolWorkPublished { work } = event else {
                    panic!("BiDi action must not fall back to source-shaped capture: {event:?}");
                };
                (
                    work.publish_sequence().get(),
                    work.bidi_channel_owner_action_kind(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            works,
            vec![
                (1, Some(BidiChannelOwnerActionKind::StartListener)),
                (2, Some(BidiChannelOwnerActionKind::ReleaseObjectGroup)),
            ],
            "concrete actions must retain publication order instead of regrouping releases"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn document_node_snapshot_for_backend_node_id_reads_live_renderer_snapshot() {
        let mut ctx = TestContext::new();
        let mut browser_context = BrowserContext::new("BID-runtime-node-snapshot".to_owned());
        browser_context.set_active_target_id("TID-runtime-node-snapshot".to_owned());
        ctx.conn.browser_context = Some(browser_context);
        ctx.install_navigation_fixture_for_session_owner(
            "data:text/html,<html><body><article id='target'>live</article></body></html>",
            None,
        )
        .await;
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 1,
            "method": "Runtime.evaluate",
            "params": { "expression": "document.querySelector('#target')" }
        }))
        .await;
        let evaluated = ctx.take_response_by_id(1);
        let object_id = evaluated["result"]["result"]["objectId"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                panic!("Runtime.evaluate should return target objectId: {evaluated}")
            });

        ctx.process_async(json!({
            "id": 2,
            "method": "DOM.describeNode",
            "params": { "objectId": object_id, "depth": 0 }
        }))
        .await;
        let described = ctx.take_response_by_id(2);
        let backend_node_id = described["result"]["node"]["backendNodeId"]
            .as_u64()
            .and_then(|node_id| u32::try_from(node_id).ok())
            .expect("DOM.describeNode should return backendNodeId");

        let owner = CommandOwnerScope::capture(&ctx.conn, None);
        let snapshot = ctx
            .conn
            .document_node_snapshot_for_backend_node_id_for_owner_async(
                &owner,
                backend_node_id,
                1,
                false,
            )
            .await
            .expect("document node id snapshot command should complete")
            .expect("target node snapshot should exist");
        assert_eq!(snapshot.snapshot.local_name, "article");
        assert!(
            snapshot
                .snapshot
                .attributes
                .iter()
                .any(|attribute| attribute.local_name == "id" && attribute.value == "target"),
            "snapshot should preserve target id attribute: {snapshot:?}"
        );
        let backend_node_id = snapshot
            .snapshot
            .backend_node_id
            .expect("renderer snapshot should assign backendNodeId");
        assert!(
            is_renderer_backend_node_id(backend_node_id),
            "node-id snapshot helper should return renderer backend id namespace: {snapshot:?}"
        );
        assert!(
            snapshot
                .snapshot
                .children
                .iter()
                .any(|child| child.node_value == "live"),
            "depth=1 snapshot should include text child: {snapshot:?}"
        );
    }

    #[test]
    fn runtime_realm_inventory_conversion_keeps_context_without_native_realm_id() {
        let event = runtime_realm_info_to_execution_context_event(
            RendererRuntimeRealmInfo {
                context_id: 7,
                realm_id: None,
                frame_id: Some("FRAME-child".to_owned()),
                origin: "https://example.test".to_owned(),
                name: String::new(),
                is_default: true,
                context_type: "default".to_owned(),
                grant_universal_access: None,
            },
            Some("FRAME-owner"),
            None,
        )
        .expect("Script.getRealms must not fail when DevTools attaches after context creation");
        assert_eq!(event.context_id, Some(7));
        assert_eq!(
            event.frame_id.as_ref().map(|frame_id| frame_id.as_str()),
            Some("FRAME-child")
        );
        assert_eq!(
            event.realm_id, None,
            "protocol should not synthesize a realm id when renderer did not capture V8 uniqueId"
        );
    }

    #[test]
    fn runtime_realm_inventory_conversion_uses_owner_frame_when_renderer_frame_is_missing() {
        let event = runtime_realm_info_to_execution_context_event(
            RendererRuntimeRealmInfo {
                context_id: 9,
                realm_id: Some("native-realm-9".to_owned()),
                frame_id: None,
                origin: "https://example.test".to_owned(),
                name: "https://example.test/page".to_owned(),
                is_default: true,
                context_type: "default".to_owned(),
                grant_universal_access: None,
            },
            Some("FRAME-owner"),
            Some(DevToolsTargetId::from("TARGET-1")),
        )
        .expect("native renderer realm ids should convert");
        assert_eq!(
            event.realm_id.as_ref().map(|realm_id| realm_id.as_str()),
            Some("TARGET-1:native-realm-9"),
            "external realm ids must include the target owner because native V8 uniqueIds are only unique within a renderer runtime"
        );
        assert_eq!(
            event.frame_id.as_ref().map(|frame_id| frame_id.as_str()),
            Some("FRAME-owner"),
            "owner frame is used only when renderer realm inventory has no per-realm frame id"
        );
        assert_eq!(
            event.target_id.as_ref().map(|target_id| target_id.as_str()),
            Some("TARGET-1")
        );
    }

    #[test]
    fn route_inspector_notifications_strip_stale_session_id_without_current_session() {
        let mut conn = CdpConnection::default();
        let mut response_events = Vec::new();
        let mut background_events = Vec::new();

        let current_seen = conn.route_inspector_messages_into(
            vec![json!({
                "method": "Runtime.executionContextCreated",
                "sessionId": "STALE",
                "params": {
                    "context": {
                        "id": 7,
                        "origin": "https://example.test",
                        "name": "",
                        "uniqueId": "realm-7",
                        "auxData": {
                            "isDefault": true,
                            "type": "default",
                            "frameId": "FRAME-1"
                        }
                    }
                }
            })],
            None,
            None,
            &mut response_events,
            &mut background_events,
        );

        assert!(!current_seen);
        assert!(
            response_events.is_empty(),
            "inspector notifications must not be routed through command response output"
        );
        assert_eq!(background_events.len(), 1);
        assert!(
            background_events[0].protocol_message().is_none(),
            "runtime context notification should remain typed until wire projection"
        );
        let (message, automation_event) = background_events[0].clone().into_parts();
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::RuntimeExecutionContextCreated(_))
        ));
        assert!(
            message.get("sessionId").is_none(),
            "notifications routed without a current session must not leak a stale sessionId"
        );
        assert_eq!(message["method"], json!("Runtime.executionContextCreated"));
        assert_eq!(message["params"]["context"]["id"], json!(7));
        assert_eq!(message["params"]["context"]["uniqueId"], json!("realm-7"));
    }

    #[test]
    fn route_inspector_runtime_context_notifications_use_current_session() {
        let mut conn = CdpConnection::default();
        let mut response_events = Vec::new();
        let mut background_events = Vec::new();

        let current_seen = conn.route_inspector_messages_into(
            vec![
                json!({
                    "method": "Runtime.executionContextDestroyed",
                    "sessionId": "STALE",
                    "params": {
                        "executionContextId": 7,
                        "executionContextUniqueId": "realm-7"
                    }
                }),
                json!({
                    "method": "Runtime.executionContextsCleared",
                    "sessionId": "STALE",
                    "params": {}
                }),
            ],
            None,
            Some("SID-1"),
            &mut response_events,
            &mut background_events,
        );

        assert!(!current_seen);
        assert!(
            response_events.is_empty(),
            "inspector notifications must not be routed through command response output"
        );
        assert_eq!(background_events.len(), 2);
        assert!(
            background_events[0].protocol_message().is_none(),
            "destroyed notification should remain typed until wire projection"
        );
        assert!(
            background_events[1].protocol_message().is_none(),
            "cleared notification should remain typed until wire projection"
        );
        let (destroyed, destroyed_automation_event) = background_events[0].clone().into_parts();
        let (cleared, cleared_automation_event) = background_events[1].clone().into_parts();
        assert!(matches!(
            destroyed_automation_event,
            Some(AutomationEvent::RuntimeExecutionContextDestroyed(_))
        ));
        assert!(matches!(
            cleared_automation_event,
            Some(AutomationEvent::RuntimeExecutionContextsCleared(_))
        ));
        assert_eq!(destroyed["sessionId"], json!("SID-1"));
        assert_eq!(
            destroyed["method"],
            json!("Runtime.executionContextDestroyed")
        );
        assert_eq!(destroyed["params"]["executionContextId"], json!(7));
        assert_eq!(
            destroyed["params"]["executionContextUniqueId"],
            json!("realm-7")
        );
        assert_eq!(cleared["sessionId"], json!("SID-1"));
        assert_eq!(cleared["method"], json!("Runtime.executionContextsCleared"));
        assert_eq!(cleared["params"], json!({}));
    }

    #[test]
    fn pending_inspector_await_registry_scopes_entries_to_devtools_session() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-owner".to_owned());
        browser_context.set_active_target_id("TID-active".to_owned());
        browser_context.attach_active_session("SID-active".to_owned());
        browser_context.insert_page_target_host(PageTargetHost::with_url(
            "TID-bg".to_owned(),
            Some("SID-bg".to_owned()),
            "about:blank#bg".to_owned(),
        ));
        conn.browser_context = Some(browser_context);

        conn.register_pending_inspector_await(1, Some("SID-active"));
        conn.register_pending_inspector_await(2, Some("SID-bg"));

        {
            let browser_context = conn.browser_context.as_ref().expect("browser context");
            assert!(
                browser_context.active_page_target().devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .has_pending_inspector_awaits(),
                "active DevTools session should physically store its pending await"
            );
            assert!(
                browser_context
                    .background_target("TID-bg")
                    .filter(|target| target.has_non_default_session_state())
                    .is_some_and(|state| state.devtools_sessions
                        [moli_page_types::DevToolsSessionKey::Primary]
                        .has_pending_inspector_awaits()),
                "parked DevTools session should physically store its pending await"
            );
        }

        assert!(conn.has_pending_inspector_awaits_for_session_owner(Some("SID-active")));
        assert!(conn.has_pending_inspector_awaits_for_session_owner(Some("SID-bg")));

        let mut direct_events = Vec::new();
        let mut claimed_events = Vec::new();
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            &mut direct_events,
            &mut claimed_events,
            Some("SID-active"),
            "Page closed",
        );
        assert!(claimed_events.is_empty());
        assert_eq!(direct_events.len(), 1);
        let (message, automation_event) = direct_events.remove(0).into_parts();
        assert!(automation_event.is_none());
        assert_eq!(message["id"], json!(1));
        assert_eq!(message["sessionId"], json!("SID-active"));

        assert!(!conn.has_pending_inspector_awaits_for_session_owner(Some("SID-active")));
        assert!(conn.has_pending_inspector_awaits_for_session_owner(Some("SID-bg")));

        let mut response_events = Vec::new();
        let mut background_events = Vec::new();
        let seen = conn.route_inspector_messages_into(
            vec![json!({
                "id": 2,
                "result": { "result": { "type": "string", "value": "bg" } }
            })],
            None,
            Some("SID-bg"),
            &mut response_events,
            &mut background_events,
        );
        assert!(background_events.is_empty());
        assert!(!seen);
        assert_eq!(response_events.len(), 1);
        let message = response_events[0]
            .protocol_message()
            .expect("owner routed response should carry protocol message");
        assert_eq!(message["id"], json!(2));
        assert_eq!(message["sessionId"], json!("SID-bg"));
        assert!(!conn.has_pending_inspector_awaits());
    }

    #[test]
    fn same_pending_inspector_await_id_is_isolated_by_devtools_session() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-same-id".to_owned());
        browser_context.set_active_target_id("TID-active".to_owned());
        browser_context.attach_active_session("SID-active".to_owned());
        browser_context.insert_page_target_host(PageTargetHost::with_url(
            "TID-bg".to_owned(),
            Some("SID-bg".to_owned()),
            "about:blank#bg".to_owned(),
        ));
        conn.browser_context = Some(browser_context);

        conn.register_pending_inspector_await(1, Some("SID-active"));
        conn.register_pending_inspector_await(1, Some("SID-bg"));
        conn.register_runtime_await_job_for_owner(
            1,
            &CommandOwnerScope::for_session("SID-active"),
            None,
            "evaluate",
        );
        conn.register_runtime_await_job_for_owner(
            1,
            &CommandOwnerScope::for_session("SID-bg"),
            None,
            "evaluate",
        );

        assert_eq!(conn.pending_runtime_await_jobs.len(), 2);
        let claimed = conn
            .claim_pending_inspector_await_for_scheduler_deferred_reply(
                1,
                &CommandOwnerScope::for_session("SID-active"),
            )
            .expect("active session await should be independently claimable");
        assert!(conn.has_claimed_pending_inspector_awaits_for_session_owner(Some("SID-active")));
        assert!(!conn.has_unclaimed_pending_inspector_awaits_for_session_owner(Some("SID-active")));
        assert!(conn.has_unclaimed_pending_inspector_awaits_for_session_owner(Some("SID-bg")));

        conn.cancel_claimed_pending_inspector_await_for_scheduler_deferred_reply(
            Some(claimed),
            "forgotten",
        );
        assert!(!conn.has_pending_inspector_awaits_for_session_owner(Some("SID-active")));
        assert!(conn.has_pending_inspector_awaits_for_session_owner(Some("SID-bg")));
        assert_eq!(conn.pending_runtime_await_jobs.len(), 1);
        assert_eq!(
            conn.runtime_await_job_trace_fields(1, Some("SID-bg"))["sessionId"],
            json!("SID-bg")
        );

        let mut response_events = Vec::new();
        let mut background_events = Vec::new();
        let stale_session_seen = conn.route_inspector_messages_into(
            vec![json!({
                "id": 1,
                "result": { "result": { "type": "string", "value": "stale" } }
            })],
            None,
            Some("SID-active"),
            &mut response_events,
            &mut background_events,
        );
        assert!(!stale_session_seen);
        assert!(response_events.is_empty());
        assert!(background_events.is_empty());
        assert!(conn.has_pending_inspector_awaits_for_session_owner(Some("SID-bg")));
        assert_eq!(conn.pending_runtime_await_jobs.len(), 1);

        let background_seen = conn.route_inspector_messages_into(
            vec![json!({
                "id": 1,
                "result": { "result": { "type": "string", "value": "bg" } }
            })],
            Some(1),
            Some("SID-bg"),
            &mut response_events,
            &mut background_events,
        );
        assert!(background_seen);
        assert!(background_events.is_empty());
        assert_eq!(response_events.len(), 1);
        let message = response_events[0]
            .protocol_message()
            .expect("background response should remain a protocol response");
        assert_eq!(message["id"], json!(1));
        assert_eq!(message["sessionId"], json!("SID-bg"));
        assert!(!conn.has_pending_inspector_awaits());
        assert!(conn.pending_runtime_await_jobs.is_empty());
    }

    #[test]
    fn failed_await_registration_does_not_discard_existing_renderer_owner() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-duplicate-owner".to_owned());
        browser_context.set_active_target_id("TID-duplicate-owner".to_owned());
        browser_context.attach_active_session("SID-duplicate-owner".to_owned());
        conn.browser_context = Some(browser_context);

        let original = conn
            .try_register_renderer_call_for_session_owner(
                Some("SID-duplicate-owner"),
                17,
                None,
                renderer_command_descriptor_for_test(17),
            )
            .expect("first renderer command should own the frontend id")
            .correlation();
        conn.try_register_pending_inspector_await_with_object_group_for_owner(
            17,
            &CommandOwnerScope::for_session("SID-duplicate-owner"),
            None,
        )
        .expect("await state is registered before renderer dispatch");
        assert_eq!(
            conn.try_register_renderer_call_for_session_owner(
                Some("SID-duplicate-owner"),
                17,
                None,
                renderer_command_descriptor_for_test(17),
            )
            .unwrap_err(),
            "Duplicate `id` in protocol request"
        );

        conn.forget_pending_inspector_await(17, Some("SID-duplicate-owner"));

        assert_eq!(
            conn.take_renderer_call_for_frontend_for_session_owner(
                Some("SID-duplicate-owner"),
                17,
            ),
            Some(original),
            "failed await dispatch must not consume the older command's correlation"
        );
    }

    #[test]
    fn bidi_listener_cancellation_discards_correlation_registered_first() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-listener-cancel".to_owned());
        browser_context.set_active_target_id("TID-listener-cancel".to_owned());
        browser_context.attach_active_session("SID-listener-cancel".to_owned());
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        conn.browser_context = Some(browser_context);

        conn.try_register_renderer_call_for_session_owner(
            Some("SID-listener-cancel"),
            23,
            Some(RendererAgentAttachmentId::allocate()),
            renderer_command_descriptor_for_test(23),
        )
        .expect("listener renderer command should register before listener ownership");
        let listener =
            bidi_channel_listener_residence_for_test(&conn, "SID-listener-cancel", "cancel");
        conn.register_pending_bidi_channel_listener(23, Some("SID-listener-cancel"), listener);

        let mut direct_events = Vec::new();
        let mut claimed_events = Vec::new();
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            &mut direct_events,
            &mut claimed_events,
            Some("SID-listener-cancel"),
            "Page navigated",
        );

        assert!(direct_events.is_empty());
        assert!(claimed_events.is_empty());
        assert!(
            conn.try_register_renderer_call_for_session_owner(
                Some("SID-listener-cancel"),
                23,
                None,
                renderer_command_descriptor_for_test(23),
            )
            .is_ok(),
            "listener cancellation must release the frontend command id"
        );
    }

    #[test]
    fn non_await_cancellation_releases_frontend_command_id() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-command-cancel".to_owned());
        browser_context.set_active_target_id("TID-command-cancel".to_owned());
        browser_context.attach_active_session("SID-command-cancel".to_owned());
        conn.browser_context = Some(browser_context);

        conn.try_register_renderer_call_for_session_owner(
            Some("SID-command-cancel"),
            29,
            None,
            renderer_command_descriptor_for_test(29),
        )
        .expect("non-await command should register a renderer correlation");

        conn.forget_pending_inspector_await(29, Some("SID-command-cancel"));

        assert!(
            conn.try_register_renderer_call_for_session_owner(
                Some("SID-command-cancel"),
                29,
                None,
                renderer_command_descriptor_for_test(29),
            )
            .is_ok(),
            "cancelled non-await command must release the frontend command id"
        );
    }

    #[tokio::test]
    async fn terminal_session_cleanup_completes_non_await_once_and_releases_frontend_id() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-terminal".to_owned());
        browser_context.set_active_target_id("TID-terminal".to_owned());
        browser_context.attach_active_session("SID-terminal".to_owned());
        conn.browser_context = Some(browser_context);

        let attachment = RendererAgentAttachmentId::allocate();
        let prepared = conn
            .try_register_renderer_call_for_session_owner(
                Some("SID-terminal"),
                31,
                Some(attachment),
                RendererCommandDescriptor::from_synthesized_payload(
                    json!({
                        "id": 31,
                        "method": "Console.clearMessages",
                        "params": {},
                    })
                    .to_string(),
                )
                .unwrap(),
            )
            .expect("non-await command should register");
        let (correlation, old_sender, response_receiver) = prepared.into_parts();
        let response_receiver = response_receiver
            .expect("a synthesized CommandReply call must allocate a response receiver");

        let mut direct_events = Vec::new();
        let mut claimed_events = Vec::new();
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            &mut direct_events,
            &mut claimed_events,
            Some("SID-terminal"),
            "Inspector detached",
        );

        assert!(claimed_events.is_empty());
        assert_eq!(direct_events.len(), 1);
        let (message, automation_event) = direct_events.remove(0).into_parts();
        assert!(automation_event.is_none());
        assert_eq!(message["id"], json!(31));
        assert_eq!(message["sessionId"], json!("SID-terminal"));
        assert_eq!(message["error"]["code"], json!(-32000));
        assert_eq!(message["error"]["message"], json!("Inspector detached"));
        assert!(
            old_sender
                .send(json!({
                    "id": correlation.renderer_call_id().get(),
                    "result": {},
                }))
                .is_err(),
            "terminal transition must invalidate the renderer's old response lease"
        );

        let completion = response_receiver
            .await
            .expect("terminal transition should complete the shared receiver");
        assert_eq!(completion.renderer_agent_attachment_id(), None);
        let terminal = completion
            .output
            .protocol_response(completion.call_id)
            .expect("terminal response payload");
        assert_eq!(terminal["error"]["code"], json!(-32000));
        assert_eq!(terminal["error"]["message"], json!("Inspector detached"));
        assert!(
            conn.try_register_renderer_call_for_session_owner(
                Some("SID-terminal"),
                31,
                None,
                renderer_command_descriptor_for_test(31),
            )
            .is_ok(),
            "terminal cleanup must release the frontend command id"
        );
    }

    #[test]
    fn pending_inspector_await_response_routes_through_owner_runtime_response() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-owner-output".to_owned());
        browser_context.set_active_target_id("TID-active".to_owned());
        browser_context.attach_active_session("SID-active".to_owned());
        browser_context.insert_page_target_host(PageTargetHost::with_url(
            "TID-bg".to_owned(),
            Some("SID-bg".to_owned()),
            "about:blank#bg".to_owned(),
        ));
        conn.browser_context = Some(browser_context);

        conn.try_register_pending_inspector_await_with_object_group_for_owner(
            77,
            &CommandOwnerScope::for_session("SID-bg"),
            Some("runtime-group"),
        )
        .unwrap();
        conn.register_runtime_await_job_for_owner(
            77,
            &CommandOwnerScope::for_session("SID-bg"),
            Some("runtime-group"),
            "evaluate",
        );

        let mut response_events = Vec::new();
        let mut background_events = Vec::new();
        let seen = conn.route_inspector_messages_with_background_events_into(
            vec![json!({
                "id": 77,
                "result": {
                    "result": {
                        "type": "object",
                        "objectId": "object-bg-1"
                    }
                }
            })],
            Some(77),
            Some("SID-bg"),
            &mut response_events,
            &mut background_events,
        );

        assert!(
            seen,
            "matching owner runtime response should complete the current command"
        );
        assert!(
            background_events.is_empty(),
            "plain Runtime.evaluate response should not become a side event"
        );
        assert_eq!(response_events.len(), 1);
        let message = response_events[0]
            .protocol_message()
            .expect("owner routed response should carry protocol message");
        assert_eq!(message["id"], json!(77));
        assert_eq!(message["sessionId"], json!("SID-bg"));
        assert!(
            !conn.has_pending_inspector_awaits(),
            "owner runtime response should consume pending inspector await state"
        );
        assert_eq!(
            conn.runtime_remote_object_group_for_session_owner(Some("SID-bg"), "object-bg-1"),
            Some("runtime-group".to_owned()),
            "owner runtime response should register handles against the producing owner"
        );
    }

    #[tokio::test]
    async fn failing_pending_awaits_retire_concrete_listener_work_without_client_error() {
        let mut conn = connection_with_bidi_page_session();

        conn.register_pending_inspector_await(1, Some("SID-active"));
        conn.register_runtime_remote_object_ids_for_session_owner_with_group(
            Some("SID-active"),
            vec!["channel-proxy".to_owned()],
            "webdriver-bidi-channel-test",
        );
        let listener = bidi_channel_listener_residence_for_test(&conn, "SID-active", "test");
        conn.register_pending_bidi_channel_listener(2, Some("SID-active"), listener.clone());
        conn.publish_bidi_channel_listener_start(listener);
        let held_listener_work = take_only_protocol_work(&mut conn);

        conn.runtime_session_owner_slot_mut(Some("SID-active"))
            .expect("test runtime slot")
            .replace_page_attachment_id_for_test();

        let mut direct_events = Vec::new();
        let mut claimed_events = Vec::new();
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            &mut direct_events,
            &mut claimed_events,
            Some("SID-active"),
            "Page navigated",
        );

        assert!(claimed_events.is_empty());
        assert_eq!(direct_events.len(), 1);
        let (message, automation_event) = direct_events.remove(0).into_parts();
        assert!(automation_event.is_none());
        assert_eq!(message["id"], json!(1));
        assert_eq!(message["sessionId"], json!("SID-active"));
        assert!(!conn.has_pending_inspector_awaits_for_session_owner(Some("SID-active")));
        assert!(
            conn.runtime_remote_object_group_for_session_owner(Some("SID-active"), "channel-proxy")
                .is_none(),
            "invalidated BiDi listener should remove its channel object group"
        );
        let outcome = conn
            .complete_ready_protocol_scheduler_work_turn(held_listener_work)
            .await;
        assert!(
            outcome.into_parts().0.is_empty(),
            "stale concrete listener work must not produce protocol output"
        );
        assert!(
            !conn.has_pending_inspector_awaits_for_session_owner(Some("SID-active")),
            "held listener work must not enter the replacement Page runtime"
        );

        let mut response_events = Vec::new();
        let mut background_events = Vec::new();
        let seen = conn.route_inspector_messages_with_background_events_into(
            vec![json!({
                "id": 2,
                "result": { "result": { "type": "string", "value": "late" } }
            })],
            None,
            Some("SID-active"),
            &mut response_events,
            &mut background_events,
        );
        assert!(!seen);
        assert!(
            response_events.is_empty(),
            "stale listener reply must not surface as a protocol message: {response_events:?}"
        );
        assert!(
            background_events.is_empty(),
            "stale listener reply must not surface as script.message"
        );
    }

    #[tokio::test]
    async fn failed_bidi_listener_reply_publishes_concrete_object_group_release() {
        let mut conn = connection_with_bidi_page_session();
        let listener = bidi_channel_listener_residence_for_test(&conn, "SID-active", "error");
        conn.register_runtime_remote_object_ids_for_session_owner_with_group(
            Some("SID-active"),
            vec!["channel-proxy-error".to_owned()],
            "webdriver-bidi-channel-error",
        );
        conn.register_pending_bidi_channel_listener(7, Some("SID-active"), listener);

        let mut response_events = Vec::new();
        let mut background_events = Vec::new();
        let seen = conn.route_inspector_messages_with_background_events_into(
            vec![json!({
                "id": 7,
                "error": {
                    "code": -32000,
                    "message": "Cannot find context with specified id"
                }
            })],
            None,
            Some("SID-active"),
            &mut response_events,
            &mut background_events,
        );

        assert!(!seen);
        assert!(response_events.is_empty());
        assert!(background_events.is_empty());
        let work = take_only_protocol_work(&mut conn);
        assert_eq!(
            work.bidi_channel_owner_action_kind(),
            Some(BidiChannelOwnerActionKind::ReleaseObjectGroup)
        );
        conn.complete_ready_protocol_scheduler_work_turn(work).await;
        assert!(
            conn.runtime_remote_object_group_for_session_owner(
                Some("SID-active"),
                "channel-proxy-error"
            )
            .is_none(),
            "the concrete release must own and consume the listener's object group"
        );
    }

    #[tokio::test]
    async fn stale_bidi_object_group_release_does_not_mutate_replacement_page_state() {
        let mut conn = connection_with_bidi_page_session();
        let listener =
            bidi_channel_listener_residence_for_test(&conn, "SID-active", "stale-release");
        conn.register_pending_bidi_channel_listener(8, Some("SID-active"), listener);

        let mut response_events = Vec::new();
        let mut background_events = Vec::new();
        conn.route_inspector_messages_with_background_events_into(
            vec![json!({
                "id": 8,
                "error": {
                    "code": -32000,
                    "message": "Cannot find context with specified id"
                }
            })],
            None,
            Some("SID-active"),
            &mut response_events,
            &mut background_events,
        );
        let work = take_only_protocol_work(&mut conn);

        conn.runtime_session_owner_slot_mut(Some("SID-active"))
            .expect("test runtime slot")
            .replace_page_attachment_id_for_test();
        conn.register_runtime_remote_object_ids_for_session_owner_with_group(
            Some("SID-active"),
            vec!["replacement-object".to_owned()],
            "webdriver-bidi-channel-stale-release",
        );

        conn.complete_ready_protocol_scheduler_work_turn(work).await;

        assert_eq!(
            conn.runtime_remote_object_group_for_session_owner(
                Some("SID-active"),
                "replacement-object"
            ),
            Some("webdriver-bidi-channel-stale-release".to_owned()),
            "an old release must not touch an identically named group in the replacement Page"
        );
    }

    #[test]
    fn stale_bidi_listener_reply_does_not_emit_or_restart_on_replacement_page() {
        let mut conn = connection_with_bidi_page_session();
        let listener = bidi_channel_listener_residence_for_test(&conn, "SID-active", "stale-reply");
        conn.register_pending_bidi_channel_listener(9, Some("SID-active"), listener);

        conn.runtime_session_owner_slot_mut(Some("SID-active"))
            .expect("test runtime slot")
            .replace_page_attachment_id_for_test();
        conn.register_runtime_remote_object_ids_for_session_owner_with_group(
            Some("SID-active"),
            vec!["replacement-object".to_owned()],
            "webdriver-bidi-channel-stale-reply",
        );

        let mut response_events = Vec::new();
        let mut background_events = Vec::new();
        let seen = conn.route_inspector_messages_with_background_events_into(
            vec![json!({
                "id": 9,
                "result": {
                    "result": {
                        "type": "string",
                        "value": "late message"
                    }
                }
            })],
            None,
            Some("SID-active"),
            &mut response_events,
            &mut background_events,
        );

        assert!(!seen);
        assert!(
            response_events.is_empty() && background_events.is_empty(),
            "a stale listener reply must not emit script.message: response={response_events:?}, background={background_events:?}"
        );
        assert!(
            conn.take_scheduler_events().is_empty(),
            "a stale listener reply must not restart itself on the replacement Page"
        );
        assert_eq!(
            conn.runtime_remote_object_group_for_session_owner(
                Some("SID-active"),
                "replacement-object"
            ),
            Some("webdriver-bidi-channel-stale-reply".to_owned()),
            "discarding the old reply must not clean up replacement runtime state"
        );
    }

    #[test]
    fn bidi_listener_result_handles_stay_in_user_object_group() {
        let listener = PendingBidiChannelListener::new(
            Some(DevToolsTargetId::from("TID-active")),
            Some(crate::devtools_runtime::DevToolsRealmId::from(
                "realm-active",
            )),
            crate::devtools_runtime::DevToolsRemoteHandleId::from("channel-proxy"),
            "webdriver-bidi-channel-infra".to_owned(),
            crate::devtools_runtime::DevToolsBidiChannelProperties {
                channel: "preload".to_owned(),
                ownership: DevToolsResultOwnership::Root,
                serialization_options: None,
            },
        )
        .expect("test listener should include target and realm");

        let command: Value =
            serde_json::from_str(&bidi_channel_listener_call_function_json(9, &listener))
                .expect("listener command should serialize as JSON");

        assert_eq!(command["params"]["objectId"], json!("channel-proxy"));
        assert_eq!(command["params"]["objectGroup"], json!("webdriver-bidi"));
        assert_ne!(
            command["params"]["objectGroup"],
            json!(listener.channel_object_group()),
            "script.message data handles must not be tied to the channel infra group"
        );
    }

    #[test]
    fn bidi_listener_uses_deep_serialization_additional_parameters() {
        let listener = PendingBidiChannelListener::new(
            Some(DevToolsTargetId::from("TID-active")),
            Some(crate::devtools_runtime::DevToolsRealmId::from(
                "realm-active",
            )),
            crate::devtools_runtime::DevToolsRemoteHandleId::from("channel-proxy"),
            "webdriver-bidi-channel-infra".to_owned(),
            crate::devtools_runtime::DevToolsBidiChannelProperties {
                channel: "preload".to_owned(),
                ownership: DevToolsResultOwnership::None,
                serialization_options: Some(
                    crate::devtools_runtime::DevToolsSerializationOptions {
                        max_object_depth: Some(1),
                        max_dom_depth: Some(2),
                        include_shadow_tree: Some("open".to_owned()),
                    },
                ),
            },
        )
        .expect("test listener should include target and realm");

        let command: Value =
            serde_json::from_str(&bidi_channel_listener_call_function_json(9, &listener))
                .expect("listener command should serialize as JSON");

        assert_eq!(
            command["params"]["serializationOptions"]["serialization"],
            json!("deep")
        );
        assert_eq!(
            command["params"]["serializationOptions"]["maxDepth"],
            json!(1)
        );
        assert_eq!(
            command["params"]["serializationOptions"]["additionalParameters"]["maxNodeDepth"],
            json!(2)
        );
        assert_eq!(
            command["params"]["serializationOptions"]["additionalParameters"]["includeShadowTree"],
            json!("open")
        );
    }
}
