use crate::page_task_queue::{
    PageIntersectionObserverTargetEffect, PageIntersectionObserverTurnAction,
    PageIntersectionObserverTurnOutcome, RendererPageIntersectionObserverTask,
};

use crate::script_vm::ScriptVm;

use super::{
    AuthorizedCurrentWindowDocumentTask, IntoPageTaskCompletion, PageTaskCompletion, PageVm,
};

impl IntoPageTaskCompletion for PageIntersectionObserverTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageIntersectionObserverTargetEffect::CallbackInvokedForCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
            }
            PageIntersectionObserverTargetEffect::CurrentOwnerCallbackRetired => {
                PageTaskCompletion::CheckpointOnly
            }
            PageIntersectionObserverTargetEffect::DiscardedStaleOwner { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

pub(crate) type AuthorizedCurrentPageIntersectionObserverTask =
    AuthorizedCurrentWindowDocumentTask<RendererPageIntersectionObserverTask>;

impl PageVm {
    pub(in crate::runtime) fn apply_selected_page_intersection_observer_turn(
        &mut self,
        task: RendererPageIntersectionObserverTask,
    ) -> anyhow::Result<PageIntersectionObserverTurnOutcome> {
        self.apply_selected_window_document_task(
            task,
            ScriptVm::current_pending_intersection_observer_owner,
            ScriptVm::apply_current_intersection_observer_task_body,
            ScriptVm::discard_stale_intersection_observer_task,
            |current_owner| PageIntersectionObserverTargetEffect::DiscardedStaleOwner {
                current_owner,
            },
        )
    }
}
