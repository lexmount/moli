use std::{collections::hash_map::Entry, future::Future, pin::Pin};

use moli_page_types::{FrontendCommandId, RendererCallId, RendererInspectorResponseDelivery};
use moli_protocol_cdp::CdpRendererCommandReplayDispatch;
use moli_shared_worker::SharedWorkerInstanceId;
use serde_json::{Map, Value, json};

use crate::automation::{
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
    owner: CommandOwnerScope,
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
        owner: &CommandOwnerScope,
        message: Value,
    ) -> Self {
        Self {
            command_id,
            owner: owner.clone(),
            object_group: entry.object_group().map(str::to_owned),
            message,
            bidi_channel_listener: entry.bidi_channel_listener().cloned(),
        }
    }

    fn session_id(&self) -> Option<&str> {
        self.owner.session_id()
    }

    fn owner(&self) -> &CommandOwnerScope {
        &self.owner
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
    // dispatch still owns the adapter-reply waiter.
    AdapterReply(Option<RuntimeInspectorResponseReceiver>),
    SessionSink,
}

impl RuntimeProtocolResponseRoute {
    fn for_registered_delivery(
        delivery: RendererInspectorResponseDelivery,
        receiver: Option<RuntimeInspectorResponseReceiver>,
    ) -> Self {
        match delivery {
            RendererInspectorResponseDelivery::AdapterReply => Self::AdapterReply(Some(
                receiver.expect("a registered adapter-reply route must allocate its receiver"),
            )),
            RendererInspectorResponseDelivery::SessionSink => {
                assert!(
                    receiver.is_none(),
                    "a session-sink response cannot retain an adapter-reply receiver"
                );
                Self::SessionSink
            }
        }
    }

    const fn without_local_receiver_for_delivery(
        delivery: RendererInspectorResponseDelivery,
    ) -> Self {
        match delivery {
            RendererInspectorResponseDelivery::AdapterReply => Self::AdapterReply(None),
            RendererInspectorResponseDelivery::SessionSink => Self::SessionSink,
        }
    }

    const fn adapter_reply_without_receiver() -> Self {
        Self::AdapterReply(None)
    }

    const fn delivery(&self) -> RendererInspectorResponseDelivery {
        match self {
            Self::AdapterReply(_) => RendererInspectorResponseDelivery::AdapterReply,
            Self::SessionSink => RendererInspectorResponseDelivery::SessionSink,
        }
    }

    fn take_adapter_reply_receiver(&mut self) -> Option<RuntimeInspectorResponseReceiver> {
        match self {
            Self::AdapterReply(receiver) => receiver.take(),
            Self::SessionSink => None,
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
    pending_session_response: Option<moli_core::page::PendingWorkerRuntimeInspectorSessionResponse>,
}

impl CompletedWorkerRuntimeProtocolDispatch {
    fn adapter_reply(messages: Vec<RendererRuntimeInspectorMessage>) -> Self {
        Self {
            messages,
            session_response_predecessor: None,
            session_response_succeeded: None,
            pending_session_response: None,
        }
    }

    fn devtools_session(
        completed: moli_core::page::CompletedWorkerRuntimeInspectorCommandDispatch,
    ) -> Self {
        let (messages, pending_session_response) = completed.into_parts();
        Self {
            messages,
            session_response_predecessor: None,
            session_response_succeeded: None,
            pending_session_response: Some(pending_session_response),
        }
    }

    async fn wait_for_session_response(&mut self) -> Result<(), String> {
        let Some(pending) = self.pending_session_response.take() else {
            return Ok(());
        };
        let (predecessor, response_succeeded) = pending.wait().await?;
        self.session_response_predecessor = Some(predecessor);
        self.session_response_succeeded = Some(response_succeeded);
        Ok(())
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

impl CompletedRuntimeEnableEventsDispatch {
    pub(crate) fn renderer_output_predecessor(&self) -> Option<RendererOutputFence> {
        self.completion.renderer_output_predecessor()
    }
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
        self.response_route.take_adapter_reply_receiver()
    }

    pub(crate) const fn response_delivery(&self) -> RendererInspectorResponseDelivery {
        self.response_route.delivery()
    }
}

impl CompletedSharedWorkerRuntimeProtocolMessageDispatch {
    pub(crate) async fn wait_for_session_response(&mut self) -> Result<(), String> {
        self.dispatch.wait_for_session_response().await
    }

    pub(crate) fn take_deferred_response_receiver(
        &mut self,
    ) -> Option<RuntimeInspectorResponseReceiver> {
        self.response_route.take_adapter_reply_receiver()
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
    pub(crate) async fn wait_for_session_response(&mut self) -> Result<(), String> {
        self.dispatch.wait_for_session_response().await
    }

    pub(crate) fn take_deferred_response_receiver(
        &mut self,
    ) -> Option<RuntimeInspectorResponseReceiver> {
        self.response_route.take_adapter_reply_receiver()
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

mod execution_contexts;
mod inspector_awaits;
mod inspector_routing;
mod protocol_dispatch;
mod remote_objects;
mod runtime_dispatch;

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
mod tests;
