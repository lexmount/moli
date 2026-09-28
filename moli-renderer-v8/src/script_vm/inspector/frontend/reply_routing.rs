use super::*;
use crate::devtools::pause::{
    RendererInspectorPauseNotificationRoute, RendererInspectorPausePrefaceGuard,
};
use crate::runtime::RendererDevToolsSessionOutputHost;
use moli_page_types::RendererInspectorResponseDelivery;

impl InspectorOutbound {
    pub(in crate::script_vm::inspector) fn for_frontend(
        agent_token: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
        session_route: RendererInspectorSessionOutboundRoute,
        output_journal: Option<RendererTurnOutputJournal>,
    ) -> Self {
        let mut outbound = Self::for_agent(agent_token);
        outbound.session = Some(session);
        outbound.output_journal = output_journal;
        *outbound.session_route.borrow_mut() = Some(session_route);
        outbound
    }

    pub(in crate::script_vm::inspector) fn with_debugger_pauses_enabled(
        mut self,
        enabled: bool,
    ) -> Self {
        self.debugger_pauses_enabled = enabled;
        self
    }

    pub(in crate::script_vm::inspector) fn stage_pause_preface(
        &self,
        messages: Vec<RendererRuntimeInspectorMessage>,
    ) -> Option<RendererInspectorPausePrefaceGuard> {
        self.session_route
            .borrow()
            .as_ref()
            .and_then(|route| route.stage_pause_preface(messages))
    }

    /// Registers a real frontend command with its explicit terminal response
    /// destination. Internal Classic/BiDi adapter calls intentionally keep
    /// using `register_response_callback`, which always preserves their
    /// private adapter-reply capability.
    pub(in crate::script_vm) fn register_frontend_response_callback(
        &self,
        callback: RendererRuntimeInspectorResponseSender,
    ) {
        let callback = self.route_frontend_response(callback);
        self.register_response_callback(callback);
    }

    pub(in crate::script_vm) fn route_frontend_response(
        &self,
        callback: RendererRuntimeInspectorResponseSender,
    ) -> RendererRuntimeInspectorResponseSender {
        match callback.response_delivery() {
            RendererInspectorResponseDelivery::AdapterReply => callback,
            RendererInspectorResponseDelivery::SessionSink => {
                let host = self
                    .session
                    .clone()
                    .zip(self.output_journal.clone())
                    .zip(callback.renderer_agent_attachment_id())
                    .map(|((session, output_journal), attachment_id)| {
                        RendererDevToolsSessionOutputHost::new(
                            self.agent_token,
                            session,
                            attachment_id,
                            output_journal,
                        )
                    })
                    .expect("frontend session output requires an attachment-scoped Page journal");
                callback.route_to_devtools_session_output(host)
            }
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

    pub(in crate::script_vm) fn internal_dispatch_call_id_is_available(
        &self,
        call_id: i32,
    ) -> bool {
        let routing_available = {
            let routing = self.response_routing.borrow();
            !routing.pending_response_callbacks.contains_key(&call_id)
                && !routing.canceled_response_callbacks.contains_key(&call_id)
                && !routing
                    .internal_dispatch_response_call_ids
                    .contains(&call_id)
        };
        routing_available
            && !self
                .state()
                .borrow()
                .messages
                .iter()
                .any(|message| message.value.id_i64() == Some(i64::from(call_id)))
    }

    pub(in crate::script_vm::inspector) fn values_after(
        &self,
        snapshot_len: usize,
    ) -> Vec<InspectorJson> {
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
        command_output: Option<crate::devtools::pause::RendererInspectorPauseCommandOutputRoute>,
        value: Option<InspectorJson>,
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
        if let Some(value) = value {
            preface.push(RendererRuntimeInspectorMessage::from_v8_inspector_message(
                value,
            ));
        }
        if preface.is_empty() {
            if let Some(recorder) = command_turn_output {
                output_journal.append_records(recorder.drain_records());
            }
            let _ = output_journal.publish_pending();
            return;
        }
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

    pub(super) fn push_frontend_value(
        &self,
        session_route: RendererInspectorSessionOutboundRoute,
        value: InspectorJson,
    ) {
        match session_route.route_notification(&value) {
            RendererInspectorPauseNotificationRoute::OrdinaryTurn => self.push_local_value(value),
            RendererInspectorPauseNotificationRoute::PublishImmediately {
                preface,
                command_output,
            } => self.publish_pause_value(preface, command_output, Some(value)),
            RendererInspectorPauseNotificationRoute::PublishPrefix { preface } => {
                self.publish_pause_value(preface, None, None);
            }
            RendererInspectorPauseNotificationRoute::Drop => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devtools::pause::RendererInspectorPauseBridge;
    use crate::runtime::{
        PageId, RendererInspectorPauseCommandEffect, RendererOutputItem,
        RendererOutputStreamIdentity,
    };
    use crate::runtime::{RendererInspectorCommandRoute, RendererInspectorIngressTicket};

    fn routed_outbound() -> (
        InspectorOutbound,
        RendererInspectorPauseBridge,
        RendererTurnOutputJournal,
    ) {
        let journal = RendererTurnOutputJournal::new(
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(41)),
        );
        let pause_bridge = RendererInspectorPauseBridge::default();
        pause_bridge.configure_page_route(journal.clone());
        let agent_token = RendererDevToolsAgentToken::allocate();
        let session = DevToolsSessionKey::Primary;
        let io_ingress = crate::devtools::ingress::io::RendererInspectorIoIngress::new(
            pause_bridge.pause_loop_wake(),
            None,
        );
        let main_ingress = crate::devtools::ingress::main::RendererInspectorMainIngress::new(
            crate::devtools::route::RendererInspectorSessionExecutorRouteId::new(1),
            pause_bridge.pause_loop_wake(),
        );
        let outbound = InspectorOutbound::for_frontend(
            agent_token,
            session.clone(),
            crate::devtools::target::RendererDevToolsTargetHandle::new(
                pause_bridge.clone(),
                main_ingress,
                io_ingress,
            )
            .outbound_route(agent_token, session.clone()),
            Some(journal.clone()),
        );
        (outbound, pause_bridge, journal)
    }

    #[test]
    fn internal_response_does_not_complete_a_same_id_frontend_pause_command() {
        let (outbound, bridge, _) = routed_outbound();
        let route = outbound
            .session_route
            .borrow()
            .clone()
            .expect("frontend route");
        route.route_notification(
            &serde_json::json!({
                "method": "Debugger.paused", "params": {"callFrames": []}
            })
            .into(),
        );
        assert!(bridge.enter_pause().is_some());
        let dispatch = bridge.begin_command_dispatch(
            1,
            &RendererInspectorIngressTicket::new(None, None, RendererInspectorCommandRoute::Io),
            RendererInspectorPauseCommandEffect::Resume,
            Some(-1),
        );
        let recorder = RendererCommandTurnOutputRecorder::default();
        outbound
            .begin_command_turn_output(DevToolsSessionKey::Primary, recorder.clone())
            .expect("command output scope");
        let response = serde_json::json!({"id": -1, "result": {}});
        assert_eq!(
            outbound.capture_internal_response(-1, || {
                outbound.push_response_value(-1, response.clone());
            }),
            Some(response)
        );
        outbound.end_command_turn_output(&recorder);
        assert!(recorder.finish().is_empty());
        assert!(outbound.take_pending_messages().is_empty());
        drop(dispatch);
        bridge.leave_pause();
        assert!(
            matches!(
                route.route_notification(
                    &serde_json::json!({"method": "Debugger.resumed", "params": {}}).into()
                ),
                RendererInspectorPauseNotificationRoute::PublishImmediately {
                    command_output: None,
                    ..
                }
            ),
            "the internal response must not mark the frontend resume command successful"
        );
    }

    #[test]
    fn instrumentation_pause_prefix_publishes_context_created_with_bound_origin() {
        let (outbound, pause_bridge, journal) = routed_outbound();
        let recorder = RendererCommandTurnOutputRecorder::default();
        outbound
            .begin_command_turn_output(DevToolsSessionKey::Primary, recorder)
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
                        if value.payload().method() == Some("Runtime.consoleAPICalled")
                ))
        ));
        assert_eq!(
            outbound
                .take_pending_messages()
                .into_iter()
                .map(InspectorJson::into_value)
                .collect::<Vec<_>>(),
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
                        if value.payload().method() == Some("Runtime.consoleAPICalled")
                ))
        ));
    }
}
