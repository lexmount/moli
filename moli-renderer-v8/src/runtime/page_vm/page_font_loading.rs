use crate::page_task_queue::{
    PageFontLoadingTargetEffect, PageFontLoadingTurnAction, PageFontLoadingTurnOutcome,
    RendererPageFontLoadingTask,
};

use crate::script_vm::ScriptVm;

use super::{
    AuthorizedCurrentWindowDocumentTask, IntoPageTaskCompletion, PageTaskCompletion, PageVm,
};

impl IntoPageTaskCompletion for PageFontLoadingTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageFontLoadingTargetEffect::ReadinessCheckedForCurrentOwner
            | PageFontLoadingTargetEffect::CallbackInvokedForCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
            }
            PageFontLoadingTargetEffect::CurrentOwnerCallbackRetired => {
                PageTaskCompletion::CheckpointOnly
            }
            PageFontLoadingTargetEffect::DiscardedStaleOwner { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

pub(crate) type AuthorizedCurrentPageFontLoadingTask =
    AuthorizedCurrentWindowDocumentTask<RendererPageFontLoadingTask>;

impl PageVm {
    pub(in crate::runtime) fn apply_selected_page_font_loading_turn(
        &mut self,
        task: RendererPageFontLoadingTask,
    ) -> anyhow::Result<PageFontLoadingTurnOutcome> {
        self.apply_selected_window_document_task(
            task,
            ScriptVm::current_pending_font_loading_owner,
            ScriptVm::apply_current_font_loading_task_body,
            ScriptVm::discard_stale_font_loading_task,
            |current_owner| PageFontLoadingTargetEffect::DiscardedStaleOwner { current_owner },
        )
    }
}
