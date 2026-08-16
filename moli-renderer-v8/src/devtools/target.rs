use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken};

use super::{
    ingress::{io::RendererInspectorIoIngress, main::RendererInspectorMainIngress},
    pause::{RendererInspectorPauseBridge, RendererInspectorSessionOutboundRoute},
};
use crate::runtime::PageId;

/// Cloneable control-plane handle for one renderer DevTools target.
///
/// The handle is safe to retain outside the isolate owner stack. It exposes
/// only ingress and lifecycle coordination; all V8 session access remains in
/// the renderer-local Inspector executor.
#[derive(Clone)]
pub(crate) struct RendererDevToolsTargetHandle {
    pause: RendererInspectorPauseBridge,
    main: RendererInspectorMainIngress,
    io: RendererInspectorIoIngress,
}

impl RendererDevToolsTargetHandle {
    pub(crate) fn new(
        pause: RendererInspectorPauseBridge,
        main: RendererInspectorMainIngress,
        io: RendererInspectorIoIngress,
    ) -> Self {
        Self { pause, main, io }
    }

    pub(crate) fn pause(&self) -> RendererInspectorPauseBridge {
        self.pause.clone()
    }

    pub(crate) fn pause_ref(&self) -> &RendererInspectorPauseBridge {
        &self.pause
    }

    pub(crate) fn main_ref(&self) -> &RendererInspectorMainIngress {
        &self.main
    }

    pub(crate) fn io_ref(&self) -> &RendererInspectorIoIngress {
        &self.io
    }

    pub(crate) fn outbound_route(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
    ) -> RendererInspectorSessionOutboundRoute {
        self.pause
            .outbound_route(self.clone(), agent_token, session)
    }

    pub(crate) fn detach_session(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: &DevToolsSessionKey,
    ) {
        self.main.detach_session(agent_token, session);
        self.io.detach_session(agent_token, session);
    }

    pub(crate) fn close(&self, message: &str) -> bool {
        self.pause.close_target();
        self.main.close(message);
        self.io.close(message);
        self.io.terminate_execution_for_target_close()
    }

    pub(crate) fn detach_page(&self, page_id: PageId, message: &str) {
        if self.pause.detach_page(page_id) {
            self.main.cancel_all_queued(message);
            self.io.cancel_all_queued(message);
        }
    }
}

impl std::fmt::Debug for RendererDevToolsTargetHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RendererDevToolsTargetHandle")
            .field("pause", &self.pause)
            .field("main", &self.main)
            .field("io", &self.io)
            .finish()
    }
}
