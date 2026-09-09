use crate::page_task_queue::{
    PagePromiseRejectionTargetEffect, PagePromiseRejectionTurnAction,
    PagePromiseRejectionTurnOutcome, RendererPagePromiseRejectionTask,
};

use super::{
    AuthorizedCurrentWindowDocumentTask, IntoPageTaskCompletion, PageTaskCompletion, PageVm,
};

impl IntoPageTaskCompletion for PagePromiseRejectionTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PagePromiseRejectionTargetEffect::DispatchedToCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
            }
            PagePromiseRejectionTargetEffect::CurrentOwnerHadNoEventTarget => {
                PageTaskCompletion::CheckpointOnly
            }
            PagePromiseRejectionTargetEffect::DiscardedStaleOwner { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

pub(crate) type AuthorizedCurrentPagePromiseRejection =
    AuthorizedCurrentWindowDocumentTask<RendererPagePromiseRejectionTask>;

impl PageVm {
    pub(in crate::runtime) fn apply_selected_page_promise_rejection_turn(
        &mut self,
        task: RendererPagePromiseRejectionTask,
    ) -> anyhow::Result<PagePromiseRejectionTurnOutcome> {
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        let current = self.vm().current_pending_promise_rejection_owner(
            task_id,
            self.document_lifecycle.identity().document,
        );
        let target_effect =
            match self.authorize_current_window_document_task(task, owner, kind, current) {
                Ok(authorization) => self
                    .vm_mut()
                    .apply_current_promise_rejection_body(authorization)?,
                Err(stale) => {
                    if stale.may_discard_local_payload() {
                        self.vm_mut().discard_stale_promise_rejection_task(task_id);
                    }
                    PagePromiseRejectionTargetEffect::DiscardedStaleOwner {
                        current_owner: stale.current_owner(),
                    }
                }
            };
        Ok(PagePromiseRejectionTurnOutcome::new(
            PagePromiseRejectionTurnAction {
                owner,
                task_id,
                kind,
                target_effect,
            },
        ))
    }
}
