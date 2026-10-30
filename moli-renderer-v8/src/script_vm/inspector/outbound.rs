#[cfg(feature = "devtools")]
use crate::devtools::pause::RendererInspectorSessionOutboundRoute;
use crate::inspector_session::InspectorSessionOutput;
use crate::runtime::{
    PendingRendererOutputRecord, RendererCommandTurnOutputRecorder, RendererProtocolObservation,
    RendererRuntimeCommandOutputRecorder, RendererRuntimeInspectorMessage,
    RendererRuntimeInspectorMessageBatch, RendererRuntimeInspectorResponseSender,
    RendererTurnOutputJournal,
};
use anyhow::Result;
use moli_inspector_json::InspectorJson;
use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::{Rc, Weak},
};

#[cfg(feature = "devtools")]
#[path = "frontend/reply_routing.rs"]
mod frontend;

/// Push-based ordered outbound handling for inspector messages.
///
/// V8 Inspector calls `send_response` / `send_notification` from the
/// `RendererInspectorChannel`. Both serialized messages and deferred response
/// callbacks remain local to the renderer agent that owns the V8 session.
/// Protocol attachment validation is the only frontend route owner.
///
/// Active dispatch responses are captured by snapshot-tail: callers snapshot
/// `len()` before invoking V8 and take the messages appended after that
/// snapshot. Deferred responses must use a registered per-command callback; a
/// late response with no callback owner is stale and is dropped instead of
/// being queued for the next unrelated CDP command.
#[derive(Default)]
struct InspectorOutboundMessageState {
    messages: VecDeque<InspectorOutboundMessage>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct InspectorOutboundMessage {
    pub(super) agent_token: RendererDevToolsAgentToken,
    pub(super) value: InspectorJson,
}

#[derive(Default)]
struct InspectorResponseRoutingState {
    deactivated: bool,
    notifications_retired: bool,
    // These fields belong to one agent-local V8 session channel and never
    // cross a renderer agent boundary.
    pending_response_callbacks: HashMap<i32, RendererRuntimeInspectorResponseSender>,
    canceled_response_callbacks: HashMap<i32, Option<String>>,
    dispatch_response_capture_depth: usize,
    internal_dispatch_response_call_ids: Vec<i32>,
    runtime_command_output: Option<RendererRuntimeCommandOutputRecorder>,
    runtime_command_output_suppression_depth: usize,
    runtime_command_output_queue_snapshot_len: Option<usize>,
    command_turn_output: Option<(DevToolsSessionKey, RendererCommandTurnOutputRecorder)>,
}

type SharedInspectorOutboundMessageState = Rc<RefCell<InspectorOutboundMessageState>>;
type SharedInspectorResponseRoutingState = Rc<RefCell<InspectorResponseRoutingState>>;

#[derive(Clone)]
pub(in crate::script_vm) struct InspectorOutbound {
    native_session: Rc<RefCell<Weak<v8::inspector::V8InspectorSession>>>,
    debugger_pauses_enabled: bool,
    agent_token: RendererDevToolsAgentToken,
    session: Option<DevToolsSessionKey>,
    output_journal: Option<RendererTurnOutputJournal>,
    messages: SharedInspectorOutboundMessageState,
    response_routing: SharedInspectorResponseRoutingState,
    #[cfg(feature = "devtools")]
    session_route: Rc<RefCell<Option<RendererInspectorSessionOutboundRoute>>>,
}

impl Default for InspectorOutbound {
    fn default() -> Self {
        Self::for_agent(RendererDevToolsAgentToken::allocate())
    }
}

impl InspectorSessionOutput for InspectorOutbound {
    fn capture_internal_response(&self, call_id: i32, dispatch: impl FnOnce()) -> Option<Value> {
        let snapshot = self.len();
        {
            let _internal_capture = self.capture_internal_dispatch_response(call_id);
            let _dispatch_capture = self.capture_dispatch_responses();
            dispatch();
        }
        self.take_response_for_call_id_after(snapshot, i64::from(call_id))
    }
}

impl InspectorOutbound {
    pub(super) fn prepare_native_session_binding(
        &mut self,
    ) -> Rc<RefCell<Weak<v8::inspector::V8InspectorSession>>> {
        // Reattach may reuse the message store, but an old native channel must
        // retain its own binding and can never clean a successor's groups.
        self.native_session = Default::default();
        self.native_session.clone()
    }

    fn release_abandoned_group(&self, group: Option<&str>) {
        if let Some(group) = group
            && let Some(session) = self.native_session.borrow().upgrade()
        {
            session.release_object_group(v8::inspector::StringView::from(group.as_bytes()));
        }
    }

    pub(super) fn for_agent(agent_token: RendererDevToolsAgentToken) -> Self {
        Self {
            native_session: Default::default(),
            debugger_pauses_enabled: false,
            agent_token,
            session: None,
            output_journal: None,
            messages: Rc::new(RefCell::new(InspectorOutboundMessageState::default())),
            response_routing: Rc::new(RefCell::new(InspectorResponseRoutingState::default())),
            #[cfg(feature = "devtools")]
            session_route: Rc::new(RefCell::new(None)),
        }
    }

    pub(super) fn debugger_pauses_enabled(&self) -> bool {
        self.debugger_pauses_enabled
    }

    pub(super) fn with_output_journal(
        mut self,
        output_journal: Option<RendererTurnOutputJournal>,
    ) -> Self {
        self.output_journal = output_journal;
        self
    }
}

impl InspectorOutbound {
    fn state(&self) -> SharedInspectorOutboundMessageState {
        Rc::clone(&self.messages)
    }

    #[cfg(test)]
    pub(super) fn response_callback_counts(&self) -> (usize, usize) {
        let response_routing = self.response_routing.borrow();
        (
            response_routing.pending_response_callbacks.len(),
            response_routing.canceled_response_callbacks.len(),
        )
    }

    pub(super) fn retire_notifications(&self) {
        self.response_routing.borrow_mut().notifications_retired = true;
    }

    pub(super) fn deactivate(&self) {
        {
            let mut guard = self.response_routing.borrow_mut();
            guard.deactivated = true;
            guard.notifications_retired = true;
            debug_assert_eq!(guard.dispatch_response_capture_depth, 0);
            debug_assert!(guard.internal_dispatch_response_call_ids.is_empty());
            debug_assert!(guard.runtime_command_output.is_none());
            debug_assert_eq!(guard.runtime_command_output_suppression_depth, 0);
            debug_assert!(guard.runtime_command_output_queue_snapshot_len.is_none());
            // Detaching an Inspector frontend is itself allowed to be the
            // operation performed by a renderer command turn. Remove this
            // outbound's route from the shared recorder now; the enclosing
            // scope will later call `end_command_turn_output`, which is
            // intentionally conditional and therefore becomes a no-op.
            // Records already appended to the recorder remain owned by the
            // command and are not discarded with the detached session route.
            guard.command_turn_output = None;
            guard.pending_response_callbacks.clear();
            guard.canceled_response_callbacks.clear();
        }
        let mut messages = self.messages.borrow_mut();
        messages.messages.clear();
        #[cfg(feature = "devtools")]
        if let Some(route) = self.session_route.borrow_mut().take() {
            route.deactivate_session();
        }
    }

    pub(super) fn push_message(&self, mut message: v8::UniquePtr<v8::inspector::StringBuffer>) {
        let Some(message) = message.as_mut() else {
            return;
        };
        let view = message.string();
        let message_units = view.len();
        let message_width = if view.is_8bit() { 8 } else { 16 };
        match moli_v8_util::inspector_protocol_json(view) {
            Ok(value) => self.push_value(value),
            Err(error) => {
                let raw = moli_v8_util::inspector_protocol_message_text(view);
                tracing::warn!(target: "moli::cdp::inspector",
                    %error, %raw, message_units, message_width,
                    "failed to decode inspector protocol message; dropping");
            }
        }
    }

    pub(super) fn push_response_message(
        &self,
        call_id: i32,
        mut message: v8::UniquePtr<v8::inspector::StringBuffer>,
    ) {
        let Some(message) = message.as_mut() else {
            return;
        };
        let view = message.string();
        let message_units = view.len();
        let message_width = if view.is_8bit() { 8 } else { 16 };
        match moli_v8_util::inspector_protocol_json(view) {
            Ok(value) if value.id_i64() == Some(i64::from(call_id)) => self.push_response_value(call_id, value),
            Ok(_) => self.push_response_value(call_id, serde_json::json!({"id":call_id,"error":{"code":-32603,"message":"Inspector response ID mismatch"}})),
            Err(error) => {
                let raw = moli_v8_util::inspector_protocol_message_text(view);
                tracing::warn!(target: "moli::cdp::inspector",
                    %error, %raw, call_id, message_units, message_width,
                    "failed to decode inspector protocol response; returning protocol error");
                self.push_response_value(
                    call_id,
                    serde_json::json!({
                        "id": call_id,
                        "error": {
                            "code": -32603,
                            "message": format!("Failed to decode Inspector response: {error}"),
                        },
                    }),
                );
            }
        }
    }

    pub(super) fn push_value(&self, value: impl Into<InspectorJson>) {
        if self.response_routing.borrow().notifications_retired {
            return;
        }
        let value = value.into();
        #[cfg(feature = "devtools")]
        if let Some(session_route) = self.session_route.borrow().clone() {
            return self.push_frontend_value(session_route, value);
        }
        self.push_local_value(value);
    }

    fn push_local_value(&self, value: InspectorJson) {
        let (runtime_command_output, command_turn_output, dispatch_is_active) = {
            let routing = self.response_routing.borrow();
            if routing.notifications_retired {
                return;
            }
            if routing.runtime_command_output_suppression_depth == 0 {
                (
                    routing.runtime_command_output.clone(),
                    routing.command_turn_output.clone(),
                    routing.dispatch_response_capture_depth > 0,
                )
            } else {
                (None, None, routing.dispatch_response_capture_depth > 0)
            }
        };
        if let Some((session, recorder)) = command_turn_output {
            recorder.push_runtime_inspector_message(self.agent_token, session, value);
            return;
        } else if let Some(recorder) = runtime_command_output {
            recorder.push_inspector_message(value.clone());
        } else if !dispatch_is_active
            && let (Some(output_journal), Some(session)) = (&self.output_journal, &self.session)
        {
            // Only notifications produced outside an Inspector command
            // dispatch belong to the ordinary live stream. V8 can emit
            // `executionContextCreated` synchronously while handling
            // `Runtime.enable`; that notification and the matching response
            // are one command-local FIFO prefix and must be returned to the
            // dispatch owner together. Otherwise the late-enable replay would
            // escape into the live stream and become a second, independently
            // routed producer.
            output_journal.append(PendingRendererOutputRecord::observation(
                None,
                RendererProtocolObservation::RuntimeInspector(
                    RendererRuntimeInspectorMessageBatch::new(
                        self.agent_token,
                        session.clone(),
                        vec![RendererRuntimeInspectorMessage::from_v8_inspector_message(
                            value,
                        )],
                    ),
                ),
            ));
            return;
        }
        self.queue_local_value(value);
    }

    fn queue_local_value(&self, value: InspectorJson) {
        self.state()
            .borrow_mut()
            .messages
            .push_back(InspectorOutboundMessage {
                agent_token: self.agent_token,
                value,
            });
    }

    pub(super) fn push_response_value(&self, call_id: i32, value: impl Into<InspectorJson>) {
        let value = value.into();
        let mut guard = self.response_routing.borrow_mut();
        if guard.deactivated {
            return;
        }
        if guard
            .internal_dispatch_response_call_ids
            .last()
            .is_some_and(|active_call_id| *active_call_id == call_id)
        {
            drop(guard);
            self.push_local_value(value);
            return;
        }
        #[cfg(feature = "devtools")]
        if let Some(session_route) = self.session_route.borrow().as_ref() {
            session_route.mark_command_response(call_id, !value.has_field("error"));
        }
        let callback = guard.pending_response_callbacks.remove(&call_id);
        if let Some(callback) = callback {
            let cleanup_group = callback.abandoned_object_group().map(str::to_owned);
            if cleanup_group.is_some() && !callback.is_open() {
                drop(guard);
                self.release_abandoned_group(cleanup_group.as_deref());
                return;
            }
            let recorder = (guard.runtime_command_output_suppression_depth == 0)
                .then(|| guard.runtime_command_output.clone())
                .flatten();
            drop(guard);
            if let Some(recorder) = recorder.filter(|recorder| recorder.owns_response(call_id)) {
                recorder.park_response(callback, value);
                return;
            }
            if let Err(message) = callback.send(value) {
                self.release_abandoned_group(cleanup_group.as_deref());
                tracing::debug!(
                    call_id,
                    message = ?message,
                    "dropping runtime inspector response because deferred receiver was closed"
                );
            }
            return;
        }
        if let Some(cleanup_group) = guard.canceled_response_callbacks.remove(&call_id) {
            drop(guard);
            self.release_abandoned_group(cleanup_group.as_deref());
            tracing::debug!(
                call_id,
                message = ?value,
                "dropping stale runtime inspector response for canceled deferred callback"
            );
            return;
        }
        if guard.dispatch_response_capture_depth > 0 {
            drop(guard);
            // A protocol response belongs only to the command completion. It
            // must not pass through `push_local_value`, because that would
            // append it to the command's concrete notification stream when a
            // command-turn recorder is active.
            self.queue_local_value(value);
        } else {
            tracing::debug!(
                call_id,
                message = ?value,
                "dropping stale runtime inspector response without a registered deferred callback"
            );
        }
    }

    pub(in crate::script_vm) fn register_response_callback(
        &self,
        callback: RendererRuntimeInspectorResponseSender,
    ) {
        let call_id = callback.call_id();
        let previous = {
            let mut guard = self.response_routing.borrow_mut();
            guard.canceled_response_callbacks.remove(&call_id);
            guard.pending_response_callbacks.insert(call_id, callback)
        };
        debug_assert!(
            previous.is_none(),
            "runtime inspector response callback registered twice for call id {call_id}"
        );
    }

    pub(in crate::script_vm) fn cancel_response_callback(&self, call_id: i32) {
        let mut guard = self.response_routing.borrow_mut();
        if let Some(callback) = guard.pending_response_callbacks.remove(&call_id) {
            guard.canceled_response_callbacks.insert(
                call_id,
                callback.abandoned_object_group().map(str::to_owned),
            );
        } else {
            guard
                .canceled_response_callbacks
                .entry(call_id)
                .or_insert(None);
        }
    }

    pub(in crate::script_vm) fn len(&self) -> usize {
        self.state().borrow().messages.len()
    }

    pub(in crate::script_vm) fn take_messages_after(
        &self,
        snapshot_len: usize,
    ) -> Vec<InspectorJson> {
        let state = self.state();
        let mut guard = state.borrow_mut();
        if snapshot_len >= guard.messages.len() {
            return Vec::new();
        }
        guard
            .messages
            .drain(snapshot_len..)
            .map(|message| message.value)
            .collect()
    }

    pub(in crate::script_vm) fn discard_messages_after(&self, snapshot_len: usize) {
        let state = self.state();
        let mut guard = state.borrow_mut();
        if snapshot_len < guard.messages.len() {
            guard.messages.drain(snapshot_len..);
        }
    }

    /// Removes one internal command response while preserving notifications
    /// emitted by the same inspector dispatch.
    pub(in crate::script_vm) fn take_response_for_call_id_after(
        &self,
        snapshot_len: usize,
        call_id: i64,
    ) -> Option<Value> {
        let state = self.state();
        let mut guard = state.borrow_mut();
        let position = guard
            .messages
            .iter()
            .enumerate()
            .skip(snapshot_len)
            .find_map(|(index, message)| {
                (message.value.id_i64() == Some(call_id)).then_some(index)
            });
        position
            .and_then(|position| guard.messages.remove(position))
            .map(|message| message.value.into_value())
    }

    // DOMDebugger's internal Runtime.callFunctionOn can execute JS, unlike the
    // synchronous stack-capture setter, so it still needs an unoccupied ID.

    #[cfg(any(test, feature = "devtools"))]
    pub(in crate::script_vm) fn take_pending_messages(&self) -> Vec<InspectorJson> {
        let state = self.state();
        let mut guard = state.borrow_mut();
        guard
            .messages
            .drain(..)
            .map(|message| message.value)
            .collect()
    }

    pub(in crate::script_vm) fn capture_dispatch_responses(
        &self,
    ) -> InspectorDispatchResponseCapture {
        let mut routing = self.response_routing.borrow_mut();
        routing.dispatch_response_capture_depth += 1;
        InspectorDispatchResponseCapture {
            outbound: self.clone(),
        }
    }

    pub(in crate::script_vm) fn capture_internal_dispatch_response(
        &self,
        call_id: i32,
    ) -> InspectorInternalDispatchResponseCapture {
        {
            let mut routing = self.response_routing.borrow_mut();
            routing.internal_dispatch_response_call_ids.push(call_id);
            routing.runtime_command_output_suppression_depth += 1;
        }
        InspectorInternalDispatchResponseCapture {
            outbound: self.clone(),
            call_id,
        }
    }

    pub(in crate::script_vm) fn begin_runtime_command_output(
        &self,
        recorder: RendererRuntimeCommandOutputRecorder,
    ) {
        let mut routing = self.response_routing.borrow_mut();
        debug_assert!(
            routing.runtime_command_output.is_none(),
            "runtime inspector command output scopes cannot overlap"
        );
        routing.runtime_command_output_queue_snapshot_len = Some(self.len());
        routing.runtime_command_output = Some(recorder);
    }

    fn begin_command_turn_output(
        &self,
        session: DevToolsSessionKey,
        recorder: RendererCommandTurnOutputRecorder,
    ) -> Result<()> {
        let mut routing = self.response_routing.borrow_mut();
        anyhow::ensure!(
            routing.command_turn_output.is_none() && routing.dispatch_response_capture_depth == 0,
            "renderer command-turn output scopes cannot overlap"
        );
        routing.command_turn_output = Some((session, recorder));
        Ok(())
    }

    fn end_command_turn_output(&self, recorder: &RendererCommandTurnOutputRecorder) {
        let mut routing = self.response_routing.borrow_mut();
        if routing
            .command_turn_output
            .as_ref()
            .is_some_and(|(_, active)| active.records_into_same_sink(recorder))
        {
            debug_assert_eq!(routing.dispatch_response_capture_depth, 0);
            routing.command_turn_output = None;
        }
    }
}

pub(in crate::script_vm) struct ScriptVmInspectorCommandTurnOutputScope {
    outbounds: Vec<InspectorOutbound>,
    recorder: RendererCommandTurnOutputRecorder,
}

impl ScriptVmInspectorCommandTurnOutputScope {
    pub(super) fn begin(
        sessions: impl Iterator<Item = (DevToolsSessionKey, InspectorOutbound)>,
        recorder: RendererCommandTurnOutputRecorder,
    ) -> Result<Self> {
        let mut scope = Self {
            outbounds: Vec::new(),
            recorder: recorder.clone(),
        };
        for (session, outbound) in sessions {
            outbound.begin_command_turn_output(session, recorder.clone())?;
            scope.outbounds.push(outbound);
        }
        Ok(scope)
    }
}

impl Drop for ScriptVmInspectorCommandTurnOutputScope {
    fn drop(&mut self) {
        for outbound in &self.outbounds {
            outbound.end_command_turn_output(&self.recorder);
        }
    }
}

pub(in crate::script_vm) struct InspectorDispatchResponseCapture {
    outbound: InspectorOutbound,
}

impl Drop for InspectorDispatchResponseCapture {
    fn drop(&mut self) {
        let mut guard = self.outbound.response_routing.borrow_mut();
        guard.dispatch_response_capture_depth = guard
            .dispatch_response_capture_depth
            .checked_sub(1)
            .expect("inspector dispatch response capture depth underflow");
    }
}

pub(in crate::script_vm) struct InspectorInternalDispatchResponseCapture {
    outbound: InspectorOutbound,
    call_id: i32,
}

impl Drop for InspectorInternalDispatchResponseCapture {
    fn drop(&mut self) {
        let active_call_id = self
            .outbound
            .response_routing
            .borrow_mut()
            .internal_dispatch_response_call_ids
            .pop();
        assert_eq!(
            active_call_id,
            Some(self.call_id),
            "inspector internal dispatch response captures must be dropped in stack order"
        );
        let mut guard = self.outbound.response_routing.borrow_mut();
        guard.runtime_command_output_suppression_depth = guard
            .runtime_command_output_suppression_depth
            .checked_sub(1)
            .expect("runtime command output suppression depth underflow");
    }
}
