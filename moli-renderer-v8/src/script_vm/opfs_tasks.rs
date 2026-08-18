use anyhow::{Result, ensure};

use super::ScriptVm;
use crate::{
    page_task_queue::{RendererPageOpfsTaskId, RendererPageOpfsTaskOwner},
    runtime::AuthorizedCurrentPageOpfsTask,
};

impl ScriptVm {
    pub(crate) fn current_pending_opfs_task_execution_context(
        &self,
        task: RendererPageOpfsTaskId,
    ) -> Option<crate::native_bridge::WindowExecutionContextIdentity> {
        self._context_host
            .borrow()
            .current_pending_opfs_task_execution_context(task)
    }

    /// Settle one page-side OPFS task body after the Page arbiter has matched
    /// its exact PageVm, Window realm, and pending entry.
    ///
    /// Storage settlement may queue Promise reactions, but this body
    /// deliberately leaves them pending. The unique selected Page-task
    /// dispatcher owns the enclosing storage task's microtask checkpoint.
    pub(crate) fn apply_current_opfs_task_body(
        &mut self,
        authorization: AuthorizedCurrentPageOpfsTask,
    ) -> Result<()> {
        let task = authorization.into_task();
        let owner = task.owner();
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_opfs_task_for_exact_owner(owner.execution_context(), owner.task())
            .ok_or_else(|| {
                anyhow::anyhow!("authorized OPFS task lost its exact pending settlement")
            })?;
        let relevant_context = pending.execution_context.into_binding();
        let locator = pending.locator;
        let handle_access = pending.handle_access;
        let settlement = pending.settlement;
        let (_, bound_dispatch_scope, realm_token, context) = relevant_context.into_parts();
        let context_ptr: *const v8::Global<v8::Context> = &context;
        let result = task.into_result();
        self.with_context_scope_by_ptr(context_ptr, move |scope, _host_ptr| {
            ensure!(
                crate::native_bridge::current_runtime_observable_context_token(scope)
                    == Some(realm_token),
                "authorized OPFS task lost its relevant Window realm before settlement"
            );
            let previous_dispatch_scope = bound_dispatch_scope.enter(scope);
            match settlement {
                crate::opfs_owner_tasks::OpfsTaskSettlement::Promise(resolver) => {
                    let resolver = v8::Local::new(scope, &resolver);
                    crate::context_bootstrap::settle_opfs_task_result(
                        scope,
                        resolver,
                        &locator,
                        handle_access.as_ref(),
                        result,
                    );
                }
                crate::opfs_owner_tasks::OpfsTaskSettlement::Move {
                    resolver,
                    handle,
                    mutation,
                } => {
                    let resolver = v8::Local::new(scope, &resolver);
                    let handle = v8::Local::new(scope, &handle);
                    crate::context_bootstrap::settle_opfs_move_task_result(
                        scope,
                        resolver,
                        handle,
                        &locator,
                        handle_access.as_ref(),
                        result,
                    );
                    drop(mutation);
                }
                crate::opfs_owner_tasks::OpfsTaskSettlement::DirectoryIterator {
                    registry,
                    iterator_id,
                    keep_alive,
                } => {
                    crate::context_bootstrap::settle_opfs_directory_iterator_task_result(
                        scope,
                        &registry,
                        iterator_id,
                        &locator,
                        handle_access.as_ref(),
                        result,
                    );
                    drop(keep_alive);
                }
            }
            bound_dispatch_scope.defer_restore(scope, previous_dispatch_scope);
            tracing::debug!(
                task_id = owner.task().task_id(),
                execution_context = ?owner.execution_context(),
                "settled OPFS task body in relevant Window execution context"
            );
            Ok(())
        })
    }

    pub(crate) fn discard_stale_opfs_task(&mut self, owner: RendererPageOpfsTaskOwner) {
        let _ = self
            ._context_host
            .borrow_mut()
            .take_pending_opfs_task_for_exact_owner(owner.execution_context(), owner.task());
    }
}
