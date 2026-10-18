use crate::page_task_queue::{
    PageFileReadingTargetEffect, PageFileReadingTurnAction, PageFileReadingTurnOutcome,
    RendererPageFileReadingOwner, RendererPageFileReadingTask, RendererPageFileReadingTaskId,
    RendererPageFileReadingTaskKind,
};

use super::{
    AuthorizedCurrentWindowDocumentTask, IntoPageTaskCompletion, PageTaskCompletion, PageVm,
};

impl IntoPageTaskCompletion for PageFileReadingTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageFileReadingTargetEffect::CallbackInvokedForCurrentOwner
            | PageFileReadingTargetEffect::FileReaderPhaseAdvanced
            | PageFileReadingTargetEffect::BlobReadCompleted => {
                PageTaskCompletion::CallbackCompletion
            }
            PageFileReadingTargetEffect::CurrentOwnerCallbackRetired => {
                PageTaskCompletion::CheckpointOnly
            }
            PageFileReadingTargetEffect::DiscardedStaleOwner { .. }
            | PageFileReadingTargetEffect::DiscardedStaleReaderRequest => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

pub(crate) type AuthorizedCurrentPageFileReadingTask =
    AuthorizedCurrentWindowDocumentTask<RendererPageFileReadingTask>;

impl PageVm {
    fn current_page_file_reading_owner(
        &self,
        task_id: RendererPageFileReadingTaskId,
    ) -> Option<(
        RendererPageFileReadingOwner,
        RendererPageFileReadingTaskKind,
    )> {
        self.vm().current_pending_file_reading_owner(
            task_id,
            self.document_lifecycle.identity().document,
        )
    }

    pub(in crate::runtime) fn apply_selected_page_file_reading_turn(
        &mut self,
        task: RendererPageFileReadingTask,
    ) -> anyhow::Result<PageFileReadingTurnOutcome> {
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        let current = self.current_page_file_reading_owner(task_id);
        let target_effect =
            match self.authorize_current_window_document_task(task, owner, kind, current) {
                Ok(authorization) => self
                    .vm_mut()
                    .apply_current_file_reading_body(authorization)?,
                Err(stale) => {
                    if stale.may_discard_local_payload() {
                        self.vm_mut().discard_stale_file_reading_task(task_id);
                    }
                    let current_owner = stale.current_owner();
                    tracing::debug!(
                        ?owner,
                        ?current_owner,
                        ?task_id,
                        ?kind,
                        "discarded stale exact-Document file-reading task"
                    );
                    PageFileReadingTargetEffect::DiscardedStaleOwner { current_owner }
                }
            };
        let action = PageFileReadingTurnAction {
            owner,
            task_id,
            kind,
            target_effect,
        };
        Ok(PageFileReadingTurnOutcome::new(action))
    }
}
