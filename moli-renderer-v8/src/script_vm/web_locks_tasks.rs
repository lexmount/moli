use super::ScriptVm;
use crate::runtime::AuthorizedCurrentPageWebLocksTask;

impl ScriptVm {
    pub(crate) fn current_pending_web_locks_task_execution_context(
        &self,
        task: crate::page_task_queue::RendererPageWebLocksTaskId,
    ) -> Option<crate::native_bridge::WindowExecutionContextIdentity> {
        self._context_host
            .borrow()
            .current_pending_web_locks_task_execution_context(task)
    }

    pub(crate) fn apply_current_web_locks_task_body(
        &mut self,
        authorization: AuthorizedCurrentPageWebLocksTask,
    ) -> anyhow::Result<()> {
        let task = authorization.into_task();
        let owner = task.owner();
        let client = self
            ._context_host
            .borrow()
            .web_locks_client_for_exact_owner(owner.execution_context(), owner.task())
            .ok_or_else(|| anyhow::anyhow!("authorized Web Locks task lost its exact client"))?;
        let dispatch = client.relevant_context.dispatch_scope();
        let context_ptr =
            client.relevant_context.context_global() as *const v8::Global<v8::Context>;
        self.with_context_scope_by_ptr(context_ptr, move |scope, _host_ptr| {
            let previous = dispatch.enter(scope);
            crate::context_bootstrap::web_locks::dispatch(scope, &client.state, task.into_result());
            dispatch.defer_restore(scope, previous);
            Ok(())
        })
    }
}
