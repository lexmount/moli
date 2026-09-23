use anyhow::{Result, anyhow};

use super::ScriptVm;
use crate::{
    native_bridge::PendingHistoryTraversalAction,
    page_task_queue::{
        RendererPageHistoryTraversalOwner, RendererPageHistoryTraversalTask,
        RendererPageHistoryTraversalTaskId, RendererPageHistoryTraversalTaskKind,
    },
    runtime::AuthorizedCurrentPageHistoryTraversal,
};

impl ScriptVm {
    pub(crate) fn queue_top_level_history_traversal_by_delta(
        &mut self,
        delta: i64,
    ) -> Result<bool> {
        self.with_default_context_scope(|scope, _host_ptr| {
            Ok(crate::context_bootstrap::queue_top_level_history_traversal_by_delta(scope, delta))
        })
    }

    pub(crate) fn current_pending_history_traversal_owner(
        &self,
        task_id: RendererPageHistoryTraversalTaskId,
        root_document: crate::runtime::RendererDocumentToken,
    ) -> Option<(
        RendererPageHistoryTraversalOwner,
        RendererPageHistoryTraversalTaskKind,
    )> {
        let (execution_context, target, kind) = self
            ._context_host
            .borrow()
            .current_pending_history_traversal_task_owner(task_id)?;
        Some((
            RendererPageHistoryTraversalOwner::new(root_document, execution_context, target),
            kind,
        ))
    }

    /// Apply one history-traversal task only after the Page arbiter has
    /// matched its PageVm namespace, Promise-relevant realm, target
    /// LocalWindow, and concrete operation kind.
    /// Apply one authorized history-traversal body without ending the selected
    /// Page task.
    ///
    /// The Page dispatcher owns the ordinary microtask checkpoint, child
    /// record synchronization, and runtime follow-up after this method
    /// returns.
    pub(crate) fn apply_current_history_traversal_task_body(
        &mut self,
        authorization: AuthorizedCurrentPageHistoryTraversal,
    ) -> Result<()> {
        let task = authorization.into_task();
        let owner = task.owner();
        let task_id = task.task_id();
        let queued = self
            ._context_host
            .borrow_mut()
            .take_pending_history_traversal_task_for_exact_owner(
                task_id,
                owner.execution_context(),
                owner.target(),
                task.kind(),
            )
            .ok_or_else(|| {
                anyhow!("authorized history traversal lost its exact pending payload")
            })?;

        let Some(queued) = self
            ._context_host
            .borrow_mut()
            .defer_pending_history_traversal_task(queued)
        else {
            return Ok(());
        };
        let execution_context = queued.execution_context;
        let action = queued.action;
        let (bound_owner, bound_dispatch_scope, _realm_token, context) =
            queued.relevant_context.into_parts();
        debug_assert_eq!(execution_context, owner.execution_context());
        debug_assert_eq!(bound_owner, owner.execution_context().owner());
        debug_assert_eq!(
            bound_dispatch_scope,
            owner.execution_context().dispatch_scope()
        );

        let context_ptr: *const v8::Global<v8::Context> = &context;
        self.with_context_scope_by_ptr(context_ptr, move |scope, host_ptr| {
            let previous_dispatch_scope = bound_dispatch_scope.enter(scope);
            crate::context_bootstrap::apply_authorized_history_traversal_task(
                scope,
                unsafe { &mut *host_ptr },
                action,
            );
            bound_dispatch_scope.defer_restore(scope, previous_dispatch_scope);
            Ok(())
        })
    }

    /// Settle retained results without applying a retired target's traversal.
    /// The Page arbiter must first validate the task's root-document namespace.
    pub(crate) fn reject_stale_history_traversal_results(
        &mut self,
        task: RendererPageHistoryTraversalTask,
    ) -> Result<bool> {
        let owner = task.owner();
        let Some(queued) = self
            ._context_host
            .borrow_mut()
            .take_pending_history_traversal_task_for_exact_owner(
                task.task_id(),
                owner.execution_context(),
                owner.target(),
                task.kind(),
            )
        else {
            return Ok(false);
        };
        let results = match queued.action {
            PendingHistoryTraversalAction::ByDelta { .. } => Vec::new(),
            PendingHistoryTraversalAction::SameDocument(traversal) => traversal.results,
            PendingHistoryTraversalAction::CrossDocument(traversal) => traversal.results,
        };
        if results.is_empty() {
            return Ok(false);
        }
        // The retired realm owns these Promises and their rejection value.
        // Keep its captured context; looking up a live Window could select a
        // replacement realm or lose the results when the frame was removed.
        let (_, bound_dispatch_scope, _, context) = queued.relevant_context.into_parts();
        let context_ptr: *const v8::Global<v8::Context> = &context;
        self.with_context_scope_by_ptr(context_ptr, move |scope, _host_ptr| {
            let previous_dispatch_scope = bound_dispatch_scope.enter(scope);
            crate::context_bootstrap::reject_canceled_history_traversal_results(scope, &results);
            bound_dispatch_scope.defer_restore(scope, previous_dispatch_scope);
            Ok(true)
        })
    }
}
