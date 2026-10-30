use super::InspectorOutbound;
use super::agent_sessions::RendererInspectorSessions;
use super::context_registry::DocumentInspectorContextGroupId;
use super::v8_backend::{RendererInspectorIsolateBackend, RendererInspectorIsolateBackendHandle};
use crate::protocol_types::RuntimeBindingRegistration;
use crate::runtime::RendererTurnOutputJournal;
use moli_page_types::RendererDevToolsAgentToken;
#[cfg(feature = "devtools")]
use std::rc::Weak;
use std::{cell::RefCell, rc::Rc};

#[cfg(feature = "devtools")]
#[path = "frontend/agent.rs"]
mod frontend;

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
    debugger_pauses_enabled: bool,
    token: RendererDevToolsAgentToken,
    context_group_id: DocumentInspectorContextGroupId,
    isolate_backend: RendererInspectorIsolateBackendHandle,
    sessions: RendererInspectorSessions,
    output_journal: Option<RendererTurnOutputJournal>,
}

impl Drop for RendererDevToolsAgentState {
    fn drop(&mut self) {
        self.isolate_backend
            .remove_context_group(self.context_group_id);
    }
}

#[derive(Clone)]
pub(crate) struct RendererDomDebuggerPauseScheduler {
    #[cfg(feature = "devtools")]
    state: Weak<RefCell<RendererDevToolsAgentState>>,
}

/// Weak capability to wrap a native API exception in an existing frontend's
/// V8 session. The owning renderer agent retains all RemoteObject authority.
#[derive(Clone)]
#[cfg(feature = "devtools")]
pub(crate) struct RendererInspectorObjectWrapper {
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

impl RendererDevToolsAgent {
    pub(super) fn new(isolate_backend: RendererInspectorIsolateBackendHandle) -> Self {
        let context_group_id = DocumentInspectorContextGroupId::next();
        isolate_backend.bind_context_group(context_group_id);
        Self {
            state: Rc::new(RefCell::new(RendererDevToolsAgentState {
                debugger_pauses_enabled: false,
                token: RendererDevToolsAgentToken::allocate(),
                context_group_id,
                isolate_backend,
                sessions: RendererInspectorSessions::default(),
                output_journal: None,
            })),
        }
    }

    pub(super) fn configure_debugger(&self, enabled: bool) {
        self.state.borrow_mut().debugger_pauses_enabled = enabled;
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
            #[cfg(feature = "devtools")]
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

    pub(super) fn cancel_internal_runtime_evaluate_response(&self, call_id: i32) {
        self.state
            .borrow()
            .sessions
            .cancel_internal_runtime_evaluate_response(call_id);
    }

    pub(super) fn retire_notifications(&self) {
        self.state.borrow().sessions.retire_notifications();
    }

    pub(super) fn deactivate_all_routes(&self) {
        self.state.borrow().sessions.deactivate_all();
    }

    #[cfg(any(test, feature = "devtools"))]
    pub(super) fn session_count(&self) -> usize {
        self.state.borrow().sessions.frontend_len()
    }

    pub(super) fn assert_isolate_backend(&self, backend: &RendererInspectorIsolateBackend) {
        self.state.borrow().isolate_backend.assert_matches(backend);
    }
}

#[cfg(all(test, feature = "devtools"))]
impl RendererDevToolsAgent {
    pub(super) fn is_same_agent(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.state, &other.state)
    }

    pub(super) fn shares_isolate_backend_with(&self, other: &Self) -> bool {
        let isolate_backend = self.state.borrow().isolate_backend.clone();
        isolate_backend.is_same_backend(&other.state.borrow().isolate_backend)
    }
}
