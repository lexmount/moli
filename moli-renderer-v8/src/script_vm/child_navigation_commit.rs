use anyhow::{Result, ensure};

use super::{
    ScriptVm, child_document_lifecycle::ChildDocumentLifecycleOwner,
    child_document_script_scheduler::ChildDocumentScriptSchedulerOwner,
};
use crate::{
    page_task_queue::{
        RendererPageChildNavigationCommitOwner, RendererPageChildNavigationCommitTask,
    },
    runtime::{AuthorizedCurrentPageChildNavigationCommit, RendererDocumentToken},
};

#[cfg(test)]
use crate::page_task_queue::{
    RendererPageNavigationAndTraversalHead, RendererPageReadyDescriptor, RendererPageSchedulerTask,
};

impl ScriptVm {
    pub(crate) fn current_child_navigation_commit_owner(
        &self,
        child_handle: crate::document_runtime::DomHandle,
        root_document: RendererDocumentToken,
    ) -> Option<RendererPageChildNavigationCommitOwner> {
        let commit = self
            ._context_host
            .borrow()
            .current_child_navigation_commit_task(child_handle)?;
        Some(RendererPageChildNavigationCommitOwner::new(
            root_document,
            commit,
        ))
    }

    /// Apply one navigation-commit body after the Page arbiter matched the
    /// PageVm namespace, child scheduler lane and navigation-load generation.
    ///
    /// This function deliberately does not perform the surrounding HTML
    /// task's microtask checkpoint. Owner transitions and typed child
    /// follow-ups remain part of this body; the selected Page dispatcher ends
    /// the task only after they have been committed.
    pub(crate) fn apply_current_child_navigation_commit_body(
        &mut self,
        authorization: AuthorizedCurrentPageChildNavigationCommit,
    ) -> Result<()> {
        let task = authorization.into_task();
        let commit = task.owner().commit();
        let context_host = self._context_host.clone();
        let run = self.with_default_context_scope(move |scope, _host_ptr| {
            let mut host = context_host.borrow_mut();
            ensure!(
                host.claim_child_navigation_commit_task(commit),
                "authorized child navigation commit lost its exact admission reservation"
            );
            Ok(host.run_child_frame_navigation_commit_task(scope, commit))
        })?;

        let (ready_work, parser_stop_action, owner_transition) = run.into_parts();
        if let Some(transition) = owner_transition {
            self.apply_child_document_owner_transition(transition);
        }
        if let Some(action) = parser_stop_action {
            ChildDocumentLifecycleOwner::new(self).notify_parser_stop_action(action)?;
        }
        let mut ready_owner = ChildDocumentScriptSchedulerOwner::new(self);
        for work in ready_work {
            ready_owner.notify_parser_classic_next_owner_action(work);
        }
        self.apply_pending_child_document_owner_retirements();
        Ok(())
    }

    pub(crate) fn discard_stale_child_navigation_commit(
        &mut self,
        task: RendererPageChildNavigationCommitTask,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .retire_child_navigation_commit_task(task.owner().commit())
    }

    /// Drive one real navigation-and-traversal task in low-level ScriptVm
    /// semantic fixtures without recreating the deleted child-pump queue.
    #[cfg(test)]
    pub(crate) fn run_next_child_navigation_commit_body_for_test(
        &mut self,
    ) -> Result<Option<crate::page_task_queue::PageChildNavigationCommitTargetEffect>> {
        let source = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .expect("child-navigation fixture must retain its production Page source")
            .task_sources();
        let Some(task) = source.take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                RendererPageReadyDescriptor::NavigationAndTraversal {
                    head: RendererPageNavigationAndTraversalHead::ChildNavigationCommit { .. },
                    ..
                }
            )
        }) else {
            return Ok(None);
        };
        let RendererPageSchedulerTask::NavigationAndTraversal(
            crate::page_task_queue::RendererPageNavigationAndTraversalTask::ChildNavigationCommit(
                task,
            ),
        ) = task
        else {
            unreachable!("selected child-navigation descriptor must dequeue its own task")
        };
        let owner = task.owner();
        let current = self.current_child_navigation_commit_owner(
            owner.commit().child_handle,
            owner.root_document(),
        );
        let target_effect = if current == Some(owner) {
            self.apply_current_child_navigation_commit_body(
                AuthorizedCurrentPageChildNavigationCommit::new_for_executor_test(task),
            )?;
            crate::page_task_queue::PageChildNavigationCommitTargetEffect::AppliedToCurrentOwner
        } else {
            self.discard_stale_child_navigation_commit(task);
            crate::page_task_queue::PageChildNavigationCommitTargetEffect::DiscardedStaleOwner {
                current_owner: current,
            }
        };
        Ok(Some(target_effect))
    }
}
