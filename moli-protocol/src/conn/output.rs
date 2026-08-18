use crate::devtools_runtime::{
    AutomationEvent, BrowserDownloadProgressEvent, BrowserDownloadWillBeginEvent, DevToolsFrameId,
    DevToolsNetworkResourceType, DevToolsRealmId, DevToolsRemoteHandleId, DevToolsTargetInfo,
    DomSetChildNodesEvent, LogEntryEvent, NavigationFrameEvent, NavigationFrameEventKind,
    NavigationLifecycleEvent, PageFileChooserOpenedEvent, PageJavaScriptDialogOpeningEvent,
    PageLifecycleEvent, RuntimeConsoleEvent, RuntimeExecutionContextEvent,
    RuntimeExecutionContextsClearedEvent, SameDocumentNavigationEvent, ScriptExceptionEvent,
    TargetAttachmentEvent, TargetDetachmentEvent, TargetLifecycleEvent, UserPromptClosedEvent,
};
use moli_core::{
    RendererRuntimeInspectorAsyncCompletion,
    page::{
        RendererAgentAttachmentId, RendererDocumentToken, RendererLifecycleEpoch,
        RendererRuntimeCommandOutput, RendererRuntimeInspectorMessage, WebSocketFrameDirection,
        WebSocketFrameOpcode,
    },
};
use moli_page_types::{
    DevToolsSessionKey, FrontendCommandId, InspectorIssueSnapshot, RendererCallId,
};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;

use super::{
    CdpConnection, DevToolsDocumentLifecycleWaitKey,
    state::{BrowserContext, DocumentNavigationToken},
};

mod delivery_route;
use delivery_route::ProtocolDeliveryRoute;

pub type BackgroundEventSender = UnboundedSender<BackgroundProtocolEvent>;

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeInspectorResponseReady {
    frontend_command_id: FrontendCommandId,
    session_key: DevToolsSessionKey,
    expected_renderer_call_id: Option<RendererCallId>,
    response: Result<RendererRuntimeInspectorAsyncCompletion, String>,
}

impl RuntimeInspectorResponseReady {
    pub fn new(
        command_id: u64,
        session_id: Option<&str>,
        response: Result<RendererRuntimeInspectorAsyncCompletion, String>,
    ) -> Self {
        Self {
            frontend_command_id: FrontendCommandId::new(command_id),
            session_key: DevToolsSessionKey::from_wire_session_id(session_id),
            expected_renderer_call_id: None,
            response,
        }
    }

    pub fn command_id(&self) -> u64 {
        self.frontend_command_id.get()
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_key.wire_session_id()
    }

    pub fn error(&self) -> Option<&str> {
        self.response.as_ref().err().map(String::as_str)
    }

    pub(crate) fn renderer_call_id(&self) -> Option<RendererCallId> {
        self.response
            .as_ref()
            .ok()
            .map(|completion| RendererCallId::new(completion.call_id))
    }

    #[cfg(test)]
    pub(crate) fn new_correlated(
        command_id: u64,
        session_id: Option<&str>,
        renderer_call_id: RendererCallId,
        response: Result<RendererRuntimeInspectorAsyncCompletion, String>,
    ) -> Self {
        let mut ready = Self::new(command_id, session_id, response);
        ready.bind_renderer_call_id(renderer_call_id);
        ready
    }

    pub(crate) fn bind_renderer_call_id(&mut self, renderer_call_id: RendererCallId) {
        assert!(
            self.expected_renderer_call_id
                .replace(renderer_call_id)
                .is_none(),
            "runtime Inspector response cannot change renderer correlation"
        );
    }

    pub(crate) fn has_bound_renderer_call_id(&self) -> bool {
        self.expected_renderer_call_id.is_some()
    }

    pub(crate) fn replace_with_error(&mut self, message: impl Into<String>) {
        self.response = Err(message.into());
    }

    pub(crate) fn renderer_agent_attachment_id(&self) -> Option<RendererAgentAttachmentId> {
        self.response
            .as_ref()
            .ok()
            .and_then(RendererRuntimeInspectorAsyncCompletion::renderer_agent_attachment_id)
    }

    pub fn renderer_output_predecessor(&self) -> Option<moli_core::RendererOutputFence> {
        self.response
            .as_ref()
            .ok()
            .and_then(RendererRuntimeInspectorAsyncCompletion::renderer_output_predecessor)
    }

    pub(crate) fn into_renderer_command_output(
        self,
    ) -> (
        u64,
        RendererRuntimeCommandOutput,
        Option<moli_core::RendererOutputFence>,
    ) {
        let command_id = self.frontend_command_id.get();
        let expected_renderer_call_id = self.expected_renderer_call_id;
        let renderer_output_predecessor = self.renderer_output_predecessor();
        let (attachment_id, mut output) = match self.response {
            Ok(completion)
                if expected_renderer_call_id
                    .is_some_and(|call_id| call_id.get() == completion.call_id)
                    && completion
                        .output
                        .protocol_response(completion.call_id)
                        .is_some() =>
            {
                let attachment_id = completion.renderer_agent_attachment_id();
                let mut output = completion.output;
                let rewritten = rewrite_runtime_command_output_response_id(
                    &mut output,
                    completion.call_id,
                    command_id,
                );
                debug_assert!(rewritten);
                (attachment_id, output)
            }
            Ok(completion)
                if expected_renderer_call_id
                    .is_some_and(|call_id| call_id.get() == completion.call_id) =>
            {
                (
                    completion.renderer_agent_attachment_id(),
                    runtime_command_output_from_response_payload(
                        command_id,
                        BackgroundCommandResponsePayload::internal_error(
                            "RuntimeInspectorResponseMissingProtocolResponse".to_owned(),
                        ),
                    ),
                )
            }
            Ok(completion) => {
                let message =
                    runtime_command_output_response_message(&completion.output, completion.call_id)
                        .cloned()
                        .unwrap_or(Value::Null);
                let payload = runtime_inspector_mismatched_completion_payload(
                    command_id,
                    completion.call_id,
                    &message,
                );
                (
                    completion.renderer_agent_attachment_id(),
                    runtime_command_output_from_response_payload(command_id, payload),
                )
            }
            Err(message) => (
                None,
                runtime_command_output_from_response_payload(
                    command_id,
                    BackgroundCommandResponsePayload::internal_error(message),
                ),
            ),
        };
        if let Some(attachment_id) = attachment_id {
            output.bind_renderer_agent_attachment(attachment_id);
        }
        (command_id, output, renderer_output_predecessor)
    }

    pub fn into_protocol_message_for_typed_runtime_route(self) -> Value {
        let command_id = self.frontend_command_id.get();
        command_response_payload_protocol_message(command_id, self.into_command_response_payload())
    }

    pub(crate) fn into_command_response_payload(self) -> BackgroundCommandResponsePayload {
        match self.response {
            Ok(completion)
                if self
                    .expected_renderer_call_id
                    .is_some_and(|call_id| call_id.get() == completion.call_id) =>
            {
                completion
                    .output
                    .into_protocol_response(completion.call_id)
                    .map(BackgroundCommandResponsePayload::from_owned_runtime_inspector_message)
                    .unwrap_or_else(|| {
                        BackgroundCommandResponsePayload::internal_error(
                            "RuntimeInspectorResponseMissingProtocolResponse".to_owned(),
                        )
                    })
            }
            Ok(completion) => {
                let message =
                    runtime_command_output_response_message(&completion.output, completion.call_id)
                        .cloned()
                        .unwrap_or(Value::Null);
                runtime_inspector_mismatched_completion_payload(
                    self.frontend_command_id.get(),
                    completion.call_id,
                    &message,
                )
            }
            Err(message) => BackgroundCommandResponsePayload::internal_error(message),
        }
    }
}

fn rewrite_runtime_command_output_response_id(
    output: &mut RendererRuntimeCommandOutput,
    renderer_call_id: i32,
    frontend_command_id: u64,
) -> bool {
    let (renderer_agent_attachment_id, v8_state_update, mut messages) =
        std::mem::take(output).into_parts();
    let mut rewritten = false;
    for runtime_message in &mut messages {
        let RendererRuntimeInspectorMessage::Protocol(message) = runtime_message else {
            continue;
        };
        if message
            .renderer_call_id()
            .is_some_and(|call_id| call_id.get() == renderer_call_id)
        {
            message.value_mut()["id"] = json!(frontend_command_id);
            rewritten = true;
            break;
        }
    }
    *output = RendererRuntimeCommandOutput::from_parts(
        renderer_agent_attachment_id,
        v8_state_update,
        messages,
    );
    rewritten
}

fn runtime_command_output_response_message(
    output: &RendererRuntimeCommandOutput,
    call_id: i32,
) -> Option<&Value> {
    output.protocol_response(call_id)
}

fn runtime_command_output_from_response_payload(
    command_id: u64,
    payload: BackgroundCommandResponsePayload,
) -> RendererRuntimeCommandOutput {
    RendererRuntimeCommandOutput::from_inspector_message(RendererRuntimeInspectorMessage::protocol(
        command_response_payload_protocol_message(command_id, payload),
    ))
}

fn runtime_inspector_mismatched_completion_payload(
    expected_command_id: u64,
    actual_call_id: i32,
    message: &Value,
) -> BackgroundCommandResponsePayload {
    let payload = BackgroundCommandResponsePayload::from_runtime_inspector_message(message);
    if actual_call_id < 0 && matches!(payload, BackgroundCommandResponsePayload::Error { .. }) {
        return payload;
    }
    BackgroundCommandResponsePayload::internal_error(format!(
        "RuntimeInspectorResponseIdMismatch(expected={}, got={})",
        expected_command_id, actual_call_id
    ))
}

fn command_response_payload_protocol_message(
    command_id: u64,
    payload: BackgroundCommandResponsePayload,
) -> Value {
    match payload {
        BackgroundCommandResponsePayload::Success { result } => {
            build_command_success_response(Some(command_id), result, None)
        }
        BackgroundCommandResponsePayload::Error {
            code,
            message,
            data,
        } => {
            let mut error = json!({ "code": code, "message": message });
            if let Some(data) = data {
                error["data"] = data;
            }
            json!({
                "id": command_id,
                "error": error,
            })
        }
    }
}

pub type RuntimeInspectorAsyncCompletionReceiver =
    tokio::sync::oneshot::Receiver<RendererRuntimeInspectorAsyncCompletion>;
pub type RuntimeInspectorResponseReadySender = UnboundedSender<RuntimeInspectorResponseReady>;

/// One concrete protocol payload plus the exact route that authorizes its
/// eventual delivery.
///
/// The payload is frozen when the producer creates the event. The route is a
/// separate capability: scheduler residence and navigation gating move this
/// whole value, so neither a replacement Document nor a reused subscription
/// can inherit an older payload. This mirrors Chromium's separation between a
/// concrete Inspector notification and the DevTools session queue that owns
/// its delivery.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolDeliveryEnvelope {
    payload: BackgroundProtocolEventPayload,
    route: ProtocolDeliveryRoute,
}

/// Existing protocol producers use this domain name pervasively. It now names
/// the delivery envelope, not a payload enum; route metadata can therefore no
/// longer be represented as a recursive event variant.
pub type BackgroundProtocolEvent = ProtocolDeliveryEnvelope;

macro_rules! define_background_protocol_event_payloads {
    ($( $variant:ident => $constructor:ident($payload:ty $(,)?) ),+ $(,)?) => {
        #[derive(Debug, Clone, PartialEq)]
        enum BackgroundProtocolEventPayload {
            $(
                $variant($payload),
            )+
        }

        impl ProtocolDeliveryEnvelope {
            $(
                fn $constructor(payload: $payload) -> Self {
                    Self::from_payload(BackgroundProtocolEventPayload::$variant(payload))
                }
            )+
        }
    };
}

define_background_protocol_event_payloads!(
    Protocol => wrap_protocol(BackgroundProtocolMessageEvent),
    CommandResponse => wrap_command_response_event(Box<BackgroundCommandResponseEvent>),
    NetworkRequestWillBeSentExtraInfo => wrap_network_request_will_be_sent_extra_info(
        Box<BackgroundNetworkRequestWillBeSentExtraInfoEvent>
    ),
    NetworkResponseReceivedExtraInfo => wrap_network_response_received_extra_info(
        Box<BackgroundNetworkResponseReceivedExtraInfoEvent>
    ),
    NetworkWebSocketCreated => wrap_network_websocket_created(Box<BackgroundNetworkWebSocketCreatedEvent>),
    NetworkWebSocketWillSendHandshakeRequest => wrap_network_websocket_will_send_handshake_request(
        Box<BackgroundNetworkWebSocketWillSendHandshakeRequestEvent>,
    ),
    NetworkWebSocketHandshakeResponseReceived => wrap_network_websocket_handshake_response_received(
        Box<BackgroundNetworkWebSocketHandshakeResponseReceivedEvent>,
    ),
    NetworkWebSocketFrame => wrap_network_websocket_frame(Box<BackgroundNetworkWebSocketFrameEvent>),
    ConsoleMessageAdded => wrap_console_message_added(Box<BackgroundConsoleMessageAddedEvent>),
    LogEntryAdded => wrap_log_entry_added(Box<BackgroundLogEntryAddedEvent>),
    RuntimeConsoleApiCalled => wrap_runtime_console_api_called(Box<BackgroundRuntimeConsoleApiCalledEvent>),
    RuntimeExceptionThrown => wrap_runtime_exception_thrown(Box<BackgroundRuntimeExceptionThrownEvent>),
    RuntimeBindingCalled => wrap_runtime_binding_called(Box<BackgroundRuntimeBindingCalledEvent>),
    RuntimeExecutionContextCreated => wrap_runtime_execution_context_created(Box<BackgroundRuntimeExecutionContextEvent>),
    RuntimeExecutionContextDestroyed => wrap_runtime_execution_context_destroyed(Box<BackgroundRuntimeExecutionContextEvent>),
    RuntimeExecutionContextsCleared => wrap_runtime_execution_contexts_cleared(Box<BackgroundRuntimeExecutionContextsClearedEvent>),
    DomSetChildNodes => wrap_dom_set_child_nodes(Box<BackgroundDomSetChildNodesEvent>),
    PageNavigationFrame => wrap_page_navigation_frame_event(Box<BackgroundPageNavigationFrameEvent>),
    PageDocumentOpened => wrap_page_document_opened_event(Box<BackgroundPageDocumentOpenedEvent>),
    PageDomContentLoaded => wrap_page_dom_content_loaded(Box<BackgroundNavigationLifecycleEvent>),
    PageLoad => wrap_page_load_event(Box<BackgroundNavigationLifecycleEvent>),
    PageLifecycle => wrap_page_lifecycle_event(Box<BackgroundPageLifecycleEvent>),
    PageSameDocumentNavigation => wrap_page_same_document_navigation_event(Box<BackgroundSameDocumentNavigationEvent>),
    PageFrameAttached => wrap_page_frame_attached_event(Box<BackgroundPageFrameAttachedEvent>),
    PageFrameDetached => wrap_page_frame_detached_event(Box<BackgroundPageFrameDetachedEvent>),
    PageFileChooserOpened => wrap_page_file_chooser_opened_event(Box<BackgroundPageFileChooserOpenedEvent>),
    BrowserDownloadWillBegin => wrap_browser_download_will_begin_event(Box<BackgroundBrowserDownloadWillBeginEvent>),
    BrowserDownloadProgress => wrap_browser_download_progress_event(Box<BackgroundBrowserDownloadProgressEvent>),
    PageJavaScriptDialogOpening => wrap_page_javascript_dialog_opening_event(Box<BackgroundPageJavaScriptDialogOpeningEvent>),
    PageJavaScriptDialogClosed => wrap_page_javascript_dialog_closed_event(Box<BackgroundPageJavaScriptDialogClosedEvent>),
    PageScreencastFrame => wrap_page_screencast_frame_event(Box<BackgroundPageScreencastFrameEvent>),
    PageScreencastVisibilityChanged => wrap_page_screencast_visibility_changed_event(Box<BackgroundPageScreencastVisibilityChangedEvent>),
    InspectorTargetCrashed => wrap_inspector_target_crashed_event(Box<BackgroundInspectorTargetCrashedEvent>),
    InspectorTargetReloadedAfterCrash => wrap_inspector_target_reloaded_after_crash_event(Box<BackgroundInspectorTargetReloadedAfterCrashEvent>),
    InspectorDetached => wrap_inspector_detached_event(Box<BackgroundInspectorDetachedEvent>),
    ServiceWorkerRegistrationUpdated => wrap_service_worker_registration_updated_event(Box<BackgroundServiceWorkerRegistrationUpdatedEvent>),
    ServiceWorkerVersionUpdated => wrap_service_worker_version_updated_event(Box<BackgroundServiceWorkerVersionUpdatedEvent>),
    ServiceWorkerErrorReported => wrap_service_worker_error_reported_event(Box<BackgroundServiceWorkerErrorReportedEvent>),
    TargetInfoChanged => wrap_target_info_changed_event(Box<BackgroundTargetInfoChangedEvent>),
    TargetCreated => wrap_target_created_event(Box<BackgroundTargetCreatedEvent>),
    TargetAttached => wrap_target_attached_event(Box<BackgroundTargetAttachedEvent>),
    TargetDetached => wrap_target_detached_event(Box<BackgroundTargetDetachedEvent>),
    TargetDestroyed => wrap_target_destroyed_event(Box<BackgroundTargetDestroyedEvent>),
    TargetCrashed => wrap_target_crashed_event(Box<BackgroundTargetCrashedEvent>),
    TargetReceivedMessageFromTarget => wrap_target_received_message_from_target_event(Box<BackgroundTargetReceivedMessageFromTargetEvent>),
    AutomationOnly => wrap_automation_only_event(Box<AutomationEvent>),
    RuntimeInspectorResponseReady => wrap_runtime_inspector_response_ready_event(Box<RuntimeInspectorResponseReady>),
);

impl BackgroundProtocolEventPayload {
    fn protocol_session_id(&self) -> Option<&str> {
        match self {
            Self::Protocol(event) => event.message.get("sessionId").and_then(Value::as_str),
            Self::CommandResponse(event) => event.session_id.as_deref(),
            Self::NetworkRequestWillBeSentExtraInfo(event) => event.session_id.as_deref(),
            Self::NetworkResponseReceivedExtraInfo(event) => event.session_id.as_deref(),
            Self::NetworkWebSocketCreated(event) => event.session_id.as_deref(),
            Self::NetworkWebSocketWillSendHandshakeRequest(event) => event.session_id.as_deref(),
            Self::NetworkWebSocketHandshakeResponseReceived(event) => event.session_id.as_deref(),
            Self::NetworkWebSocketFrame(event) => event.session_id.as_deref(),
            Self::ConsoleMessageAdded(event) => event.session_id.as_deref(),
            Self::LogEntryAdded(event) => event.session_id.as_deref(),
            Self::RuntimeConsoleApiCalled(event) => event.session_id.as_deref(),
            Self::RuntimeExceptionThrown(event) => event.session_id.as_deref(),
            Self::RuntimeBindingCalled(event) => event.session_id.as_deref(),
            Self::RuntimeExecutionContextCreated(event)
            | Self::RuntimeExecutionContextDestroyed(event) => event.session_id.as_deref(),
            Self::RuntimeExecutionContextsCleared(event) => event.session_id.as_deref(),
            Self::DomSetChildNodes(event) => event.session_id.as_deref(),
            Self::PageNavigationFrame(event) => event.session_id.as_deref(),
            Self::PageDocumentOpened(event) => event.session_id.as_deref(),
            Self::PageDomContentLoaded(event) | Self::PageLoad(event) => {
                event.session_id.as_deref()
            }
            Self::PageLifecycle(event) => event.session_id.as_deref(),
            Self::PageSameDocumentNavigation(event) => event.session_id.as_deref(),
            Self::PageFrameAttached(event) => event.session_id.as_deref(),
            Self::PageFrameDetached(event) => event.session_id.as_deref(),
            Self::PageFileChooserOpened(event) => event.session_id.as_deref(),
            Self::BrowserDownloadWillBegin(event) => event.session_id.as_deref(),
            Self::BrowserDownloadProgress(event) => event.session_id.as_deref(),
            Self::PageJavaScriptDialogOpening(event) => event.session_id.as_deref(),
            Self::PageJavaScriptDialogClosed(event) => event.session_id.as_deref(),
            Self::PageScreencastFrame(event) => event.session_id.as_deref(),
            Self::PageScreencastVisibilityChanged(event) => event.session_id.as_deref(),
            Self::InspectorTargetCrashed(event) => event.session_id.as_deref(),
            Self::InspectorTargetReloadedAfterCrash(event) => event.session_id.as_deref(),
            Self::InspectorDetached(event) => event.session_id.as_deref(),
            Self::ServiceWorkerRegistrationUpdated(event) => event.session_id.as_deref(),
            Self::ServiceWorkerVersionUpdated(event) => event.session_id.as_deref(),
            Self::ServiceWorkerErrorReported(event) => event.session_id.as_deref(),
            Self::TargetInfoChanged(event) => event.session_id.as_deref(),
            Self::TargetCreated(event) => event.session_id.as_deref(),
            Self::TargetAttached(event) => event
                .event
                .parent_session_id
                .as_ref()
                .map(crate::devtools_runtime::DevToolsSessionId::as_str),
            Self::TargetDetached(event) => event
                .event
                .parent_session_id
                .as_ref()
                .map(crate::devtools_runtime::DevToolsSessionId::as_str),
            Self::TargetDestroyed(event) => event.session_id.as_deref(),
            Self::TargetCrashed(event) => event.session_id.as_deref(),
            Self::TargetReceivedMessageFromTarget(_) | Self::AutomationOnly(_) => None,
            Self::RuntimeInspectorResponseReady(response) => response.session_id(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundCommandResponsePayload {
    Success {
        result: Value,
    },
    Error {
        code: i32,
        message: String,
        data: Option<Value>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum BackgroundCommandResponsePayloadRef<'a> {
    Success {
        result: &'a Value,
    },
    Error {
        code: i32,
        message: &'a str,
        data: Option<&'a Value>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundProtocolMessageEvent {
    message: Value,
    automation_event: Option<Box<AutomationEvent>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundNetworkRequestWillBeSentExtraInfoEvent {
    session_id: Option<String>,
    request_id: String,
    headers: serde_json::Map<String, Value>,
    cookie_access_report: Value,
    associated_cookies: Vec<Value>,
    request_time: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundNetworkResponseReceivedExtraInfoEvent {
    session_id: Option<String>,
    request_id: String,
    headers: serde_json::Map<String, Value>,
    status_code: u16,
    cookie_reports: Vec<Value>,
    blocked_cookies: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundNetworkWebSocketCreatedEvent {
    session_id: Option<String>,
    request_id: String,
    url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundNetworkWebSocketWillSendHandshakeRequestEvent {
    session_id: Option<String>,
    request_id: String,
    timestamp: f64,
    headers: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundNetworkWebSocketHandshakeResponseReceivedEvent {
    session_id: Option<String>,
    request_id: String,
    timestamp: f64,
    status: u16,
    status_text: String,
    headers: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundNetworkWebSocketFrameEvent {
    session_id: Option<String>,
    request_id: String,
    timestamp: f64,
    direction: WebSocketFrameDirection,
    opcode: WebSocketFrameOpcode,
    payload_length: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundCommandResponseEvent {
    command_id: Option<u64>,
    session_id: Option<String>,
    response: BackgroundCommandResponse,
}

#[derive(Debug, Clone, PartialEq)]
enum BackgroundCommandResponse {
    Success {
        result: Value,
    },
    Error {
        code: i32,
        message: String,
        data: Option<Value>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundConsoleMessageAddedEvent {
    session_id: Option<String>,
    source: String,
    level: String,
    text: String,
    url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundLogEntryAddedEvent {
    session_id: Option<String>,
    source: String,
    level: String,
    text: String,
    url: String,
    timestamp: f64,
    network_request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundRuntimeConsoleApiCalledEvent {
    session_id: Option<String>,
    event: RuntimeConsoleEvent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundRuntimeExceptionThrownEvent {
    session_id: Option<String>,
    event: ScriptExceptionEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundRuntimeBindingCalledEvent {
    session_id: Option<String>,
    name: String,
    payload: String,
    execution_context_id: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundRuntimeExecutionContextEvent {
    session_id: Option<String>,
    event: RuntimeExecutionContextEvent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundRuntimeExecutionContextsClearedEvent {
    session_id: Option<String>,
    event: RuntimeExecutionContextsClearedEvent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundDomSetChildNodesEvent {
    session_id: Option<String>,
    parent_node_id: u32,
    nodes: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundPageNavigationFrameEvent {
    session_id: Option<String>,
    event: NavigationFrameEvent,
    unreachable_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundPageDocumentOpenedEvent {
    session_id: Option<String>,
    frame_id: String,
    parent_frame_id: Option<String>,
    loader_id: String,
    url: String,
    frame_name: Option<String>,
    security_origin: String,
    secure_context_type: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundNavigationLifecycleEvent {
    session_id: Option<String>,
    event: NavigationLifecycleEvent,
    renderer_document: Option<RendererDocumentToken>,
    renderer_epoch: Option<RendererLifecycleEpoch>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundPageLifecycleEvent {
    session_id: Option<String>,
    event: PageLifecycleEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundSameDocumentNavigationEvent {
    session_id: Option<String>,
    event: SameDocumentNavigationEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundPageFrameAttachedEvent {
    session_id: Option<String>,
    frame_id: String,
    parent_frame_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundPageFrameDetachedEvent {
    session_id: Option<String>,
    frame_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundPageFileChooserOpenedEvent {
    session_id: Option<String>,
    frame_id: String,
    mode: String,
    backend_node_id: u32,
    element_shared_id: Option<DevToolsRemoteHandleId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundBrowserDownloadWillBeginEvent {
    session_id: Option<String>,
    frame_id: String,
    guid: String,
    url: String,
    suggested_filename: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundBrowserDownloadProgressEvent {
    session_id: Option<String>,
    guid: String,
    state: String,
    received_bytes: u64,
    total_bytes: u64,
    file_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundPageJavaScriptDialogOpeningEvent {
    session_id: Option<String>,
    frame_id: Option<String>,
    url: String,
    message: String,
    dialog_type: String,
    has_browser_handler: bool,
    default_prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundPageJavaScriptDialogClosedEvent {
    session_id: Option<String>,
    target_id: Option<String>,
    frame_id: String,
    prompt_type: String,
    accepted: bool,
    user_text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageScreencastFrameMetadata {
    pub offset_top: f64,
    pub page_scale_factor: f64,
    pub device_width: f64,
    pub device_height: f64,
    pub scroll_offset_x: f64,
    pub scroll_offset_y: f64,
    pub timestamp: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundPageScreencastFrameEvent {
    session_id: Option<String>,
    data: String,
    metadata: PageScreencastFrameMetadata,
    generation: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundPageScreencastVisibilityChangedEvent {
    session_id: Option<String>,
    visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundInspectorTargetCrashedEvent {
    session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundInspectorTargetReloadedAfterCrashEvent {
    session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundInspectorDetachedEvent {
    session_id: Option<String>,
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackgroundServiceWorkerRegistration {
    pub(crate) registration_id: String,
    pub(crate) scope_url: String,
    pub(crate) is_deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackgroundServiceWorkerVersion {
    pub(crate) version_id: String,
    pub(crate) registration_id: String,
    pub(crate) script_url: String,
    pub(crate) running_status: String,
    pub(crate) status: String,
    pub(crate) controlled_clients: Vec<String>,
    pub(crate) target_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackgroundServiceWorkerErrorMessage {
    pub(crate) error_message: String,
    pub(crate) registration_id: String,
    pub(crate) version_id: String,
    pub(crate) source_url: String,
    pub(crate) line_number: u32,
    pub(crate) column_number: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundServiceWorkerRegistrationUpdatedEvent {
    session_id: Option<String>,
    registrations: Vec<BackgroundServiceWorkerRegistration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundServiceWorkerVersionUpdatedEvent {
    session_id: Option<String>,
    versions: Vec<BackgroundServiceWorkerVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundServiceWorkerErrorReportedEvent {
    session_id: Option<String>,
    error_message: BackgroundServiceWorkerErrorMessage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundTargetInfoChangedEvent {
    session_id: Option<String>,
    target_info: DevToolsTargetInfo,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundTargetCreatedEvent {
    session_id: Option<String>,
    event: TargetLifecycleEvent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundTargetAttachedEvent {
    event: TargetAttachmentEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundTargetDetachedEvent {
    event: TargetDetachmentEvent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundTargetDestroyedEvent {
    session_id: Option<String>,
    event: TargetLifecycleEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundTargetCrashedEvent {
    session_id: Option<String>,
    target_id: String,
    status: String,
    error_code: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundTargetReceivedMessageFromTargetEvent {
    target_session_id: String,
    nested_event: Box<BackgroundProtocolEvent>,
}

impl ProtocolDeliveryEnvelope {
    fn from_payload(payload: BackgroundProtocolEventPayload) -> Self {
        let route = ProtocolDeliveryRoute::for_wire_session(payload.protocol_session_id());
        Self { payload, route }
    }

    /// Returns the wire session frozen by construction or concrete fan-out.
    /// `None` means the root session; it never asks the current target state to
    /// infer a destination later.
    #[doc(hidden)]
    pub fn protocol_session_id(&self) -> Option<&str> {
        self.route.wire_session_id()
    }

    pub(crate) fn navigation_gate_target_id(&self) -> Option<&str> {
        if let Some(target_id) = self.route.navigation_gate_target_id() {
            return Some(target_id);
        }
        let BackgroundProtocolEventPayload::Protocol(event) = &self.payload else {
            return None;
        };
        match event.automation_event.as_deref()? {
            AutomationEvent::NetworkBeforeRequestSent(event)
            | AutomationEvent::NetworkResponseStarted(event)
            | AutomationEvent::NetworkResponseCompleted(event)
            | AutomationEvent::NetworkFetchError(event)
            | AutomationEvent::NetworkAuthRequired(event)
            | AutomationEvent::RequestPaused(event) => Some(event.target_id.as_str()),
            AutomationEvent::TargetCreated(event) | AutomationEvent::TargetDestroyed(event) => {
                Some(event.target_id.as_str())
            }
            AutomationEvent::TargetAttached(event) => Some(event.target_id.as_str()),
            AutomationEvent::TargetDetached(event) => Some(event.target_id.as_str()),
            AutomationEvent::NavigationFrame(event) => Some(event.target_id.as_str()),
            AutomationEvent::NavigationStarted(event)
            | AutomationEvent::DomContentLoaded(event)
            | AutomationEvent::Load(event) => Some(event.target_id.as_str()),
            AutomationEvent::PageLifecycle(event) => Some(event.target_id.as_str()),
            AutomationEvent::SameDocumentNavigation(event) => Some(event.target_id.as_str()),
            AutomationEvent::UserPromptClosed(event) => {
                event.target_id.as_ref().map(|target_id| target_id.as_str())
            }
            AutomationEvent::LogEntryAdded(event) => {
                event.target_id.as_ref().map(|target_id| target_id.as_str())
            }
            AutomationEvent::RuntimeConsoleApiCalled(event) => {
                event.target_id.as_ref().map(|target_id| target_id.as_str())
            }
            AutomationEvent::RuntimeExecutionContextCreated(event)
            | AutomationEvent::RuntimeExecutionContextDestroyed(event) => {
                event.target_id.as_ref().map(|target_id| target_id.as_str())
            }
            AutomationEvent::RuntimeExecutionContextsCleared(event) => {
                event.target_id.as_ref().map(|target_id| target_id.as_str())
            }
            AutomationEvent::ScriptMessage(event) => {
                event.target_id.as_ref().map(|target_id| target_id.as_str())
            }
            AutomationEvent::ScriptException(event) => {
                event.target_id.as_ref().map(|target_id| target_id.as_str())
            }
            AutomationEvent::PageJavaScriptDialogOpening(_)
            | AutomationEvent::PageFileChooserOpened(_)
            | AutomationEvent::BrowserDownloadWillBegin(_)
            | AutomationEvent::BrowserDownloadProgress(_)
            | AutomationEvent::DomSetChildNodes(_) => None,
        }
    }

    pub fn route_is_current(&self, conn: &CdpConnection) -> bool {
        self.route.is_current(conn)
    }

    /// Adds the exact Page attachment and root Document that authorize a
    /// child-frame event. The capability lives beside the payload rather than
    /// recursively wrapping it as another event kind.
    pub(crate) fn bind_to_root_document_route(
        mut self,
        conn: &CdpConnection,
        root_document: moli_core::RendererDocumentLifecycleIdentity,
    ) -> Option<Self> {
        let binding = conn.target_root_document_protocol_attachment_identity_for_session(
            self.protocol_session_id(),
            root_document,
        )?;
        self.route.bind_root_document(binding);
        Some(self)
    }

    fn bind_to_page_subscription_route(
        mut self,
        session_id: Option<&str>,
        generation: u64,
    ) -> Self {
        self.route.bind_page_subscription(session_id, generation);
        self
    }

    fn bind_to_browser_download_subscription_route(
        mut self,
        session_id: Option<&str>,
        generation: u64,
    ) -> Self {
        self.route
            .bind_browser_download_subscription(session_id, generation);
        self
    }

    pub fn immediate(message: Value) -> Self {
        Self::wrap_protocol(BackgroundProtocolMessageEvent {
            message,
            automation_event: None,
        })
    }

    pub fn immediate_automation_event(message: Value, automation_event: AutomationEvent) -> Self {
        Self::wrap_protocol(BackgroundProtocolMessageEvent {
            message,
            automation_event: Some(Box::new(automation_event)),
        })
    }

    pub(crate) fn audits_issue_added(
        session_id: Option<&str>,
        issue: &InspectorIssueSnapshot,
        frame_id: &str,
        loader_id: &str,
    ) -> Self {
        Self::immediate(build_event(
            "Audits.issueAdded",
            json!({
                "issue": crate::domains::audits_output_state::inspector_issue_protocol_value(
                    issue,
                    frame_id,
                    loader_id,
                ),
            }),
            session_id,
        ))
    }

    pub(crate) fn page_download_will_begin(
        session_id: Option<&str>,
        subscription_generation: u64,
        frame_id: &str,
        guid: &str,
        url: &str,
        suggested_filename: &str,
    ) -> Self {
        Self::wrap_protocol(BackgroundProtocolMessageEvent {
            message: build_event(
                "Page.downloadWillBegin",
                json!({
                    "frameId": frame_id,
                    "guid": guid,
                    "url": url,
                    "suggestedFilename": suggested_filename,
                }),
                session_id,
            ),
            automation_event: None,
        })
        .bind_to_page_subscription_route(session_id, subscription_generation)
    }

    pub(crate) fn page_download_progress(
        session_id: Option<&str>,
        subscription_generation: u64,
        guid: &str,
        state: &str,
        received_bytes: u64,
        total_bytes: u64,
    ) -> Self {
        Self::wrap_protocol(BackgroundProtocolMessageEvent {
            message: build_event(
                "Page.downloadProgress",
                json!({
                    "guid": guid,
                    "state": state,
                    "receivedBytes": received_bytes,
                    "totalBytes": total_bytes,
                }),
                session_id,
            ),
            automation_event: None,
        })
        .bind_to_page_subscription_route(session_id, subscription_generation)
    }

    pub(crate) fn automation_only(automation_event: AutomationEvent) -> Self {
        Self::wrap_automation_only_event(Box::new(automation_event))
    }

    pub fn runtime_inspector_response_ready(response: RuntimeInspectorResponseReady) -> Self {
        Self::wrap_runtime_inspector_response_ready_event(Box::new(response))
    }

    pub fn command_response(
        command_id: Option<u64>,
        session_id: Option<&str>,
        response: BackgroundCommandResponsePayload,
    ) -> Self {
        match response {
            BackgroundCommandResponsePayload::Success { result } => {
                Self::command_success(command_id, session_id, result)
            }
            BackgroundCommandResponsePayload::Error {
                code,
                message,
                data,
            } => Self::command_error(command_id, session_id, code, message, data),
        }
    }

    pub fn command_success(
        command_id: Option<u64>,
        session_id: Option<&str>,
        result: Value,
    ) -> Self {
        Self::wrap_command_response_event(Box::new(BackgroundCommandResponseEvent {
            command_id,
            session_id: session_id.map(str::to_owned),
            response: BackgroundCommandResponse::Success { result },
        }))
    }

    pub fn command_error(
        command_id: Option<u64>,
        session_id: Option<&str>,
        code: i32,
        message: String,
        data: Option<Value>,
    ) -> Self {
        Self::wrap_command_response_event(Box::new(BackgroundCommandResponseEvent {
            command_id,
            session_id: session_id.map(str::to_owned),
            response: BackgroundCommandResponse::Error {
                code,
                message,
                data,
            },
        }))
    }

    pub(crate) fn command_response_payload_ref(
        &self,
    ) -> Option<(
        Option<u64>,
        Option<&str>,
        BackgroundCommandResponsePayloadRef<'_>,
    )> {
        match &self.payload {
            BackgroundProtocolEventPayload::CommandResponse(event) => Some((
                event.command_id,
                event.session_id.as_deref(),
                event.response.payload_ref(),
            )),
            _ => None,
        }
    }

    pub fn into_command_response_payload(
        self,
    ) -> Result<
        (
            Option<u64>,
            Option<String>,
            BackgroundCommandResponsePayload,
        ),
        Self,
    > {
        let Self { payload, route } = self;
        match payload {
            BackgroundProtocolEventPayload::CommandResponse(event) => Ok((
                event.command_id,
                event.session_id,
                event.response.into_payload(),
            )),
            payload => Err(Self { payload, route }),
        }
    }

    pub(crate) fn network_request_will_be_sent_extra_info(
        session_id: Option<&str>,
        request_id: &str,
        headers: serde_json::Map<String, Value>,
        cookie_access_report: Value,
        associated_cookies: Vec<Value>,
        request_time: f64,
    ) -> Self {
        Self::wrap_network_request_will_be_sent_extra_info(Box::new(
            BackgroundNetworkRequestWillBeSentExtraInfoEvent {
                session_id: session_id.map(str::to_owned),
                request_id: request_id.to_owned(),
                headers,
                cookie_access_report,
                associated_cookies,
                request_time,
            },
        ))
    }

    pub(crate) fn network_response_received_extra_info(
        session_id: Option<&str>,
        request_id: &str,
        headers: serde_json::Map<String, Value>,
        status_code: u16,
        cookie_reports: Vec<Value>,
        blocked_cookies: Vec<Value>,
    ) -> Self {
        Self::wrap_network_response_received_extra_info(Box::new(
            BackgroundNetworkResponseReceivedExtraInfoEvent {
                session_id: session_id.map(str::to_owned),
                request_id: request_id.to_owned(),
                headers,
                status_code,
                cookie_reports,
                blocked_cookies,
            },
        ))
    }

    pub(crate) fn network_websocket_created(
        session_id: Option<&str>,
        request_id: &str,
        url: &str,
    ) -> Self {
        Self::wrap_network_websocket_created(Box::new(BackgroundNetworkWebSocketCreatedEvent {
            session_id: session_id.map(str::to_owned),
            request_id: request_id.to_owned(),
            url: url.to_owned(),
        }))
    }

    pub(crate) fn network_websocket_will_send_handshake_request(
        session_id: Option<&str>,
        request_id: &str,
        timestamp: f64,
        headers: serde_json::Map<String, Value>,
    ) -> Self {
        Self::wrap_network_websocket_will_send_handshake_request(Box::new(
            BackgroundNetworkWebSocketWillSendHandshakeRequestEvent {
                session_id: session_id.map(str::to_owned),
                request_id: request_id.to_owned(),
                timestamp,
                headers,
            },
        ))
    }

    pub(crate) fn network_websocket_handshake_response_received(
        session_id: Option<&str>,
        request_id: &str,
        timestamp: f64,
        status: u16,
        status_text: &str,
        headers: serde_json::Map<String, Value>,
    ) -> Self {
        Self::wrap_network_websocket_handshake_response_received(Box::new(
            BackgroundNetworkWebSocketHandshakeResponseReceivedEvent {
                session_id: session_id.map(str::to_owned),
                request_id: request_id.to_owned(),
                timestamp,
                status,
                status_text: status_text.to_owned(),
                headers,
            },
        ))
    }

    pub(crate) fn network_websocket_frame(
        session_id: Option<&str>,
        request_id: &str,
        timestamp: f64,
        direction: WebSocketFrameDirection,
        opcode: WebSocketFrameOpcode,
        payload_length: usize,
    ) -> Self {
        Self::wrap_network_websocket_frame(Box::new(BackgroundNetworkWebSocketFrameEvent {
            session_id: session_id.map(str::to_owned),
            request_id: request_id.to_owned(),
            timestamp,
            direction,
            opcode,
            payload_length,
        }))
    }

    pub(crate) fn console_message_added(
        session_id: Option<&str>,
        source: &str,
        level: &str,
        text: &str,
        url: &str,
    ) -> Self {
        Self::wrap_console_message_added(Box::new(BackgroundConsoleMessageAddedEvent {
            session_id: session_id.map(str::to_owned),
            source: source.to_owned(),
            level: level.to_owned(),
            text: text.to_owned(),
            url: url.to_owned(),
        }))
    }

    pub(crate) fn log_entry_added(
        session_id: Option<&str>,
        source: &str,
        level: &str,
        text: &str,
        url: &str,
        timestamp: f64,
        network_request_id: Option<&str>,
    ) -> Self {
        Self::wrap_log_entry_added(Box::new(BackgroundLogEntryAddedEvent {
            session_id: session_id.map(str::to_owned),
            source: source.to_owned(),
            level: level.to_owned(),
            text: text.to_owned(),
            url: url.to_owned(),
            timestamp,
            network_request_id: network_request_id.map(str::to_owned),
        }))
    }

    pub(crate) fn runtime_console_api_called(
        session_id: Option<&str>,
        event: RuntimeConsoleEvent,
    ) -> Self {
        Self::wrap_runtime_console_api_called(Box::new(BackgroundRuntimeConsoleApiCalledEvent {
            session_id: session_id.map(str::to_owned),
            event,
        }))
    }

    pub(crate) fn runtime_exception_thrown(
        session_id: Option<&str>,
        event: ScriptExceptionEvent,
    ) -> Self {
        Self::wrap_runtime_exception_thrown(Box::new(BackgroundRuntimeExceptionThrownEvent {
            session_id: session_id.map(str::to_owned),
            event,
        }))
    }

    pub(crate) fn runtime_binding_called(
        session_id: Option<&str>,
        name: impl Into<String>,
        payload: impl Into<String>,
        execution_context_id: i64,
    ) -> Self {
        Self::wrap_runtime_binding_called(Box::new(BackgroundRuntimeBindingCalledEvent {
            session_id: session_id.map(str::to_owned),
            name: name.into(),
            payload: payload.into(),
            execution_context_id,
        }))
    }

    pub(crate) fn runtime_execution_context_created(
        session_id: Option<&str>,
        event: RuntimeExecutionContextEvent,
    ) -> Self {
        Self::wrap_runtime_execution_context_created(Box::new(
            BackgroundRuntimeExecutionContextEvent {
                session_id: session_id.map(str::to_owned),
                event,
            },
        ))
    }

    pub(crate) fn runtime_execution_context_destroyed(
        session_id: Option<&str>,
        event: RuntimeExecutionContextEvent,
    ) -> Self {
        Self::wrap_runtime_execution_context_destroyed(Box::new(
            BackgroundRuntimeExecutionContextEvent {
                session_id: session_id.map(str::to_owned),
                event,
            },
        ))
    }

    pub(crate) fn runtime_execution_contexts_cleared(
        session_id: Option<&str>,
        event: RuntimeExecutionContextsClearedEvent,
    ) -> Self {
        Self::wrap_runtime_execution_contexts_cleared(Box::new(
            BackgroundRuntimeExecutionContextsClearedEvent {
                session_id: session_id.map(str::to_owned),
                event,
            },
        ))
    }

    pub(crate) fn dom_set_child_nodes(
        session_id: Option<&str>,
        parent_node_id: u32,
        nodes: Vec<Value>,
    ) -> Self {
        Self::wrap_dom_set_child_nodes(Box::new(BackgroundDomSetChildNodesEvent {
            session_id: session_id.map(str::to_owned),
            parent_node_id,
            nodes,
        }))
    }

    pub(crate) fn dom_attribute_modified(
        session_id: Option<&str>,
        node_id: u32,
        name: &str,
        value: &str,
    ) -> Self {
        Self::immediate(build_event(
            "DOM.attributeModified",
            json!({
                "nodeId": node_id,
                "name": name,
                "value": value,
            }),
            session_id,
        ))
    }

    pub(crate) fn dom_attribute_removed(
        session_id: Option<&str>,
        node_id: u32,
        name: &str,
    ) -> Self {
        Self::immediate(build_event(
            "DOM.attributeRemoved",
            json!({
                "nodeId": node_id,
                "name": name,
            }),
            session_id,
        ))
    }

    pub(crate) fn dom_character_data_modified(
        session_id: Option<&str>,
        node_id: u32,
        character_data: &str,
    ) -> Self {
        Self::immediate(build_event(
            "DOM.characterDataModified",
            json!({
                "nodeId": node_id,
                "characterData": character_data,
            }),
            session_id,
        ))
    }

    pub(crate) fn dom_child_node_count_updated(
        session_id: Option<&str>,
        node_id: u32,
        child_node_count: usize,
    ) -> Self {
        Self::immediate(build_event(
            "DOM.childNodeCountUpdated",
            json!({
                "nodeId": node_id,
                "childNodeCount": child_node_count,
            }),
            session_id,
        ))
    }

    pub(crate) fn dom_child_node_inserted(
        session_id: Option<&str>,
        parent_node_id: u32,
        previous_node_id: u32,
        node: Value,
    ) -> Self {
        Self::immediate(build_event(
            "DOM.childNodeInserted",
            json!({
                "parentNodeId": parent_node_id,
                "previousNodeId": previous_node_id,
                "node": node,
            }),
            session_id,
        ))
    }

    pub(crate) fn dom_child_node_removed(
        session_id: Option<&str>,
        parent_node_id: u32,
        node_id: u32,
    ) -> Self {
        Self::immediate(build_event(
            "DOM.childNodeRemoved",
            json!({
                "parentNodeId": parent_node_id,
                "nodeId": node_id,
            }),
            session_id,
        ))
    }

    pub(crate) fn page_navigation_frame(
        session_id: Option<&str>,
        event: NavigationFrameEvent,
    ) -> Self {
        Self::wrap_page_navigation_frame_event(Box::new(BackgroundPageNavigationFrameEvent {
            session_id: session_id.map(str::to_owned),
            event,
            unreachable_url: None,
        }))
    }

    pub(crate) fn page_navigation_frame_with_unreachable_url(
        session_id: Option<&str>,
        event: NavigationFrameEvent,
        unreachable_url: impl Into<String>,
    ) -> Self {
        Self::wrap_page_navigation_frame_event(Box::new(BackgroundPageNavigationFrameEvent {
            session_id: session_id.map(str::to_owned),
            event,
            unreachable_url: Some(unreachable_url.into()),
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn page_document_opened(
        session_id: Option<&str>,
        frame_id: impl Into<String>,
        parent_frame_id: Option<String>,
        loader_id: impl Into<String>,
        url: impl Into<String>,
        frame_name: Option<String>,
        security_origin: impl Into<String>,
        secure_context_type: impl Into<String>,
    ) -> Self {
        Self::wrap_page_document_opened_event(Box::new(BackgroundPageDocumentOpenedEvent {
            session_id: session_id.map(str::to_owned),
            frame_id: frame_id.into(),
            parent_frame_id,
            loader_id: loader_id.into(),
            url: url.into(),
            frame_name,
            security_origin: security_origin.into(),
            secure_context_type: secure_context_type.into(),
        }))
    }

    pub(crate) fn page_dom_content_loaded(
        session_id: Option<&str>,
        event: NavigationLifecycleEvent,
    ) -> Self {
        Self::wrap_page_dom_content_loaded(Box::new(BackgroundNavigationLifecycleEvent {
            session_id: session_id.map(str::to_owned),
            event,
            renderer_document: None,
            renderer_epoch: None,
        }))
    }

    pub(crate) fn page_load_for_renderer_document(
        session_id: Option<&str>,
        event: NavigationLifecycleEvent,
        renderer_document: RendererDocumentToken,
        renderer_epoch: RendererLifecycleEpoch,
    ) -> Self {
        Self::wrap_page_load_event(Box::new(BackgroundNavigationLifecycleEvent {
            session_id: session_id.map(str::to_owned),
            event,
            renderer_document: Some(renderer_document),
            renderer_epoch: Some(renderer_epoch),
        }))
    }

    pub(crate) fn page_lifecycle(session_id: Option<&str>, event: PageLifecycleEvent) -> Self {
        Self::wrap_page_lifecycle_event(Box::new(BackgroundPageLifecycleEvent {
            session_id: session_id.map(str::to_owned),
            event,
        }))
    }

    pub(crate) fn page_same_document_navigation(
        session_id: Option<&str>,
        event: SameDocumentNavigationEvent,
    ) -> Self {
        Self::wrap_page_same_document_navigation_event(Box::new(
            BackgroundSameDocumentNavigationEvent {
                session_id: session_id.map(str::to_owned),
                event,
            },
        ))
    }

    pub(crate) fn page_frame_attached(
        session_id: Option<&str>,
        frame_id: impl Into<String>,
        parent_frame_id: impl Into<String>,
    ) -> Self {
        Self::wrap_page_frame_attached_event(Box::new(BackgroundPageFrameAttachedEvent {
            session_id: session_id.map(str::to_owned),
            frame_id: frame_id.into(),
            parent_frame_id: parent_frame_id.into(),
        }))
    }

    pub(crate) fn page_frame_detached(
        session_id: Option<&str>,
        frame_id: impl Into<String>,
    ) -> Self {
        Self::wrap_page_frame_detached_event(Box::new(BackgroundPageFrameDetachedEvent {
            session_id: session_id.map(str::to_owned),
            frame_id: frame_id.into(),
        }))
    }

    pub(crate) fn page_file_chooser_opened(
        session_id: Option<&str>,
        frame_id: &str,
        mode: &str,
        backend_node_id: u32,
        element_shared_id: Option<DevToolsRemoteHandleId>,
    ) -> Self {
        Self::wrap_page_file_chooser_opened_event(Box::new(BackgroundPageFileChooserOpenedEvent {
            session_id: session_id.map(str::to_owned),
            frame_id: frame_id.to_owned(),
            mode: mode.to_owned(),
            backend_node_id,
            element_shared_id,
        }))
    }

    pub(crate) fn browser_download_will_begin(
        session_id: Option<&str>,
        subscription_generation: Option<u64>,
        frame_id: &str,
        guid: &str,
        url: &str,
        suggested_filename: &str,
    ) -> Self {
        let event = Self::wrap_browser_download_will_begin_event(Box::new(
            BackgroundBrowserDownloadWillBeginEvent {
                session_id: session_id.map(str::to_owned),
                frame_id: frame_id.to_owned(),
                guid: guid.to_owned(),
                url: url.to_owned(),
                suggested_filename: suggested_filename.to_owned(),
            },
        ));
        if let Some(generation) = subscription_generation {
            event.bind_to_browser_download_subscription_route(session_id, generation)
        } else {
            event
        }
    }

    pub(crate) fn browser_download_progress(
        session_id: Option<&str>,
        subscription_generation: Option<u64>,
        guid: &str,
        state: &str,
        received_bytes: u64,
        total_bytes: u64,
        file_path: Option<&str>,
    ) -> Self {
        let event = Self::wrap_browser_download_progress_event(Box::new(
            BackgroundBrowserDownloadProgressEvent {
                session_id: session_id.map(str::to_owned),
                guid: guid.to_owned(),
                state: state.to_owned(),
                received_bytes,
                total_bytes,
                file_path: file_path.map(str::to_owned),
            },
        ));
        if let Some(generation) = subscription_generation {
            event.bind_to_browser_download_subscription_route(session_id, generation)
        } else {
            event
        }
    }

    pub(crate) fn automation_download_will_begin(
        frame_id: &str,
        guid: &str,
        url: &str,
        suggested_filename: &str,
    ) -> Self {
        Self::automation_only(AutomationEvent::BrowserDownloadWillBegin(
            BrowserDownloadWillBeginEvent {
                frame_id: DevToolsFrameId::from(frame_id),
                guid: guid.to_owned(),
                url: url.to_owned(),
                suggested_filename: suggested_filename.to_owned(),
            },
        ))
    }

    pub(crate) fn automation_download_progress(
        guid: &str,
        state: &str,
        received_bytes: u64,
        total_bytes: u64,
        file_path: Option<&str>,
    ) -> Self {
        Self::automation_only(AutomationEvent::BrowserDownloadProgress(
            BrowserDownloadProgressEvent {
                guid: guid.to_owned(),
                state: state.to_owned(),
                received_bytes,
                total_bytes,
                file_path: file_path.map(str::to_owned),
            },
        ))
    }

    pub(crate) fn page_javascript_dialog_opening(
        session_id: Option<&str>,
        event: PageJavaScriptDialogOpeningEvent,
    ) -> Self {
        Self::wrap_page_javascript_dialog_opening_event(Box::new(
            BackgroundPageJavaScriptDialogOpeningEvent {
                session_id: session_id.map(str::to_owned),
                frame_id: event.frame_id.as_ref().map(|id| id.as_str().to_owned()),
                url: event.url,
                message: event.message,
                dialog_type: event.dialog_type,
                has_browser_handler: event.has_browser_handler,
                default_prompt: event.default_prompt,
            },
        ))
    }

    pub(crate) fn page_javascript_dialog_closed(
        session_id: Option<&str>,
        event: UserPromptClosedEvent,
    ) -> Self {
        Self::wrap_page_javascript_dialog_closed_event(Box::new(
            BackgroundPageJavaScriptDialogClosedEvent {
                session_id: session_id.map(str::to_owned),
                target_id: event.target_id.as_ref().map(|id| id.as_str().to_owned()),
                frame_id: event.frame_id.as_str().to_owned(),
                prompt_type: event.prompt_type,
                accepted: event.accepted,
                user_text: event.user_text,
            },
        ))
    }

    pub(crate) fn page_screencast_visibility_changed(
        session_id: Option<&str>,
        visible: bool,
    ) -> Self {
        Self::wrap_page_screencast_visibility_changed_event(Box::new(
            BackgroundPageScreencastVisibilityChangedEvent {
                session_id: session_id.map(str::to_owned),
                visible,
            },
        ))
    }

    pub(crate) fn page_screencast_frame(
        session_id: Option<&str>,
        data: String,
        metadata: PageScreencastFrameMetadata,
        generation: i32,
    ) -> Self {
        Self::wrap_page_screencast_frame_event(Box::new(BackgroundPageScreencastFrameEvent {
            session_id: session_id.map(str::to_owned),
            data,
            metadata,
            generation,
        }))
    }

    pub(crate) fn page_window_open(
        session_id: Option<&str>,
        url: &str,
        window_name: &str,
        window_features: &[String],
        user_gesture: bool,
    ) -> Self {
        Self::immediate(build_event(
            "Page.windowOpen",
            json!({
                "url": url,
                "windowName": window_name,
                "windowFeatures": window_features,
                "userGesture": user_gesture,
            }),
            session_id,
        ))
    }

    pub(crate) fn inspector_target_crashed(session_id: Option<&str>) -> Self {
        Self::wrap_inspector_target_crashed_event(Box::new(BackgroundInspectorTargetCrashedEvent {
            session_id: session_id.map(str::to_owned),
        }))
    }

    pub(crate) fn inspector_target_reloaded_after_crash(session_id: Option<&str>) -> Self {
        Self::wrap_inspector_target_reloaded_after_crash_event(Box::new(
            BackgroundInspectorTargetReloadedAfterCrashEvent {
                session_id: session_id.map(str::to_owned),
            },
        ))
    }

    pub(crate) fn inspector_detached(session_id: Option<&str>, reason: impl Into<String>) -> Self {
        Self::wrap_inspector_detached_event(Box::new(BackgroundInspectorDetachedEvent {
            session_id: session_id.map(str::to_owned),
            reason: reason.into(),
        }))
    }

    pub(crate) fn service_worker_registration_updated(
        session_id: Option<&str>,
        registrations: Vec<BackgroundServiceWorkerRegistration>,
    ) -> Self {
        Self::wrap_service_worker_registration_updated_event(Box::new(
            BackgroundServiceWorkerRegistrationUpdatedEvent {
                session_id: session_id.map(str::to_owned),
                registrations,
            },
        ))
    }

    pub(crate) fn service_worker_version_updated(
        session_id: Option<&str>,
        versions: Vec<BackgroundServiceWorkerVersion>,
    ) -> Self {
        Self::wrap_service_worker_version_updated_event(Box::new(
            BackgroundServiceWorkerVersionUpdatedEvent {
                session_id: session_id.map(str::to_owned),
                versions,
            },
        ))
    }

    pub(crate) fn service_worker_error_reported(
        session_id: Option<&str>,
        error_message: BackgroundServiceWorkerErrorMessage,
    ) -> Self {
        Self::wrap_service_worker_error_reported_event(Box::new(
            BackgroundServiceWorkerErrorReportedEvent {
                session_id: session_id.map(str::to_owned),
                error_message,
            },
        ))
    }

    pub(crate) fn target_info_changed(
        session_id: Option<&str>,
        target_info: DevToolsTargetInfo,
    ) -> Self {
        Self::wrap_target_info_changed_event(Box::new(BackgroundTargetInfoChangedEvent {
            session_id: session_id.map(str::to_owned),
            target_info,
        }))
    }

    pub(crate) fn target_created(session_id: Option<&str>, event: TargetLifecycleEvent) -> Self {
        Self::wrap_target_created_event(Box::new(BackgroundTargetCreatedEvent {
            session_id: session_id.map(str::to_owned),
            event,
        }))
    }

    pub(crate) fn target_attached(event: TargetAttachmentEvent) -> Self {
        Self::wrap_target_attached_event(Box::new(BackgroundTargetAttachedEvent { event }))
    }

    pub(crate) fn target_detached(event: TargetDetachmentEvent) -> Self {
        Self::wrap_target_detached_event(Box::new(BackgroundTargetDetachedEvent { event }))
    }

    pub(crate) fn target_destroyed(session_id: Option<&str>, event: TargetLifecycleEvent) -> Self {
        Self::wrap_target_destroyed_event(Box::new(BackgroundTargetDestroyedEvent {
            session_id: session_id.map(str::to_owned),
            event,
        }))
    }

    pub(crate) fn target_crashed(
        session_id: Option<&str>,
        target_id: impl Into<String>,
        status: impl Into<String>,
        error_code: i32,
    ) -> Self {
        Self::wrap_target_crashed_event(Box::new(BackgroundTargetCrashedEvent {
            session_id: session_id.map(str::to_owned),
            target_id: target_id.into(),
            status: status.into(),
            error_code,
        }))
    }

    pub(crate) fn target_received_message_from_target(
        target_session_id: impl Into<String>,
        nested_event: BackgroundProtocolEvent,
    ) -> Self {
        Self::wrap_target_received_message_from_target_event(Box::new(
            BackgroundTargetReceivedMessageFromTargetEvent {
                target_session_id: target_session_id.into(),
                nested_event: Box::new(nested_event),
            },
        ))
    }

    pub(crate) fn ensure_protocol_session_id(&mut self, session_id: Option<&str>) {
        let Some(session_id) = session_id else {
            return;
        };
        match &mut self.payload {
            BackgroundProtocolEventPayload::Protocol(event) => {
                if event.message.get("sessionId").is_none() {
                    event.message["sessionId"] = json!(session_id);
                }
            }
            BackgroundProtocolEventPayload::CommandResponse(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::NetworkRequestWillBeSentExtraInfo(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::NetworkResponseReceivedExtraInfo(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::NetworkWebSocketCreated(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::NetworkWebSocketWillSendHandshakeRequest(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::NetworkWebSocketHandshakeResponseReceived(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::NetworkWebSocketFrame(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::ConsoleMessageAdded(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::LogEntryAdded(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::RuntimeConsoleApiCalled(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::RuntimeExceptionThrown(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::RuntimeBindingCalled(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::RuntimeExecutionContextCreated(event)
            | BackgroundProtocolEventPayload::RuntimeExecutionContextDestroyed(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::RuntimeExecutionContextsCleared(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::DomSetChildNodes(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::PageNavigationFrame(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::PageDocumentOpened(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::PageDomContentLoaded(event)
            | BackgroundProtocolEventPayload::PageLoad(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::PageLifecycle(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::PageSameDocumentNavigation(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::PageFrameAttached(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::PageFrameDetached(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::PageFileChooserOpened(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::BrowserDownloadWillBegin(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::BrowserDownloadProgress(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::PageJavaScriptDialogOpening(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::PageJavaScriptDialogClosed(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::PageScreencastFrame(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::PageScreencastVisibilityChanged(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::InspectorTargetCrashed(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::InspectorTargetReloadedAfterCrash(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::InspectorDetached(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::ServiceWorkerRegistrationUpdated(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::ServiceWorkerVersionUpdated(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::ServiceWorkerErrorReported(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::TargetInfoChanged(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::TargetCreated(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::TargetAttached(event) => {
                if event.event.parent_session_id.is_none() {
                    event.event.parent_session_id = Some(session_id.into());
                }
            }
            BackgroundProtocolEventPayload::TargetDetached(event) => {
                if event.event.parent_session_id.is_none() {
                    event.event.parent_session_id = Some(session_id.into());
                }
            }
            BackgroundProtocolEventPayload::TargetDestroyed(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::TargetCrashed(event) => {
                if event.session_id.is_none() {
                    event.session_id = Some(session_id.to_owned());
                }
            }
            BackgroundProtocolEventPayload::TargetReceivedMessageFromTarget(_) => {}
            BackgroundProtocolEventPayload::AutomationOnly(_)
            | BackgroundProtocolEventPayload::RuntimeInspectorResponseReady(_) => {}
        }
        self.route.ensure_wire_session_id(session_id);
    }

    pub fn should_wait_for_background_navigation_completion(&self) -> bool {
        self.is_non_document_network_event() && !self.is_fetch_interception_control_path_event()
    }

    pub fn is_non_document_network_event(&self) -> bool {
        let Some(resource_type) = self.network_resource_type() else {
            return false;
        };
        resource_type != DevToolsNetworkResourceType::Document
    }

    /// Returns whether this event is an already-produced CDP Network-domain
    /// observation.
    ///
    /// Main-document load scheduling may delay Page-side effects produced by
    /// a timer, but it must not delay network facts that Chromium exposes as
    /// soon as they occur. This classification uses the typed protocol method
    /// rather than a renderer wake source, because one renderer publication
    /// can contain both kinds of output.
    pub fn is_network_protocol_observation(&self) -> bool {
        self.protocol_method()
            .is_some_and(|method| method.starts_with("Network."))
    }

    pub fn is_document_network_response_started(&self) -> bool {
        let Some(message) = self.protocol_message() else {
            return false;
        };
        message.get("method").and_then(Value::as_str) == Some("Network.responseReceived")
            && self.network_resource_type() == Some(DevToolsNetworkResourceType::Document)
    }

    pub fn trace_network_summary(
        &self,
    ) -> Option<(&str, Option<&str>, Option<&str>, Option<&str>)> {
        match &self.payload {
            BackgroundProtocolEventPayload::NetworkRequestWillBeSentExtraInfo(event) => {
                return Some((
                    "Network.requestWillBeSentExtraInfo",
                    None,
                    Some(event.request_id.as_str()),
                    None,
                ));
            }
            BackgroundProtocolEventPayload::NetworkResponseReceivedExtraInfo(event) => {
                return Some((
                    "Network.responseReceivedExtraInfo",
                    None,
                    Some(event.request_id.as_str()),
                    None,
                ));
            }
            BackgroundProtocolEventPayload::NetworkWebSocketCreated(event) => {
                return Some((
                    "Network.webSocketCreated",
                    Some("WebSocket"),
                    Some(event.request_id.as_str()),
                    Some(event.url.as_str()),
                ));
            }
            BackgroundProtocolEventPayload::NetworkWebSocketWillSendHandshakeRequest(event) => {
                return Some((
                    "Network.webSocketWillSendHandshakeRequest",
                    Some("WebSocket"),
                    Some(event.request_id.as_str()),
                    None,
                ));
            }
            BackgroundProtocolEventPayload::NetworkWebSocketHandshakeResponseReceived(event) => {
                return Some((
                    "Network.webSocketHandshakeResponseReceived",
                    Some("WebSocket"),
                    Some(event.request_id.as_str()),
                    None,
                ));
            }
            BackgroundProtocolEventPayload::NetworkWebSocketFrame(event) => {
                return Some((
                    network_websocket_frame_protocol_method(event.direction),
                    Some("WebSocket"),
                    Some(event.request_id.as_str()),
                    None,
                ));
            }
            _ => {}
        }
        let message = self.protocol_message()?;
        let method = message.get("method")?.as_str()?;
        if !method.starts_with("Network.") && !method.starts_with("Fetch.") {
            return None;
        }
        let params = message.get("params")?;
        let request_id = params
            .get("requestId")
            .or_else(|| params.get("networkId"))
            .and_then(Value::as_str);
        let url = params
            .pointer("/request/url")
            .or_else(|| params.pointer("/response/url"))
            .or_else(|| params.get("documentURL"))
            .and_then(Value::as_str);
        Some((
            method,
            self.network_resource_type()
                .map(DevToolsNetworkResourceType::as_cdp_type),
            request_id,
            url,
        ))
    }

    fn is_fetch_interception_control_path_event(&self) -> bool {
        match self.protocol_method() {
            Some("Fetch.requestPaused" | "Fetch.authRequired") => true,
            Some(method) if method.starts_with("Network.") => self.has_blocked_intercepts(),
            _ => false,
        }
    }

    fn has_blocked_intercepts(&self) -> bool {
        self.automation_event_has_blocked_intercepts()
            || self.protocol_message_has_blocked_intercepts()
    }

    fn automation_event_has_blocked_intercepts(&self) -> bool {
        let BackgroundProtocolEventPayload::Protocol(event) = &self.payload else {
            return false;
        };
        match event.automation_event.as_deref() {
            Some(
                AutomationEvent::NetworkBeforeRequestSent(event)
                | AutomationEvent::NetworkResponseStarted(event)
                | AutomationEvent::NetworkResponseCompleted(event)
                | AutomationEvent::NetworkFetchError(event)
                | AutomationEvent::NetworkAuthRequired(event)
                | AutomationEvent::RequestPaused(event),
            ) => !event.blocked_intercepts.is_empty(),
            _ => false,
        }
    }

    fn protocol_message_has_blocked_intercepts(&self) -> bool {
        self.protocol_message()
            .and_then(|message| message.get("params"))
            .and_then(|params| params.get("__moliBlockedInterceptors"))
            .and_then(Value::as_array)
            .is_some_and(|blocked_intercepts| !blocked_intercepts.is_empty())
    }

    fn network_resource_type(&self) -> Option<DevToolsNetworkResourceType> {
        if let Some(resource_type) = self.automation_event_network_resource_type() {
            return Some(resource_type);
        }
        self.protocol_message_network_resource_type()
    }

    fn automation_event_network_resource_type(&self) -> Option<DevToolsNetworkResourceType> {
        let BackgroundProtocolEventPayload::Protocol(event) = &self.payload else {
            return None;
        };
        match event.automation_event.as_deref()? {
            AutomationEvent::NetworkBeforeRequestSent(event)
            | AutomationEvent::NetworkResponseStarted(event)
            | AutomationEvent::NetworkResponseCompleted(event)
            | AutomationEvent::NetworkFetchError(event)
            | AutomationEvent::NetworkAuthRequired(event)
            | AutomationEvent::RequestPaused(event) => event.resource_type,
            _ => None,
        }
    }

    fn protocol_message_network_resource_type(&self) -> Option<DevToolsNetworkResourceType> {
        let message = self.protocol_message()?;
        let method = message.get("method")?.as_str()?;
        let params = message.get("params")?;
        match method {
            "Network.requestWillBeSent" | "Network.responseReceived" | "Network.loadingFailed" => {
                params
                    .get("type")?
                    .as_str()
                    .and_then(DevToolsNetworkResourceType::from_cdp_type)
            }
            "Network.webSocketFrameError" | "Network.webSocketClosed" => {
                Some(DevToolsNetworkResourceType::WebSocket)
            }
            "Fetch.requestPaused" | "Fetch.authRequired" => params
                .get("resourceType")?
                .as_str()
                .and_then(DevToolsNetworkResourceType::from_cdp_type),
            _ => None,
        }
    }

    pub fn into_protocol_message(self) -> Value {
        match self.payload {
            BackgroundProtocolEventPayload::Protocol(event) => {
                strip_moli_private_protocol_fields(event.message)
            }
            BackgroundProtocolEventPayload::CommandResponse(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::NetworkRequestWillBeSentExtraInfo(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::NetworkResponseReceivedExtraInfo(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::NetworkWebSocketCreated(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::NetworkWebSocketWillSendHandshakeRequest(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::NetworkWebSocketHandshakeResponseReceived(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::NetworkWebSocketFrame(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::ConsoleMessageAdded(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::LogEntryAdded(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::RuntimeConsoleApiCalled(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::RuntimeExceptionThrown(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::RuntimeBindingCalled(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::RuntimeExecutionContextCreated(event) => {
                (*event).into_context_created_protocol_message()
            }
            BackgroundProtocolEventPayload::RuntimeExecutionContextDestroyed(event) => {
                (*event).into_context_destroyed_protocol_message()
            }
            BackgroundProtocolEventPayload::RuntimeExecutionContextsCleared(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::DomSetChildNodes(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::PageNavigationFrame(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::PageDocumentOpened(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::PageDomContentLoaded(event) => {
                (*event).into_dom_content_loaded_protocol_message()
            }
            BackgroundProtocolEventPayload::PageLoad(event) => {
                (*event).into_load_protocol_message()
            }
            BackgroundProtocolEventPayload::PageLifecycle(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::PageSameDocumentNavigation(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::PageFrameAttached(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::PageFrameDetached(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::PageFileChooserOpened(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::BrowserDownloadWillBegin(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::BrowserDownloadProgress(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::PageJavaScriptDialogOpening(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::PageJavaScriptDialogClosed(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::PageScreencastFrame(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::PageScreencastVisibilityChanged(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::InspectorTargetCrashed(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::InspectorTargetReloadedAfterCrash(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::InspectorDetached(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::ServiceWorkerRegistrationUpdated(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::ServiceWorkerVersionUpdated(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::ServiceWorkerErrorReported(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::TargetInfoChanged(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::TargetCreated(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::TargetAttached(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::TargetDetached(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::TargetDestroyed(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::TargetCrashed(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::TargetReceivedMessageFromTarget(event) => {
                (*event).into_protocol_message()
            }
            BackgroundProtocolEventPayload::AutomationOnly(_) => json!({
                "error": {
                    "code": -32000,
                    "message": "InternalAutomationOnlyEventNotRouted",
                },
            }),
            BackgroundProtocolEventPayload::RuntimeInspectorResponseReady(_) => json!({
                "error": {
                    "code": -32000,
                    "message": "InternalRuntimeInspectorResponseReadyNotRouted",
                },
            }),
        }
    }

    pub fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        match self.payload {
            BackgroundProtocolEventPayload::Protocol(event) => (
                event.message,
                event
                    .automation_event
                    .map(|automation_event| *automation_event),
            ),
            BackgroundProtocolEventPayload::CommandResponse(event) => {
                ((*event).into_protocol_message(), None)
            }
            BackgroundProtocolEventPayload::NetworkRequestWillBeSentExtraInfo(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::NetworkResponseReceivedExtraInfo(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::NetworkWebSocketCreated(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::NetworkWebSocketWillSendHandshakeRequest(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::NetworkWebSocketHandshakeResponseReceived(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::NetworkWebSocketFrame(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::ConsoleMessageAdded(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::LogEntryAdded(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::RuntimeConsoleApiCalled(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::RuntimeExceptionThrown(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::RuntimeBindingCalled(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::RuntimeExecutionContextCreated(event) => {
                (*event).into_context_created_parts()
            }
            BackgroundProtocolEventPayload::RuntimeExecutionContextDestroyed(event) => {
                (*event).into_context_destroyed_parts()
            }
            BackgroundProtocolEventPayload::RuntimeExecutionContextsCleared(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::DomSetChildNodes(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::PageNavigationFrame(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::PageDocumentOpened(event) => {
                ((*event).into_protocol_message(), None)
            }
            BackgroundProtocolEventPayload::PageDomContentLoaded(event) => {
                (*event).into_dom_content_loaded_parts()
            }
            BackgroundProtocolEventPayload::PageLoad(event) => (*event).into_load_parts(),
            BackgroundProtocolEventPayload::PageLifecycle(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::PageSameDocumentNavigation(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::PageFrameAttached(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::PageFrameDetached(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::PageFileChooserOpened(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::BrowserDownloadWillBegin(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::BrowserDownloadProgress(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::PageJavaScriptDialogOpening(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::PageJavaScriptDialogClosed(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::PageScreencastFrame(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::PageScreencastVisibilityChanged(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::InspectorTargetCrashed(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::InspectorTargetReloadedAfterCrash(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::InspectorDetached(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::ServiceWorkerRegistrationUpdated(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::ServiceWorkerVersionUpdated(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::ServiceWorkerErrorReported(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::TargetInfoChanged(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::TargetCreated(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::TargetAttached(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::TargetDetached(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::TargetDestroyed(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::TargetCrashed(event) => (*event).into_parts(),
            BackgroundProtocolEventPayload::TargetReceivedMessageFromTarget(event) => {
                (*event).into_parts()
            }
            BackgroundProtocolEventPayload::AutomationOnly(event) => (
                json!({
                    "method": "Moli.automationOnly",
                    "params": {},
                }),
                Some(*event),
            ),
            BackgroundProtocolEventPayload::RuntimeInspectorResponseReady(_) => (
                json!({
                    "error": {
                        "code": -32000,
                        "message": "InternalRuntimeInspectorResponseReadyNotRouted",
                    },
                }),
                None,
            ),
        }
    }

    pub fn take_runtime_inspector_response_ready(
        self,
    ) -> Result<RuntimeInspectorResponseReady, Self> {
        let Self { payload, route } = self;
        match payload {
            BackgroundProtocolEventPayload::RuntimeInspectorResponseReady(response) => {
                Ok(*response)
            }
            payload => Err(Self { payload, route }),
        }
    }

    #[cfg(test)]
    pub(crate) fn as_runtime_inspector_response_ready(
        &self,
    ) -> Option<&RuntimeInspectorResponseReady> {
        match &self.payload {
            BackgroundProtocolEventPayload::RuntimeInspectorResponseReady(response) => {
                Some(response)
            }
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn is_target_detached(&self) -> bool {
        matches!(
            &self.payload,
            BackgroundProtocolEventPayload::TargetDetached(_)
        )
    }

    pub fn protocol_message(&self) -> Option<&Value> {
        match &self.payload {
            BackgroundProtocolEventPayload::Protocol(event) => Some(&event.message),
            BackgroundProtocolEventPayload::CommandResponse(_) => None,
            BackgroundProtocolEventPayload::NetworkRequestWillBeSentExtraInfo(_) => None,
            BackgroundProtocolEventPayload::NetworkResponseReceivedExtraInfo(_) => None,
            BackgroundProtocolEventPayload::NetworkWebSocketCreated(_) => None,
            BackgroundProtocolEventPayload::NetworkWebSocketWillSendHandshakeRequest(_) => None,
            BackgroundProtocolEventPayload::NetworkWebSocketHandshakeResponseReceived(_) => None,
            BackgroundProtocolEventPayload::NetworkWebSocketFrame(_) => None,
            BackgroundProtocolEventPayload::ConsoleMessageAdded(_) => None,
            BackgroundProtocolEventPayload::LogEntryAdded(_) => None,
            BackgroundProtocolEventPayload::RuntimeConsoleApiCalled(_) => None,
            BackgroundProtocolEventPayload::RuntimeExceptionThrown(_) => None,
            BackgroundProtocolEventPayload::RuntimeBindingCalled(_) => None,
            BackgroundProtocolEventPayload::RuntimeExecutionContextCreated(_) => None,
            BackgroundProtocolEventPayload::RuntimeExecutionContextDestroyed(_) => None,
            BackgroundProtocolEventPayload::RuntimeExecutionContextsCleared(_) => None,
            BackgroundProtocolEventPayload::DomSetChildNodes(_) => None,
            BackgroundProtocolEventPayload::PageNavigationFrame(_) => None,
            BackgroundProtocolEventPayload::PageDocumentOpened(_) => None,
            BackgroundProtocolEventPayload::PageDomContentLoaded(_) => None,
            BackgroundProtocolEventPayload::PageLoad(_) => None,
            BackgroundProtocolEventPayload::PageLifecycle(_) => None,
            BackgroundProtocolEventPayload::PageSameDocumentNavigation(_) => None,
            BackgroundProtocolEventPayload::PageFrameAttached(_) => None,
            BackgroundProtocolEventPayload::PageFrameDetached(_) => None,
            BackgroundProtocolEventPayload::PageFileChooserOpened(_) => None,
            BackgroundProtocolEventPayload::BrowserDownloadWillBegin(_) => None,
            BackgroundProtocolEventPayload::BrowserDownloadProgress(_) => None,
            BackgroundProtocolEventPayload::PageJavaScriptDialogOpening(_) => None,
            BackgroundProtocolEventPayload::PageJavaScriptDialogClosed(_) => None,
            BackgroundProtocolEventPayload::PageScreencastFrame(_) => None,
            BackgroundProtocolEventPayload::PageScreencastVisibilityChanged(_) => None,
            BackgroundProtocolEventPayload::InspectorTargetCrashed(_) => None,
            BackgroundProtocolEventPayload::InspectorTargetReloadedAfterCrash(_) => None,
            BackgroundProtocolEventPayload::InspectorDetached(_) => None,
            BackgroundProtocolEventPayload::ServiceWorkerRegistrationUpdated(_) => None,
            BackgroundProtocolEventPayload::ServiceWorkerVersionUpdated(_) => None,
            BackgroundProtocolEventPayload::ServiceWorkerErrorReported(_) => None,
            BackgroundProtocolEventPayload::TargetInfoChanged(_) => None,
            BackgroundProtocolEventPayload::TargetCreated(_) => None,
            BackgroundProtocolEventPayload::TargetAttached(_) => None,
            BackgroundProtocolEventPayload::TargetDetached(_) => None,
            BackgroundProtocolEventPayload::TargetDestroyed(_) => None,
            BackgroundProtocolEventPayload::TargetCrashed(_) => None,
            BackgroundProtocolEventPayload::TargetReceivedMessageFromTarget(_) => None,
            BackgroundProtocolEventPayload::AutomationOnly(_) => None,
            BackgroundProtocolEventPayload::RuntimeInspectorResponseReady(_) => None,
        }
    }

    pub fn protocol_method(&self) -> Option<&str> {
        match &self.payload {
            BackgroundProtocolEventPayload::Protocol(event) => {
                event.message.get("method").and_then(Value::as_str)
            }
            BackgroundProtocolEventPayload::CommandResponse(_) => None,
            BackgroundProtocolEventPayload::NetworkRequestWillBeSentExtraInfo(_) => {
                Some("Network.requestWillBeSentExtraInfo")
            }
            BackgroundProtocolEventPayload::NetworkResponseReceivedExtraInfo(_) => {
                Some("Network.responseReceivedExtraInfo")
            }
            BackgroundProtocolEventPayload::NetworkWebSocketCreated(_) => {
                Some("Network.webSocketCreated")
            }
            BackgroundProtocolEventPayload::NetworkWebSocketWillSendHandshakeRequest(_) => {
                Some("Network.webSocketWillSendHandshakeRequest")
            }
            BackgroundProtocolEventPayload::NetworkWebSocketHandshakeResponseReceived(_) => {
                Some("Network.webSocketHandshakeResponseReceived")
            }
            BackgroundProtocolEventPayload::NetworkWebSocketFrame(event) => {
                Some(network_websocket_frame_protocol_method(event.direction))
            }
            BackgroundProtocolEventPayload::ConsoleMessageAdded(_) => Some("Console.messageAdded"),
            BackgroundProtocolEventPayload::LogEntryAdded(_) => Some("Log.entryAdded"),
            BackgroundProtocolEventPayload::RuntimeConsoleApiCalled(_) => {
                Some("Runtime.consoleAPICalled")
            }
            BackgroundProtocolEventPayload::RuntimeExceptionThrown(_) => {
                Some("Runtime.exceptionThrown")
            }
            BackgroundProtocolEventPayload::RuntimeBindingCalled(_) => {
                Some("Runtime.bindingCalled")
            }
            BackgroundProtocolEventPayload::RuntimeExecutionContextCreated(_) => {
                Some("Runtime.executionContextCreated")
            }
            BackgroundProtocolEventPayload::RuntimeExecutionContextDestroyed(_) => {
                Some("Runtime.executionContextDestroyed")
            }
            BackgroundProtocolEventPayload::RuntimeExecutionContextsCleared(_) => {
                Some("Runtime.executionContextsCleared")
            }
            BackgroundProtocolEventPayload::DomSetChildNodes(_) => Some("DOM.setChildNodes"),
            BackgroundProtocolEventPayload::PageNavigationFrame(event) => {
                Some(navigation_frame_protocol_method(event.event.kind))
            }
            BackgroundProtocolEventPayload::PageDocumentOpened(_) => Some("Page.documentOpened"),
            BackgroundProtocolEventPayload::PageDomContentLoaded(_) => {
                Some("Page.domContentEventFired")
            }
            BackgroundProtocolEventPayload::PageLoad(_) => Some("Page.loadEventFired"),
            BackgroundProtocolEventPayload::PageLifecycle(_) => Some("Page.lifecycleEvent"),
            BackgroundProtocolEventPayload::PageSameDocumentNavigation(_) => {
                Some("Page.navigatedWithinDocument")
            }
            BackgroundProtocolEventPayload::PageFrameAttached(_) => Some("Page.frameAttached"),
            BackgroundProtocolEventPayload::PageFrameDetached(_) => Some("Page.frameDetached"),
            BackgroundProtocolEventPayload::PageFileChooserOpened(_) => {
                Some("Page.fileChooserOpened")
            }
            BackgroundProtocolEventPayload::BrowserDownloadWillBegin(_) => {
                Some("Browser.downloadWillBegin")
            }
            BackgroundProtocolEventPayload::BrowserDownloadProgress(_) => {
                Some("Browser.downloadProgress")
            }
            BackgroundProtocolEventPayload::PageJavaScriptDialogOpening(_) => {
                Some("Page.javascriptDialogOpening")
            }
            BackgroundProtocolEventPayload::PageJavaScriptDialogClosed(_) => {
                Some("Page.javascriptDialogClosed")
            }
            BackgroundProtocolEventPayload::PageScreencastFrame(_) => Some("Page.screencastFrame"),
            BackgroundProtocolEventPayload::PageScreencastVisibilityChanged(_) => {
                Some("Page.screencastVisibilityChanged")
            }
            BackgroundProtocolEventPayload::InspectorTargetCrashed(_) => {
                Some("Inspector.targetCrashed")
            }
            BackgroundProtocolEventPayload::InspectorTargetReloadedAfterCrash(_) => {
                Some("Inspector.targetReloadedAfterCrash")
            }
            BackgroundProtocolEventPayload::InspectorDetached(_) => Some("Inspector.detached"),
            BackgroundProtocolEventPayload::ServiceWorkerRegistrationUpdated(_) => {
                Some("ServiceWorker.workerRegistrationUpdated")
            }
            BackgroundProtocolEventPayload::ServiceWorkerVersionUpdated(_) => {
                Some("ServiceWorker.workerVersionUpdated")
            }
            BackgroundProtocolEventPayload::ServiceWorkerErrorReported(_) => {
                Some("ServiceWorker.workerErrorReported")
            }
            BackgroundProtocolEventPayload::TargetInfoChanged(_) => {
                Some("Target.targetInfoChanged")
            }
            BackgroundProtocolEventPayload::TargetCreated(_) => Some("Target.targetCreated"),
            BackgroundProtocolEventPayload::TargetAttached(_) => Some("Target.attachedToTarget"),
            BackgroundProtocolEventPayload::TargetDetached(_) => Some("Target.detachedFromTarget"),
            BackgroundProtocolEventPayload::TargetDestroyed(_) => Some("Target.targetDestroyed"),
            BackgroundProtocolEventPayload::TargetCrashed(_) => Some("Target.targetCrashed"),
            BackgroundProtocolEventPayload::TargetReceivedMessageFromTarget(_) => {
                Some("Target.receivedMessageFromTarget")
            }
            BackgroundProtocolEventPayload::AutomationOnly(_)
            | BackgroundProtocolEventPayload::RuntimeInspectorResponseReady(_) => None,
        }
    }

    pub fn matches_document_load_wait_key(&self, key: &DevToolsDocumentLifecycleWaitKey) -> bool {
        let BackgroundProtocolEventPayload::PageLoad(event) = &self.payload else {
            return false;
        };
        event.renderer_document == Some(key.renderer_document)
            && event.renderer_epoch == Some(key.renderer_epoch)
            && event.event.frame_id.as_str() == key.frame_id
            && event
                .event
                .loader_id
                .as_ref()
                .map(|loader_id| loader_id.as_str())
                == Some(key.loader_id.as_str())
    }

    pub fn download_will_begin_frame_id(&self) -> Option<&str> {
        match &self.payload {
            BackgroundProtocolEventPayload::BrowserDownloadWillBegin(event) => {
                Some(event.frame_id.as_str())
            }
            BackgroundProtocolEventPayload::Protocol(event) => {
                match event.automation_event.as_deref() {
                    Some(AutomationEvent::BrowserDownloadWillBegin(event)) => {
                        Some(event.frame_id.as_str())
                    }
                    _ => None,
                }
            }
            BackgroundProtocolEventPayload::AutomationOnly(event) => match event.as_ref() {
                AutomationEvent::BrowserDownloadWillBegin(event) => Some(event.frame_id.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn protocol_message_id(&self) -> Option<u64> {
        match &self.payload {
            BackgroundProtocolEventPayload::Protocol(event) => {
                event.message.get("id").and_then(Value::as_u64)
            }
            BackgroundProtocolEventPayload::CommandResponse(event) => event.command_id,
            BackgroundProtocolEventPayload::NetworkRequestWillBeSentExtraInfo(_)
            | BackgroundProtocolEventPayload::NetworkResponseReceivedExtraInfo(_)
            | BackgroundProtocolEventPayload::NetworkWebSocketCreated(_)
            | BackgroundProtocolEventPayload::NetworkWebSocketWillSendHandshakeRequest(_)
            | BackgroundProtocolEventPayload::NetworkWebSocketHandshakeResponseReceived(_)
            | BackgroundProtocolEventPayload::NetworkWebSocketFrame(_)
            | BackgroundProtocolEventPayload::ConsoleMessageAdded(_)
            | BackgroundProtocolEventPayload::LogEntryAdded(_)
            | BackgroundProtocolEventPayload::RuntimeConsoleApiCalled(_)
            | BackgroundProtocolEventPayload::RuntimeExceptionThrown(_)
            | BackgroundProtocolEventPayload::RuntimeBindingCalled(_)
            | BackgroundProtocolEventPayload::RuntimeExecutionContextCreated(_)
            | BackgroundProtocolEventPayload::RuntimeExecutionContextDestroyed(_)
            | BackgroundProtocolEventPayload::RuntimeExecutionContextsCleared(_)
            | BackgroundProtocolEventPayload::DomSetChildNodes(_)
            | BackgroundProtocolEventPayload::PageNavigationFrame(_)
            | BackgroundProtocolEventPayload::PageDocumentOpened(_)
            | BackgroundProtocolEventPayload::PageDomContentLoaded(_)
            | BackgroundProtocolEventPayload::PageLoad(_)
            | BackgroundProtocolEventPayload::PageLifecycle(_)
            | BackgroundProtocolEventPayload::PageSameDocumentNavigation(_)
            | BackgroundProtocolEventPayload::PageFrameAttached(_)
            | BackgroundProtocolEventPayload::PageFrameDetached(_)
            | BackgroundProtocolEventPayload::PageFileChooserOpened(_)
            | BackgroundProtocolEventPayload::BrowserDownloadWillBegin(_)
            | BackgroundProtocolEventPayload::BrowserDownloadProgress(_)
            | BackgroundProtocolEventPayload::PageJavaScriptDialogOpening(_)
            | BackgroundProtocolEventPayload::PageJavaScriptDialogClosed(_)
            | BackgroundProtocolEventPayload::PageScreencastFrame(_)
            | BackgroundProtocolEventPayload::PageScreencastVisibilityChanged(_)
            | BackgroundProtocolEventPayload::InspectorTargetCrashed(_)
            | BackgroundProtocolEventPayload::InspectorTargetReloadedAfterCrash(_)
            | BackgroundProtocolEventPayload::InspectorDetached(_)
            | BackgroundProtocolEventPayload::ServiceWorkerRegistrationUpdated(_)
            | BackgroundProtocolEventPayload::ServiceWorkerVersionUpdated(_)
            | BackgroundProtocolEventPayload::ServiceWorkerErrorReported(_)
            | BackgroundProtocolEventPayload::TargetInfoChanged(_)
            | BackgroundProtocolEventPayload::TargetCreated(_)
            | BackgroundProtocolEventPayload::TargetAttached(_)
            | BackgroundProtocolEventPayload::TargetDetached(_)
            | BackgroundProtocolEventPayload::TargetDestroyed(_)
            | BackgroundProtocolEventPayload::TargetCrashed(_)
            | BackgroundProtocolEventPayload::TargetReceivedMessageFromTarget(_) => None,
            BackgroundProtocolEventPayload::AutomationOnly(_)
            | BackgroundProtocolEventPayload::RuntimeInspectorResponseReady(_) => None,
        }
    }

    pub fn has_protocol_wire_message(&self) -> bool {
        matches!(
            &self.payload,
            BackgroundProtocolEventPayload::Protocol(_)
                | BackgroundProtocolEventPayload::CommandResponse(_)
                | BackgroundProtocolEventPayload::NetworkRequestWillBeSentExtraInfo(_)
                | BackgroundProtocolEventPayload::NetworkResponseReceivedExtraInfo(_)
                | BackgroundProtocolEventPayload::NetworkWebSocketCreated(_)
                | BackgroundProtocolEventPayload::NetworkWebSocketWillSendHandshakeRequest(_)
                | BackgroundProtocolEventPayload::NetworkWebSocketHandshakeResponseReceived(_)
                | BackgroundProtocolEventPayload::NetworkWebSocketFrame(_)
                | BackgroundProtocolEventPayload::ConsoleMessageAdded(_)
                | BackgroundProtocolEventPayload::LogEntryAdded(_)
                | BackgroundProtocolEventPayload::RuntimeConsoleApiCalled(_)
                | BackgroundProtocolEventPayload::RuntimeExceptionThrown(_)
                | BackgroundProtocolEventPayload::RuntimeBindingCalled(_)
                | BackgroundProtocolEventPayload::RuntimeExecutionContextCreated(_)
                | BackgroundProtocolEventPayload::RuntimeExecutionContextDestroyed(_)
                | BackgroundProtocolEventPayload::RuntimeExecutionContextsCleared(_)
                | BackgroundProtocolEventPayload::DomSetChildNodes(_)
                | BackgroundProtocolEventPayload::PageNavigationFrame(_)
                | BackgroundProtocolEventPayload::PageDocumentOpened(_)
                | BackgroundProtocolEventPayload::PageDomContentLoaded(_)
                | BackgroundProtocolEventPayload::PageLoad(_)
                | BackgroundProtocolEventPayload::PageLifecycle(_)
                | BackgroundProtocolEventPayload::PageSameDocumentNavigation(_)
                | BackgroundProtocolEventPayload::PageFrameAttached(_)
                | BackgroundProtocolEventPayload::PageFrameDetached(_)
                | BackgroundProtocolEventPayload::PageFileChooserOpened(_)
                | BackgroundProtocolEventPayload::BrowserDownloadWillBegin(_)
                | BackgroundProtocolEventPayload::BrowserDownloadProgress(_)
                | BackgroundProtocolEventPayload::PageJavaScriptDialogOpening(_)
                | BackgroundProtocolEventPayload::PageJavaScriptDialogClosed(_)
                | BackgroundProtocolEventPayload::PageScreencastFrame(_)
                | BackgroundProtocolEventPayload::PageScreencastVisibilityChanged(_)
                | BackgroundProtocolEventPayload::InspectorTargetCrashed(_)
                | BackgroundProtocolEventPayload::InspectorTargetReloadedAfterCrash(_)
                | BackgroundProtocolEventPayload::InspectorDetached(_)
                | BackgroundProtocolEventPayload::ServiceWorkerRegistrationUpdated(_)
                | BackgroundProtocolEventPayload::ServiceWorkerVersionUpdated(_)
                | BackgroundProtocolEventPayload::ServiceWorkerErrorReported(_)
                | BackgroundProtocolEventPayload::TargetInfoChanged(_)
                | BackgroundProtocolEventPayload::TargetCreated(_)
                | BackgroundProtocolEventPayload::TargetAttached(_)
                | BackgroundProtocolEventPayload::TargetDetached(_)
                | BackgroundProtocolEventPayload::TargetDestroyed(_)
                | BackgroundProtocolEventPayload::TargetCrashed(_)
                | BackgroundProtocolEventPayload::TargetReceivedMessageFromTarget(_)
        )
    }

    pub(crate) fn matches_console_message_added(
        &self,
        source: &str,
        level: &str,
        text: &str,
    ) -> bool {
        match &self.payload {
            BackgroundProtocolEventPayload::Protocol(event) => {
                event.message["method"] == json!("Console.messageAdded")
                    && event.message["params"]["message"]["source"] == json!(source)
                    && event.message["params"]["message"]["level"] == json!(level)
                    && event.message["params"]["message"]["text"] == json!(text)
            }
            BackgroundProtocolEventPayload::ConsoleMessageAdded(event) => {
                event.source == source && event.level == level && event.text == text
            }
            _ => false,
        }
    }

    pub(crate) fn protocol_params_mut(&mut self) -> Option<&mut Value> {
        match &mut self.payload {
            BackgroundProtocolEventPayload::Protocol(event) => event.message.get_mut("params"),
            BackgroundProtocolEventPayload::CommandResponse(_) => None,
            BackgroundProtocolEventPayload::NetworkRequestWillBeSentExtraInfo(_) => None,
            BackgroundProtocolEventPayload::NetworkResponseReceivedExtraInfo(_) => None,
            BackgroundProtocolEventPayload::NetworkWebSocketCreated(_) => None,
            BackgroundProtocolEventPayload::NetworkWebSocketWillSendHandshakeRequest(_) => None,
            BackgroundProtocolEventPayload::NetworkWebSocketHandshakeResponseReceived(_) => None,
            BackgroundProtocolEventPayload::NetworkWebSocketFrame(_) => None,
            BackgroundProtocolEventPayload::ConsoleMessageAdded(_) => None,
            BackgroundProtocolEventPayload::LogEntryAdded(_) => None,
            BackgroundProtocolEventPayload::RuntimeConsoleApiCalled(_) => None,
            BackgroundProtocolEventPayload::RuntimeExceptionThrown(_) => None,
            BackgroundProtocolEventPayload::RuntimeBindingCalled(_) => None,
            BackgroundProtocolEventPayload::RuntimeExecutionContextCreated(_) => None,
            BackgroundProtocolEventPayload::RuntimeExecutionContextDestroyed(_) => None,
            BackgroundProtocolEventPayload::RuntimeExecutionContextsCleared(_) => None,
            BackgroundProtocolEventPayload::DomSetChildNodes(_) => None,
            BackgroundProtocolEventPayload::PageNavigationFrame(_) => None,
            BackgroundProtocolEventPayload::PageDocumentOpened(_) => None,
            BackgroundProtocolEventPayload::PageDomContentLoaded(_) => None,
            BackgroundProtocolEventPayload::PageLoad(_) => None,
            BackgroundProtocolEventPayload::PageLifecycle(_) => None,
            BackgroundProtocolEventPayload::PageSameDocumentNavigation(_) => None,
            BackgroundProtocolEventPayload::PageFrameAttached(_) => None,
            BackgroundProtocolEventPayload::PageFrameDetached(_) => None,
            BackgroundProtocolEventPayload::PageFileChooserOpened(_) => None,
            BackgroundProtocolEventPayload::BrowserDownloadWillBegin(_) => None,
            BackgroundProtocolEventPayload::BrowserDownloadProgress(_) => None,
            BackgroundProtocolEventPayload::PageJavaScriptDialogOpening(_) => None,
            BackgroundProtocolEventPayload::PageJavaScriptDialogClosed(_) => None,
            BackgroundProtocolEventPayload::PageScreencastFrame(_) => None,
            BackgroundProtocolEventPayload::PageScreencastVisibilityChanged(_) => None,
            BackgroundProtocolEventPayload::InspectorTargetCrashed(_) => None,
            BackgroundProtocolEventPayload::InspectorTargetReloadedAfterCrash(_) => None,
            BackgroundProtocolEventPayload::InspectorDetached(_) => None,
            BackgroundProtocolEventPayload::ServiceWorkerRegistrationUpdated(_) => None,
            BackgroundProtocolEventPayload::ServiceWorkerVersionUpdated(_) => None,
            BackgroundProtocolEventPayload::ServiceWorkerErrorReported(_) => None,
            BackgroundProtocolEventPayload::TargetInfoChanged(_) => None,
            BackgroundProtocolEventPayload::TargetCreated(_) => None,
            BackgroundProtocolEventPayload::TargetAttached(_) => None,
            BackgroundProtocolEventPayload::TargetDetached(_) => None,
            BackgroundProtocolEventPayload::TargetDestroyed(_) => None,
            BackgroundProtocolEventPayload::TargetCrashed(_) => None,
            BackgroundProtocolEventPayload::TargetReceivedMessageFromTarget(_) => None,
            BackgroundProtocolEventPayload::AutomationOnly(_) => None,
            BackgroundProtocolEventPayload::RuntimeInspectorResponseReady(_) => None,
        }
    }
}

impl BackgroundCommandResponseEvent {
    fn into_protocol_message(self) -> Value {
        let mut message = match self.response {
            BackgroundCommandResponse::Success { result } => {
                return build_command_success_response(self.command_id, result, self.session_id);
            }
            BackgroundCommandResponse::Error {
                code,
                message,
                data,
            } => {
                let mut error = json!({ "code": code, "message": message });
                if let Some(data) = data {
                    error["data"] = data;
                }
                json!({ "id": self.command_id, "error": error })
            }
        };
        if let Some(session_id) = self.session_id {
            message["sessionId"] = json!(session_id);
        }
        message
    }
}

pub(crate) fn build_command_success_response(
    command_id: Option<u64>,
    result: Value,
    session_id: Option<String>,
) -> Value {
    let mut message = serde_json::Map::new();
    message.insert(
        "id".to_owned(),
        command_id.map(Value::from).unwrap_or(Value::Null),
    );
    message.insert("result".to_owned(), result);
    if let Some(session_id) = session_id {
        message.insert("sessionId".to_owned(), Value::String(session_id));
    }
    Value::Object(message)
}

impl BackgroundCommandResponsePayload {
    pub(crate) fn from_runtime_inspector_message(message: &Value) -> Self {
        if let Some(result) = message.get("result") {
            return Self::Success {
                result: result.clone(),
            };
        }
        if let Some(error) = message.get("error") {
            let code = error
                .get("code")
                .and_then(Value::as_i64)
                .and_then(|code| i32::try_from(code).ok())
                .unwrap_or(-32000);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Runtime inspector command failed")
                .to_owned();
            let data = error.get("data").cloned();
            return Self::Error {
                code,
                message,
                data,
            };
        }
        Self::internal_error("MissingDevToolsCommandResult")
    }

    pub(crate) fn from_owned_runtime_inspector_message(mut message: Value) -> Self {
        let Some(message) = message.as_object_mut() else {
            return Self::internal_error("MissingDevToolsCommandResult");
        };
        if let Some(result) = message.remove("result") {
            return Self::Success { result };
        }
        if let Some(error) = message.remove("error") {
            let Value::Object(mut error) = error else {
                return Self::Error {
                    code: -32000,
                    message: "Runtime inspector command failed".to_owned(),
                    data: None,
                };
            };
            let code = error
                .remove("code")
                .and_then(|code| code.as_i64())
                .and_then(|code| i32::try_from(code).ok())
                .unwrap_or(-32000);
            let message = error
                .remove("message")
                .and_then(|message| message.as_str().map(str::to_owned))
                .unwrap_or_else(|| "Runtime inspector command failed".to_owned());
            let data = error.remove("data");
            return Self::Error {
                code,
                message,
                data,
            };
        }
        Self::internal_error("MissingDevToolsCommandResult")
    }

    fn internal_error(message: impl Into<String>) -> Self {
        Self::Error {
            code: -32000,
            message: message.into(),
            data: None,
        }
    }
}

impl BackgroundCommandResponse {
    fn payload_ref(&self) -> BackgroundCommandResponsePayloadRef<'_> {
        match self {
            Self::Success { result } => BackgroundCommandResponsePayloadRef::Success { result },
            Self::Error {
                code,
                message,
                data,
            } => BackgroundCommandResponsePayloadRef::Error {
                code: *code,
                message,
                data: data.as_ref(),
            },
        }
    }

    fn into_payload(self) -> BackgroundCommandResponsePayload {
        match self {
            Self::Success { result } => BackgroundCommandResponsePayload::Success { result },
            Self::Error {
                code,
                message,
                data,
            } => BackgroundCommandResponsePayload::Error {
                code,
                message,
                data,
            },
        }
    }
}

impl BackgroundNetworkRequestWillBeSentExtraInfoEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Network.requestWillBeSentExtraInfo",
            json!({
                "requestId": self.request_id,
                "associatedCookies": self.associated_cookies,
                "headers": self.headers,
                "connectTiming": { "requestTime": self.request_time },
                "cookieAccessReport": self.cookie_access_report,
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundNetworkResponseReceivedExtraInfoEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Network.responseReceivedExtraInfo",
            json!({
                "requestId": self.request_id,
                "blockedCookies": self.blocked_cookies,
                "headers": self.headers,
                "resourceIPAddressSpace": "Unknown",
                "statusCode": self.status_code,
                "cookieReports": self.cookie_reports,
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundNetworkWebSocketCreatedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Network.webSocketCreated",
            json!({
                "requestId": self.request_id,
                "url": self.url,
                "initiator": { "type": "script" },
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundNetworkWebSocketWillSendHandshakeRequestEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Network.webSocketWillSendHandshakeRequest",
            json!({
                "requestId": self.request_id,
                "timestamp": self.timestamp,
                "wallTime": self.timestamp,
                "request": {
                    "headers": self.headers,
                },
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundNetworkWebSocketHandshakeResponseReceivedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Network.webSocketHandshakeResponseReceived",
            json!({
                "requestId": self.request_id,
                "timestamp": self.timestamp,
                "response": {
                    "status": self.status,
                    "statusText": self.status_text,
                    "headers": self.headers,
                },
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundNetworkWebSocketFrameEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            network_websocket_frame_protocol_method(self.direction),
            json!({
                "requestId": self.request_id,
                "timestamp": self.timestamp,
                "response": {
                    "opcode": websocket_frame_opcode_number(self.opcode),
                    "mask": false,
                    "payloadData": "",
                    "payloadLength": self.payload_length,
                },
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

fn network_websocket_frame_protocol_method(direction: WebSocketFrameDirection) -> &'static str {
    match direction {
        WebSocketFrameDirection::Sent => "Network.webSocketFrameSent",
        WebSocketFrameDirection::Received => "Network.webSocketFrameReceived",
    }
}

fn websocket_frame_opcode_number(opcode: WebSocketFrameOpcode) -> u8 {
    match opcode {
        WebSocketFrameOpcode::Text => 1,
        WebSocketFrameOpcode::Binary => 2,
    }
}

impl BackgroundRuntimeBindingCalledEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Runtime.bindingCalled",
            json!({
                "name": self.name,
                "payload": self.payload,
                "executionContextId": self.execution_context_id,
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundRuntimeExecutionContextEvent {
    fn into_context_created_protocol_message(self) -> Value {
        build_event(
            "Runtime.executionContextCreated",
            runtime_context_created_protocol_params(self.event),
            self.session_id.as_deref(),
        )
    }

    fn into_context_created_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_context_created_protocol_message();
        (
            message,
            Some(AutomationEvent::RuntimeExecutionContextCreated(self.event)),
        )
    }

    fn into_context_destroyed_protocol_message(self) -> Value {
        build_event(
            "Runtime.executionContextDestroyed",
            runtime_context_destroyed_protocol_params(self.event),
            self.session_id.as_deref(),
        )
    }

    fn into_context_destroyed_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_context_destroyed_protocol_message();
        (
            message,
            Some(AutomationEvent::RuntimeExecutionContextDestroyed(
                self.event,
            )),
        )
    }
}

impl BackgroundRuntimeExecutionContextsClearedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Runtime.executionContextsCleared",
            runtime_contexts_cleared_protocol_params(self.event),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        (
            message,
            Some(AutomationEvent::RuntimeExecutionContextsCleared(self.event)),
        )
    }
}

fn runtime_context_created_protocol_params(event: RuntimeExecutionContextEvent) -> Value {
    let mut aux_data = serde_json::Map::new();
    aux_data.insert(
        "isDefault".to_owned(),
        json!(event.is_default.unwrap_or(false)),
    );
    aux_data.insert(
        "type".to_owned(),
        json!(event.context_type.unwrap_or_else(|| "default".to_owned())),
    );
    if let Some(frame_id) = event.frame_id.as_ref() {
        aux_data.insert("frameId".to_owned(), json!(frame_id.as_str()));
    }
    if let Some(grant_universal_access) = event.grant_universal_access {
        aux_data.insert(
            "grantUniversalAccess".to_owned(),
            json!(grant_universal_access),
        );
    }
    json!({
        "context": {
            "id": event.context_id.unwrap_or_default(),
            "origin": event.origin.unwrap_or_default(),
            "name": event.name.unwrap_or_default(),
            "uniqueId": event.realm_id.as_ref().map(DevToolsRealmId::as_str).unwrap_or_default(),
            "auxData": Value::Object(aux_data),
        }
    })
}

fn runtime_context_destroyed_protocol_params(event: RuntimeExecutionContextEvent) -> Value {
    json!({
        "executionContextId": event.context_id.unwrap_or_default(),
        "executionContextUniqueId": event.realm_id.as_ref().map(DevToolsRealmId::as_str).unwrap_or_default(),
    })
}

fn runtime_contexts_cleared_protocol_params(_event: RuntimeExecutionContextsClearedEvent) -> Value {
    json!({})
}

impl BackgroundDomSetChildNodesEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "DOM.setChildNodes",
            json!({
                "parentId": self.parent_node_id,
                "nodes": self.nodes,
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        let event = DomSetChildNodesEvent {
            parent_node_id: self.parent_node_id,
            nodes: self.nodes,
        };
        (message, Some(AutomationEvent::DomSetChildNodes(event)))
    }
}

impl BackgroundPageNavigationFrameEvent {
    fn into_protocol_message(self) -> Value {
        let Self {
            session_id,
            event,
            unreachable_url,
        } = self;
        build_event(
            navigation_frame_protocol_method(event.kind),
            navigation_frame_protocol_params(event, unreachable_url.as_deref()),
            session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        (message, Some(AutomationEvent::NavigationFrame(self.event)))
    }
}

impl BackgroundNavigationLifecycleEvent {
    fn into_dom_content_loaded_protocol_message(self) -> Value {
        build_event(
            "Page.domContentEventFired",
            navigation_lifecycle_protocol_params(self.event),
            self.session_id.as_deref(),
        )
    }

    fn into_dom_content_loaded_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_dom_content_loaded_protocol_message();
        (message, Some(AutomationEvent::DomContentLoaded(self.event)))
    }

    fn into_load_protocol_message(self) -> Value {
        build_event(
            "Page.loadEventFired",
            navigation_lifecycle_protocol_params(self.event),
            self.session_id.as_deref(),
        )
    }

    fn into_load_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_load_protocol_message();
        (message, Some(AutomationEvent::Load(self.event)))
    }
}

impl BackgroundPageLifecycleEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Page.lifecycleEvent",
            json!({
                "frameId": self.event.frame_id.as_str(),
                "loaderId": self.event.loader_id.as_str(),
                "name": self.event.name,
                "timestamp": self.event.timestamp,
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        (message, Some(AutomationEvent::PageLifecycle(self.event)))
    }
}

impl BackgroundSameDocumentNavigationEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Page.navigatedWithinDocument",
            json!({
                "frameId": self.event.frame_id.as_str(),
                "url": self.event.url.as_str(),
                "navigationType": self.event.navigation_type.as_str(),
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        (
            message,
            Some(AutomationEvent::SameDocumentNavigation(self.event)),
        )
    }
}

impl BackgroundPageFrameAttachedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Page.frameAttached",
            json!({
                "frameId": self.frame_id,
                "parentFrameId": self.parent_frame_id,
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundPageDocumentOpenedEvent {
    fn into_protocol_message(self) -> Value {
        let mut frame = json!({
            "id": self.frame_id,
            "loaderId": self.loader_id,
            "url": self.url,
            "domainAndRegistry": "",
            "securityOrigin": self.security_origin,
            "mimeType": "text/html",
            "adFrameStatus": { "adFrameType": "none" },
            "secureContextType": self.secure_context_type,
            "crossOriginIsolatedContextType": "NotIsolated",
            "gatedAPIFeatures": [],
        });
        if let Some(frame_name) = self.frame_name {
            frame["name"] = json!(frame_name);
        }
        if let Some(parent_frame_id) = self.parent_frame_id {
            frame["parentId"] = json!(parent_frame_id);
        }
        build_event(
            "Page.documentOpened",
            json!({ "frame": frame }),
            self.session_id.as_deref(),
        )
    }
}

impl BackgroundPageFrameDetachedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Page.frameDetached",
            json!({
                "frameId": self.frame_id,
                "reason": "remove",
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

fn navigation_frame_protocol_method(kind: NavigationFrameEventKind) -> &'static str {
    match kind {
        NavigationFrameEventKind::Scheduled => "Page.frameScheduledNavigation",
        NavigationFrameEventKind::Requested => "Page.frameRequestedNavigation",
        NavigationFrameEventKind::StartedNavigating => "Page.frameStartedNavigating",
        NavigationFrameEventKind::StartedLoading => "Page.frameStartedLoading",
        NavigationFrameEventKind::ClearedScheduled => "Page.frameClearedScheduledNavigation",
        NavigationFrameEventKind::Navigated => "Page.frameNavigated",
        NavigationFrameEventKind::DocumentUpdated => "DOM.documentUpdated",
        NavigationFrameEventKind::StoppedLoading => "Page.frameStoppedLoading",
    }
}

fn navigation_frame_protocol_params(
    event: NavigationFrameEvent,
    unreachable_url: Option<&str>,
) -> Value {
    match event.kind {
        NavigationFrameEventKind::Scheduled => json!({
            "frameId": event.frame_id.as_str(),
            "delay": 0,
            "reason": "scriptInitiated",
            "url": event.url,
        }),
        NavigationFrameEventKind::Requested => json!({
            "frameId": event.frame_id.as_str(),
            "reason": "scriptInitiated",
            "url": event.url,
            "disposition": "currentTab",
        }),
        NavigationFrameEventKind::StartedNavigating => json!({
            "frameId": event.frame_id.as_str(),
            "url": event.url,
            "loaderId": event.loader_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
            "navigationType": "differentDocument",
        }),
        NavigationFrameEventKind::StartedLoading => {
            json!({ "frameId": event.frame_id.as_str() })
        }
        NavigationFrameEventKind::ClearedScheduled => {
            json!({ "frameId": event.frame_id.as_str() })
        }
        NavigationFrameEventKind::Navigated => {
            let mut frame = json!({
                "id": event.frame_id.as_str(),
                "loaderId": event.loader_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
                "url": event.url,
                "domainAndRegistry": "",
                "securityOrigin": event.security_origin.unwrap_or_default(),
                "mimeType": "text/html",
                "adFrameStatus": { "adFrameType": "none" },
                "secureContextType": event.secure_context_type.unwrap_or_default(),
                "crossOriginIsolatedContextType": "NotIsolated",
                "gatedAPIFeatures": [],
            });
            if let Some(parent_frame_id) = event.parent_frame_id {
                frame["parentId"] = json!(parent_frame_id.as_str());
            }
            if let Some(frame_name) = event.frame_name {
                frame["name"] = json!(frame_name);
            }
            if let Some(unreachable_url) = unreachable_url {
                frame["unreachableUrl"] = json!(unreachable_url);
            }
            json!({
                "type": "Navigation",
                "frame": frame,
            })
        }
        NavigationFrameEventKind::DocumentUpdated => json!({}),
        NavigationFrameEventKind::StoppedLoading => {
            json!({ "frameId": event.frame_id.as_str() })
        }
    }
}

fn navigation_lifecycle_protocol_params(event: NavigationLifecycleEvent) -> Value {
    json!({ "timestamp": event.timestamp })
}

impl BackgroundPageFileChooserOpenedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Page.fileChooserOpened",
            json!({
                "frameId": self.frame_id,
                "mode": self.mode,
                "backendNodeId": self.backend_node_id,
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        let event = PageFileChooserOpenedEvent {
            frame_id: DevToolsFrameId::from(self.frame_id.as_str()),
            mode: self.mode,
            backend_node_id: self.backend_node_id,
            element_shared_id: self.element_shared_id,
        };
        (message, Some(AutomationEvent::PageFileChooserOpened(event)))
    }
}

impl BackgroundBrowserDownloadWillBeginEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Browser.downloadWillBegin",
            json!({
                "frameId": self.frame_id,
                "guid": self.guid,
                "url": self.url,
                "suggestedFilename": self.suggested_filename,
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundBrowserDownloadProgressEvent {
    fn into_protocol_message(self) -> Value {
        let mut params = json!({
            "guid": self.guid,
            "state": self.state,
            "receivedBytes": self.received_bytes,
            "totalBytes": self.total_bytes,
        });
        if let Some(file_path) = self.file_path {
            params["filePath"] = json!(file_path);
        }
        build_event(
            "Browser.downloadProgress",
            params,
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundPageJavaScriptDialogOpeningEvent {
    fn into_protocol_message(self) -> Value {
        let mut params = json!({
            "url": self.url,
            "message": self.message,
            "type": self.dialog_type,
            "hasBrowserHandler": self.has_browser_handler,
            "defaultPrompt": self.default_prompt,
        });
        if let Some(frame_id) = self.frame_id {
            params["frameId"] = json!(frame_id);
        }
        build_event(
            "Page.javascriptDialogOpening",
            params,
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        let event = PageJavaScriptDialogOpeningEvent {
            frame_id: self.frame_id.as_deref().map(DevToolsFrameId::from),
            url: self.url,
            message: self.message,
            dialog_type: self.dialog_type,
            has_browser_handler: self.has_browser_handler,
            default_prompt: self.default_prompt,
        };
        (
            message,
            Some(AutomationEvent::PageJavaScriptDialogOpening(event)),
        )
    }
}

impl BackgroundPageJavaScriptDialogClosedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Page.javascriptDialogClosed",
            json!({
                "frameId": self.frame_id,
                "result": self.accepted,
                "userInput": self.user_text,
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        let event = UserPromptClosedEvent {
            target_id: self.target_id.as_deref().map(Into::into),
            frame_id: self.frame_id.as_str().into(),
            prompt_type: self.prompt_type,
            accepted: self.accepted,
            user_text: self.user_text,
        };
        (message, Some(AutomationEvent::UserPromptClosed(event)))
    }
}

impl BackgroundPageScreencastVisibilityChangedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Page.screencastVisibilityChanged",
            json!({
                "visible": self.visible,
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundPageScreencastFrameEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Page.screencastFrame",
            json!({
                "data": self.data,
                "metadata": {
                    "offsetTop": self.metadata.offset_top,
                    "pageScaleFactor": self.metadata.page_scale_factor,
                    "deviceWidth": self.metadata.device_width,
                    "deviceHeight": self.metadata.device_height,
                    "scrollOffsetX": self.metadata.scroll_offset_x,
                    "scrollOffsetY": self.metadata.scroll_offset_y,
                    "timestamp": self.metadata.timestamp,
                },
                "sessionId": self.generation,
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundInspectorTargetCrashedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Inspector.targetCrashed",
            json!({}),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundInspectorTargetReloadedAfterCrashEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Inspector.targetReloadedAfterCrash",
            json!({}),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundInspectorDetachedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Inspector.detached",
            json!({
                "reason": self.reason,
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundServiceWorkerRegistrationUpdatedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "ServiceWorker.workerRegistrationUpdated",
            json!({
                "registrations": self
                    .registrations
                    .into_iter()
                    .map(BackgroundServiceWorkerRegistration::into_protocol_value)
                    .collect::<Vec<_>>(),
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundServiceWorkerVersionUpdatedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "ServiceWorker.workerVersionUpdated",
            json!({
                "versions": self
                    .versions
                    .into_iter()
                    .map(BackgroundServiceWorkerVersion::into_protocol_value)
                    .collect::<Vec<_>>(),
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundServiceWorkerErrorReportedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "ServiceWorker.workerErrorReported",
            json!({
                "errorMessage": self.error_message.into_protocol_value(),
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundServiceWorkerRegistration {
    fn into_protocol_value(self) -> Value {
        json!({
            "registrationId": self.registration_id,
            "scopeURL": self.scope_url,
            "isDeleted": self.is_deleted,
        })
    }
}

impl BackgroundServiceWorkerVersion {
    fn into_protocol_value(self) -> Value {
        json!({
            "versionId": self.version_id,
            "registrationId": self.registration_id,
            "scriptURL": self.script_url,
            "runningStatus": self.running_status,
            "status": self.status,
            "controlledClients": self.controlled_clients,
            "targetId": self.target_id,
        })
    }
}

impl BackgroundServiceWorkerErrorMessage {
    fn into_protocol_value(self) -> Value {
        json!({
            "errorMessage": self.error_message,
            "registrationId": self.registration_id,
            "versionId": self.version_id,
            "sourceURL": self.source_url,
            "lineNumber": self.line_number,
            "columnNumber": self.column_number,
        })
    }
}

impl BackgroundTargetInfoChangedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Target.targetInfoChanged",
            json!({
                "targetInfo": self.target_info.into_cdp_value(),
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundTargetCreatedEvent {
    fn into_protocol_message(self) -> Value {
        let Self { session_id, event } = self;
        let target_info = event.target_info.unwrap_or_else(|| DevToolsTargetInfo {
            target_id: Some(event.target_id),
            kind: event.kind,
            title: String::new(),
            url: event.url,
            attached: false,
            opener_id: None,
            opener_frame_id: None,
            can_access_opener: false,
            browser_context_id: event.browser_context_id,
            moli_popup_id: None,
        });
        build_event(
            "Target.targetCreated",
            json!({ "targetInfo": target_info.into_cdp_value() }),
            session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        (message, Some(AutomationEvent::TargetCreated(self.event)))
    }
}

impl BackgroundTargetAttachedEvent {
    fn into_protocol_message(self) -> Value {
        let event = self.event;
        let parent_session_id = event.parent_session_id;
        build_event(
            "Target.attachedToTarget",
            json!({
                "sessionId": event.session_id.as_str(),
                "targetInfo": event.target_info.into_cdp_value(),
                "waitingForDebugger": event.waiting_for_debugger,
            }),
            parent_session_id.as_ref().map(|id| id.as_str()),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        (message, Some(AutomationEvent::TargetAttached(self.event)))
    }
}

impl BackgroundTargetDetachedEvent {
    fn into_protocol_message(self) -> Value {
        let event = self.event;
        let params = json!({
            "targetId": event.target_id.as_str(),
            "sessionId": event.session_id.as_str(),
        });
        build_event(
            "Target.detachedFromTarget",
            params,
            event.parent_session_id.as_ref().map(|id| id.as_str()),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        (message, Some(AutomationEvent::TargetDetached(self.event)))
    }
}

impl BackgroundTargetDestroyedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Target.targetDestroyed",
            json!({
                "targetId": self.event.target_id.as_str(),
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        (message, Some(AutomationEvent::TargetDestroyed(self.event)))
    }
}

impl BackgroundTargetCrashedEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Target.targetCrashed",
            json!({
                "targetId": self.target_id,
                "status": self.status,
                "errorCode": self.error_code,
            }),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundTargetReceivedMessageFromTargetEvent {
    fn into_protocol_message(self) -> Value {
        let nested_message = self.nested_event.into_protocol_message();
        build_event(
            "Target.receivedMessageFromTarget",
            json!({
                "message": nested_message.to_string(),
                "sessionId": self.target_session_id,
            }),
            None,
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        (self.into_protocol_message(), None)
    }
}

impl BackgroundConsoleMessageAddedEvent {
    fn into_protocol_message(self) -> Value {
        build_console_message_added_protocol_message(
            self.session_id.as_deref(),
            &self.source,
            &self.level,
            &self.text,
            &self.url,
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        let event = RuntimeConsoleEvent {
            target_id: None,
            console_type: self.level,
            text: self.text.clone(),
            args: vec![json!({
                "type": "string",
                "value": self.text,
            })],
            stack: None,
            stack_trace: None,
            execution_context_id: None,
            timestamp: None,
        };
        (
            message,
            Some(AutomationEvent::RuntimeConsoleApiCalled(event)),
        )
    }
}

impl BackgroundLogEntryAddedEvent {
    fn into_protocol_message(self) -> Value {
        build_log_entry_added_protocol_message(
            self.session_id.as_deref(),
            &self.source,
            &self.level,
            &self.text,
            &self.url,
            self.timestamp,
            self.network_request_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        let event = LogEntryEvent {
            target_id: None,
            source: self.source,
            level: self.level,
            text: self.text,
            url: Some(self.url),
            timestamp: Some(self.timestamp),
            network_request_id: self.network_request_id,
            args: Vec::new(),
        };
        (message, Some(AutomationEvent::LogEntryAdded(event)))
    }
}

impl BackgroundRuntimeConsoleApiCalledEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Runtime.consoleAPICalled",
            runtime_console_api_called_protocol_params(self.event),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        (
            message,
            Some(AutomationEvent::RuntimeConsoleApiCalled(self.event)),
        )
    }
}

impl BackgroundRuntimeExceptionThrownEvent {
    fn into_protocol_message(self) -> Value {
        build_event(
            "Runtime.exceptionThrown",
            runtime_exception_thrown_protocol_params(self.event),
            self.session_id.as_deref(),
        )
    }

    fn into_parts(self) -> (Value, Option<AutomationEvent>) {
        let message = self.clone().into_protocol_message();
        (message, Some(AutomationEvent::ScriptException(self.event)))
    }
}

fn runtime_console_api_called_protocol_params(event: RuntimeConsoleEvent) -> Value {
    let mut params = json!({
        "type": event.console_type,
        "args": event.args,
        "executionContextId": event.execution_context_id.unwrap_or_default(),
        "timestamp": event.timestamp.unwrap_or_default(),
    });
    if let Some(stack_trace) = event.stack_trace
        && let Some(params) = params.as_object_mut()
    {
        params.insert("stackTrace".to_owned(), stack_trace.into_cdp_value());
    }
    params
}

fn runtime_exception_thrown_protocol_params(event: ScriptExceptionEvent) -> Value {
    let exception = *event.exception;
    let exception_id = exception
        .exception_id
        .or_else(|| event.exception_index.map(|index| (index + 1) as u64))
        .unwrap_or_default();
    let script_id = exception.script_id.clone().unwrap_or_default();
    let line_number = exception.line_number.unwrap_or_default();
    let column_number = exception.column_number.unwrap_or_default();
    let exception_text = exception.text.clone();
    let stack_trace = exception.stack_trace.clone();
    let mut details = json!({
        "exceptionId": exception_id,
        "text": exception_text,
        "lineNumber": line_number,
        "columnNumber": column_number,
        "scriptId": script_id,
        "url": event.url.unwrap_or_default(),
        "executionContextId": event.execution_context_id.unwrap_or_default(),
        "exception": runtime_exception_remote_object(exception),
    });
    if let Some(stack_trace) = stack_trace
        && let Some(details) = details.as_object_mut()
    {
        details.insert("stackTrace".to_owned(), stack_trace.into_cdp_value());
    }
    json!({
        "timestamp": event.timestamp.unwrap_or_default(),
        "exceptionDetails": details,
    })
}

fn runtime_exception_remote_object(
    exception: crate::devtools_runtime::DevToolsScriptException,
) -> Value {
    exception
        .value
        .map(crate::cdp_projection::remote_object_from_devtools)
        .unwrap_or_else(|| {
            json!({
                "type": "object",
                "subtype": "error",
                "className": "Error",
                "description": exception.text,
            })
        })
}

fn build_console_message_added_protocol_message(
    session_id: Option<&str>,
    source: &str,
    level: &str,
    text: &str,
    url: &str,
) -> Value {
    build_event(
        "Console.messageAdded",
        json!({
            "message": {
                "source": source,
                "level": level,
                "text": text,
                "url": url,
                "line": 0,
                "column": 0,
            }
        }),
        session_id,
    )
}

fn build_log_entry_added_protocol_message(
    session_id: Option<&str>,
    source: &str,
    level: &str,
    text: &str,
    url: &str,
    timestamp: f64,
    network_request_id: Option<&str>,
) -> Value {
    let mut entry = json!({
        "source": source,
        "level": level,
        "text": text,
        "timestamp": timestamp,
        "url": url,
    });
    if let Some(network_request_id) = network_request_id {
        entry["networkRequestId"] = json!(network_request_id);
    }
    build_event(
        "Log.entryAdded",
        json!({
            "entry": entry,
        }),
        session_id,
    )
}

fn strip_moli_private_protocol_fields(mut message: Value) -> Value {
    let preserve_blocked_intercepts =
        message.get("method").and_then(Value::as_str) == Some("Fetch.requestPaused");
    if let Some(params) = message.get_mut("params").and_then(Value::as_object_mut) {
        if !preserve_blocked_intercepts {
            params.remove("__moliBlockedInterceptors");
        }
        params.remove("__moliFetchRequestId");
    }
    message
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NavigationBackgroundEvent {
    token: DocumentNavigationToken,
    event: BackgroundProtocolEvent,
}

impl NavigationBackgroundEvent {
    #[cfg(test)]
    pub(crate) fn protocol_message(token: DocumentNavigationToken, message: Value) -> Self {
        Self {
            token,
            event: BackgroundProtocolEvent::immediate(message),
        }
    }

    pub(crate) fn background_event(
        token: DocumentNavigationToken,
        event: BackgroundProtocolEvent,
    ) -> Self {
        Self { token, event }
    }

    pub(crate) fn into_background_protocol_event_if_current<'a>(
        self,
        browser_contexts: impl IntoIterator<Item = &'a BrowserContext>,
    ) -> Option<BackgroundProtocolEvent> {
        let is_current = browser_contexts.into_iter().any(|browser_context| {
            browser_context.accepts_pending_document_navigation_event(&self.token)
        });
        if !is_current {
            return None;
        }
        Some(self.event)
    }

    #[cfg(test)]
    pub(crate) fn into_protocol_message_if_current<'a>(
        self,
        browser_contexts: impl IntoIterator<Item = &'a BrowserContext>,
    ) -> Option<Value> {
        self.into_background_protocol_event_if_current(browser_contexts)
            .map(BackgroundProtocolEvent::into_protocol_message)
    }
}

pub fn build_event(method: &str, params: Value, session_id: Option<&str>) -> Value {
    let mut v = json!({ "method": method, "params": params });
    if let Some(sid) = session_id {
        v["sessionId"] = json!(sid);
    }
    v
}

#[cfg(test)]
mod tests {
    use crate::devtools_runtime::{
        AutomationEvent, DevToolsBrowserContextId, DevToolsFrameId, DevToolsLoaderId,
        DevToolsNetworkResourceType, DevToolsSessionId, DevToolsTargetId, DevToolsTargetInfo,
        DevToolsTargetKind, NavigationFrameEvent, NavigationFrameEventKind,
        NavigationLifecycleEvent, RuntimeExecutionContextEvent,
        RuntimeExecutionContextsClearedEvent, TargetAttachmentEvent, TargetDetachmentEvent,
        TargetLifecycleEvent, UserPromptClosedEvent,
    };
    use serde_json::{Value, json};

    use super::{
        BackgroundCommandResponsePayload, BackgroundProtocolEvent,
        BackgroundServiceWorkerErrorMessage, BackgroundServiceWorkerRegistration,
        BackgroundServiceWorkerVersion, NavigationBackgroundEvent, PageScreencastFrameMetadata,
        RuntimeInspectorResponseReady, build_event,
    };
    use crate::conn::{BrowserContext, CdpConnection, DevToolsDocumentLifecycleWaitKey};
    use moli_core::{
        PageId, RendererRuntimeInspectorAsyncCompletion,
        page::{
            RendererAgentAttachmentId, RendererDocumentLifecycleMilestone, RendererDocumentToken,
            RendererLifecycleEpoch, RendererRuntimeCommandOutput, RendererRuntimeInspectorMessage,
            WebSocketFrameDirection, WebSocketFrameOpcode,
        },
    };
    use moli_page_types::RendererCallId;

    #[test]
    fn owned_runtime_response_moves_large_result_without_reallocating_its_strings() {
        let message = json!({
            "id": 17,
            "result": {
                "result": {
                    "type": "object",
                    "value": ["x".repeat(1024 * 1024)]
                }
            }
        });
        let original = message
            .pointer("/result/result/value/0")
            .and_then(Value::as_str)
            .expect("large Runtime result string");
        let original_ptr = original.as_ptr();

        let payload =
            BackgroundCommandResponsePayload::from_owned_runtime_inspector_message(message);
        let event = BackgroundProtocolEvent::command_response(Some(17), None, payload);
        let (_, _, payload) = event
            .into_command_response_payload()
            .expect("typed command response");
        let BackgroundCommandResponsePayload::Success { result } = payload else {
            panic!("owned Runtime result should remain successful");
        };
        let moved = result
            .pointer("/result/value/0")
            .and_then(Value::as_str)
            .expect("moved Runtime result string");

        assert_eq!(moved.as_ptr(), original_ptr);
        assert_eq!(moved.len(), 1024 * 1024);
    }

    #[test]
    fn page_download_envelope_rejects_disabled_and_reenabled_subscription() {
        let mut conn = CdpConnection::new();
        conn.install_default_browser_target();
        assert!(conn.set_page_domain_enabled_for_session_owner(None, true));
        let first_generation = conn
            .page_domain_subscription_generation_for_session_owner(None)
            .expect("Page.enable should create a subscription generation");
        let event = BackgroundProtocolEvent::page_download_progress(
            None,
            first_generation,
            "GUID-download",
            "inProgress",
            1,
            2,
        );
        assert!(event.route_is_current(&conn));

        assert!(conn.disable_page_domain_for_session_owner(None));
        assert!(!event.route_is_current(&conn));
        conn.with_target_devtools_session_state_for_session_mut(None, |state| {
            state.page_session_state = Default::default();
        })
        .expect("default target session state");

        assert!(conn.set_page_domain_enabled_for_session_owner(None, true));
        assert!(
            !event.route_is_current(&conn),
            "re-enabling Page must not resume an old observer after default state was collapsed"
        );
        let second_generation = conn
            .page_domain_subscription_generation_for_session_owner(None)
            .expect("re-enabled Page domain should expose a generation");
        assert_ne!(second_generation, first_generation);
        let current_event = BackgroundProtocolEvent::page_download_progress(
            None,
            second_generation,
            "GUID-current",
            "completed",
            2,
            2,
        );
        assert!(current_event.route_is_current(&conn));
    }

    #[test]
    fn browser_download_route_guard_rejects_detached_and_reenabled_subscription() {
        let mut conn = CdpConnection::new();
        conn.download_behavior
            .set_browser_events_enabled_for_session(Some("SID-browser"), true);
        let first_generation = conn.download_behavior.browser_event_observers()[0].1;
        let event = BackgroundProtocolEvent::browser_download_progress(
            Some("SID-browser"),
            Some(first_generation),
            "GUID-download",
            "inProgress",
            1,
            2,
            None,
        );
        assert!(event.route_is_current(&conn));

        conn.download_behavior
            .set_browser_events_enabled_for_session(Some("SID-browser"), false);
        assert!(!event.route_is_current(&conn));

        conn.download_behavior
            .set_browser_events_enabled_for_session(Some("SID-browser"), true);
        let second_generation = conn.download_behavior.browser_event_observers()[0].1;
        assert_ne!(second_generation, first_generation);
        assert!(
            !event.route_is_current(&conn),
            "re-enabling Browser download events must not revive an old async completion"
        );
        let current_event = BackgroundProtocolEvent::browser_download_progress(
            Some("SID-browser"),
            Some(second_generation),
            "GUID-current",
            "completed",
            2,
            2,
            Some("/tmp/GUID-current"),
        );
        assert!(current_event.route_is_current(&conn));
    }

    #[test]
    fn renderer_page_load_matches_wait_key_by_document_and_epoch() {
        let page_id = PageId::new_for_testing(902);
        let document = RendererDocumentToken::new_for_testing(page_id, 3);
        let event = NavigationLifecycleEvent {
            target_id: DevToolsTargetId::from("FRAME"),
            frame_id: DevToolsFrameId::from("FRAME"),
            navigation_id: None,
            loader_id: Some(DevToolsLoaderId::from("LOADER")),
            url: String::new(),
            timestamp: 1.0,
        };
        let output = BackgroundProtocolEvent::page_load_for_renderer_document(
            None,
            event,
            document,
            RendererLifecycleEpoch(4),
        );
        let matching = DevToolsDocumentLifecycleWaitKey {
            registration_id: crate::conn::state::RendererDocumentLifecycleWaiterId::new_for_test(1),
            renderer_document: document,
            renderer_epoch: RendererLifecycleEpoch(4),
            milestone: RendererDocumentLifecycleMilestone::Load,
            frame_id: "FRAME".to_owned(),
            loader_id: "LOADER".to_owned(),
        };
        let restarted_epoch = DevToolsDocumentLifecycleWaitKey {
            renderer_epoch: RendererLifecycleEpoch(5),
            ..matching.clone()
        };

        assert!(output.matches_document_load_wait_key(&matching));
        assert!(!output.matches_document_load_wait_key(&restarted_epoch));
    }

    #[test]
    fn navigation_background_event_materializes_only_for_current_token() {
        let mut browser_context = BrowserContext::new("CTX-nav".to_owned());
        browser_context.set_active_target_id("TID-nav");
        browser_context.attach_active_session("SID-nav");
        let token = browser_context
            .start_document_navigation_for_active_target("LOADER-1".to_owned())
            .expect("active target should produce navigation token");
        let message = build_event(
            "Page.frameStartedLoading",
            json!({ "frameId": "TID-nav" }),
            Some("SID-nav"),
        );

        let event = NavigationBackgroundEvent::protocol_message(token, message.clone());

        assert_eq!(
            event.into_protocol_message_if_current(std::iter::once(&browser_context)),
            Some(message)
        );
    }

    #[test]
    fn navigation_background_event_preserves_typed_sidecar_for_current_token() {
        let mut browser_context = BrowserContext::new("CTX-nav".to_owned());
        browser_context.set_active_target_id("TID-nav");
        browser_context.attach_active_session("SID-nav");
        let token = browser_context
            .start_document_navigation_for_active_target("LOADER-1".to_owned())
            .expect("active target should produce navigation token");
        let message = build_event(
            "Page.frameStartedNavigating",
            json!({
                "frameId": "TID-nav",
                "loaderId": "LOADER-1",
                "url": "https://example.test/",
                "navigationType": "differentDocument"
            }),
            Some("SID-nav"),
        );
        let automation_event = AutomationEvent::NavigationFrame(NavigationFrameEvent {
            target_id: DevToolsTargetId::from("TID-nav"),
            frame_id: DevToolsFrameId::from("TID-nav"),
            parent_frame_id: None,
            loader_id: Some(DevToolsLoaderId::from("LOADER-1")),
            url: "https://example.test/".to_owned(),
            kind: NavigationFrameEventKind::StartedNavigating,
            frame_name: None,
            security_origin: None,
            secure_context_type: None,
        });

        let event = NavigationBackgroundEvent::background_event(
            token,
            BackgroundProtocolEvent::immediate_automation_event(
                message.clone(),
                automation_event.clone(),
            ),
        );
        let background_event = event
            .into_background_protocol_event_if_current(std::iter::once(&browser_context))
            .expect("current navigation event should materialize");
        let (actual_message, actual_automation_event) = background_event.into_parts();

        assert_eq!(actual_message, message);
        assert_eq!(actual_automation_event, Some(automation_event));
    }

    #[test]
    fn runtime_inspector_negative_call_id_preserves_error_payload() {
        let response = RuntimeInspectorResponseReady::new_correlated(
            42,
            None,
            RendererCallId::new(-1),
            Ok(
                RendererRuntimeInspectorAsyncCompletion::from_protocol_message(
                    -1,
                    json!({
                        "id": -1,
                        "error": {
                            "code": -32001,
                            "message": "renderer rejected command",
                            "data": { "reason": "negative call id" }
                        }
                    }),
                ),
            ),
        );

        let payload = response.clone().into_command_response_payload();
        let BackgroundCommandResponsePayload::Error {
            code,
            message,
            data,
        } = payload
        else {
            panic!("negative call id error completion should stay an error payload");
        };
        assert_eq!(code, -32001);
        assert_eq!(message, "renderer rejected command");
        assert_eq!(data, Some(json!({ "reason": "negative call id" })));

        let message = response.into_protocol_message_for_typed_runtime_route();
        assert_eq!(message["id"], json!(42));
        assert_eq!(message["error"]["code"], json!(-32001));
        assert_eq!(
            message["error"]["message"],
            json!("renderer rejected command")
        );
        assert_eq!(
            message["error"]["data"],
            json!({ "reason": "negative call id" })
        );
    }

    #[test]
    fn runtime_inspector_generated_error_output_preserves_renderer_attachment() {
        let attachment_id = RendererAgentAttachmentId::allocate();
        let mut output = RendererRuntimeCommandOutput::from_inspector_message(
            RendererRuntimeInspectorMessage::protocol(json!({
                "method": "Runtime.consoleAPICalled",
                "params": {},
            })),
        );
        output.bind_renderer_agent_attachment(attachment_id);
        let response = RuntimeInspectorResponseReady::new_correlated(
            42,
            None,
            RendererCallId::new(42),
            Ok(RendererRuntimeInspectorAsyncCompletion::from_command_output(42, output)),
        );

        let (command_id, output, renderer_output_predecessor) =
            response.into_renderer_command_output();

        assert_eq!(command_id, 42);
        assert_eq!(renderer_output_predecessor, None);
        assert_eq!(output.renderer_agent_attachment_id(), Some(attachment_id));
        assert_eq!(
            output
                .protocol_response(42)
                .expect("generated error output should contain the frontend response")["error"]["message"],
            json!("RuntimeInspectorResponseMissingProtocolResponse")
        );
    }

    #[test]
    fn runtime_inspector_response_restores_large_frontend_command_id() {
        let frontend_command_id = i32::MAX as u64 + 73;
        let response = RuntimeInspectorResponseReady::new_correlated(
            frontend_command_id,
            Some("SID-large-id"),
            RendererCallId::new(11),
            Ok(
                RendererRuntimeInspectorAsyncCompletion::from_protocol_message(
                    11,
                    json!({
                        "id": 11,
                        "result": { "result": { "type": "number", "value": 42 } }
                    }),
                ),
            ),
        );

        let message = response.into_protocol_message_for_typed_runtime_route();

        assert_eq!(message["id"], json!(frontend_command_id));
        assert_eq!(message["result"]["result"]["value"], json!(42));
    }

    #[test]
    fn runtime_binding_call_stays_typed_until_wire_projection() {
        let mut event =
            BackgroundProtocolEvent::runtime_binding_called(None, "bindingName", "payload", 42);

        assert!(event.protocol_message().is_none());
        assert!(event.has_protocol_wire_message());

        event.ensure_protocol_session_id(Some("SID-runtime"));
        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], json!("Runtime.bindingCalled"));
        assert_eq!(message["sessionId"], json!("SID-runtime"));
        assert_eq!(message["params"]["name"], json!("bindingName"));
        assert_eq!(message["params"]["payload"], json!("payload"));
        assert_eq!(message["params"]["executionContextId"], json!(42));
        assert_eq!(automation_event, None);
    }

    #[test]
    fn page_frame_attached_stays_typed_until_wire_projection() {
        let mut event =
            BackgroundProtocolEvent::page_frame_attached(None, "CHILD-FRAME", "PARENT-FRAME");

        assert!(event.protocol_message().is_none());
        assert_eq!(event.protocol_method(), Some("Page.frameAttached"));
        assert!(event.has_protocol_wire_message());

        event.ensure_protocol_session_id(Some("SID-page"));
        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], json!("Page.frameAttached"));
        assert_eq!(message["sessionId"], json!("SID-page"));
        assert_eq!(message["params"]["frameId"], json!("CHILD-FRAME"));
        assert_eq!(message["params"]["parentFrameId"], json!("PARENT-FRAME"));
        assert_eq!(automation_event, None);
    }

    #[test]
    fn page_frame_detached_stays_typed_until_wire_projection() {
        let mut event = BackgroundProtocolEvent::page_frame_detached(None, "CHILD-FRAME");

        assert!(event.protocol_message().is_none());
        assert_eq!(event.protocol_method(), Some("Page.frameDetached"));
        assert!(event.has_protocol_wire_message());

        event.ensure_protocol_session_id(Some("SID-page"));
        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], json!("Page.frameDetached"));
        assert_eq!(message["sessionId"], json!("SID-page"));
        assert_eq!(message["params"]["frameId"], json!("CHILD-FRAME"));
        assert_eq!(message["params"]["reason"], json!("remove"));
        assert_eq!(automation_event, None);
    }

    #[test]
    fn javascript_dialog_closed_stays_typed_until_wire_projection() {
        let event = BackgroundProtocolEvent::page_javascript_dialog_closed(
            Some("SID-dialog"),
            UserPromptClosedEvent {
                target_id: Some(DevToolsTargetId::from("TID-dialog")),
                frame_id: DevToolsFrameId::from("FRAME-dialog"),
                prompt_type: "prompt".to_owned(),
                accepted: true,
                user_text: "typed response".to_owned(),
            },
        );

        assert!(event.protocol_message().is_none());
        assert!(event.has_protocol_wire_message());

        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], json!("Page.javascriptDialogClosed"));
        assert_eq!(message["sessionId"], json!("SID-dialog"));
        assert_eq!(message["params"]["frameId"], json!("FRAME-dialog"));
        assert_eq!(message["params"]["result"], json!(true));
        assert_eq!(message["params"]["userInput"], json!("typed response"));
        assert_eq!(
            automation_event,
            Some(AutomationEvent::UserPromptClosed(UserPromptClosedEvent {
                target_id: Some(DevToolsTargetId::from("TID-dialog")),
                frame_id: DevToolsFrameId::from("FRAME-dialog"),
                prompt_type: "prompt".to_owned(),
                accepted: true,
                user_text: "typed response".to_owned(),
            }))
        );
    }

    #[test]
    fn screencast_visibility_stays_typed_until_wire_projection() {
        let event =
            BackgroundProtocolEvent::page_screencast_visibility_changed(Some("SID-page"), true);

        assert!(event.protocol_message().is_none());
        assert!(event.has_protocol_wire_message());

        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], json!("Page.screencastVisibilityChanged"));
        assert_eq!(message["sessionId"], json!("SID-page"));
        assert_eq!(message["params"]["visible"], json!(true));
        assert_eq!(automation_event, None);
    }

    #[test]
    fn screencast_frame_stays_typed_until_wire_projection() {
        let event = BackgroundProtocolEvent::page_screencast_frame(
            Some("SID-page"),
            "encoded-frame".to_owned(),
            PageScreencastFrameMetadata {
                offset_top: 0.0,
                page_scale_factor: 1.0,
                device_width: 800.0,
                device_height: 600.0,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                timestamp: 1234.5,
            },
            7,
        );

        assert!(event.protocol_message().is_none());
        assert_eq!(event.protocol_method(), Some("Page.screencastFrame"));
        assert!(event.has_protocol_wire_message());

        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], json!("Page.screencastFrame"));
        assert_eq!(message["sessionId"], json!("SID-page"));
        assert_eq!(message["params"]["data"], json!("encoded-frame"));
        assert_eq!(message["params"]["sessionId"], json!(7));
        assert_eq!(message["params"]["metadata"]["deviceWidth"], json!(800.0));
        assert_eq!(message["params"]["metadata"]["timestamp"], json!(1234.5));
        assert_eq!(automation_event, None);
    }

    #[test]
    fn inspector_events_stay_typed_until_wire_projection() {
        let events = vec![
            BackgroundProtocolEvent::inspector_target_crashed(Some("SID-crashed")),
            BackgroundProtocolEvent::inspector_target_reloaded_after_crash(Some("SID-reloaded")),
            BackgroundProtocolEvent::inspector_detached(Some("SID-detached"), "Target detached"),
        ];

        for event in &events {
            assert!(event.protocol_message().is_none());
            assert!(event.has_protocol_wire_message());
        }

        let parts = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();

        assert_eq!(parts[0].0["method"], json!("Inspector.targetCrashed"));
        assert_eq!(parts[0].0["sessionId"], json!("SID-crashed"));
        assert_eq!(parts[0].0["params"], json!({}));
        assert_eq!(parts[0].1, None);

        assert_eq!(
            parts[1].0["method"],
            json!("Inspector.targetReloadedAfterCrash")
        );
        assert_eq!(parts[1].0["sessionId"], json!("SID-reloaded"));
        assert_eq!(parts[1].0["params"], json!({}));
        assert_eq!(parts[1].1, None);

        assert_eq!(parts[2].0["method"], json!("Inspector.detached"));
        assert_eq!(parts[2].0["sessionId"], json!("SID-detached"));
        assert_eq!(parts[2].0["params"]["reason"], json!("Target detached"));
        assert_eq!(parts[2].1, None);
    }

    #[test]
    fn service_worker_events_stay_typed_until_wire_projection() {
        let events = vec![
            BackgroundProtocolEvent::service_worker_registration_updated(
                Some("SID-service-worker"),
                vec![BackgroundServiceWorkerRegistration {
                    registration_id: "41".to_owned(),
                    scope_url: "https://example.test/app/".to_owned(),
                    is_deleted: false,
                }],
            ),
            BackgroundProtocolEvent::service_worker_version_updated(
                Some("SID-service-worker"),
                vec![BackgroundServiceWorkerVersion {
                    version_id: "7".to_owned(),
                    registration_id: "41".to_owned(),
                    script_url: "https://example.test/service-worker.js".to_owned(),
                    running_status: "running".to_owned(),
                    status: "activated".to_owned(),
                    controlled_clients: vec!["TID-page".to_owned()],
                    target_id: "TID-service-worker".to_owned(),
                }],
            ),
            BackgroundProtocolEvent::service_worker_error_reported(
                Some("SID-service-worker"),
                BackgroundServiceWorkerErrorMessage {
                    error_message: "boom".to_owned(),
                    registration_id: "41".to_owned(),
                    version_id: "7".to_owned(),
                    source_url: "https://example.test/service-worker.js".to_owned(),
                    line_number: 12,
                    column_number: 34,
                },
            ),
        ];

        for event in &events {
            assert!(event.protocol_message().is_none());
            assert!(event.has_protocol_wire_message());
        }

        let parts = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();

        assert_eq!(
            parts[0].0["method"],
            json!("ServiceWorker.workerRegistrationUpdated")
        );
        assert_eq!(parts[0].0["sessionId"], json!("SID-service-worker"));
        assert_eq!(
            parts[0].0["params"]["registrations"][0],
            json!({
                "registrationId": "41",
                "scopeURL": "https://example.test/app/",
                "isDeleted": false
            })
        );
        assert_eq!(parts[0].1, None);

        assert_eq!(
            parts[1].0["method"],
            json!("ServiceWorker.workerVersionUpdated")
        );
        assert_eq!(parts[1].0["sessionId"], json!("SID-service-worker"));
        assert_eq!(
            parts[1].0["params"]["versions"][0],
            json!({
                "versionId": "7",
                "registrationId": "41",
                "scriptURL": "https://example.test/service-worker.js",
                "runningStatus": "running",
                "status": "activated",
                "controlledClients": ["TID-page"],
                "targetId": "TID-service-worker"
            })
        );
        assert_eq!(parts[1].1, None);

        assert_eq!(
            parts[2].0["method"],
            json!("ServiceWorker.workerErrorReported")
        );
        assert_eq!(parts[2].0["sessionId"], json!("SID-service-worker"));
        assert_eq!(
            parts[2].0["params"]["errorMessage"],
            json!({
                "errorMessage": "boom",
                "registrationId": "41",
                "versionId": "7",
                "sourceURL": "https://example.test/service-worker.js",
                "lineNumber": 12,
                "columnNumber": 34
            })
        );
        assert_eq!(parts[2].1, None);
    }

    #[test]
    fn target_info_changed_stays_typed_until_wire_projection() {
        let event = BackgroundProtocolEvent::target_info_changed(
            None,
            DevToolsTargetInfo {
                target_id: Some(DevToolsTargetId::from("TID-target-info")),
                kind: DevToolsTargetKind::Page,
                title: "Target title".to_owned(),
                url: "https://example.test/target".to_owned(),
                attached: true,
                opener_id: Some(DevToolsTargetId::from("TID-opener")),
                opener_frame_id: Some(DevToolsFrameId::from("FRAME-opener")),
                can_access_opener: true,
                browser_context_id: Some(DevToolsBrowserContextId::from("BID-target-info")),
                moli_popup_id: None,
            },
        );

        assert!(event.protocol_message().is_none());
        assert!(event.has_protocol_wire_message());

        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], json!("Target.targetInfoChanged"));
        assert!(message.get("sessionId").is_none());
        assert_eq!(
            message["params"]["targetInfo"],
            json!({
                "targetId": "TID-target-info",
                "type": "page",
                "title": "Target title",
                "url": "https://example.test/target",
                "attached": true,
                "canAccessOpener": true,
                "openerId": "TID-opener",
                "openerFrameId": "FRAME-opener",
                "browserContextId": "BID-target-info"
            })
        );
        assert_eq!(automation_event, None);
    }

    #[test]
    fn target_lifecycle_events_stay_typed_until_wire_projection() {
        let target_info = DevToolsTargetInfo {
            target_id: Some(DevToolsTargetId::from("TID-created")),
            kind: DevToolsTargetKind::Page,
            title: String::new(),
            url: "https://example.test/created".to_owned(),
            attached: false,
            opener_id: None,
            opener_frame_id: None,
            can_access_opener: false,
            browser_context_id: Some(DevToolsBrowserContextId::from("BID-created")),
            moli_popup_id: None,
        };
        let mut attached = BackgroundProtocolEvent::target_attached(TargetAttachmentEvent {
            target_id: DevToolsTargetId::from("TID-attached"),
            session_id: DevToolsSessionId::from("SID-child"),
            parent_session_id: None,
            target_info: DevToolsTargetInfo {
                target_id: Some(DevToolsTargetId::from("TID-attached")),
                kind: DevToolsTargetKind::Page,
                title: String::new(),
                url: "about:blank".to_owned(),
                attached: true,
                opener_id: None,
                opener_frame_id: None,
                can_access_opener: false,
                browser_context_id: None,
                moli_popup_id: None,
            },
            waiting_for_debugger: true,
        });
        attached.ensure_protocol_session_id(Some("SID-parent"));
        let events = vec![
            BackgroundProtocolEvent::target_created(
                None,
                TargetLifecycleEvent {
                    target_id: DevToolsTargetId::from("TID-created"),
                    browser_context_id: Some(DevToolsBrowserContextId::from("BID-created")),
                    kind: DevToolsTargetKind::Page,
                    url: "https://example.test/created".to_owned(),
                    target_info: Some(target_info),
                },
            ),
            attached,
            BackgroundProtocolEvent::target_detached(TargetDetachmentEvent {
                target_id: DevToolsTargetId::from("TID-detached"),
                session_id: DevToolsSessionId::from("SID-detached"),
                parent_session_id: Some(DevToolsSessionId::from("SID-parent")),
                reason: Some("Render process gone.".to_owned()),
            }),
        ];

        for event in &events {
            assert!(event.protocol_message().is_none());
            assert!(event.has_protocol_wire_message());
        }

        let parts = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();

        assert_eq!(parts[0].0["method"], json!("Target.targetCreated"));
        assert!(parts[0].0.get("sessionId").is_none());
        assert_eq!(
            parts[0].0["params"]["targetInfo"]["targetId"],
            json!("TID-created")
        );
        let Some(AutomationEvent::TargetCreated(event)) = &parts[0].1 else {
            panic!("expected TargetCreated automation sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-created");

        assert_eq!(parts[1].0["method"], json!("Target.attachedToTarget"));
        assert_eq!(parts[1].0["sessionId"], json!("SID-parent"));
        assert_eq!(parts[1].0["params"]["sessionId"], json!("SID-child"));
        assert_eq!(parts[1].0["params"]["waitingForDebugger"], json!(true));
        let Some(AutomationEvent::TargetAttached(event)) = &parts[1].1 else {
            panic!("expected TargetAttached automation sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-attached");
        assert_eq!(
            event.parent_session_id.as_ref().map(|id| id.as_str()),
            Some("SID-parent")
        );

        assert_eq!(parts[2].0["method"], json!("Target.detachedFromTarget"));
        assert_eq!(parts[2].0["sessionId"], json!("SID-parent"));
        assert_eq!(parts[2].0["params"]["targetId"], json!("TID-detached"));
        assert!(parts[2].0["params"].get("reason").is_none());
        let Some(AutomationEvent::TargetDetached(event)) = &parts[2].1 else {
            panic!("expected TargetDetached automation sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-detached");
        assert_eq!(event.reason.as_deref(), Some("Render process gone."));
    }

    #[test]
    fn target_destroyed_stays_typed_until_wire_projection() {
        let event = BackgroundProtocolEvent::target_destroyed(
            None,
            TargetLifecycleEvent {
                target_id: DevToolsTargetId::from("TID-destroyed"),
                browser_context_id: Some(DevToolsBrowserContextId::from("BID-destroyed")),
                kind: DevToolsTargetKind::Page,
                url: "https://example.test/destroyed".to_owned(),
                target_info: None,
            },
        );

        assert!(event.protocol_message().is_none());
        assert!(event.has_protocol_wire_message());

        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], json!("Target.targetDestroyed"));
        assert!(message.get("sessionId").is_none());
        assert_eq!(message["params"]["targetId"], json!("TID-destroyed"));
        let Some(AutomationEvent::TargetDestroyed(event)) = automation_event else {
            panic!("expected TargetDestroyed automation sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-destroyed");
        assert_eq!(
            event.browser_context_id.as_ref().map(|id| id.as_str()),
            Some("BID-destroyed")
        );
    }

    #[test]
    fn target_destroyed_preserves_owner_session_until_wire_projection() {
        let event = BackgroundProtocolEvent::target_destroyed(
            Some("SID-owner"),
            TargetLifecycleEvent {
                target_id: DevToolsTargetId::from("TID-destroyed"),
                browser_context_id: None,
                kind: DevToolsTargetKind::Page,
                url: "about:blank".to_owned(),
                target_info: None,
            },
        );

        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], json!("Target.targetDestroyed"));
        assert_eq!(message["sessionId"], json!("SID-owner"));
        assert_eq!(message["params"]["targetId"], json!("TID-destroyed"));
        let Some(AutomationEvent::TargetDestroyed(event)) = automation_event else {
            panic!("expected TargetDestroyed automation sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-destroyed");
    }

    #[test]
    fn target_crashed_stays_typed_until_wire_projection() {
        let event = BackgroundProtocolEvent::target_crashed(None, "TID-crashed", "crashed", 1);

        assert!(event.protocol_message().is_none());
        assert_eq!(event.protocol_method(), Some("Target.targetCrashed"));
        assert!(event.has_protocol_wire_message());

        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], json!("Target.targetCrashed"));
        assert!(message.get("sessionId").is_none());
        assert_eq!(message["params"]["targetId"], json!("TID-crashed"));
        assert_eq!(message["params"]["status"], json!("crashed"));
        assert_eq!(message["params"]["errorCode"], json!(1));
        assert_eq!(automation_event, None);
    }

    #[test]
    fn target_received_message_stays_typed_until_wire_projection() {
        let nested_event =
            BackgroundProtocolEvent::command_success(Some(77), None, json!({ "ok": true }));
        let mut event =
            BackgroundProtocolEvent::target_received_message_from_target("SID-child", nested_event);

        assert!(event.protocol_message().is_none());
        assert!(event.has_protocol_wire_message());

        event.ensure_protocol_session_id(Some("SID-parent"));
        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], json!("Target.receivedMessageFromTarget"));
        assert!(
            message.get("sessionId").is_none(),
            "legacy Target.receivedMessageFromTarget is routed through params.sessionId"
        );
        assert_eq!(message["params"]["sessionId"], json!("SID-child"));
        let nested: Value = serde_json::from_str(
            message["params"]["message"]
                .as_str()
                .expect("nested target message should be a string"),
        )
        .expect("nested target message should be valid json");
        assert_eq!(nested["id"], json!(77));
        assert_eq!(nested["result"], json!({ "ok": true }));
        assert_eq!(automation_event, None);
    }

    #[test]
    fn console_and_log_background_events_do_not_rewrap_cdp_params_as_sidecars() {
        let console = BackgroundProtocolEvent::console_message_added(
            Some("SID-console"),
            "javascript",
            "log",
            "hello",
            "",
        );
        let (console_message, console_event) = console.into_parts();
        assert_eq!(console_message["method"], json!("Console.messageAdded"));
        assert_eq!(console_message["sessionId"], json!("SID-console"));
        assert_eq!(console_message["params"]["message"]["text"], json!("hello"));
        let Some(AutomationEvent::RuntimeConsoleApiCalled(console_event)) = console_event else {
            panic!("Console.messageAdded should expose a RuntimeConsoleApiCalled sidecar");
        };
        assert_eq!(console_event.text, "hello");

        let log = BackgroundProtocolEvent::log_entry_added(
            Some("SID-log"),
            "javascript",
            "error",
            "boom",
            "",
            1.25,
            None,
        );
        let (log_message, log_event) = log.into_parts();
        assert_eq!(log_message["method"], json!("Log.entryAdded"));
        assert_eq!(log_message["sessionId"], json!("SID-log"));
        assert_eq!(log_message["params"]["entry"]["text"], json!("boom"));
        let Some(AutomationEvent::LogEntryAdded(log_event)) = log_event else {
            panic!("Log.entryAdded should expose a LogEntryAdded sidecar");
        };
        assert_eq!(log_event.text, "boom");
    }

    #[test]
    fn network_extra_info_events_stay_typed_until_wire_projection() {
        let request = BackgroundProtocolEvent::network_request_will_be_sent_extra_info(
            Some("SID-network"),
            "REQ-extra",
            serde_json::Map::from_iter([("Cookie".to_owned(), json!("sid=1"))]),
            json!({ "includedCookies": [], "excludedCookies": [] }),
            Vec::new(),
            12.5,
        );
        assert!(request.protocol_message().is_none());
        assert_eq!(
            request.protocol_method(),
            Some("Network.requestWillBeSentExtraInfo")
        );
        assert!(request.has_protocol_wire_message());
        let (message, automation_event) = request.into_parts();
        assert_eq!(
            message["method"],
            json!("Network.requestWillBeSentExtraInfo")
        );
        assert_eq!(message["sessionId"], json!("SID-network"));
        assert_eq!(message["params"]["requestId"], json!("REQ-extra"));
        assert_eq!(message["params"]["headers"]["Cookie"], json!("sid=1"));
        assert_eq!(message["params"]["associatedCookies"], json!([]));
        assert_eq!(
            message["params"]["connectTiming"]["requestTime"],
            json!(12.5)
        );
        assert!(automation_event.is_none());

        let response = BackgroundProtocolEvent::network_response_received_extra_info(
            Some("SID-network"),
            "REQ-extra",
            serde_json::Map::from_iter([("set-cookie".to_owned(), json!("sid=2"))]),
            302,
            vec![json!({ "status": "Include" })],
            Vec::new(),
        );
        assert!(response.protocol_message().is_none());
        assert_eq!(
            response.protocol_method(),
            Some("Network.responseReceivedExtraInfo")
        );
        let (message, automation_event) = response.into_parts();
        assert_eq!(
            message["method"],
            json!("Network.responseReceivedExtraInfo")
        );
        assert_eq!(message["sessionId"], json!("SID-network"));
        assert_eq!(message["params"]["requestId"], json!("REQ-extra"));
        assert_eq!(message["params"]["statusCode"], json!(302));
        assert_eq!(message["params"]["headers"]["set-cookie"], json!("sid=2"));
        assert_eq!(message["params"]["blockedCookies"], json!([]));
        assert_eq!(
            message["params"]["resourceIPAddressSpace"],
            json!("Unknown")
        );
        assert_eq!(
            message["params"]["cookieReports"],
            json!([{ "status": "Include" }])
        );
        assert!(automation_event.is_none());
    }

    #[test]
    fn network_websocket_events_stay_typed_until_wire_projection() {
        let events = vec![
            (
                BackgroundProtocolEvent::network_websocket_created(
                    Some("SID-ws"),
                    "REQ-ws",
                    "ws://example.test/socket",
                ),
                "Network.webSocketCreated",
            ),
            (
                BackgroundProtocolEvent::network_websocket_will_send_handshake_request(
                    Some("SID-ws"),
                    "REQ-ws",
                    11.5,
                    serde_json::Map::from_iter([(
                        "origin".to_owned(),
                        json!("https://example.test"),
                    )]),
                ),
                "Network.webSocketWillSendHandshakeRequest",
            ),
            (
                BackgroundProtocolEvent::network_websocket_handshake_response_received(
                    Some("SID-ws"),
                    "REQ-ws",
                    12.5,
                    101,
                    "Switching Protocols",
                    serde_json::Map::from_iter([("upgrade".to_owned(), json!("websocket"))]),
                ),
                "Network.webSocketHandshakeResponseReceived",
            ),
            (
                BackgroundProtocolEvent::network_websocket_frame(
                    Some("SID-ws"),
                    "REQ-ws",
                    13.5,
                    WebSocketFrameDirection::Received,
                    WebSocketFrameOpcode::Text,
                    5,
                ),
                "Network.webSocketFrameReceived",
            ),
        ];

        for (event, method) in events {
            assert!(
                event.protocol_message().is_none(),
                "WebSocket events must stay typed until wire projection"
            );
            assert_eq!(event.protocol_method(), Some(method));
            assert!(event.has_protocol_wire_message());
            let (message, automation_event) = event.into_parts();
            assert_eq!(message["method"], json!(method));
            assert_eq!(message["sessionId"], json!("SID-ws"));
            assert_eq!(message["params"]["requestId"], json!("REQ-ws"));
            assert!(automation_event.is_none());
        }
    }

    #[test]
    fn runtime_context_events_expose_method_without_raw_protocol_message() {
        let context_event = RuntimeExecutionContextEvent {
            target_id: None,
            context_id: Some(7),
            realm_id: None,
            frame_id: Some(DevToolsFrameId::from("FRAME-runtime")),
            origin: Some("https://example.test".to_owned()),
            name: Some(String::new()),
            is_default: Some(true),
            context_type: Some("default".to_owned()),
            grant_universal_access: None,
        };
        let events = vec![
            (
                BackgroundProtocolEvent::runtime_execution_contexts_cleared(
                    Some("SID-runtime"),
                    RuntimeExecutionContextsClearedEvent { target_id: None },
                ),
                "Runtime.executionContextsCleared",
            ),
            (
                BackgroundProtocolEvent::runtime_execution_context_created(
                    Some("SID-runtime"),
                    context_event.clone(),
                ),
                "Runtime.executionContextCreated",
            ),
            (
                BackgroundProtocolEvent::runtime_execution_context_destroyed(
                    Some("SID-runtime"),
                    context_event,
                ),
                "Runtime.executionContextDestroyed",
            ),
        ];

        for (event, method) in events {
            assert!(
                event.protocol_message().is_none(),
                "typed runtime context events must not expose raw protocol message state"
            );
            assert_eq!(event.protocol_method(), Some(method));
            assert!(event.has_protocol_wire_message());

            let (message, automation_event) = event.into_parts();
            assert_eq!(message["method"], json!(method));
            assert_eq!(message["sessionId"], json!("SID-runtime"));
            assert!(automation_event.is_some());
        }
    }

    #[test]
    fn page_activity_events_stay_typed_until_wire_projection() {
        let events = vec![
            BackgroundProtocolEvent::page_file_chooser_opened(
                Some("SID-page"),
                "FRAME-file",
                "selectMultiple",
                17,
                None,
            ),
            BackgroundProtocolEvent::browser_download_will_begin(
                Some("SID-page"),
                None,
                "FRAME-download",
                "GUID-download",
                "https://example.test/download",
                "download.txt",
            ),
            BackgroundProtocolEvent::browser_download_progress(
                Some("SID-page"),
                None,
                "GUID-download",
                "completed",
                123,
                123,
                Some("/tmp/download.txt"),
            ),
            BackgroundProtocolEvent::page_javascript_dialog_opening(
                Some("SID-page"),
                crate::devtools_runtime::PageJavaScriptDialogOpeningEvent {
                    frame_id: Some(DevToolsFrameId::from("FRAME-dialog")),
                    url: "https://example.test/dialog".to_owned(),
                    message: "confirm?".to_owned(),
                    dialog_type: "confirm".to_owned(),
                    has_browser_handler: true,
                    default_prompt: String::new(),
                },
            ),
        ];

        for event in &events {
            assert!(
                event.protocol_message().is_none(),
                "activity event should stay typed until wire projection: {event:?}"
            );
            assert!(event.has_protocol_wire_message());
        }

        let parts = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();
        let methods = parts
            .iter()
            .map(|(message, _)| message["method"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            methods,
            vec![
                "Page.fileChooserOpened",
                "Browser.downloadWillBegin",
                "Browser.downloadProgress",
                "Page.javascriptDialogOpening",
            ]
        );
        assert!(matches!(
            parts[0].1.as_ref(),
            Some(AutomationEvent::PageFileChooserOpened(_))
        ));
        assert_eq!(parts[1].1, None);
        assert_eq!(parts[2].1, None);
        assert!(matches!(
            parts[3].1.as_ref(),
            Some(AutomationEvent::PageJavaScriptDialogOpening(_))
        ));
        assert_eq!(parts[0].0["sessionId"], json!("SID-page"));
        assert_eq!(parts[0].0["params"]["backendNodeId"], json!(17));
        assert_eq!(
            parts[1].0["params"]["suggestedFilename"],
            json!("download.txt")
        );
        assert_eq!(parts[2].0["params"]["filePath"], json!("/tmp/download.txt"));
        assert_eq!(parts[3].0["params"]["frameId"], json!("FRAME-dialog"));
    }

    #[test]
    fn background_event_waits_for_navigation_completion_only_for_non_document_network() {
        use crate::devtools_runtime::{DevToolsRequestId, NetworkRequestEvent};

        let network_event = |resource_type: Option<DevToolsNetworkResourceType>| {
            AutomationEvent::NetworkResponseStarted(NetworkRequestEvent {
                target_id: DevToolsTargetId::from("TID-nav"),
                frame_id: Some(DevToolsFrameId::from("TID-nav")),
                request_id: DevToolsRequestId::from("REQ-nav"),
                loader_id: Some(DevToolsLoaderId::from("LOADER-nav")),
                url: "https://example.test/resource".to_owned(),
                document_url: None,
                method: None,
                request_headers: Vec::new(),
                request_body: None,
                request_initiator_type: None,
                bidi_request_initiator_type: None,
                redirect_response: None,
                redirect_has_extra_info: false,
                request_cookie_report: None,
                resource_type,
                timestamp: Some(1.0),
                wall_time: None,
                status: Some(200),
                status_text: None,
                response_headers: Vec::new(),
                response_mime_type: None,
                response_protocol: None,
                has_extra_info: false,
                encoded_data_length: Some(0),
                from_cache: false,
                fetch_request_id: None,
                error_text: None,
                loading_failed_canceled: false,
                blocked_intercepts: Vec::new(),
                network_id: None,
                auth_challenge: None,
            })
        };

        let document = BackgroundProtocolEvent::immediate_automation_event(
            json!({"method": "Network.responseReceived"}),
            network_event(Some(DevToolsNetworkResourceType::Document)),
        );
        let script = BackgroundProtocolEvent::immediate_automation_event(
            json!({"method": "Network.responseReceived"}),
            network_event(Some(DevToolsNetworkResourceType::Script)),
        );
        let lifecycle = BackgroundProtocolEvent::immediate_automation_event(
            json!({"method": "Page.frameStartedLoading"}),
            AutomationEvent::NavigationFrame(NavigationFrameEvent {
                target_id: DevToolsTargetId::from("TID-nav"),
                frame_id: DevToolsFrameId::from("TID-nav"),
                parent_frame_id: None,
                loader_id: Some(DevToolsLoaderId::from("LOADER-nav")),
                url: "https://example.test/".to_owned(),
                kind: NavigationFrameEventKind::StartedLoading,
                frame_name: None,
                security_origin: None,
                secure_context_type: None,
            }),
        );
        let protocol_document = BackgroundProtocolEvent::immediate(json!({
            "method": "Network.responseReceived",
            "params": {
                "requestId": "REQ-protocol-document",
                "type": "Document"
            }
        }));
        let protocol_script = BackgroundProtocolEvent::immediate(json!({
            "method": "Network.responseReceived",
            "params": {
                "requestId": "REQ-protocol-script",
                "type": "Script"
            }
        }));
        let blocked_protocol_script = BackgroundProtocolEvent::immediate(json!({
            "method": "Network.requestWillBeSent",
            "params": {
                "requestId": "REQ-protocol-script-blocked",
                "type": "Script",
                "__moliBlockedInterceptors": ["intercept-script"]
            }
        }));
        let fetch_request_paused = BackgroundProtocolEvent::immediate(json!({
            "method": "Fetch.requestPaused",
            "params": {
                "requestId": "INT-script",
                "resourceType": "Script"
            }
        }));
        let fetch_auth_required = BackgroundProtocolEvent::immediate(json!({
            "method": "Fetch.authRequired",
            "params": {
                "requestId": "INT-auth",
                "resourceType": "Script"
            }
        }));
        let protocol_without_type = BackgroundProtocolEvent::immediate(json!({
            "method": "Network.responseReceived",
            "params": {
                "requestId": "REQ-protocol-unknown"
            }
        }));

        assert!(!document.should_wait_for_background_navigation_completion());
        assert!(script.should_wait_for_background_navigation_completion());
        assert!(!lifecycle.should_wait_for_background_navigation_completion());
        assert!(!protocol_document.should_wait_for_background_navigation_completion());
        assert!(protocol_script.should_wait_for_background_navigation_completion());
        assert!(!blocked_protocol_script.should_wait_for_background_navigation_completion());
        assert!(!fetch_request_paused.should_wait_for_background_navigation_completion());
        assert!(!fetch_auth_required.should_wait_for_background_navigation_completion());
        assert!(!protocol_without_type.should_wait_for_background_navigation_completion());
    }

    #[test]
    fn navigation_background_event_drops_stale_token() {
        let mut browser_context = BrowserContext::new("CTX-nav".to_owned());
        browser_context.set_active_target_id("TID-nav");
        let stale = browser_context
            .start_document_navigation_for_active_target("LOADER-1".to_owned())
            .expect("active target should produce stale token");
        let current = browser_context
            .start_document_navigation_for_active_target("LOADER-2".to_owned())
            .expect("active target should produce current token");
        let message = build_event(
            "Page.frameStoppedLoading",
            json!({ "frameId": "TID-nav" }),
            None,
        );

        let stale_event = NavigationBackgroundEvent::protocol_message(stale, message.clone());
        let current_event = NavigationBackgroundEvent::protocol_message(current, message.clone());

        assert_eq!(
            stale_event.into_protocol_message_if_current(std::iter::once(&browser_context)),
            None
        );
        assert_eq!(
            current_event.into_protocol_message_if_current(std::iter::once(&browser_context)),
            Some(message)
        );
    }

    #[test]
    fn protocol_message_output_strips_moli_private_fields() {
        let message = BackgroundProtocolEvent::immediate(json!({
            "method": "Fetch.authRequired",
            "params": {
                "requestId": "FETCH-1",
                "__moliBlockedInterceptors": ["intercept-auth"],
                "__moliFetchRequestId": "FETCH-1"
            }
        }))
        .into_protocol_message();

        assert_eq!(message["params"]["requestId"], json!("FETCH-1"));
        assert!(message["params"].get("__moliBlockedInterceptors").is_none());
        assert!(message["params"].get("__moliFetchRequestId").is_none());
    }
}
