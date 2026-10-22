use crate::page_task_queue::{
    PageFontLoadingTargetEffect, PageFontLoadingTurnAction, PageFontLoadingTurnOutcome,
    RendererPageFontLoadingOwner, RendererPageFontLoadingTask, RendererPageFontLoadingTaskId,
    RendererPageFontLoadingTaskKind,
};

use super::{
    AuthorizedCurrentWindowDocumentTask, IntoPageTaskCompletion, PageTaskCompletion, PageVm,
};

impl IntoPageTaskCompletion for PageFontLoadingTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageFontLoadingTargetEffect::ReadinessCheckedForCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
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
    fn current_page_font_loading_owner(
        &self,
        task_id: RendererPageFontLoadingTaskId,
    ) -> Option<(
        RendererPageFontLoadingOwner,
        RendererPageFontLoadingTaskKind,
    )> {
        self.vm().current_pending_font_loading_owner(
            task_id,
            self.document_lifecycle.identity().document,
        )
    }

    pub(in crate::runtime) fn apply_selected_page_font_loading_turn(
        &mut self,
        task: RendererPageFontLoadingTask,
    ) -> anyhow::Result<PageFontLoadingTurnOutcome> {
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        let current = self.current_page_font_loading_owner(task_id);
        let target_effect =
            match self.authorize_current_window_document_task(task, owner, kind, current) {
                Ok(authorization) => self
                    .vm_mut()
                    .apply_current_font_loading_task_body(authorization)?,
                Err(stale) => {
                    if stale.may_discard_local_payload() {
                        self.vm_mut().discard_stale_font_loading_task(task_id);
                    }
                    let current_owner = stale.current_owner();
                    tracing::debug!(
                        ?owner,
                        ?current_owner,
                        ?task_id,
                        ?kind,
                        "discarded stale exact-Document FontLoading task"
                    );
                    PageFontLoadingTargetEffect::DiscardedStaleOwner { current_owner }
                }
            };
        let action = PageFontLoadingTurnAction {
            owner,
            task_id,
            kind,
            target_effect,
        };
        Ok(PageFontLoadingTurnOutcome::new(action))
    }
}
