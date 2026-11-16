use crate::page_task_queue::{
    PageObjectResourceSelectionTargetEffect, PageObjectResourceSelectionTurnAction,
    PageObjectResourceSelectionTurnOutcome, RendererPageObjectResourceSelectionTask,
};

use super::{
    AuthorizedCurrentWindowDocumentTask, IntoPageTaskCompletion, PageTaskCompletion, PageVm,
};

impl IntoPageTaskCompletion for PageObjectResourceSelectionTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageObjectResourceSelectionTargetEffect::DispatchedToCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
            }
            PageObjectResourceSelectionTargetEffect::CurrentOwnerHadNoEventTarget => {
                PageTaskCompletion::CheckpointOnly
            }
            PageObjectResourceSelectionTargetEffect::DiscardedStaleOwner { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

pub(crate) type AuthorizedCurrentPageObjectResourceSelection =
    AuthorizedCurrentWindowDocumentTask<RendererPageObjectResourceSelectionTask>;

impl PageVm {
    pub(in crate::runtime) fn apply_selected_page_object_resource_selection_turn(
        &mut self,
        task: RendererPageObjectResourceSelectionTask,
    ) -> anyhow::Result<PageObjectResourceSelectionTurnOutcome> {
        let owner = task.owner();
        let task_id = task.task_id();
        let current = self.vm().current_pending_object_resource_selection_owner(
            task_id,
            self.document_lifecycle.identity().document,
        );
        let target_effect =
            match self.authorize_current_window_document_task(task, owner, (), current) {
                Ok(authorization) => self
                    .vm_mut()
                    .apply_current_object_resource_selection_body(authorization)?,
                Err(stale) => {
                    if stale.may_discard_local_payload() {
                        self.vm_mut()
                            .discard_stale_object_resource_selection_task(task_id);
                    }
                    PageObjectResourceSelectionTargetEffect::DiscardedStaleOwner {
                        current_owner: stale.current_owner(),
                    }
                }
            };
        Ok(PageObjectResourceSelectionTurnOutcome::new(
            PageObjectResourceSelectionTurnAction {
                owner,
                task_id,
                kind: (),
                target_effect,
            },
        ))
    }
}
