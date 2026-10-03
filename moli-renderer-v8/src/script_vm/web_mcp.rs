use super::ScriptVm;
use moli_page_types::{DevToolsSessionKey, RendererWebMcpCommand, RendererWebMcpError};

impl ScriptVm {
    pub(super) fn queue_declarative_web_mcp(&mut self) {
        {
            let mut host = self._context_host.borrow_mut();
            if !crate::context_bootstrap::web_mcp::prepare_registration_task(&mut host) {
                return;
            }
        }
        let _ = self.with_default_context_scope(|scope, host_ptr| {
            crate::context_bootstrap::web_mcp::queue_registration_task(scope, host_ptr);
            Ok(())
        });
    }
    pub(super) fn queue_retired_web_mcp(&mut self) {
        if !self
            ._context_host
            .borrow()
            .native_bridge()
            .web_mcp
            .needs_retirement_task()
        {
            return;
        }
        let _ = self.with_default_context_scope(|scope, host_ptr| {
            crate::context_bootstrap::web_mcp::queue_retirement_task(scope, host_ptr);
            Ok(())
        });
    }
    pub(super) fn prepare_web_mcp_session(&mut self, session: &DevToolsSessionKey) {
        let inspector = &self.page_inspector;
        self.renderer_document_isolate
            .with_renderer_document_isolate_and_inspector_mut(|_, backend| {
                inspector.ensure_frontend_session(backend, session.wire_session_id());
            });
        self._context_host
            .borrow_mut()
            .native_bridge_mut()
            .web_mcp
            .set_object_wrapper(self.page_inspector.object_wrapper());
    }
    pub(crate) fn dispatch_web_mcp_command(
        &mut self,
        session: DevToolsSessionKey,
        command: RendererWebMcpCommand,
    ) -> anyhow::Result<Result<Option<u64>, RendererWebMcpError>> {
        self.queue_declarative_web_mcp();
        if matches!(command, RendererWebMcpCommand::Enable) {
            self.prepare_web_mcp_session(&session);
        }
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(crate::context_bootstrap::web_mcp::dispatch_command(
                scope, host_ptr, session, command,
            ))
        })
    }
}
