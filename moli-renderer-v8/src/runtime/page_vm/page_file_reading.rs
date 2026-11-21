use crate::page_task_queue::{
    PageFileReadingTargetEffect, PageFileReadingTurnAction, PageFileReadingTurnOutcome,
    RendererPageFileReadingTask,
};

use crate::script_vm::ScriptVm;

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
    pub(in crate::runtime) fn apply_selected_page_file_reading_turn(
        &mut self,
        task: RendererPageFileReadingTask,
    ) -> anyhow::Result<PageFileReadingTurnOutcome> {
        self.apply_selected_window_document_task(
            task,
            ScriptVm::current_pending_file_reading_owner,
            ScriptVm::apply_current_file_reading_body,
            ScriptVm::discard_stale_file_reading_task,
            |current_owner| PageFileReadingTargetEffect::DiscardedStaleOwner { current_owner },
        )
    }
}
