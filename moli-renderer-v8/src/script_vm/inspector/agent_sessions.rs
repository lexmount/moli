use super::InspectorOutbound;
use super::context_registry::DocumentInspectorContextGroupId;
#[cfg(feature = "devtools")]
use super::frontend::RendererFrontendInspectorSessions;
use super::v8_backend::RendererInspectorIsolateBackend;
use crate::protocol_types::RuntimeBindingRegistration;
use crate::runtime::RendererTurnOutputJournal;
use moli_page_types::RendererDevToolsAgentToken;
use std::rc::Rc;

#[cfg(not(feature = "devtools"))]
#[derive(Clone, Copy)]
pub(in crate::script_vm) enum InternalInspectorSessionTarget {
    InternalRuntimeEvaluate,
}

#[cfg(not(feature = "devtools"))]
pub(in crate::script_vm) type PageInspectorSessionTarget<'a> = InternalInspectorSessionTarget;

#[derive(Default)]
pub(super) struct RendererInspectorSessions {
    #[cfg(feature = "devtools")]
    pub(super) frontend: RendererFrontendInspectorSessions,
    internal_runtime_evaluate: Option<RendererInspectorSession>,
}

pub(super) struct RendererInspectorSessionDispatch {
    pub(super) session: Rc<v8::inspector::V8InspectorSession>,
    pub(super) outbound: InspectorOutbound,
    pub(super) runtime_bindings_to_replay: Vec<RuntimeBindingRegistration>,
}

impl RendererInspectorSessionDispatch {
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

impl RendererInspectorSessions {
    pub(super) fn prepare_internal_runtime_evaluate(
        &mut self,
        backend: &mut RendererInspectorIsolateBackend,
        context_group_id: DocumentInspectorContextGroupId,
        agent_token: RendererDevToolsAgentToken,
        output_journal: Option<RendererTurnOutputJournal>,
    ) -> RendererInspectorSessionDispatch {
        let session = self.internal_runtime_evaluate.get_or_insert_with(|| {
            RendererInspectorSession::connect_internal(
                backend,
                context_group_id,
                InspectorOutbound::for_agent(agent_token).with_output_journal(output_journal),
            )
        });
        RendererInspectorSessionDispatch {
            session: Rc::clone(&session.session),
            outbound: session.outbound.clone(),
            runtime_bindings_to_replay: Vec::new(),
        }
    }

    pub(super) fn cancel_internal_runtime_evaluate_response(&self, call_id: i32) {
        if let Some(session) = &self.internal_runtime_evaluate {
            session.outbound.cancel_response_callback(call_id);
        }
    }

    pub(super) fn retire_notifications(&self) {
        #[cfg(feature = "devtools")]
        self.frontend.retire_notifications();
        if let Some(session) = &self.internal_runtime_evaluate {
            session.outbound.retire_notifications();
        }
    }

    pub(super) fn deactivate_all(&self) {
        #[cfg(feature = "devtools")]
        self.frontend.deactivate();
        if let Some(session) = &self.internal_runtime_evaluate {
            session.outbound.deactivate();
        }
    }

    pub(super) fn frontend_len(&self) -> usize {
        #[cfg(feature = "devtools")]
        {
            self.frontend.len()
        }
        #[cfg(not(feature = "devtools"))]
        {
            0
        }
    }
}

pub(super) struct RendererInspectorSession {
    pub(super) session: Rc<v8::inspector::V8InspectorSession>,
    pub(super) outbound: InspectorOutbound,
}

impl RendererInspectorSession {
    fn connect_internal(
        backend: &mut RendererInspectorIsolateBackend,
        context_group_id: DocumentInspectorContextGroupId,
        outbound: InspectorOutbound,
    ) -> Self {
        Self::connect_native(backend, context_group_id, outbound, None)
    }

    pub(super) fn connect_native(
        backend: &mut RendererInspectorIsolateBackend,
        context_group_id: DocumentInspectorContextGroupId,
        mut outbound: InspectorOutbound,
        state: Option<&[u8]>,
    ) -> Self {
        let native_binding = outbound.prepare_native_session_binding();
        let session = backend.connect_session(
            context_group_id,
            v8::inspector::Channel::new(Box::new(RendererInspectorChannel {
                outbound: outbound.clone(),
            })),
            state.unwrap_or(b"{}"),
        );
        // Establish the default before publishing any frontend routes.
        // A restored session already carries its explicit pause settings.
        if state.is_none() {
            session.set_skip_all_pauses(!outbound.debugger_pauses_enabled());
        }
        let session = Rc::new(session);
        *native_binding.borrow_mut() = Rc::downgrade(&session);
        Self { session, outbound }
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
