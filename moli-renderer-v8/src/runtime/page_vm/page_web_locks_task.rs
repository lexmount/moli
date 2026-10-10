use crate::page_task_queue::{
    PageWebLocksTaskTargetEffect, PageWebLocksTaskTurnAction, PageWebLocksTaskTurnOutcome,
    RendererPageWebLocksTask, RendererPageWebLocksTaskOwner,
};

use super::PageVm;

/// Proof that the Page arbiter matched a selected completion against the exact
/// root PageVm, Window realm, and pending Promise entry.
pub(crate) struct AuthorizedCurrentPageWebLocksTask(RendererPageWebLocksTask);

impl AuthorizedCurrentPageWebLocksTask {
    fn new(task: RendererPageWebLocksTask) -> Self {
        Self(task)
    }

    pub(crate) fn into_task(self) -> RendererPageWebLocksTask {
        self.0
    }
}

impl PageVm {
    fn current_page_web_locks_task_owner(
        &self,
        expected: RendererPageWebLocksTaskOwner,
    ) -> Option<RendererPageWebLocksTaskOwner> {
        let execution_context = self
            .vm()
            .current_pending_web_locks_task_execution_context(expected.task())?;
        Some(RendererPageWebLocksTaskOwner::new(
            self.document_lifecycle.identity().document,
            execution_context,
            expected.task(),
        ))
    }

    pub(in crate::runtime) fn apply_selected_page_web_locks_task_turn(
        &mut self,
        task: RendererPageWebLocksTask,
    ) -> anyhow::Result<PageWebLocksTaskTurnOutcome> {
        let owner = task.owner();
        let current_owner = self.current_page_web_locks_task_owner(owner);
        let target_effect = if current_owner == Some(owner) {
            self.vm_mut()
                .apply_current_web_locks_task_body(AuthorizedCurrentPageWebLocksTask::new(task))?;
            PageWebLocksTaskTargetEffect::SettledCurrentOwner
        } else {
            // Realm retirement releases this client and its leases. A stale
            // event has no authority to touch a current client.
            tracing::debug!(
                ?owner,
                ?current_owner,
                "ignored stale exact-owner WebLocks task"
            );
            PageWebLocksTaskTargetEffect::IgnoredStaleOwner { current_owner }
        };
        let action = PageWebLocksTaskTurnAction {
            owner,
            target_effect,
        };
        Ok(PageWebLocksTaskTurnOutcome::new(action))
    }
}
