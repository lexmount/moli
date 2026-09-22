//! Unique task-end coordinator for `Networking::ResourceCompletion`.
//!
//! Resource terminals are heterogeneous, but their queued source is not a
//! checkpoint policy. Each domain body returns whether it entered Window code
//! and whether it released a lifecycle gate. This component consumes those
//! execution-produced facts after selection; it never claims or prioritizes
//! another task.

use anyhow::Result;

use crate::page_resource_completion::{
    PageResourceCompletionBodyActivity, PageResourceCompletionDocumentEffect,
    PageResourceCompletionPostCheckpointEffect, PageResourceCompletionTurnAction,
};

use super::PageVm;

impl PageVm {
    pub(super) fn finish_selected_page_resource_completion_task(
        &mut self,
        action: PageResourceCompletionTurnAction,
    ) -> Result<()> {
        match action.document_effect {
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner { .. } => {
                debug_assert_eq!(
                    action.body_activity,
                    PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
                    "a stale resource terminal must not enter the replacement Window"
                );
            }
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
            | PageResourceCompletionDocumentEffect::CurrentOwnerHadNoApplicablePayload
            | PageResourceCompletionDocumentEffect::SupersededDuringApplication { .. } => {
                match action.body_activity {
                    PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch => {
                        self.finish_selected_page_task_checkpoint()?;
                    }
                    PageResourceCompletionBodyActivity::PageCodeOrEventDispatchAttempted => {
                        self.vm_mut()
                            .finish_page_resource_completion_callback_checkpoint()?;
                        self.absorb_parser_no_execution_runs();
                    }
                }
            }
        }

        if action.owner.root_document() == self.document_lifecycle.identity().document {
            match action.post_checkpoint_effect {
                PageResourceCompletionPostCheckpointEffect::None => {}
                PageResourceCompletionPostCheckpointEffect::PrimeMainDocumentLifecycle {
                    owner,
                } => {
                    if self.vm().current_main_document_task_owner() == Some(owner) {
                        self.vm_mut().prime_document_lifecycle_processing_and_record_stylesheet_network_results();
                    }
                }
                PageResourceCompletionPostCheckpointEffect::CompletePopupDocumentParser {
                    completion,
                } => {
                    let step = self.vm_mut().begin_popup_document_interactive(completion)?;
                    self.finish_popup_document_lifecycle_step(step)?;
                }
            }
        }

        // These are bounded publications, not nested task execution. They
        // used to run before helper-local checkpoints; moving them here makes
        // their readiness visible only after this resource task is complete.
        self.admit_ready_parser_owned_document_script_action();
        if self.has_ready_runtime_owned_module_script_continuation_work() {
            let _ = self.vm_mut().enqueue_runtime_owned_module_continuation();
        }
        Ok(())
    }
}
