use crate::{
    page_task_queue::{
        PageMainDocumentLifecycleTurnAction, PageMainDocumentLifecycleTurnOutcome,
        RendererPageMainDocumentLifecycleCompletion, RendererPageMainDocumentLifecycleTask,
    },
    runtime::PendingDocumentLifecycleTurn,
    script_vm::{
        MainDocumentLifecycleBody, MainDocumentLifecycleTargetEffect,
        MainDocumentLifecycleTargetRejection, PostParsePageOwnedTask,
    },
};

use super::{IntoPageTaskCompletion, PageTaskCompletion, PageVm};

/// Event bodies finish their own checkpoints through the lifecycle coordinator.
/// A blocked or stale task does not enter page code.
impl IntoPageTaskCompletion for PageMainDocumentLifecycleTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        PageTaskCompletion::NoCompletion
    }
}

impl PageVm {
    pub(super) async fn run_ready_classic_defer_timers_before_domcontentloaded(
        &mut self,
        loader: &crate::network::ResourceRequestClient,
    ) -> anyhow::Result<()> {
        // Preserve the existing cross-source timer preference, after DCL has
        // acquired its FIFO position in the DOM task source.
        while self
            .vm()
            .document_runtime
            .has_ready_timeout_queued_by_classic_defer_script()
        {
            self.run_classic_defer_timer_before_domcontentloaded(loader)
                .await?;
        }
        Ok(())
    }

    pub(super) fn queue_post_parse_lifecycle_dom_task(
        &self,
        pending: &mut PendingDocumentLifecycleTurn,
        body: MainDocumentLifecycleBody,
        task: PostParsePageOwnedTask,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            pending.awaiting_dom_task.is_none() && pending.completed_task.is_none(),
            "a lifecycle resident may await only one exact task"
        );
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.vm()
            .queue_main_document_lifecycle_dom_task(body, Some(sender))?;
        pending.awaiting_dom_task = Some(
            super::super::document_lifecycle_turn::PendingDocumentLifecycleDomTask {
                task,
                completion: receiver,
            },
        );
        Ok(())
    }

    pub(super) async fn apply_selected_page_main_document_lifecycle_turn(
        &mut self,
        task: RendererPageMainDocumentLifecycleTask,
    ) -> anyhow::Result<PageMainDocumentLifecycleTurnOutcome> {
        let RendererPageMainDocumentLifecycleTask { owner, completion } = task;
        if owner.root_document == self.document_lifecycle.identity().document
            && let MainDocumentLifecycleBody::WindowLoad {
                owner: document_owner,
            } = owner.body
            && self
                .vm_mut()
                .main_document_window_load_task_is_ready(document_owner)
                == Some(false)
        {
            // The DOM queue may have run a producer since admission. Do not
            // open the milestone journal or consume its boundary token while
            // the exact Document has a new load delay.
            let completion =
                completion.expect("queued Window load must retain its lifecycle driver");
            let _ = completion.send(RendererPageMainDocumentLifecycleCompletion::LoadBlocked {
                owner: document_owner,
            });
            return Ok(PageMainDocumentLifecycleTurnOutcome::new(
                PageMainDocumentLifecycleTurnAction {
                    owner,
                    target: MainDocumentLifecycleTargetEffect::NotApplied {
                        reason: MainDocumentLifecycleTargetRejection::TransitionRejected,
                        current_owner: self.vm().current_main_document_task_owner(),
                    },
                },
            ));
        }
        let target = if owner.root_document == self.document_lifecycle.identity().document {
            let run = super::main_document_lifecycle_completion::execute_main_document_lifecycle_on_owner_local_task(
                self, owner.body,
            ).await?;
            let target = run.completion.target();
            if let crate::script_vm::MainDocumentLifecycleFollowup::ScheduleInternalLoading {
                task,
                ready_at,
            } = run.completion.into_followup()
            {
                self.vm()
                    .schedule_page_internal_loading_task(task, ready_at)?;
            }
            // The common selected Page owner reconciles document.open() after
            // the body and its checkpoints, just as for other DOM tasks.
            target
        } else {
            MainDocumentLifecycleTargetEffect::NotApplied {
                reason: MainDocumentLifecycleTargetRejection::TransitionRejected,
                current_owner: self.vm().current_main_document_task_owner(),
            }
        };
        if let Some(completion) = completion {
            // Replacement may have retired the receiver during this task.
            let _ = completion.send(RendererPageMainDocumentLifecycleCompletion::Executed);
        }
        Ok(PageMainDocumentLifecycleTurnOutcome::new(
            PageMainDocumentLifecycleTurnAction { owner, target },
        ))
    }
}
