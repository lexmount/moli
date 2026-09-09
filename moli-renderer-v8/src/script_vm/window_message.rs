use anyhow::Result;

use super::ScriptVm;
use crate::{
    native_bridge::WindowTaskTarget,
    page_task_queue::{RendererPageWindowMessageOwner, RendererPageWindowMessageTaskId},
};

impl ScriptVm {
    pub(crate) fn current_window_task_target(
        &self,
        expected: WindowTaskTarget,
    ) -> Option<WindowTaskTarget> {
        let host = self._context_host.borrow();
        host.current_window_execution_context_owner(expected.dispatch_scope())
            .map(|owner| WindowTaskTarget::new(expected.dispatch_scope(), owner))
    }

    pub(crate) fn window_message_task_is_materialized(
        &self,
        owner: RendererPageWindowMessageOwner,
        task_id: RendererPageWindowMessageTaskId,
    ) -> bool {
        let host = self._context_host.borrow();
        !host.has_pending_window_message_task(task_id)
            || host.window_message_target_is_materialized(owner.target())
    }

    /// Apply one task body only after the Page arbiter has matched the root
    /// PageVm namespace and exact LocalWindow target.
    ///
    /// Each listener performs HTML callback cleanup. The selected Page-task
    /// dispatcher still owns task-end checkpointing and child/runtime follow-up.
    pub(crate) fn apply_current_window_message_task_body(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentPageWindowMessage,
    ) -> Result<crate::window_host::WindowMessageTaskRunResult> {
        let task = authorization.into_task();
        let task_id = task.task_id();
        let expected_target = task.owner().target();
        self.with_default_context_scope(|scope, _host_ptr| {
            Ok(crate::window_host::run_current_window_message_task(
                scope,
                task_id,
                expected_target,
            ))
        })
    }

    pub(crate) fn discard_stale_window_message_task(
        &mut self,
        task_id: RendererPageWindowMessageTaskId,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .discard_pending_window_message_task(task_id)
    }
}
