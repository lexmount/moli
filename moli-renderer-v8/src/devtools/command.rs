//! Typed metadata and payloads crossing the renderer DevTools ingress boundary.

use crate::runtime::{
    RendererInspectorPageCommand, RendererPageCommand, RendererRuntimeInspectorResponseSender,
};
use crate::script_execution_control::RendererScriptExecutionControl;
use moli_page_types::{DevToolsSessionKey, RendererAgentAttachmentId};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

/// Chromium routes Page DevTools work through separate main-thread and IO
/// session ingress paths. Ordering is guaranteed within one session and one
/// route, while an IO command may overtake main-thread work that has not
/// reached its first dispatch yet.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RendererInspectorCommandRoute {
    MainThread,
    Io,
}

/// The command owns its `(Page, session, route)` lane until its first access to
/// the frontend V8 Inspector session. Protocol response completion is not part
/// of this lifetime: V8 may complete a response asynchronously, and holding the
/// lane for that response could prevent a later resume command from running.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererInspectorFirstDispatchLifecycle {
    OrderedUntilFirstDispatch,
}

/// Pause-loop transition owned by an Inspector command. This is derived once
/// when the ingress envelope is created so executors do not need to reparse
/// protocol JSON to decide how resumed/paused notifications are attributed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererInspectorPauseCommandEffect {
    None,
    Resume,
    Step,
}

impl RendererInspectorPauseCommandEffect {
    fn from_message(message: Option<&Value>) -> Self {
        match message
            .and_then(|message| message.get("method"))
            .and_then(Value::as_str)
        {
            Some("Debugger.resume") => Self::Resume,
            Some(
                "Debugger.continueToLocation"
                | "Debugger.restartFrame"
                | "Debugger.stepInto"
                | "Debugger.stepOut"
                | "Debugger.stepOver",
            ) => Self::Step,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RendererInspectorMainDispatchBoundary {
    InspectorSession,
    PageOwner,
}

static NEXT_RENDERER_INSPECTOR_INGRESS_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererInspectorIngressTicket {
    attachment: Option<RendererAgentAttachmentId>,
    session: DevToolsSessionKey,
    route: RendererInspectorCommandRoute,
    sequence: u64,
}

impl RendererInspectorIngressTicket {
    pub fn new(
        attachment: Option<RendererAgentAttachmentId>,
        inspector_session_id: Option<String>,
        route: RendererInspectorCommandRoute,
    ) -> Self {
        Self {
            attachment,
            session: DevToolsSessionKey::from_wire_session_id(
                inspector_session_id
                    .as_deref()
                    .filter(|session_id| !session_id.is_empty()),
            ),
            route,
            sequence: NEXT_RENDERER_INSPECTOR_INGRESS_SEQUENCE
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |sequence| {
                    sequence.checked_add(1)
                })
                .expect("renderer Inspector ingress sequence overflow"),
        }
    }

    pub fn attachment(&self) -> Option<RendererAgentAttachmentId> {
        self.attachment
    }

    pub fn session(&self) -> &DevToolsSessionKey {
        &self.session
    }

    pub fn route(&self) -> RendererInspectorCommandRoute {
        self.route
    }

    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub(crate) fn bind_attachment(&mut self, attachment: RendererAgentAttachmentId) {
        if let Some(bound) = self.attachment {
            assert_eq!(
                bound, attachment,
                "an Inspector ingress ticket cannot be retargeted to another attachment"
            );
        } else {
            self.attachment = Some(attachment);
        }
    }
}

/// Strongly typed DevToolsSession ingress. The payload deliberately does not
/// carry another session id: every operation that accesses a frontend
/// `V8InspectorSession` must obtain its identity and dispatch policy from this
/// envelope.
pub struct RendererInspectorCommandEnvelope {
    ticket: RendererInspectorIngressTicket,
    first_dispatch: RendererInspectorFirstDispatchLifecycle,
    pause_effect: RendererInspectorPauseCommandEffect,
    main_dispatch_boundary: RendererInspectorMainDispatchBoundary,
    payload: RendererInspectorCommandPayload,
}

/// One command delivered by the renderer's IO DevTools receiver.
///
/// Chromium chooses the IO receiver at the `DevToolsSession` boundary, before
/// it chooses the renderer agent that will execute the command. Keep that
/// ordering structural here as well: V8 Inspector, Performance, and Emulation
/// commands share one `(target, session, IO)` first-dispatch lane.
#[doc(hidden)]
pub struct RendererDevToolsIoCommandEnvelope {
    ticket: RendererInspectorIngressTicket,
    payload: RendererDevToolsIoCommandPayload,
}

pub(crate) enum RendererDevToolsIoCommandPayload {
    Inspector(RendererInspectorCommandEnvelope),
    PerformanceGetMetrics,
    SetScriptExecutionDisabled {
        control: RendererScriptExecutionControl,
        disabled: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererDevToolsIoCommandKind {
    Inspector,
    Performance,
    Emulation,
}

impl RendererDevToolsIoCommandEnvelope {
    pub(crate) fn inspector(envelope: RendererInspectorCommandEnvelope) -> Self {
        assert_eq!(
            envelope.ticket().route(),
            RendererInspectorCommandRoute::Io,
            "an Inspector payload entering the IO receiver must use the IO route"
        );
        Self {
            ticket: envelope.ticket().clone(),
            payload: RendererDevToolsIoCommandPayload::Inspector(envelope),
        }
    }

    pub(crate) fn performance_get_metrics(ticket: RendererInspectorIngressTicket) -> Self {
        Self::new_agent_command(
            ticket,
            RendererDevToolsIoCommandPayload::PerformanceGetMetrics,
        )
    }

    pub(crate) fn set_script_execution_disabled(
        ticket: RendererInspectorIngressTicket,
        control: RendererScriptExecutionControl,
        disabled: bool,
    ) -> Self {
        Self::new_agent_command(
            ticket,
            RendererDevToolsIoCommandPayload::SetScriptExecutionDisabled { control, disabled },
        )
    }

    fn new_agent_command(
        ticket: RendererInspectorIngressTicket,
        payload: RendererDevToolsIoCommandPayload,
    ) -> Self {
        assert_eq!(
            ticket.route(),
            RendererInspectorCommandRoute::Io,
            "a renderer IO agent payload must use the IO route"
        );
        Self { ticket, payload }
    }

    pub(crate) fn ticket(&self) -> &RendererInspectorIngressTicket {
        &self.ticket
    }

    pub(crate) fn first_dispatch_lifecycle(&self) -> RendererInspectorFirstDispatchLifecycle {
        RendererInspectorFirstDispatchLifecycle::OrderedUntilFirstDispatch
    }

    pub(crate) fn kind(&self) -> RendererDevToolsIoCommandKind {
        match &self.payload {
            RendererDevToolsIoCommandPayload::Inspector(_) => {
                RendererDevToolsIoCommandKind::Inspector
            }
            RendererDevToolsIoCommandPayload::PerformanceGetMetrics => {
                RendererDevToolsIoCommandKind::Performance
            }
            RendererDevToolsIoCommandPayload::SetScriptExecutionDisabled { .. } => {
                RendererDevToolsIoCommandKind::Emulation
            }
        }
    }

    pub(crate) fn inspector_envelope(&self) -> Option<&RendererInspectorCommandEnvelope> {
        match &self.payload {
            RendererDevToolsIoCommandPayload::Inspector(envelope) => Some(envelope),
            _ => None,
        }
    }

    pub(crate) fn inspector_envelope_mut(
        &mut self,
    ) -> Option<&mut RendererInspectorCommandEnvelope> {
        match &mut self.payload {
            RendererDevToolsIoCommandPayload::Inspector(envelope) => Some(envelope),
            _ => None,
        }
    }

    pub(crate) fn into_payload(self) -> RendererDevToolsIoCommandPayload {
        self.payload
    }
}

/// One command delivered by the renderer's Main DevTools receiver.
///
/// Unlike `RendererInspectorCommandEnvelope`, this envelope is deliberately
/// agent-neutral: protocol commands that ultimately need the renderer Page,
/// DOM, CSS, Accessibility, or V8 agents all enter the same Main receiver.
/// The boxed payload keeps that admission boundary structural without adding
/// a second allowlist of `RendererPageCommand` variants.
#[doc(hidden)]
pub struct RendererDevToolsMainCommandEnvelope {
    ticket: RendererInspectorIngressTicket,
    payload: Box<RendererPageCommand>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererDevToolsMainNestedDispatch {
    InspectorSession,
    PageAgent,
    OwnerOnly,
}

impl RendererDevToolsMainCommandEnvelope {
    pub(crate) fn from_protocol_command(command: RendererPageCommand) -> Self {
        Self::from_protocol_command_in_session(command, None)
    }

    pub(crate) fn from_protocol_command_in_session(
        command: RendererPageCommand,
        inspector_session_id: Option<String>,
    ) -> Self {
        let ticket = match &command {
            RendererPageCommand::Inspector(envelope) => envelope.ticket().clone(),
            _ => RendererInspectorIngressTicket::new(
                None,
                inspector_session_id,
                RendererInspectorCommandRoute::MainThread,
            ),
        };
        Self {
            ticket,
            payload: Box::new(command),
        }
    }

    pub(crate) fn ticket(&self) -> &RendererInspectorIngressTicket {
        &self.ticket
    }

    pub(crate) fn first_dispatch_lifecycle(&self) -> RendererInspectorFirstDispatchLifecycle {
        RendererInspectorFirstDispatchLifecycle::OrderedUntilFirstDispatch
    }

    pub(crate) fn nested_dispatch(&self) -> RendererDevToolsMainNestedDispatch {
        match self.payload.as_ref() {
            RendererPageCommand::Inspector(envelope)
                if envelope.can_dispatch_at_nested_inspector_session_boundary() =>
            {
                RendererDevToolsMainNestedDispatch::InspectorSession
            }
            RendererPageCommand::Inspector(_) => RendererDevToolsMainNestedDispatch::OwnerOnly,
            _ => RendererDevToolsMainNestedDispatch::PageAgent,
        }
    }

    pub(crate) fn inspector_envelope(&self) -> Option<&RendererInspectorCommandEnvelope> {
        match self.payload.as_ref() {
            RendererPageCommand::Inspector(envelope) => Some(envelope),
            _ => None,
        }
    }

    pub(crate) fn into_nested_inspector_envelope(self) -> RendererInspectorCommandEnvelope {
        assert_eq!(
            self.nested_dispatch(),
            RendererDevToolsMainNestedDispatch::InspectorSession,
            "only a session-boundary Inspector command may enter nested V8 dispatch"
        );
        let RendererPageCommand::Inspector(envelope) = *self.payload else {
            unreachable!("nested Inspector dispatch kind requires an Inspector payload")
        };
        envelope
    }

    pub(crate) fn into_inspector_envelope(self) -> Option<RendererInspectorCommandEnvelope> {
        match *self.payload {
            RendererPageCommand::Inspector(envelope) => Some(envelope),
            _ => None,
        }
    }

    pub(crate) fn into_nested_page_command(self) -> RendererPageCommand {
        assert_eq!(
            self.nested_dispatch(),
            RendererDevToolsMainNestedDispatch::PageAgent,
            "only a non-V8 Main agent command may enter nested Page dispatch"
        );
        *self.payload
    }

    pub(crate) fn into_payload(self) -> RendererPageCommand {
        *self.payload
    }
}

enum RendererInspectorCommandPayload {
    MainThread(RendererInspectorPageCommand),
    Io {
        raw_json: String,
        response: Option<RendererRuntimeInspectorResponseSender>,
    },
}

impl RendererInspectorCommandEnvelope {
    pub(crate) fn new(
        inspector_session_id: Option<String>,
        command: RendererInspectorPageCommand,
    ) -> Self {
        Self {
            ticket: RendererInspectorIngressTicket::new(
                None,
                inspector_session_id,
                RendererInspectorCommandRoute::MainThread,
            ),
            first_dispatch: RendererInspectorFirstDispatchLifecycle::OrderedUntilFirstDispatch,
            pause_effect: RendererInspectorPauseCommandEffect::None,
            main_dispatch_boundary: RendererInspectorMainDispatchBoundary::PageOwner,
            payload: RendererInspectorCommandPayload::MainThread(command),
        }
    }

    #[doc(hidden)]
    pub fn new_main_protocol(
        ticket: RendererInspectorIngressTicket,
        owner_context_resolution_action: Option<String>,
        raw_json: String,
        response: RendererRuntimeInspectorResponseSender,
    ) -> Self {
        assert_eq!(
            ticket.route(),
            RendererInspectorCommandRoute::MainThread,
            "a Main Inspector protocol payload must use the MainThread route"
        );
        let message = serde_json::from_str::<Value>(&raw_json).ok();
        let main_dispatch_boundary = if main_protocol_can_dispatch_at_inspector_session_boundary(
            owner_context_resolution_action.as_deref(),
            message.as_ref(),
        ) {
            RendererInspectorMainDispatchBoundary::InspectorSession
        } else {
            RendererInspectorMainDispatchBoundary::PageOwner
        };
        let command = match owner_context_resolution_action {
            Some(action) => RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse {
                action,
                raw_json,
                deferred_response: response,
            },
            None => RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithDeferredResponse {
                raw_json,
                deferred_response: response,
            },
        };
        Self {
            ticket,
            first_dispatch: RendererInspectorFirstDispatchLifecycle::OrderedUntilFirstDispatch,
            pause_effect: RendererInspectorPauseCommandEffect::from_message(message.as_ref()),
            main_dispatch_boundary,
            payload: RendererInspectorCommandPayload::MainThread(command),
        }
    }

    #[doc(hidden)]
    pub fn new_io(
        ticket: RendererInspectorIngressTicket,
        raw_json: String,
        response: Option<RendererRuntimeInspectorResponseSender>,
    ) -> Self {
        assert_eq!(
            ticket.route(),
            RendererInspectorCommandRoute::Io,
            "an Inspector IO payload must use the IO route"
        );
        let message = serde_json::from_str::<Value>(&raw_json).ok();
        Self {
            ticket,
            first_dispatch: RendererInspectorFirstDispatchLifecycle::OrderedUntilFirstDispatch,
            pause_effect: RendererInspectorPauseCommandEffect::from_message(message.as_ref()),
            main_dispatch_boundary: RendererInspectorMainDispatchBoundary::InspectorSession,
            payload: RendererInspectorCommandPayload::Io { raw_json, response },
        }
    }

    pub fn ticket(&self) -> &RendererInspectorIngressTicket {
        &self.ticket
    }

    pub fn first_dispatch_lifecycle(&self) -> RendererInspectorFirstDispatchLifecycle {
        self.first_dispatch
    }

    pub(crate) fn pause_effect(&self) -> RendererInspectorPauseCommandEffect {
        self.pause_effect
    }

    pub(crate) fn can_dispatch_at_nested_inspector_session_boundary(&self) -> bool {
        self.main_dispatch_boundary == RendererInspectorMainDispatchBoundary::InspectorSession
    }

    pub(crate) fn bind_attachment(&mut self, attachment: RendererAgentAttachmentId) {
        self.ticket.bind_attachment(attachment);
    }

    pub(crate) fn into_main_thread_parts(
        self,
    ) -> (RendererInspectorIngressTicket, RendererInspectorPageCommand) {
        let RendererInspectorCommandPayload::MainThread(command) = self.payload else {
            panic!("an Inspector IO envelope cannot enter Page owner dispatch");
        };
        (self.ticket, command)
    }

    fn main_thread_payload(&self) -> &RendererInspectorPageCommand {
        let RendererInspectorCommandPayload::MainThread(command) = &self.payload else {
            panic!("an Inspector IO envelope cannot enter Page owner dispatch");
        };
        command
    }

    pub(crate) fn is_main_protocol_command_with_deferred_response(&self) -> bool {
        matches!(
            self.main_thread_payload(),
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithDeferredResponse {
                ..
            } | RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse {
                ..
            }
        )
    }

    #[cfg(test)]
    pub(crate) fn main_protocol_raw_json(&self) -> &str {
        match self.main_thread_payload() {
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithDeferredResponse {
                raw_json,
                ..
            }
            | RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse {
                raw_json,
                ..
            } => raw_json,
            _ => panic!("only a deferred Main protocol command can enter the nested receiver"),
        }
    }

    pub(crate) fn main_protocol_response(&self) -> &RendererRuntimeInspectorResponseSender {
        match self.main_thread_payload() {
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithDeferredResponse {
                deferred_response,
                ..
            }
            | RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse {
                deferred_response,
                ..
            } => deferred_response,
            _ => panic!("only a deferred Main protocol command can enter the nested receiver"),
        }
    }

    pub(crate) fn into_main_protocol_parts(
        self,
    ) -> (
        RendererInspectorIngressTicket,
        String,
        RendererRuntimeInspectorResponseSender,
    ) {
        assert_eq!(
            self.main_dispatch_boundary,
            RendererInspectorMainDispatchBoundary::InspectorSession,
            "a Page-owner-dependent Main command cannot enter direct nested Inspector dispatch"
        );
        match self.payload {
            RendererInspectorCommandPayload::MainThread(
                RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithDeferredResponse {
                    raw_json,
                    deferred_response,
                }
                | RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse {
                    raw_json,
                    deferred_response,
                    ..
                },
            ) => (self.ticket, raw_json, deferred_response),
            _ => panic!("only a deferred Main protocol command can enter the nested receiver"),
        }
    }

    pub(crate) fn io_raw_json(&self) -> &str {
        let RendererInspectorCommandPayload::Io { raw_json, .. } = &self.payload else {
            panic!("a MainThread Inspector envelope cannot enter IO dispatch");
        };
        raw_json
    }

    pub(crate) fn io_response(&self) -> Option<&RendererRuntimeInspectorResponseSender> {
        let RendererInspectorCommandPayload::Io { response, .. } = &self.payload else {
            panic!("a MainThread Inspector envelope cannot enter IO dispatch");
        };
        response.as_ref()
    }

    pub(crate) fn take_io_response(&mut self) -> Option<RendererRuntimeInspectorResponseSender> {
        let RendererInspectorCommandPayload::Io { response, .. } = &mut self.payload else {
            panic!("a MainThread Inspector envelope cannot enter IO dispatch");
        };
        response.take()
    }

    pub(crate) fn requires_materialized_child_realms(&self) -> bool {
        matches!(
            self.main_thread_payload(),
            RendererInspectorPageCommand::RuntimeEnableEvents
        )
    }

    pub(crate) fn cdp_nav_timing_label(&self) -> Option<&'static str> {
        match self.main_thread_payload() {
            RendererInspectorPageCommand::RuntimeEnableEvents => Some("RuntimeEnableEvents"),
            RendererInspectorPageCommand::ApplyRuntimeProtocolState { .. } => {
                Some("ApplyRuntimeProtocolState")
            }
            RendererInspectorPageCommand::DetachRuntimeInspectorSession { .. } => {
                Some("DetachRuntimeInspectorSession")
            }
            RendererInspectorPageCommand::DocumentNodeSnapshotForObjectId { .. } => {
                Some("DocumentNodeSnapshotForObjectId")
            }
            RendererInspectorPageCommand::AccessibilityTreePayloadsForObjectId { .. } => {
                Some("AccessibilityTreePayloadsForObjectId")
            }
            RendererInspectorPageCommand::AccessibilityNodeAndAncestorPayloadsForObjectId {
                ..
            } => Some("AccessibilityNodeAndAncestorPayloadsForObjectId"),
            RendererInspectorPageCommand::AccessibilityPartialTreePayloadsForObjectId {
                ..
            } => Some("AccessibilityPartialTreePayloadsForObjectId"),
            RendererInspectorPageCommand::OuterHtmlForObjectId { .. } => {
                Some("OuterHtmlForObjectId")
            }
            RendererInspectorPageCommand::ScrollObjectNodeIntoViewIfNeeded { .. } => {
                Some("ScrollObjectNodeIntoViewIfNeeded")
            }
            RendererInspectorPageCommand::ClientRectForObjectId { .. } => {
                Some("ClientRectForObjectId")
            }
            RendererInspectorPageCommand::DocumentGeometryForObjectId { .. } => {
                Some("DocumentGeometryForObjectId")
            }
            RendererInspectorPageCommand::NodeHasGeometryForObjectId { .. } => {
                Some("NodeHasGeometryForObjectId")
            }
            RendererInspectorPageCommand::SetFileInputFilesForObjectId { .. } => {
                Some("SetFileInputFilesForObjectId")
            }
            RendererInspectorPageCommand::ResolveRuntimeObjectForBackendNodeId { .. } => {
                Some("ResolveRuntimeObjectForBackendNodeId")
            }
            RendererInspectorPageCommand::ResolveBlobObject { .. } => Some("ResolveBlobObject"),
            _ => None,
        }
    }

    pub(crate) fn uses_cpu_throttling(&self) -> bool {
        matches!(
            self.main_thread_payload(),
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessage { .. }
                | RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithDeferredResponse { .. }
                | RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolution { .. }
                | RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse { .. }
                | RendererInspectorPageCommand::DomDebuggerGetEventListeners { .. }
                | RendererInspectorPageCommand::FocusDocumentNodeForObjectId { .. }
        )
    }
}

fn main_protocol_can_dispatch_at_inspector_session_boundary(
    owner_context_resolution_action: Option<&str>,
    message: Option<&Value>,
) -> bool {
    let Some(message) = message else {
        return false;
    };
    let method = message.get("method").and_then(Value::as_str);
    let params = message.get("params").and_then(Value::as_object);

    // These transitions have renderer-owned restore state and must therefore
    // pass through PageVm even though their final sink is V8InspectorSession.
    if matches!(
        method,
        Some("Runtime.enable" | "Runtime.disable" | "Console.enable" | "Console.disable")
    ) {
        return false;
    }

    let has_owner_scoped_runtime_semantics = params.is_some_and(|params| {
        params.get("userGesture").and_then(Value::as_bool) == Some(true)
            || params.contains_key(crate::script_vm::WEBDRIVER_BIDI_FILE_PROMPT_HANDLER_PARAM)
    });
    if has_owner_scoped_runtime_semantics {
        return false;
    }

    match owner_context_resolution_action {
        // Page.createIsolatedWorld and Runtime.executionContextCreated expose
        // V8 Inspector's native context id. Context-targeted commands can
        // therefore enter Chromium's nested Main receiver without borrowing
        // PageVm; an unknown or retired id is rejected by V8 itself.
        Some("evaluate") => true,
        // A target supplied by object, unique context, or numeric Inspector
        // context is complete. Only the target-less compatibility form needs
        // PageVm to insert Moli's default execution context.
        Some("callFunctionOn") => params.is_some_and(|params| {
            params.contains_key("objectId")
                || params.contains_key("uniqueContextId")
                || params.contains_key("executionContextId")
        }),
        Some(_) => false,
        None => true,
    }
}
