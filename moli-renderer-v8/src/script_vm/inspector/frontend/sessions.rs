use super::super::InspectorOutbound;
use super::super::agent_sessions::{RendererInspectorSession, RendererInspectorSessionDispatch};
use super::super::context_registry::DocumentInspectorContextGroupId;
use super::super::v8_backend::{
    RendererInspectorIsolateBackend, RendererInspectorIsolateBackendHandle,
    RendererInspectorSessionExecutorRegistration,
};
use crate::protocol_types::RuntimeBindingRegistration;
use crate::runtime::RendererTurnOutputJournal;
use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken, V8InspectorSessionState};
use std::collections::HashMap;
use std::rc::Rc;

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
pub(in crate::script_vm::inspector) struct RendererFrontendInspectorSessions {
    sessions: HashMap<DevToolsSessionKey, RendererFrontendInspectorSession>,
    runtime_bindings_by_session: HashMap<DevToolsSessionKey, Vec<RuntimeBindingRegistration>>,
}

struct RendererFrontendInspectorSession {
    native: RendererInspectorSession,
    replayed_runtime_bindings: Vec<RuntimeBindingRegistration>,
    _executor_registration: RendererInspectorSessionExecutorRegistration,
}

impl RendererFrontendInspectorSession {
    fn v8_state(&self) -> V8InspectorSessionState {
        self.native.v8_state()
    }
}

impl RendererInspectorSession {
    fn v8_state(&self) -> V8InspectorSessionState {
        V8InspectorSessionState::from_bytes(self.session.state())
    }
}

#[derive(Clone)]
pub(in crate::script_vm::inspector) struct RendererInspectorSessionConnection {
    debugger_pauses_enabled: bool,
    isolate_backend: RendererInspectorIsolateBackendHandle,
    context_group_id: DocumentInspectorContextGroupId,
    agent_token: RendererDevToolsAgentToken,
    output_journal: Option<RendererTurnOutputJournal>,
}

impl RendererInspectorSessionConnection {
    pub(in crate::script_vm::inspector) fn new(
        isolate_backend: RendererInspectorIsolateBackendHandle,
        context_group_id: DocumentInspectorContextGroupId,
        agent_token: RendererDevToolsAgentToken,
        output_journal: Option<RendererTurnOutputJournal>,
    ) -> Self {
        Self {
            debugger_pauses_enabled: false,
            isolate_backend,
            context_group_id,
            agent_token,
            output_journal,
        }
    }

    fn outbound(&self, session_key: DevToolsSessionKey) -> InspectorOutbound {
        InspectorOutbound::for_frontend(
            self.agent_token,
            session_key.clone(),
            self.isolate_backend
                .devtools_target()
                .outbound_route(self.agent_token, session_key),
            self.output_journal.clone(),
        )
        .with_debugger_pauses_enabled(self.debugger_pauses_enabled)
    }

    pub(in crate::script_vm::inspector) fn with_debugger_pauses_enabled(
        mut self,
        enabled: bool,
    ) -> Self {
        self.debugger_pauses_enabled = enabled;
        self
    }

    fn connect_frontend(
        &self,
        backend: &mut RendererInspectorIsolateBackend,
        session_key: DevToolsSessionKey,
        outbound: InspectorOutbound,
        state: Option<&V8InspectorSessionState>,
    ) -> RendererFrontendInspectorSession {
        let native = RendererInspectorSession::connect_native(
            backend,
            self.context_group_id,
            outbound,
            state.map(V8InspectorSessionState::as_bytes),
        );
        let executor_registration = self.isolate_backend.register_session_executor_route(
            self.context_group_id,
            self.agent_token,
            session_key.clone(),
            &native.session,
            native.outbound.clone(),
        );
        #[cfg(feature = "devtools")]
        self.isolate_backend
            .devtools_target()
            .confirm_session_connection(self.agent_token, session_key);
        RendererFrontendInspectorSession {
            native,
            replayed_runtime_bindings: Vec::new(),
            _executor_registration: executor_registration,
        }
    }
}

impl RendererFrontendInspectorSessions {
    pub(in crate::script_vm::inspector) fn prepare_frontend(
        &mut self,
        backend: &mut RendererInspectorIsolateBackend,
        connection: &RendererInspectorSessionConnection,
        session_key: DevToolsSessionKey,
    ) -> RendererInspectorSessionDispatch {
        let stored_runtime_bindings = self
            .runtime_bindings_by_session
            .get(&session_key)
            .cloned()
            .unwrap_or_default();
        let session = self.sessions.entry(session_key.clone()).or_insert_with(|| {
            let outbound = connection.outbound(session_key.clone());
            connection.connect_frontend(backend, session_key.clone(), outbound, None)
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
        RendererInspectorSessionDispatch {
            session: Rc::clone(&session.native.session),
            outbound: session.native.outbound.clone(),
            runtime_bindings_to_replay,
        }
    }

    pub(in crate::script_vm::inspector) fn v8_state(
        &self,
        session_key: &DevToolsSessionKey,
    ) -> Option<V8InspectorSessionState> {
        self.sessions
            .get(session_key)
            .map(RendererFrontendInspectorSession::v8_state)
    }

    pub(in crate::script_vm::inspector) fn v8_states(
        &self,
    ) -> Vec<(DevToolsSessionKey, V8InspectorSessionState)> {
        let mut states = self
            .sessions
            .iter()
            .map(|(session_key, session)| (session_key.clone(), session.v8_state()))
            .collect::<Vec<_>>();
        states.sort_by(|(left, _), (right, _)| left.cmp(right));
        states
    }

    pub(in crate::script_vm::inspector) fn reattach_frontend(
        &mut self,
        backend: &mut RendererInspectorIsolateBackend,
        connection: &RendererInspectorSessionConnection,
        session_key: DevToolsSessionKey,
        v8_state: &V8InspectorSessionState,
    ) {
        let previous = self.sessions.remove(&session_key);
        let (outbound, replayed_runtime_bindings) = match previous {
            Some(previous) => {
                let RendererFrontendInspectorSession {
                    native: RendererInspectorSession { session, outbound },
                    replayed_runtime_bindings,
                    _executor_registration,
                } = previous;
                drop(session);
                drop(_executor_registration);
                (outbound, replayed_runtime_bindings)
            }
            None => (connection.outbound(session_key.clone()), Vec::new()),
        };
        let mut session =
            connection.connect_frontend(backend, session_key.clone(), outbound, Some(v8_state));
        session.replayed_runtime_bindings = replayed_runtime_bindings;
        self.sessions.insert(session_key, session);
    }

    pub(in crate::script_vm::inspector) fn ensure_frontend(
        &mut self,
        backend: &mut RendererInspectorIsolateBackend,
        connection: &RendererInspectorSessionConnection,
        session_key: DevToolsSessionKey,
    ) {
        self.sessions.entry(session_key.clone()).or_insert_with(|| {
            let outbound = connection.outbound(session_key.clone());
            connection.connect_frontend(backend, session_key.clone(), outbound, None)
        });
    }

    pub(in crate::script_vm::inspector) fn retain_replayed_runtime_bindings(
        &mut self,
        session_key: &DevToolsSessionKey,
        bindings: &[RuntimeBindingRegistration],
    ) {
        if let Some(session) = self.sessions.get_mut(session_key) {
            session
                .replayed_runtime_bindings
                .retain(|binding| bindings.contains(binding));
        }
    }

    pub(in crate::script_vm::inspector) fn set_runtime_bindings(
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

    pub(in crate::script_vm::inspector) fn runtime_bindings(
        &self,
        session_key: &DevToolsSessionKey,
    ) -> Vec<RuntimeBindingRegistration> {
        self.runtime_bindings_by_session
            .get(session_key)
            .cloned()
            .unwrap_or_default()
    }

    pub(in crate::script_vm::inspector) fn end_runtime_command_output(
        &self,
        session_key: &DevToolsSessionKey,
    ) {
        if let Some(session) = self.sessions.get(session_key) {
            session.native.outbound.end_runtime_command_output();
        }
    }

    pub(in crate::script_vm::inspector) fn remove_frontend(
        &mut self,
        session_key: &DevToolsSessionKey,
    ) -> bool {
        let removed_configuration = self
            .runtime_bindings_by_session
            .remove(session_key)
            .is_some();
        if let Some(session) = self.sessions.remove(session_key) {
            session.native.outbound.deactivate();
            drop(session);
            true
        } else {
            removed_configuration
        }
    }

    pub(in crate::script_vm::inspector) fn cancel_response_callback(
        &self,
        session_key: &DevToolsSessionKey,
        call_id: i32,
    ) {
        if let Some(session) = self.sessions.get(session_key) {
            session.native.outbound.cancel_response_callback(call_id);
        }
    }

    pub(in crate::script_vm::inspector) fn frontend_routes(
        &self,
    ) -> Vec<(DevToolsSessionKey, InspectorOutbound)> {
        self.sessions
            .iter()
            .map(|(key, session)| (key.clone(), session.native.outbound.clone()))
            .collect()
    }

    pub(in crate::script_vm::inspector) fn frontend_sessions(
        &self,
        session_keys: &[DevToolsSessionKey],
    ) -> Vec<Rc<v8::inspector::V8InspectorSession>> {
        session_keys
            .iter()
            .filter_map(|session_key| {
                self.sessions
                    .get(session_key)
                    .map(|session| Rc::clone(&session.native.session))
            })
            .collect()
    }

    pub(in crate::script_vm::inspector) fn frontend_session_and_outbound(
        &self,
        session_key: &DevToolsSessionKey,
    ) -> Option<(Rc<v8::inspector::V8InspectorSession>, InspectorOutbound)> {
        self.sessions.get(session_key).map(|session| {
            (
                Rc::clone(&session.native.session),
                session.native.outbound.clone(),
            )
        })
    }

    pub(in crate::script_vm::inspector) fn retire_notifications(&self) {
        for session in self.sessions.values() {
            session.native.outbound.retire_notifications();
        }
    }

    pub(in crate::script_vm::inspector) fn deactivate(&self) {
        for session in self.sessions.values() {
            session.native.outbound.deactivate();
        }
    }

    pub(in crate::script_vm::inspector) fn len(&self) -> usize {
        self.sessions.len()
    }
}

pub(in crate::script_vm::inspector) fn inspector_session_key(
    inspector_session_id: Option<&str>,
) -> DevToolsSessionKey {
    DevToolsSessionKey::from_wire_session_id(inspector_session_id.filter(|id| !id.is_empty()))
}

#[cfg(test)]
mod tests;
