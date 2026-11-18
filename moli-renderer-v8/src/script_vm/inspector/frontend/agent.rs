use super::{
    InspectorOutbound, RendererDevToolsAgent, RendererDevToolsAgentState,
    RendererDomDebuggerPauseScheduler, RendererDomDebuggerScheduledPause,
    RendererInspectorIsolateBackend, RendererInspectorObjectWrapper, RuntimeBindingRegistration,
};
use crate::script_vm::inspector::frontend::RendererInspectorSessionConnection;
use moli_page_types::{DevToolsSessionKey, V8InspectorSessionState};
use std::rc::Rc;

impl RendererInspectorObjectWrapper {
    pub(crate) fn wrap<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        session_key: &DevToolsSessionKey,
        value: v8::Local<'s, v8::Value>,
    ) -> Option<serde_json::Value> {
        let state = self.state.upgrade()?;
        let session = state
            .borrow()
            .sessions
            .frontend
            .frontend_session_and_outbound(session_key)?
            .0;
        let context = v8::Local::<v8::Object>::try_from(value)
            .ok()
            .and_then(|object| object.get_creation_context(scope))
            .unwrap_or_else(|| scope.get_current_context());
        let remote = session.wrap_object(
            scope,
            context,
            value,
            v8::inspector::StringView::from(&b""[..]),
            false,
        )?;
        let json = v8::crdtp::cbor_to_json(&remote.to_bytes())?;
        serde_json::from_slice(&json).ok()
    }
}

impl RendererDomDebuggerPauseScheduler {
    pub(crate) fn debugger_pauses_enabled(&self) -> bool {
        self.state
            .upgrade()
            .is_some_and(|state| state.borrow().debugger_pauses_enabled)
    }

    pub(crate) fn schedule_pause_on_next_statement(
        &self,
        session_keys: &[DevToolsSessionKey],
        reason: &str,
        detail: &str,
    ) -> RendererDomDebuggerScheduledPause {
        let sessions = self
            .state
            .upgrade()
            .map(|state| {
                state
                    .borrow()
                    .sessions
                    .frontend
                    .frontend_sessions(session_keys)
            })
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
                    .frontend
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

impl RendererDevToolsAgent {}

impl RendererDevToolsAgent {
    pub(in crate::script_vm::inspector) fn object_wrapper(&self) -> RendererInspectorObjectWrapper {
        RendererInspectorObjectWrapper {
            state: Rc::downgrade(&self.state),
        }
    }

    pub(in crate::script_vm::inspector) fn with_frontend<T>(
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
                .frontend
                .prepare_frontend(backend, &connection, session_key)
        };
        dispatch.run(op)
    }

    pub(in crate::script_vm::inspector) fn v8_state(
        &self,
        session_key: &DevToolsSessionKey,
    ) -> Option<V8InspectorSessionState> {
        self.state.borrow().sessions.frontend.v8_state(session_key)
    }

    pub(in crate::script_vm::inspector) fn v8_states(
        &self,
    ) -> Vec<(DevToolsSessionKey, V8InspectorSessionState)> {
        self.state.borrow().sessions.frontend.v8_states()
    }

    pub(in crate::script_vm::inspector) fn reattach_frontend(
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
            .frontend
            .reattach_frontend(backend, &connection, session_key, v8_state);
    }

    pub(in crate::script_vm::inspector) fn ensure_frontend(
        &self,
        backend: &mut RendererInspectorIsolateBackend,
        session_key: DevToolsSessionKey,
    ) {
        let mut state = self.state.borrow_mut();
        state.isolate_backend.assert_matches(backend);
        let connection = session_connection(&state);
        state
            .sessions
            .frontend
            .ensure_frontend(backend, &connection, session_key);
    }

    pub(in crate::script_vm::inspector) fn set_runtime_bindings(
        &self,
        session_key: DevToolsSessionKey,
        bindings: &[RuntimeBindingRegistration],
    ) {
        self.state
            .borrow_mut()
            .sessions
            .frontend
            .set_runtime_bindings(session_key, bindings);
    }

    pub(in crate::script_vm::inspector) fn runtime_bindings(
        &self,
        session_key: &DevToolsSessionKey,
    ) -> Vec<RuntimeBindingRegistration> {
        self.state
            .borrow()
            .sessions
            .frontend
            .runtime_bindings(session_key)
    }

    pub(in crate::script_vm::inspector) fn end_runtime_command_output(
        &self,
        session_key: &DevToolsSessionKey,
    ) {
        self.state
            .borrow()
            .sessions
            .frontend
            .end_runtime_command_output(session_key);
    }

    pub(in crate::script_vm::inspector) fn remove_frontend(
        &self,
        session_key: &DevToolsSessionKey,
    ) -> bool {
        self.state
            .borrow_mut()
            .sessions
            .frontend
            .remove_frontend(session_key)
    }

    pub(in crate::script_vm::inspector) fn cancel_response_callback(
        &self,
        session_key: &DevToolsSessionKey,
        call_id: i32,
    ) {
        self.state
            .borrow()
            .sessions
            .frontend
            .cancel_response_callback(session_key, call_id);
    }

    pub(in crate::script_vm::inspector) fn frontend_routes(
        &self,
    ) -> Vec<(DevToolsSessionKey, InspectorOutbound)> {
        self.state.borrow().sessions.frontend.frontend_routes()
    }
}

fn session_connection(state: &RendererDevToolsAgentState) -> RendererInspectorSessionConnection {
    RendererInspectorSessionConnection::new(
        state.isolate_backend.clone(),
        state.context_group_id,
        state.token,
        state.output_journal.clone(),
    )
    .with_debugger_pauses_enabled(state.debugger_pauses_enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script_vm::inspector::context_registry::DocumentInspectorContextRegistrationId;
    use crate::script_vm::inspector::v8_backend::RendererInspectorIsolateBackendHandle;
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
