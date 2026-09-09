use crate::page_task_queue::{
    PageBitmapTaskTargetEffect, PageBitmapTaskTurnAction, PageBitmapTaskTurnOutcome,
    RendererPageBitmapTask, RendererPageBitmapTaskOwner,
};

use super::PageVm;

/// Proof that the Page arbiter matched a selected completion against the exact
/// root PageVm, Window realm, and pending Promise entry.
pub(crate) struct AuthorizedCurrentPageBitmapTask(RendererPageBitmapTask);

impl AuthorizedCurrentPageBitmapTask {
    fn new(task: RendererPageBitmapTask) -> Self {
        Self(task)
    }

    pub(crate) fn into_task(self) -> RendererPageBitmapTask {
        self.0
    }
}

impl PageVm {
    fn current_page_bitmap_task_owner(
        &self,
        expected: RendererPageBitmapTaskOwner,
    ) -> Option<RendererPageBitmapTaskOwner> {
        let execution_context = self
            .vm()
            .current_pending_bitmap_task_execution_context(expected.task())?;
        Some(RendererPageBitmapTaskOwner::new(
            self.document_lifecycle.identity().document,
            execution_context,
            expected.task(),
        ))
    }

    pub(in crate::runtime) fn apply_selected_page_bitmap_task_turn(
        &mut self,
        task: RendererPageBitmapTask,
    ) -> anyhow::Result<PageBitmapTaskTurnOutcome> {
        let owner = task.owner();
        let current_owner = self.current_page_bitmap_task_owner(owner);
        let target_effect = if current_owner == Some(owner) {
            self.vm_mut()
                .apply_current_bitmap_task_body(AuthorizedCurrentPageBitmapTask::new(task))?;
            PageBitmapTaskTargetEffect::SettledCurrentOwner
        } else {
            // A root mismatch means `task_id` belongs to another PageVm
            // namespace and must not be used to touch this PageVm's pending
            // map. Within the same root, exact cleanup is safe and prevents a
            // retired realm from retaining its resolver.
            if owner.root_document() == self.document_lifecycle.identity().document {
                self.vm_mut().discard_stale_bitmap_task(owner);
            }
            tracing::debug!(
                ?owner,
                ?current_owner,
                "ignored stale exact-owner Bitmap task"
            );
            PageBitmapTaskTargetEffect::IgnoredStaleOwner { current_owner }
        };
        let action = PageBitmapTaskTurnAction {
            owner,
            target_effect,
        };
        Ok(PageBitmapTaskTurnOutcome::new(action))
    }
}
