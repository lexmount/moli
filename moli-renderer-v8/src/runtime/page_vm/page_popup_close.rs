use crate::page_task_queue::{
    PagePopupCloseTargetEffect, PagePopupCloseTurnAction, PagePopupCloseTurnOutcome,
    RendererPagePopupCloseOwner, RendererPagePopupCloseTask,
};

use super::{IntoPageTaskCompletion, PageTaskCompletion, PageVm};

impl IntoPageTaskCompletion for PagePopupCloseTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PagePopupCloseTargetEffect::DefinitelyClosed => PageTaskCompletion::CheckpointOnly,
            PagePopupCloseTargetEffect::DiscardedStaleOwner { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

/// Proof that the Page arbiter matched the PageVm namespace and a live
/// lightweight popup whose closing flag is set.
pub(crate) struct AuthorizedCurrentPagePopupClose(RendererPagePopupCloseTask);

impl AuthorizedCurrentPagePopupClose {
    fn new(task: RendererPagePopupCloseTask) -> Self {
        Self(task)
    }

    pub(crate) fn into_task(self) -> RendererPagePopupCloseTask {
        self.0
    }
}

impl PageVm {
    fn current_page_popup_close_owner(
        &self,
        expected: RendererPagePopupCloseOwner,
    ) -> Option<RendererPagePopupCloseOwner> {
        if expected.root_document() != self.document_lifecycle.identity().document {
            return None;
        }
        self.vm().current_popup_close_owner(
            expected.popup_id(),
            self.document_lifecycle.identity().document,
        )
    }

    pub(in crate::runtime) fn apply_selected_page_popup_close_turn(
        &mut self,
        task: RendererPagePopupCloseTask,
    ) -> anyhow::Result<PagePopupCloseTurnOutcome> {
        let owner = task.owner();
        let current_owner = self.current_page_popup_close_owner(owner);
        let target_effect = if current_owner == Some(owner) {
            self.vm_mut()
                .apply_current_popup_close_body(AuthorizedCurrentPagePopupClose::new(task))?;
            PagePopupCloseTargetEffect::DefinitelyClosed
        } else {
            tracing::debug!(
                ?owner,
                ?current_owner,
                "discarded stale exact-popup close task"
            );
            PagePopupCloseTargetEffect::DiscardedStaleOwner { current_owner }
        };
        let action = PagePopupCloseTurnAction {
            owner,
            target_effect,
        };
        Ok(PagePopupCloseTurnOutcome::new(action))
    }
}
