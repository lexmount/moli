use super::{
    InspectorOutbound,
    agent_sessions::{RendererDevToolsAgentSessions, RendererDevToolsSessionConnection},
    context_registry::DocumentInspectorContextGroupId,
    v8_backend::{RendererInspectorIsolateBackend, RendererInspectorIsolateBackendHandle},
};
use crate::protocol_types::RuntimeBindingRegistration;
use crate::runtime::RendererTurnOutputJournal;
use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken, V8InspectorSessionState};
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

/// Inspector state owned by one renderer local-root agent.
///
/// Same-agent document replacement keeps this owner and its V8 sessions;
/// cross-Page replacement creates a new owner and restores sessions by cookie.
#[derive(Clone)]
pub(super) struct RendererDevToolsAgent {
    state: Rc<RefCell<RendererDevToolsAgentState>>,
}

impl std::fmt::Debug for RendererDevToolsAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.borrow();
        f.debug_struct("RendererDevToolsAgent")
            .field("token", &state.token)
            .field("context_group_id", &state.context_group_id.get())
            .field("isolate_backend", &state.isolate_backend)
            .field("session_count", &state.sessions.frontend_len())
            .finish_non_exhaustive()
    }
}

struct RendererDevToolsAgentState {
    token: RendererDevToolsAgentToken,
    context_group_id: DocumentInspectorContextGroupId,
    isolate_backend: RendererInspectorIsolateBackendHandle,
    sessions: RendererDevToolsAgentSessions,
    output_journal: Option<RendererTurnOutputJournal>,
}

#[derive(Clone)]
pub(crate) struct RendererDomDebuggerPauseScheduler {
    state: Weak<RefCell<RendererDevToolsAgentState>>,
}

#[must_use]
pub(crate) struct RendererDomDebuggerScheduledPause {
    sessions: Vec<Rc<v8::inspector::V8InspectorSession>>,
}

impl Drop for RendererDomDebuggerScheduledPause {
    fn drop(&mut self) {
        for session in &self.sessions {
            session.cancel_pause_on_next_statement();
        }
    }
}

impl RendererDomDebuggerPauseScheduler {
    pub(crate) fn schedule_pause_on_next_statement(
        &self,
        session_keys: &[DevToolsSessionKey],
        reason: &str,
        detail: &str,
    ) -> RendererDomDebuggerScheduledPause {
        let sessions = self
            .state
            .upgrade()
            .map(|state| state.borrow().sessions.frontend_sessions(session_keys))
            .unwrap_or_default();
        for session in &sessions {
            session.schedule_pause_on_next_statement(
                v8::inspector::StringView::from(reason.as_bytes()),
                v8::inspector::StringView::from(detail.as_bytes()),
            );
        }
        RendererDomDebuggerScheduledPause { sessions }
    }

    pub(crate) fn break_program_for_sessions(
        &self,
        pauses: Vec<(DevToolsSessionKey, String)>,
        reason: &str,
    ) {
        self.break_program_for_sessions_with_prefaces(
            pauses
                .into_iter()
                .map(|(session, detail)| (session, detail, Vec::new()))
                .collect(),
            reason,
        );
    }

    pub(crate) fn break_program_for_sessions_with_prefaces(
        &self,
        pauses: Vec<(
            DevToolsSessionKey,
            String,
            Vec<crate::runtime::RendererRuntimeInspectorMessage>,
        )>,
        reason: &str,
    ) {
        for (session_key, detail, preface) in pauses {
            let route = self.state.upgrade().and_then(|state| {
                state
                    .borrow()
                    .sessions
                    .frontend_session_and_outbound(&session_key)
            });
            let Some((session, outbound)) = route else {
                continue;
            };
            let _preface = outbound.stage_pause_preface(preface);
            session.break_program(
                v8::inspector::StringView::from(reason.as_bytes()),
                v8::inspector::StringView::from(detail.as_bytes()),
            );
        }
    }
}

impl RendererDevToolsAgent {
    pub(super) fn new(isolate_backend: RendererInspectorIsolateBackendHandle) -> Self {
        Self {
            state: Rc::new(RefCell::new(RendererDevToolsAgentState {
                token: RendererDevToolsAgentToken::allocate(),
                context_group_id: DocumentInspectorContextGroupId::next(),
                isolate_backend,
                sessions: RendererDevToolsAgentSessions::default(),
                output_journal: None,
            })),
        }
    }

    pub(super) fn bind_output_journal(&self, output_journal: RendererTurnOutputJournal) {
        let mut state = self.state.borrow_mut();
        assert_eq!(
            state.sessions.frontend_len(),
            0,
            "renderer output journal must be bound before Inspector frontend sessions"
        );
        assert!(
            state.output_journal.is_none(),
            "renderer DevTools agent output journal was bound twice"
        );
        state.output_journal = Some(output_journal);
    }

    pub(super) fn token(&self) -> RendererDevToolsAgentToken {
        self.state.borrow().token
    }

    pub(super) fn dom_debugger_pause_scheduler(&self) -> RendererDomDebuggerPauseScheduler {
        RendererDomDebuggerPauseScheduler {
            state: Rc::downgrade(&self.state),
        }
    }

    pub(super) fn context_group_id(&self) -> DocumentInspectorContextGroupId {
        self.state.borrow().context_group_id
    }

    pub(super) fn isolate_backend_handle(&self) -> RendererInspectorIsolateBackendHandle {
        self.state.borrow().isolate_backend.clone()
    }

    pub(super) fn with_internal_runtime_evaluate<T>(
        &self,
        backend: &mut RendererInspectorIsolateBackend,
        op: impl FnOnce(
            &v8::inspector::V8InspectorSession,
            InspectorOutbound,
            Vec<RuntimeBindingRegistration>,
        ) -> T,
    ) -> T {
        let dispatch = {
            let mut state = self.state.borrow_mut();
            state.isolate_backend.assert_matches(backend);
            let context_group_id = state.context_group_id;
            let token = state.token;
            let output_journal = state.output_journal.clone();
            state.sessions.prepare_internal_runtime_evaluate(
                backend,
                context_group_id,
                token,
                output_journal,
            )
        };
        dispatch.run(op)
    }

    pub(super) fn with_frontend<T>(
        &self,
        backend: &mut RendererInspectorIsolateBackend,
        session_key: DevToolsSessionKey,
        op: impl FnOnce(
            &v8::inspector::V8InspectorSession,
            InspectorOutbound,
            Vec<RuntimeBindingRegistration>,
        ) -> T,
    ) -> T {
        let dispatch = {
            let mut state = self.state.borrow_mut();
            state.isolate_backend.assert_matches(backend);
            let connection = session_connection(&state);
            state
                .sessions
                .prepare_frontend(backend, &connection, session_key)
        };
        dispatch.run(op)
    }

    pub(super) fn v8_state(
        &self,
        session_key: &DevToolsSessionKey,
    ) -> Option<V8InspectorSessionState> {
        self.state.borrow().sessions.v8_state(session_key)
    }

    pub(super) fn v8_states(&self) -> Vec<(DevToolsSessionKey, V8InspectorSessionState)> {
        self.state.borrow().sessions.v8_states()
    }

    pub(super) fn reattach_frontend(
        &self,
        backend: &mut RendererInspectorIsolateBackend,
        session_key: DevToolsSessionKey,
        v8_state: &V8InspectorSessionState,
    ) {
        let mut state = self.state.borrow_mut();
        state.isolate_backend.assert_matches(backend);
        let connection = session_connection(&state);
        state
            .sessions
            .reattach_frontend(backend, &connection, session_key, v8_state);
    }

    pub(super) fn ensure_frontend(
        &self,
        backend: &mut RendererInspectorIsolateBackend,
        session_key: DevToolsSessionKey,
    ) {
        let mut state = self.state.borrow_mut();
        state.isolate_backend.assert_matches(backend);
        let connection = session_connection(&state);
        state
            .sessions
            .ensure_frontend(backend, &connection, session_key);
    }

    pub(super) fn set_runtime_bindings(
        &self,
        session_key: DevToolsSessionKey,
        bindings: &[RuntimeBindingRegistration],
    ) {
        self.state
            .borrow_mut()
            .sessions
            .set_runtime_bindings(session_key, bindings);
    }

    pub(super) fn runtime_bindings(
        &self,
        session_key: &DevToolsSessionKey,
    ) -> Vec<RuntimeBindingRegistration> {
        self.state.borrow().sessions.runtime_bindings(session_key)
    }

    pub(super) fn end_runtime_command_output(&self, session_key: &DevToolsSessionKey) {
        self.state
            .borrow()
            .sessions
            .end_runtime_command_output(session_key);
    }

    pub(super) fn remove_frontend(&self, session_key: &DevToolsSessionKey) -> bool {
        self.state
            .borrow_mut()
            .sessions
            .remove_frontend(session_key)
    }

    pub(super) fn cancel_response_callback(&self, session_key: &DevToolsSessionKey, call_id: i32) {
        self.state
            .borrow()
            .sessions
            .cancel_response_callback(session_key, call_id);
    }

    pub(super) fn cancel_internal_runtime_evaluate_response(&self, call_id: i32) {
        self.state
            .borrow()
            .sessions
            .cancel_internal_runtime_evaluate_response(call_id);
    }

    pub(super) fn frontend_routes(&self) -> Vec<(DevToolsSessionKey, InspectorOutbound)> {
        self.state.borrow().sessions.frontend_routes()
    }

    pub(super) fn frontend_outbound(
        &self,
        session_key: &DevToolsSessionKey,
    ) -> Option<InspectorOutbound> {
        self.state
            .borrow()
            .sessions
            .frontend_session_and_outbound(session_key)
            .map(|(_, outbound)| outbound)
    }

    pub(super) fn deactivate_all_routes(&self) {
        self.state.borrow().sessions.deactivate_all();
    }

    pub(super) fn session_count(&self) -> usize {
        self.state.borrow().sessions.frontend_len()
    }

    pub(super) fn assert_isolate_backend(&self, backend: &RendererInspectorIsolateBackend) {
        self.state.borrow().isolate_backend.assert_matches(backend);
    }
}

fn session_connection(state: &RendererDevToolsAgentState) -> RendererDevToolsSessionConnection {
    RendererDevToolsSessionConnection::new(
        state.context_group_id,
        state.token,
        state.output_journal.clone(),
    )
}

#[cfg(test)]
impl RendererDevToolsAgent {
    pub(super) fn is_same_agent(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.state, &other.state)
    }

    pub(super) fn shares_isolate_backend_with(&self, other: &Self) -> bool {
        let isolate_backend = self.state.borrow().isolate_backend.clone();
        isolate_backend.is_same_backend(&other.state.borrow().isolate_backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script_vm::inspector::context_registry::DocumentInspectorContextRegistrationId;
    use std::pin::pin;

    #[test]
    fn same_agent_context_reset_keeps_its_frontend_v8_session() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let mut backend = RendererInspectorIsolateBackend::new(&mut isolate);
        let agent = RendererDevToolsAgent::new(backend.handle());
        let session_key = DevToolsSessionKey::Primary;

        let session_before =
            agent.with_frontend(&mut backend, session_key.clone(), |session, _, _| {
                std::ptr::from_ref(session)
            });
        let default_context = {
            let scope = pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            backend.context_created_with_unique_id(
                context,
                agent.context_group_id(),
                b"https://document-open.test/",
                b"https://document-open.test",
                br#"{"isDefault":true,"type":"default"}"#,
            );
            v8::Global::new(scope, context)
        };
        backend.context_registry.set_default_context(
            agent.context_group_id(),
            default_context,
            DocumentInspectorContextRegistrationId::next(),
        );

        assert!(
            backend.reset_default_context_group_before_replacement(agent.context_group_id()),
            "same-agent document replacement should reset its existing context group"
        );
        let session_after = agent.with_frontend(&mut backend, session_key, |session, _, _| {
            std::ptr::from_ref(session)
        });

        assert_eq!(
            session_before, session_after,
            "same-agent document replacement must not detach its frontend V8 session"
        );
        assert_eq!(agent.session_count(), 1);
    }

    #[test]
    fn agent_rejects_a_different_isolate_inspector_backend() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let backend = RendererInspectorIsolateBackend::new(&mut isolate);
        let agent =
            RendererDevToolsAgent::new(RendererInspectorIsolateBackendHandle::new_for_test());

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            agent.assert_isolate_backend(&backend);
        }));

        assert!(
            result.is_err(),
            "an agent must not connect sessions or contexts through another isolate's backend"
        );
    }
}
