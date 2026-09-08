use crate::page_task_queue::{
    PageScriptPreparationErrorTargetEffect, PageScriptPreparationErrorTurnAction,
    PageScriptPreparationErrorTurnOutcome, RendererPageScriptPreparationErrorTask,
};

use super::{
    AuthorizedCurrentWindowDocumentTask, IntoPageTaskCompletion, PageTaskCompletion, PageVm,
};

impl IntoPageTaskCompletion for PageScriptPreparationErrorTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageScriptPreparationErrorTargetEffect::DispatchedToCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
            }
            PageScriptPreparationErrorTargetEffect::CurrentOwnerHadNoEventTarget => {
                PageTaskCompletion::CheckpointOnly
            }
            PageScriptPreparationErrorTargetEffect::DiscardedStaleOwner { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

pub(crate) type AuthorizedCurrentPageScriptPreparationError =
    AuthorizedCurrentWindowDocumentTask<RendererPageScriptPreparationErrorTask>;

impl PageVm {
    pub(in crate::runtime) fn apply_selected_page_script_preparation_error_turn(
        &mut self,
        task: RendererPageScriptPreparationErrorTask,
    ) -> anyhow::Result<PageScriptPreparationErrorTurnOutcome> {
        let owner = task.owner();
        let task_id = task.task_id();
        let current = self.vm().current_pending_script_preparation_error_owner(
            task_id,
            self.document_lifecycle.identity().document,
        );
        let target_effect =
            match self.authorize_current_window_document_task(task, owner, (), current) {
                Ok(authorization) => self
                    .vm_mut()
                    .apply_current_script_preparation_error_body(authorization)?,
                Err(stale) => {
                    if stale.may_discard_local_payload() {
                        self.vm_mut()
                            .discard_stale_script_preparation_error_task(task_id);
                    }
                    PageScriptPreparationErrorTargetEffect::DiscardedStaleOwner {
                        current_owner: stale.current_owner(),
                    }
                }
            };
        Ok(PageScriptPreparationErrorTurnOutcome::new(
            PageScriptPreparationErrorTurnAction {
                owner,
                task_id,
                kind: (),
                target_effect,
            },
        ))
    }
}
