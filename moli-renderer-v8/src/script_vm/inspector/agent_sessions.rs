use super::{
    InspectorOutbound,
    context_registry::DocumentInspectorContextGroupId,
    v8_backend::{RendererInspectorIsolateBackend, RendererInspectorSessionExecutorRegistration},
};
use crate::protocol_types::RuntimeBindingRegistration;
use crate::runtime::RendererTurnOutputJournal;
use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken, V8InspectorSessionState};
use std::{collections::HashMap, rc::Rc};

#[derive(Clone, Copy)]
pub(in crate::script_vm) enum PageInspectorSessionTarget<'a> {
    Frontend(Option<&'a str>),
    InternalRuntimeEvaluate,
}

impl<'a> PageInspectorSessionTarget<'a> {
    pub(in crate::script_vm) fn frontend_session_id(self) -> Option<Option<&'a str>> {
        match self {
            Self::Frontend(inspector_session_id) => Some(inspector_session_id),
            Self::InternalRuntimeEvaluate => None,
        }
    }
}

#[derive(Default)]
pub(super) struct RendererDevToolsAgentSessions {
    frontend: HashMap<DevToolsSessionKey, RendererDevToolsSession>,
    runtime_bindings_by_session: HashMap<DevToolsSessionKey, Vec<RuntimeBindingRegistration>>,
    internal_runtime_evaluate: Option<RendererDevToolsSession>,
}

pub(super) struct RendererDevToolsSessionDispatch {
    session: Rc<v8::inspector::V8InspectorSession>,
    outbound: InspectorOutbound,
    runtime_bindings_to_replay: Vec<RuntimeBindingRegistration>,
}

impl RendererDevToolsSessionDispatch {
    pub(super) fn run<T>(
        self,
        op: impl FnOnce(
            &v8::inspector::V8InspectorSession,
            InspectorOutbound,
            Vec<RuntimeBindingRegistration>,
        ) -> T,
    ) -> T {
        op(
            self.session.as_ref(),
            self.outbound,
            self.runtime_bindings_to_replay,
        )
    }
}

#[derive(Clone)]
pub(super) struct RendererDevToolsSessionConnection {
    context_group_id: DocumentInspectorContextGroupId,
    agent_token: RendererDevToolsAgentToken,
    output_journal: Option<RendererTurnOutputJournal>,
}

impl RendererDevToolsSessionConnection {
    pub(super) fn new(
        context_group_id: DocumentInspectorContextGroupId,
        agent_token: RendererDevToolsAgentToken,
        output_journal: Option<RendererTurnOutputJournal>,
    ) -> Self {
        Self {
            context_group_id,
            agent_token,
            output_journal,
        }
    }

    fn outbound(
        &self,
        backend: &RendererInspectorIsolateBackend,
        session_key: DevToolsSessionKey,
    ) -> InspectorOutbound {
        InspectorOutbound::for_frontend(
            self.agent_token,
            session_key.clone(),
            backend
                .devtools_target()
                .outbound_route(self.agent_token, session_key),
            self.output_journal.clone(),
        )
    }
}

impl RendererDevToolsAgentSessions {
    pub(super) fn prepare_internal_runtime_evaluate(
        &mut self,
        backend: &mut RendererInspectorIsolateBackend,
        context_group_id: DocumentInspectorContextGroupId,
        agent_token: RendererDevToolsAgentToken,
        output_journal: Option<RendererTurnOutputJournal>,
    ) -> RendererDevToolsSessionDispatch {
        let session = self.internal_runtime_evaluate.get_or_insert_with(|| {
            RendererDevToolsSession::connect(
                backend,
                context_group_id,
                InspectorOutbound::for_agent(agent_token).with_output_journal(output_journal),
                None,
                None,
            )
        });
        RendererDevToolsSessionDispatch {
            session: Rc::clone(&session.session),
            outbound: session.outbound.clone(),
            runtime_bindings_to_replay: Vec::new(),
        }
    }

    pub(super) fn prepare_frontend(
        &mut self,
        backend: &mut RendererInspectorIsolateBackend,
        connection: &RendererDevToolsSessionConnection,
        session_key: DevToolsSessionKey,
    ) -> RendererDevToolsSessionDispatch {
        let stored_runtime_bindings = self
            .runtime_bindings_by_session
            .get(&session_key)
            .cloned()
            .unwrap_or_default();
        let session = self.frontend.entry(session_key.clone()).or_insert_with(|| {
            let outbound = connection.outbound(backend, session_key.clone());
            RendererDevToolsSession::connect(
                backend,
                connection.context_group_id,
                outbound,
                None,
                Some((connection.agent_token, session_key.clone())),
            )
        });
        session
            .replayed_runtime_bindings
            .retain(|binding| stored_runtime_bindings.contains(binding));
        let runtime_bindings_to_replay = stored_runtime_bindings
            .iter()
            .filter(|binding| !session.replayed_runtime_bindings.contains(*binding))
            .cloned()
            .collect::<Vec<_>>();
        session
            .replayed_runtime_bindings
            .extend(runtime_bindings_to_replay.iter().cloned());
        RendererDevToolsSessionDispatch {
            session: Rc::clone(&session.session),
            outbound: session.outbound.clone(),
            runtime_bindings_to_replay,
        }
    }

    pub(super) fn v8_state(
        &self,
        session_key: &DevToolsSessionKey,
    ) -> Option<V8InspectorSessionState> {
        self.frontend
            .get(session_key)
            .map(RendererDevToolsSession::v8_state)
    }

    pub(super) fn v8_states(&self) -> Vec<(DevToolsSessionKey, V8InspectorSessionState)> {
        let mut states = self
            .frontend
            .iter()
            .map(|(session_key, session)| (session_key.clone(), session.v8_state()))
            .collect::<Vec<_>>();
        states.sort_by(|(left, _), (right, _)| left.cmp(right));
        states
    }

    pub(super) fn reattach_frontend(
        &mut self,
        backend: &mut RendererInspectorIsolateBackend,
        connection: &RendererDevToolsSessionConnection,
        session_key: DevToolsSessionKey,
        v8_state: &V8InspectorSessionState,
    ) {
        let previous = self.frontend.remove(&session_key);
        let (outbound, replayed_runtime_bindings) = match previous {
            Some(previous) => {
                let RendererDevToolsSession {
                    session,
                    outbound,
                    replayed_runtime_bindings,
                    _executor_registration,
                } = previous;
                drop(session);
                drop(_executor_registration);
                (outbound, replayed_runtime_bindings)
            }
            None => (
                connection.outbound(backend, session_key.clone()),
                Vec::new(),
            ),
        };
        let mut session = RendererDevToolsSession::connect(
            backend,
            connection.context_group_id,
            outbound,
            Some(v8_state),
            Some((connection.agent_token, session_key.clone())),
        );
        session.replayed_runtime_bindings = replayed_runtime_bindings;
        self.frontend.insert(session_key, session);
    }

    pub(super) fn ensure_frontend(
        &mut self,
        backend: &mut RendererInspectorIsolateBackend,
        connection: &RendererDevToolsSessionConnection,
        session_key: DevToolsSessionKey,
    ) {
        self.frontend.entry(session_key.clone()).or_insert_with(|| {
            let outbound = connection.outbound(backend, session_key.clone());
            RendererDevToolsSession::connect(
                backend,
                connection.context_group_id,
                outbound,
                None,
                Some((connection.agent_token, session_key.clone())),
            )
        });
    }

    pub(super) fn retain_replayed_runtime_bindings(
        &mut self,
        session_key: &DevToolsSessionKey,
        bindings: &[RuntimeBindingRegistration],
    ) {
        if let Some(session) = self.frontend.get_mut(session_key) {
            session
                .replayed_runtime_bindings
                .retain(|binding| bindings.contains(binding));
        }
    }

    pub(super) fn set_runtime_bindings(
        &mut self,
        session_key: DevToolsSessionKey,
        bindings: &[RuntimeBindingRegistration],
    ) {
        if bindings.is_empty() {
            self.runtime_bindings_by_session.remove(&session_key);
        } else {
            self.runtime_bindings_by_session
                .insert(session_key.clone(), bindings.to_vec());
        }
        self.retain_replayed_runtime_bindings(&session_key, bindings);
    }

    pub(super) fn runtime_bindings(
        &self,
        session_key: &DevToolsSessionKey,
    ) -> Vec<RuntimeBindingRegistration> {
        self.runtime_bindings_by_session
            .get(session_key)
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn end_runtime_command_output(&self, session_key: &DevToolsSessionKey) {
        if let Some(session) = self.frontend.get(session_key) {
            session.outbound.end_runtime_command_output();
        }
    }

    pub(super) fn remove_frontend(&mut self, session_key: &DevToolsSessionKey) -> bool {
        let removed_configuration = self
            .runtime_bindings_by_session
            .remove(session_key)
            .is_some();
        if let Some(session) = self.frontend.remove(session_key) {
            session.outbound.deactivate();
            drop(session);
            true
        } else {
            removed_configuration
        }
    }

    pub(super) fn cancel_response_callback(&self, session_key: &DevToolsSessionKey, call_id: i32) {
        if let Some(session) = self.frontend.get(session_key) {
            session.outbound.cancel_response_callback(call_id);
        }
    }

    pub(super) fn cancel_internal_runtime_evaluate_response(&self, call_id: i32) {
        if let Some(session) = &self.internal_runtime_evaluate {
            session.outbound.cancel_response_callback(call_id);
        }
    }

    pub(super) fn frontend_routes(&self) -> Vec<(DevToolsSessionKey, InspectorOutbound)> {
        self.frontend
            .iter()
            .map(|(key, session)| (key.clone(), session.outbound.clone()))
            .collect()
    }

    pub(super) fn frontend_sessions(
        &self,
        session_keys: &[DevToolsSessionKey],
    ) -> Vec<Rc<v8::inspector::V8InspectorSession>> {
        session_keys
            .iter()
            .filter_map(|session_key| {
                self.frontend
                    .get(session_key)
                    .map(|session| Rc::clone(&session.session))
            })
            .collect()
    }

    pub(super) fn frontend_session_and_outbound(
        &self,
        session_key: &DevToolsSessionKey,
    ) -> Option<(Rc<v8::inspector::V8InspectorSession>, InspectorOutbound)> {
        self.frontend
            .get(session_key)
            .map(|session| (Rc::clone(&session.session), session.outbound.clone()))
    }

    pub(super) fn deactivate_all(&self) {
        for session in self.frontend.values() {
            session.outbound.deactivate();
        }
        if let Some(session) = &self.internal_runtime_evaluate {
            session.outbound.deactivate();
        }
    }

    pub(super) fn frontend_len(&self) -> usize {
        self.frontend.len()
    }
}

struct RendererDevToolsSession {
    session: Rc<v8::inspector::V8InspectorSession>,
    outbound: InspectorOutbound,
    replayed_runtime_bindings: Vec<RuntimeBindingRegistration>,
    _executor_registration: Option<RendererInspectorSessionExecutorRegistration>,
}

impl RendererDevToolsSession {
    fn connect(
        backend: &mut RendererInspectorIsolateBackend,
        context_group_id: DocumentInspectorContextGroupId,
        outbound: InspectorOutbound,
        state: Option<&V8InspectorSessionState>,
        frontend_session_route: Option<(RendererDevToolsAgentToken, DevToolsSessionKey)>,
    ) -> Self {
        let first_attach_state = b"{}";
        let state = state.map_or(
            first_attach_state.as_slice(),
            V8InspectorSessionState::as_bytes,
        );
        let session = backend.connect_session(
            context_group_id,
            v8::inspector::Channel::new(Box::new(RendererInspectorChannel {
                outbound: outbound.clone(),
            })),
            state,
        );
        let session = Rc::new(session);
        let executor_registration = frontend_session_route.map(|(agent_token, session_key)| {
            backend.register_session_executor_route(
                context_group_id,
                agent_token,
                session_key,
                &session,
                outbound.clone(),
            )
        });
        Self {
            session,
            outbound,
            replayed_runtime_bindings: Vec::new(),
            _executor_registration: executor_registration,
        }
    }

    fn v8_state(&self) -> V8InspectorSessionState {
        V8InspectorSessionState::from_bytes(self.session.state())
    }
}

struct RendererInspectorChannel {
    outbound: InspectorOutbound,
}

impl v8::inspector::ChannelImpl for RendererInspectorChannel {
    fn send_response(&self, call_id: i32, message: v8::UniquePtr<v8::inspector::StringBuffer>) {
        self.outbound.push_response_message(call_id, message);
    }

    fn send_notification(&self, message: v8::UniquePtr<v8::inspector::StringBuffer>) {
        self.outbound.push_message(message);
    }

    fn flush_protocol_notifications(&self) {}
}

pub(super) fn inspector_session_key(inspector_session_id: Option<&str>) -> DevToolsSessionKey {
    DevToolsSessionKey::from_wire_session_id(inspector_session_id.filter(|id| !id.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script_vm::inspector::context_registry::DocumentInspectorContextRegistrationId;
    use serde_json::Value;
    use std::pin::pin;

    fn state_json(state: &V8InspectorSessionState) -> Value {
        let json = v8::crdtp::cbor_to_json(state.as_bytes())
            .expect("V8 Inspector session state should be valid CBOR");
        serde_json::from_slice(&json).expect("V8 Inspector session state should contain JSON")
    }

    fn dispatch(session: &RendererDevToolsSession, request: &str) -> Vec<Value> {
        let snapshot = session.outbound.len();
        let _response_capture = session.outbound.capture_dispatch_responses();
        session
            .session
            .dispatch_protocol_message(v8::inspector::StringView::from(request.as_bytes()));
        session.outbound.take_messages_after(snapshot)
    }

    fn assert_successful_response(messages: &[Value], call_id: u64) {
        let response = messages
            .iter()
            .find(|message| message.get("id").and_then(Value::as_u64) == Some(call_id))
            .unwrap_or_else(|| panic!("missing Inspector response {call_id}: {messages:?}"));
        assert!(
            response.get("error").is_none(),
            "Inspector response {call_id} failed: {response:?}"
        );
    }

    #[test]
    fn frontend_reattach_preserves_replay_state_without_exposing_internal_session() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
        let context_group_id = DocumentInspectorContextGroupId::next();
        let agent_token = RendererDevToolsAgentToken::allocate();
        let connection =
            RendererDevToolsSessionConnection::new(context_group_id, agent_token, None);
        let session_key = DevToolsSessionKey::Attached("SID-agent-sessions".to_owned());
        let binding = RuntimeBindingRegistration {
            name: "bridge".to_owned(),
            execution_context_name: Some("utility".to_owned()),
        };
        let mut sessions = RendererDevToolsAgentSessions::default();
        sessions.set_runtime_bindings(session_key.clone(), std::slice::from_ref(&binding));

        let first_replay = sessions
            .prepare_frontend(&mut backend, &connection, session_key.clone())
            .run(|_, _, replay| replay);
        let second_replay = sessions
            .prepare_frontend(&mut backend, &connection, session_key.clone())
            .run(|_, _, replay| replay);
        assert_eq!(first_replay, vec![binding.clone()]);
        assert!(second_replay.is_empty(), "a live session must replay once");

        sessions
            .prepare_internal_runtime_evaluate(&mut backend, context_group_id, agent_token, None)
            .run(|_, _, _| {});
        assert_eq!(
            sessions.frontend_routes().len(),
            1,
            "the internal Runtime.evaluate session must not become frontend-visible"
        );
        assert_eq!(
            sessions.v8_states().len(),
            1,
            "the internal Runtime.evaluate session must not enter restore snapshots"
        );

        sessions.reattach_frontend(
            &mut backend,
            &connection,
            session_key.clone(),
            &V8InspectorSessionState::from_bytes(Vec::new()),
        );
        let replay_after_reattach = sessions
            .prepare_frontend(&mut backend, &connection, session_key)
            .run(|_, _, replay| replay);
        assert!(
            replay_after_reattach.is_empty(),
            "reattach must preserve session-local binding replay bookkeeping"
        );
    }

    #[test]
    fn runtime_binding_configuration_is_agent_local() {
        let session_key = DevToolsSessionKey::Attached("SID-agent-local-config".to_owned());
        let binding = RuntimeBindingRegistration {
            name: "bridge".to_owned(),
            execution_context_name: Some("utility".to_owned()),
        };
        let mut first_agent_sessions = RendererDevToolsAgentSessions::default();
        let mut replacement_agent_sessions = RendererDevToolsAgentSessions::default();

        first_agent_sessions
            .set_runtime_bindings(session_key.clone(), std::slice::from_ref(&binding));

        assert_eq!(
            first_agent_sessions.runtime_bindings(&session_key),
            vec![binding.clone()]
        );
        assert!(
            replacement_agent_sessions
                .runtime_bindings(&session_key)
                .is_empty(),
            "a replacement agent must not inherit renderer-local configuration"
        );

        replacement_agent_sessions
            .set_runtime_bindings(session_key.clone(), std::slice::from_ref(&binding));
        assert_eq!(
            replacement_agent_sessions.runtime_bindings(&session_key),
            vec![binding],
            "the protocol restore path must configure the replacement agent explicitly"
        );
    }

    #[test]
    fn inspector_session_state_round_trips_runtime_and_profiler_domains() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
        let context_group_id = DocumentInspectorContextGroupId::next();
        let registration_id = DocumentInspectorContextRegistrationId::next();

        let default_context = {
            let scope = pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            backend.context_created_with_unique_id(
                context,
                context_group_id,
                b"https://example.test/",
                b"https://example.test",
                br#"{"isDefault":true,"type":"default"}"#,
            );
            v8::Global::new(scope, context)
        };
        backend.context_registry.set_default_context(
            context_group_id,
            default_context,
            registration_id,
        );

        let first_attach = RendererDevToolsSession::connect(
            &mut backend,
            context_group_id,
            InspectorOutbound::for_agent(RendererDevToolsAgentToken::allocate()),
            None,
            None,
        );
        assert_successful_response(
            &dispatch(&first_attach, r#"{"id":1,"method":"Runtime.enable"}"#),
            1,
        );
        assert_successful_response(
            &dispatch(
                &first_attach,
                r#"{"id":2,"method":"Profiler.setSamplingInterval","params":{"interval":937}}"#,
            ),
            2,
        );
        assert_successful_response(
            &dispatch(&first_attach, r#"{"id":3,"method":"Profiler.enable"}"#),
            3,
        );

        let state = first_attach.v8_state();
        assert!(
            !state.is_empty(),
            "stateful Inspector session should emit state"
        );
        let initial_state_json = state_json(&state);
        let initial_state_text = initial_state_json.to_string();
        assert!(
            initial_state_text.contains("runtime")
                && initial_state_text.contains("profiler")
                && initial_state_text.contains("937"),
            "state should contain Runtime and Profiler options: {initial_state_json}"
        );
        drop(first_attach);

        let restored = RendererDevToolsSession::connect(
            &mut backend,
            context_group_id,
            InspectorOutbound::for_agent(RendererDevToolsAgentToken::allocate()),
            Some(&state),
            None,
        );
        let restored_messages = restored.outbound.values_after(0);
        let restored_context = restored_messages
            .iter()
            .find(|message| {
                message.get("method").and_then(Value::as_str)
                    == Some("Runtime.executionContextCreated")
            })
            .unwrap_or_else(|| {
                panic!(
                    "restored Runtime.enable should publish the existing context: {restored_messages:?}"
                )
            });
        assert_eq!(
            restored_context["params"]["context"]["origin"], "https://example.test",
            "V8ContextInfo must carry the embedder-bound origin without outbound JSON rewriting"
        );
        let restored_state = restored.v8_state();
        assert_eq!(
            state_json(&restored_state),
            initial_state_json,
            "V8 should preserve opaque Runtime and Profiler state across reconnect"
        );
    }

    #[test]
    fn runtime_reattach_before_replacement_context_reports_clear_then_created() {
        crate::ensure_v8_for_test();
        let runtime_state = {
            let mut isolate = v8::Isolate::new(Default::default());
            let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
            let context_group_id = DocumentInspectorContextGroupId::next();
            let registration_id = DocumentInspectorContextRegistrationId::next();
            let default_context = {
                let scope = pin!(v8::HandleScope::new(&mut isolate));
                let scope = &mut scope.init();
                let context = v8::Context::new(scope, Default::default());
                backend.context_created_with_unique_id(
                    context,
                    context_group_id,
                    b"https://example.test/old",
                    b"https://example.test",
                    br#"{"isDefault":true,"type":"default"}"#,
                );
                v8::Global::new(scope, context)
            };
            backend.context_registry.set_default_context(
                context_group_id,
                default_context,
                registration_id,
            );

            let session = RendererDevToolsSession::connect(
                &mut backend,
                context_group_id,
                InspectorOutbound::for_agent(RendererDevToolsAgentToken::allocate()),
                None,
                None,
            );
            assert_successful_response(
                &dispatch(&session, r#"{"id":1,"method":"Runtime.enable"}"#),
                1,
            );
            session.v8_state()
        };

        let mut replacement_isolate = v8::Isolate::new(Default::default());
        let mut replacement_backend =
            RendererInspectorIsolateBackend::new(&mut replacement_isolate);
        let replacement_context_group_id = DocumentInspectorContextGroupId::next();
        let replacement_registration_id = DocumentInspectorContextRegistrationId::next();
        let restored = RendererDevToolsSession::connect(
            &mut replacement_backend,
            replacement_context_group_id,
            InspectorOutbound::for_agent(RendererDevToolsAgentToken::allocate()),
            Some(&runtime_state),
            None,
        );

        let replacement_context = {
            let scope = pin!(v8::HandleScope::new(&mut replacement_isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            replacement_backend.context_created_with_unique_id(
                context,
                replacement_context_group_id,
                b"https://example.test/new",
                b"https://example.test",
                br#"{"isDefault":true,"type":"default"}"#,
            );
            v8::Global::new(scope, context)
        };
        replacement_backend.context_registry.set_default_context(
            replacement_context_group_id,
            replacement_context,
            replacement_registration_id,
        );

        let messages = restored.outbound.values_after(0);
        let cleared_index = messages
            .iter()
            .position(|message| {
                message.get("method").and_then(Value::as_str)
                    == Some("Runtime.executionContextsCleared")
            })
            .unwrap_or_else(|| {
                panic!("restored Runtime session should clear old contexts: {messages:?}")
            });
        let created_index = messages
            .iter()
            .position(|message| {
                message.get("method").and_then(Value::as_str)
                    == Some("Runtime.executionContextCreated")
            })
            .unwrap_or_else(|| {
                panic!("restored Runtime session should report the new context: {messages:?}")
            });
        assert!(
            cleared_index < created_index,
            "replacement context must be reported after old contexts are cleared: {messages:?}"
        );
    }

    #[test]
    fn inspector_session_accepts_empty_reattach_state() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
        let context_group_id = DocumentInspectorContextGroupId::next();
        let empty_state = V8InspectorSessionState::from_bytes(Vec::new());

        let session = RendererDevToolsSession::connect(
            &mut backend,
            context_group_id,
            InspectorOutbound::for_agent(RendererDevToolsAgentToken::allocate()),
            Some(&empty_state),
            None,
        );
        assert_successful_response(
            &dispatch(&session, r#"{"id":1,"method":"Runtime.enable"}"#),
            1,
        );
        assert!(
            v8::crdtp::cbor_to_json(&session.session.state()).is_some(),
            "empty reattach should produce a valid V8 state cookie after dispatch"
        );
    }
}
