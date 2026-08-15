use crate::runtime::{
    PendingRendererOutputRecord, RendererCommandTurnOutputRecorder, RendererProtocolObservation,
    RendererRuntimeCommandOutputRecorder, RendererRuntimeInspectorMessage,
    RendererRuntimeInspectorMessageBatch, RendererRuntimeInspectorResponseSender,
    RendererTurnOutputJournal,
};
use crate::script_vm::inspector_pause::{
    RendererInspectorPauseNotificationRoute, RendererInspectorPauseOutboundRoute,
    RendererInspectorPausePrefaceGuard,
};
use anyhow::Result;
use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
};

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
    pub(super) value: Value,
}

#[derive(Default)]
struct InspectorResponseRoutingState {
    // These fields belong to one agent-local V8 session channel and never
    // cross a renderer agent boundary.
    pending_response_callbacks: HashMap<i32, RendererRuntimeInspectorResponseSender>,
    canceled_response_callbacks: HashSet<i32>,
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
    agent_token: RendererDevToolsAgentToken,
    session: Option<DevToolsSessionKey>,
    output_journal: Option<RendererTurnOutputJournal>,
    messages: SharedInspectorOutboundMessageState,
    response_routing: SharedInspectorResponseRoutingState,
    pause_route: Rc<RefCell<Option<RendererInspectorPauseOutboundRoute>>>,
}

impl Default for InspectorOutbound {
    fn default() -> Self {
        Self::for_agent(RendererDevToolsAgentToken::allocate())
    }
}

impl InspectorOutbound {
    pub(super) fn for_agent(agent_token: RendererDevToolsAgentToken) -> Self {
        Self {
            agent_token,
            session: None,
            output_journal: None,
            messages: Rc::new(RefCell::new(InspectorOutboundMessageState::default())),
            response_routing: Rc::new(RefCell::new(InspectorResponseRoutingState::default())),
            pause_route: Rc::new(RefCell::new(None)),
        }
    }

    pub(super) fn for_frontend(
        agent_token: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
        pause_route: RendererInspectorPauseOutboundRoute,
        output_journal: Option<RendererTurnOutputJournal>,
    ) -> Self {
        let mut outbound = Self::for_agent(agent_token);
        outbound.session = Some(session);
        outbound.output_journal = output_journal;
        *outbound.pause_route.borrow_mut() = Some(pause_route);
        outbound
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

    pub(super) fn deactivate(&self) {
        {
            let mut guard = self.response_routing.borrow_mut();
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
        if let Some(route) = self.pause_route.borrow_mut().take() {
            route.detach_session();
        }
    }

    pub(super) fn push_message(&self, mut message: v8::UniquePtr<v8::inspector::StringBuffer>) {
        let Some(message) = message.as_mut() else {
            return;
        };
        let view = message.string();
        let message_units = view.len();
        let message_width = if view.is_8bit() { 8 } else { 16 };
        match moli_v8_util::decode_inspector_protocol_message(view) {
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
        match moli_v8_util::decode_inspector_protocol_message(view) {
            Ok(value) => self.push_response_value(call_id, value),
            Err(error) => {
                let raw = moli_v8_util::inspector_protocol_message_text(view);
                tracing::warn!(target: "moli::cdp::inspector",
                    %error, %raw, message_units, message_width,
                    "failed to decode inspector protocol response; dropping");
            }
        }
    }

    pub(super) fn push_value(&self, value: Value) {
        if let Some(pause_route) = self.pause_route.borrow().clone() {
            match pause_route.route_notification(&value) {
                RendererInspectorPauseNotificationRoute::OrdinaryTurn => {
                    return self.push_local_value(value);
                }
                RendererInspectorPauseNotificationRoute::PublishImmediately {
                    preface,
                    command_output,
                } => return self.publish_pause_value(preface, command_output, value),
                RendererInspectorPauseNotificationRoute::Drop => return,
            }
        }
        self.push_local_value(value);
    }

    /// Publishes the exact Inspector prefix that makes a nested debugger pause
    /// externally observable.
    ///
    /// A paused V8 loop cannot return to the ordinary Page-turn settlement
    /// boundary until the frontend resumes it. Moving the active command
    /// recorder prefix into the same Page stream before blocking preserves
    /// source FIFO without a second pause mailbox or a source-shaped wake.
    fn publish_pause_value(
        &self,
        mut preface: Vec<RendererRuntimeInspectorMessage>,
        command_output: Option<
            crate::script_vm::inspector_pause::RendererInspectorPauseCommandOutputRoute,
        >,
        value: Value,
    ) {
        let session = self
            .session
            .clone()
            .expect("a debugger pause notification requires a frontend Inspector session");
        let (command_turn_output, causal_command) = {
            let routing = self.response_routing.borrow();
            (
                routing
                    .command_turn_output
                    .as_ref()
                    .map(|(_, recorder)| recorder.clone()),
                routing
                    .runtime_command_output
                    .as_ref()
                    .map(RendererRuntimeCommandOutputRecorder::causal_identity),
            )
        };
        let output_journal = self
            .output_journal
            .as_ref()
            .expect("a debugger pause notification requires a concrete Page output stream");
        preface.push(RendererRuntimeInspectorMessage::from_v8_inspector_message(
            value,
        ));
        let (causal_command, batch) = match command_output {
            Some(command_output) => (
                Some(command_output.causal_identity),
                RendererRuntimeInspectorMessageBatch::new_after_command_response(
                    self.agent_token,
                    session,
                    preface,
                ),
            ),
            None => (
                causal_command,
                RendererRuntimeInspectorMessageBatch::new(self.agent_token, session, preface),
            ),
        };
        let record = PendingRendererOutputRecord::observation(
            causal_command,
            RendererProtocolObservation::RuntimeInspector(batch),
        );
        if let Some(recorder) = command_turn_output {
            recorder.push_record(record);
            output_journal.append_records(recorder.drain_records());
        } else {
            output_journal.append(record);
        }
        let _ = output_journal.publish_pending();
    }

    pub(super) fn stage_pause_preface(
        &self,
        messages: Vec<RendererRuntimeInspectorMessage>,
    ) -> Option<RendererInspectorPausePrefaceGuard> {
        self.pause_route
            .borrow()
            .as_ref()
            .and_then(|route| route.stage_pause_preface(messages))
    }

    fn push_local_value(&self, value: Value) {
        let (runtime_command_output, command_turn_output, dispatch_is_active) = {
            let routing = self.response_routing.borrow();
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

    fn queue_local_value(&self, value: Value) {
        self.state()
            .borrow_mut()
            .messages
            .push_back(InspectorOutboundMessage {
                agent_token: self.agent_token,
                value,
            });
    }

    pub(super) fn push_response_value(&self, call_id: i32, value: Value) {
        if let Some(pause_route) = self.pause_route.borrow().as_ref() {
            pause_route.mark_command_response(call_id, value.get("error").is_none());
        }
        let mut guard = self.response_routing.borrow_mut();
        if guard
            .internal_dispatch_response_call_ids
            .last()
            .is_some_and(|active_call_id| *active_call_id == call_id)
        {
            drop(guard);
            self.push_local_value(value);
            return;
        }
        let callback = guard.pending_response_callbacks.remove(&call_id);
        if let Some(callback) = callback {
            let recorder = (guard.runtime_command_output_suppression_depth == 0)
                .then(|| guard.runtime_command_output.clone())
                .flatten();
            drop(guard);
            if let Some(recorder) = recorder.filter(|recorder| recorder.owns_response(call_id)) {
                recorder.park_response(callback, value);
                return;
            }
            if let Err(message) = callback.send(value) {
                tracing::debug!(
                    call_id,
                    message = ?message,
                    "dropping runtime inspector response because deferred receiver was closed"
                );
            }
            return;
        }
        if guard.canceled_response_callbacks.remove(&call_id) {
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
        guard.pending_response_callbacks.remove(&call_id);
        guard.canceled_response_callbacks.insert(call_id);
    }

    pub(in crate::script_vm) fn len(&self) -> usize {
        self.state().borrow().messages.len()
    }

    pub(in crate::script_vm) fn take_messages_after(&self, snapshot_len: usize) -> Vec<Value> {
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

    /// Moves notifications produced by one internal Inspector bootstrap
    /// command into this Page's concrete output stream.
    ///
    /// `Runtime.enable` and `Console.enable` are dispatched internally while a
    /// new Page is still being constructed. Their synthetic responses remain
    /// private to the bootstrap call, but notifications such as
    /// `Runtime.executionContextCreated` are real frontend observations. They
    /// must be frozen at this dispatch boundary so page commit can publish
    /// them before its exact cursor; leaving them in the agent-local queue
    /// would make a later turn rediscover output owned by page creation.
    pub(in crate::script_vm) fn append_messages_after_to_output_journal(
        &self,
        snapshot_len: usize,
    ) -> Result<()> {
        let Some(output_journal) = self.output_journal.as_ref() else {
            anyhow::bail!("Inspector bootstrap notifications require a Page output journal");
        };
        let Some(session) = self.session.clone() else {
            anyhow::bail!("Inspector bootstrap notifications require a frontend session");
        };
        for message in self.take_pending_tagged_messages_after(snapshot_len) {
            output_journal.append(PendingRendererOutputRecord::observation(
                None,
                RendererProtocolObservation::RuntimeInspector(
                    RendererRuntimeInspectorMessageBatch::new(
                        message.agent_token,
                        session.clone(),
                        vec![RendererRuntimeInspectorMessage::from_v8_inspector_message(
                            message.value,
                        )],
                    ),
                ),
            ));
        }
        Ok(())
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
                (message.value.get("id").and_then(Value::as_i64) == Some(call_id)).then_some(index)
            });
        position
            .and_then(|position| guard.messages.remove(position))
            .map(|message| message.value)
    }

    pub(in crate::script_vm) fn internal_dispatch_call_id_is_available(
        &self,
        call_id: i32,
    ) -> bool {
        let routing_available = {
            let routing = self.response_routing.borrow();
            !routing.pending_response_callbacks.contains_key(&call_id)
                && !routing.canceled_response_callbacks.contains(&call_id)
                && !routing
                    .internal_dispatch_response_call_ids
                    .contains(&call_id)
        };
        routing_available
            && !self.state().borrow().messages.iter().any(|message| {
                message.value.get("id").and_then(Value::as_i64) == Some(i64::from(call_id))
            })
    }

    pub(super) fn values_after(&self, snapshot_len: usize) -> Vec<Value> {
        let state = self.state();
        let guard = state.borrow();
        if snapshot_len >= guard.messages.len() {
            return Vec::new();
        }
        guard
            .messages
            .iter()
            .skip(snapshot_len)
            .map(|message| message.value.clone())
            .collect()
    }

    pub(in crate::script_vm) fn take_pending_messages(&self) -> Vec<Value> {
        let state = self.state();
        let mut guard = state.borrow_mut();
        guard
            .messages
            .drain(..)
            .map(|message| message.value)
            .collect()
    }

    fn take_pending_tagged_messages_after(
        &self,
        snapshot_len: usize,
    ) -> Vec<InspectorOutboundMessage> {
        let state = self.state();
        let mut guard = state.borrow_mut();
        if snapshot_len >= guard.messages.len() {
            return Vec::new();
        }
        guard.messages.drain(snapshot_len..).collect()
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

    pub(in crate::script_vm) fn end_runtime_command_output(&self) {
        let snapshot_len = {
            let mut routing = self.response_routing.borrow_mut();
            routing.runtime_command_output = None;
            routing.runtime_command_output_queue_snapshot_len.take()
        };
        if let Some(snapshot_len) = snapshot_len {
            self.discard_messages_after(snapshot_len);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{PageId, RendererOutputItem, RendererOutputStreamIdentity};

    #[test]
    fn instrumentation_pause_prefix_publishes_context_created_with_bound_origin() {
        let journal = RendererTurnOutputJournal::new(
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(41)),
        );
        let pause_bridge =
            crate::script_vm::inspector_pause::RendererInspectorPauseBridge::default();
        pause_bridge.configure_page_route(journal.clone());
        let agent_token = RendererDevToolsAgentToken::allocate();
        let session = DevToolsSessionKey::Primary;
        let io_ingress = crate::script_vm::inspector_io::RendererInspectorIoIngress::new(
            pause_bridge.pause_loop_wake(),
            None,
        );
        let outbound = InspectorOutbound::for_frontend(
            agent_token,
            session.clone(),
            pause_bridge.outbound_route(io_ingress, agent_token, session.clone()),
            Some(journal.clone()),
        );
        let recorder = RendererCommandTurnOutputRecorder::default();
        outbound
            .begin_command_turn_output(session, recorder)
            .expect("command output scope");

        outbound.push_value(serde_json::json!({
            "method": "Runtime.executionContextCreated",
            "params": {
                "context": {
                    "id": 41,
                    "origin": "https://example.test",
                    "auxData": {"isDefault": true, "type": "default"}
                }
            }
        }));
        outbound.push_value(serde_json::json!({
            "method": "Debugger.paused",
            "params": {"reason": "instrumentation", "callFrames": []}
        }));

        assert_eq!(
            journal.pending_len(),
            0,
            "the pause prefix must cross the resolved publication boundary immediately"
        );
        assert!(
            pause_bridge.is_pause_active(),
            "the regression fixture must exercise the immediate pause route"
        );
    }

    #[test]
    fn command_response_stays_out_of_concrete_notification_records() {
        let outbound = InspectorOutbound::for_agent(RendererDevToolsAgentToken::allocate());
        let session = DevToolsSessionKey::Primary;
        let recorder = RendererCommandTurnOutputRecorder::default();
        outbound
            .begin_command_turn_output(session, recorder.clone())
            .expect("command output scope");

        {
            let _capture = outbound.capture_dispatch_responses();
            outbound.push_value(serde_json::json!({
                "method": "Runtime.consoleAPICalled",
                "params": {}
            }));
            outbound.push_response_value(
                7,
                serde_json::json!({"id": 7, "result": {"value": "response"}}),
            );
        }
        outbound.end_command_turn_output(&recorder);

        let records = recorder.finish();
        assert_eq!(records.len(), 1);
        assert!(matches!(
            records[0].item(),
            RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(batch))
                if batch.messages.iter().any(|message| matches!(
                    message,
                    RendererRuntimeInspectorMessage::Protocol(value)
                        if value["method"] == "Runtime.consoleAPICalled"
                ))
        ));
        assert_eq!(
            outbound.take_pending_messages(),
            vec![serde_json::json!({
                "id": 7,
                "result": {"value": "response"}
            })]
        );
    }

    #[test]
    fn frontend_can_deactivate_during_its_command_turn_without_losing_records() {
        let outbound = InspectorOutbound::for_agent(RendererDevToolsAgentToken::allocate());
        let session = DevToolsSessionKey::Primary;
        let recorder = RendererCommandTurnOutputRecorder::default();
        outbound
            .begin_command_turn_output(session, recorder.clone())
            .expect("command output scope");
        outbound.push_value(serde_json::json!({
            "method": "Runtime.consoleAPICalled",
            "params": {"type": "log"}
        }));

        outbound.deactivate();
        outbound.end_command_turn_output(&recorder);

        let records = recorder.finish();
        assert_eq!(records.len(), 1);
        assert!(matches!(
            records[0].item(),
            RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(batch))
                if batch.messages.iter().any(|message| matches!(
                    message,
                    RendererRuntimeInspectorMessage::Protocol(value)
                        if value["method"] == "Runtime.consoleAPICalled"
                ))
        ));
    }
}
