use crate::page_task_queue::{
    PageHistoryTraversalTargetEffect, PageHistoryTraversalTurnAction,
    PageHistoryTraversalTurnOutcome, RendererPageHistoryTraversalOwner,
    RendererPageHistoryTraversalTask, RendererPageHistoryTraversalTaskId,
    RendererPageHistoryTraversalTaskKind,
};

use super::{IntoPageTaskCompletion, PageTaskCompletion, PageVm};

impl IntoPageTaskCompletion for PageHistoryTraversalTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageHistoryTraversalTargetEffect::AppliedToCurrentOwner
            | PageHistoryTraversalTargetEffect::RejectedStaleResults => {
                // A traversal can dispatch Navigation API, popstate, hashchange
                // and child-traversal callbacks, and can itself publish child
                // work. Preserve the established full post-checkpoint
                // reconciliation even when a particular traversal has no
                // registered listener. Rejected results can also publish work
                // through their Promise reactions after the target retires.
                PageTaskCompletion::CallbackCompletion
            }
            PageHistoryTraversalTargetEffect::DiscardedStaleOwner { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

/// Proof that the Page arbiter matched the PageVm namespace, relevant Window
/// realm, traversed LocalWindow, and operation kind before V8 application.
pub(crate) struct AuthorizedCurrentPageHistoryTraversal(RendererPageHistoryTraversalTask);

impl AuthorizedCurrentPageHistoryTraversal {
    fn new(task: RendererPageHistoryTraversalTask) -> Self {
        Self(task)
    }

    pub(crate) fn into_task(self) -> RendererPageHistoryTraversalTask {
        self.0
    }
}

impl PageVm {
    fn current_page_history_traversal_owner(
        &self,
        task_id: RendererPageHistoryTraversalTaskId,
    ) -> Option<(
        RendererPageHistoryTraversalOwner,
        RendererPageHistoryTraversalTaskKind,
    )> {
        self.vm().current_pending_history_traversal_owner(
            task_id,
            self.document_lifecycle.identity().document,
        )
    }

    pub(in crate::runtime) fn apply_selected_page_history_traversal_turn(
        &mut self,
        task: RendererPageHistoryTraversalTask,
    ) -> anyhow::Result<PageHistoryTraversalTurnOutcome> {
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        let current = self.current_page_history_traversal_owner(task_id);
        let target_effect = if current == Some((owner, kind)) {
            self.vm_mut().apply_current_history_traversal_task_body(
                AuthorizedCurrentPageHistoryTraversal::new(task),
            )?;
            PageHistoryTraversalTargetEffect::AppliedToCurrentOwner
        } else {
            // Local task ids restart in each PageVm. Only a same-root stale
            // task may clean this Host's retained payload; an old PageVm task
            // must never consume a naturally reused id in its replacement.
            let rejected_results = owner.root_document()
                == self.document_lifecycle.identity().document
                && self.vm_mut().reject_stale_history_traversal_results(task)?;
            tracing::debug!(
                ?owner,
                ?current,
                ?task_id,
                ?kind,
                "discarded stale exact-owner history traversal"
            );
            if rejected_results {
                PageHistoryTraversalTargetEffect::RejectedStaleResults
            } else {
                PageHistoryTraversalTargetEffect::DiscardedStaleOwner {
                    current_owner: current.map(|(owner, _)| owner),
                }
            }
        };
        let action = PageHistoryTraversalTurnAction {
            owner,
            task_id,
            kind,
            target_effect,
        };
        Ok(PageHistoryTraversalTurnOutcome::new(action))
    }
}
