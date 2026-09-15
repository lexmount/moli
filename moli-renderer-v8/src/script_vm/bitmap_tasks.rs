use anyhow::Result;

use super::ScriptVm;
use crate::{
    context_bootstrap::settle_bitmap_task_result, page_task_queue::RendererPageBitmapTaskOwner,
    runtime::AuthorizedCurrentPageBitmapTask,
};

impl ScriptVm {
    #[cfg(test)]
    pub(crate) fn register_pending_bitmap_task_producer_for_executor_test(
        &mut self,
    ) -> Result<crate::page_task_queue::RendererPageBitmapTaskProducer> {
        self.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let resolver = v8::PromiseResolver::new(scope)
                .expect("Bitmap executor test resolver should exist");
            unsafe { &mut *host_ptr }
                .register_pending_bitmap_task(scope, resolver)
                .ok_or_else(|| {
                    anyhow::anyhow!("Bitmap executor test must capture the current Window realm")
                })
        })
    }

    pub(crate) fn current_pending_bitmap_task_execution_context(
        &self,
        task: crate::page_task_queue::RendererPageBitmapTaskId,
    ) -> Option<crate::native_bridge::WindowExecutionContextIdentity> {
        self._context_host
            .borrow()
            .current_pending_bitmap_task_execution_context(task)
    }

    /// Settle one page-side Bitmap Promise body only after the Page arbiter
    /// has authorized its exact PageVm and Window realm.
    ///
    /// The selected Page-task dispatcher owns the enclosing task's microtask
    /// checkpoint. This method deliberately leaves reactions queued after
    /// resolving or rejecting the Promise.
    pub(crate) fn apply_current_bitmap_task_body(
        &mut self,
        authorization: AuthorizedCurrentPageBitmapTask,
    ) -> Result<()> {
        let task = authorization.into_task();
        let owner = task.owner();
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_bitmap_task_for_exact_owner(owner.execution_context(), owner.task())
            .ok_or_else(|| {
                anyhow::anyhow!("authorized Bitmap task lost its exact pending Promise")
            })?;

        let (bound_owner, bound_dispatch_scope, _realm_token, context) =
            pending.relevant_context.into_parts();
        debug_assert_eq!(bound_owner, owner.execution_context().owner());
        debug_assert_eq!(
            bound_dispatch_scope,
            owner.execution_context().dispatch_scope()
        );
        let context_ptr: *const v8::Global<v8::Context> = &context;
        let resolver = pending.resolver;
        self.with_context_scope_by_ptr(context_ptr, move |scope, _host_ptr| {
            let previous_dispatch_scope = bound_dispatch_scope.enter(scope);
            let resolver = v8::Local::new(scope, &resolver);
            settle_bitmap_task_result(scope, resolver, task.into_result());
            bound_dispatch_scope.defer_restore(scope, previous_dispatch_scope);
            tracing::debug!(
                task_id = owner.task().task_id(),
                execution_context = ?owner.execution_context(),
                "settled Bitmap task body in relevant Window execution context"
            );
            Ok(())
        })
    }

    pub(crate) fn discard_stale_bitmap_task(&mut self, owner: RendererPageBitmapTaskOwner) {
        let _ = self
            ._context_host
            .borrow_mut()
            .take_pending_bitmap_task_for_exact_owner(owner.execution_context(), owner.task());
    }
}
