use crate::page_task_queue::{
    PagePopupDocumentLifecycleTargetEffect, PagePopupDocumentLifecycleTurnAction,
    PagePopupDocumentLifecycleTurnOutcome, RendererPagePopupDocumentLifecycleOwner,
    RendererPagePopupDocumentLifecycleTask,
};

use crate::script_vm::PopupDocumentLifecycleStep;

use super::{IntoPageTaskCompletion, PageTaskCompletion, PageVm};

impl IntoPageTaskCompletion for PagePopupDocumentLifecycleTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        // Current bodies complete their checkpoints in the lifecycle
        // coordinator. Stale entries never enter the replacement agent.
        PageTaskCompletion::NoCompletion
    }
}

/// Proof that the Page arbiter matched the PageVm namespace and the exact
/// lightweight-popup Document navigation before entering V8.
pub(crate) struct AuthorizedCurrentPagePopupDocumentLifecycle(
    RendererPagePopupDocumentLifecycleTask,
);

impl AuthorizedCurrentPagePopupDocumentLifecycle {
    fn new(task: RendererPagePopupDocumentLifecycleTask) -> Self {
        Self(task)
    }

    pub(crate) fn into_task(self) -> RendererPagePopupDocumentLifecycleTask {
        self.0
    }
}

impl PageVm {
    pub(super) async fn finish_popup_parser_producing_callback_task(
        &mut self,
        root_document: crate::runtime::RendererDocumentToken,
        completions: Vec<crate::native_bridge::PopupDocumentParserCompletion>,
        loader: &crate::network::ResourceRequestClient,
    ) -> anyhow::Result<()> {
        if completions.is_empty() {
            return self.finish_selected_page_callback_task(loader).await;
        }
        // These obligations were returned by this callback body. Finish its
        // script reactions before entering parser completion, without draining
        // unrelated runtime work into a replacement Document.
        self.vm_mut()
            .finish_page_resource_completion_callback_checkpoint()?;
        self.absorb_parser_no_execution_runs();
        if root_document == self.document_lifecycle.identity().document {
            for completion in completions {
                let step = self.vm_mut().begin_popup_document_interactive(completion)?;
                self.finish_popup_document_lifecycle_step(step)?;
            }
        }
        Ok(())
    }

    pub(super) fn finish_popup_document_lifecycle_step(
        &mut self,
        mut step: PopupDocumentLifecycleStep,
    ) -> anyhow::Result<()> {
        let result: anyhow::Result<()> = (|| loop {
            match step {
                PopupDocumentLifecycleStep::Completed => break Ok(()),
                PopupDocumentLifecycleStep::Checkpoint(checkpoint) => {
                    self.vm_mut().finish_popup_document_lifecycle_checkpoint()?;
                    step = self
                        .vm_mut()
                        .resume_popup_document_lifecycle_after_checkpoint(checkpoint)?;
                }
            }
        })();
        self.vm_mut().finish_popup_document_lifecycle_turn(result)?;
        self.absorb_parser_no_execution_runs();
        self.vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        Ok(())
    }

    fn current_page_popup_document_lifecycle_owner(
        &self,
        expected: RendererPagePopupDocumentLifecycleOwner,
    ) -> Option<RendererPagePopupDocumentLifecycleOwner> {
        self.vm().current_popup_document_lifecycle_owner(
            expected.target(),
            self.document_lifecycle.identity().document,
            expected.event(),
        )
    }

    pub(in crate::runtime) fn apply_selected_page_popup_document_lifecycle_turn(
        &mut self,
        task: RendererPagePopupDocumentLifecycleTask,
    ) -> anyhow::Result<PagePopupDocumentLifecycleTurnOutcome> {
        let owner = task.owner();
        let current_owner = self.current_page_popup_document_lifecycle_owner(owner);
        let target_effect = if current_owner == Some(owner) {
            let step = self.vm_mut().begin_current_popup_document_lifecycle_body(
                AuthorizedCurrentPagePopupDocumentLifecycle::new(task),
            )?;
            self.finish_popup_document_lifecycle_step(step)?;
            PagePopupDocumentLifecycleTargetEffect::DispatchedToCurrentOwner
        } else {
            if owner.root_document() == self.document_lifecycle.identity().document {
                self.vm_mut()
                    .discard_stale_popup_document_lifecycle_task(owner.target());
            }
            tracing::debug!(
                ?owner,
                ?current_owner,
                "discarded stale exact-Document popup lifecycle event"
            );
            PagePopupDocumentLifecycleTargetEffect::DiscardedStaleOwner { current_owner }
        };
        let action = PagePopupDocumentLifecycleTurnAction {
            owner,
            target_effect,
        };
        Ok(PagePopupDocumentLifecycleTurnOutcome::new(action))
    }
}
