use crate::page_task_queue::{
    PageIntersectionObserverTargetEffect, PageIntersectionObserverTurnAction,
    PageIntersectionObserverTurnOutcome, RendererPageIntersectionObserverOwner,
    RendererPageIntersectionObserverTask, RendererPageIntersectionObserverTaskId,
    RendererPageIntersectionObserverTaskKind,
};

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
    fn current_page_intersection_observer_owner(
        &self,
        task_id: RendererPageIntersectionObserverTaskId,
    ) -> Option<(
        RendererPageIntersectionObserverOwner,
        RendererPageIntersectionObserverTaskKind,
    )> {
        self.vm().current_pending_intersection_observer_owner(
            task_id,
            self.document_lifecycle.identity().document,
        )
    }

    pub(in crate::runtime) fn apply_selected_page_intersection_observer_turn(
        &mut self,
        task: RendererPageIntersectionObserverTask,
    ) -> anyhow::Result<PageIntersectionObserverTurnOutcome> {
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        let current = self.current_page_intersection_observer_owner(task_id);
        let target_effect =
            match self.authorize_current_window_document_task(task, owner, kind, current) {
                Ok(authorization) => self
                    .vm_mut()
                    .apply_current_intersection_observer_task_body(authorization)?,
                Err(stale) => {
                    if stale.may_discard_local_payload() {
                        self.vm_mut()
                            .discard_stale_intersection_observer_task(task_id);
                    }
                    let current_owner = stale.current_owner();
                    tracing::debug!(
                        ?owner,
                        ?current_owner,
                        ?task_id,
                        ?kind,
                        "discarded stale exact-Document IntersectionObserver task"
                    );
                    PageIntersectionObserverTargetEffect::DiscardedStaleOwner { current_owner }
                }
            };
        let action = PageIntersectionObserverTurnAction {
            owner,
            task_id,
            kind,
            target_effect,
        };
        Ok(PageIntersectionObserverTurnOutcome::new(action))
    }
}
