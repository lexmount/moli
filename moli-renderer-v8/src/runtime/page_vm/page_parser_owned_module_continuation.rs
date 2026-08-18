//! Task-end coordinator for one selected parser-owned module continuation.
//!
//! Module graph/evaluation settlement remains in the parser/module owner. This
//! component consumes only the execution-produced body fact, submits the
//! surrounding HTML task end, and primes lifecycle/style successors. It never
//! selects or executes a second Page task.

use crate::page_task_queue::{
    PageParserOwnedModuleContinuationBodyActivity, PageParserOwnedModuleContinuationTargetEffect,
    PageParserOwnedModuleContinuationTurnAction, RendererPageMainDocumentRuntimeOwner,
};

use super::{
    PageVm,
    parser_task_completion::{
        MainParserContinuationBodyActivity, MainParserContinuationTaskEffect,
    },
};

impl PageVm {
    pub(super) fn parser_owned_module_target_effect(
        selected_owner: RendererPageMainDocumentRuntimeOwner,
        effect: MainParserContinuationTaskEffect,
    ) -> anyhow::Result<PageParserOwnedModuleContinuationTargetEffect> {
        Ok(match effect {
            MainParserContinuationTaskEffect::NotApplied => {
                PageParserOwnedModuleContinuationTargetEffect::CurrentOwnerReservationSpent
            }
            MainParserContinuationTaskEffect::Applied {
                owner: effect_owner,
                activity,
            } => {
                anyhow::ensure!(
                    effect_owner == selected_owner.document_owner(),
                    "parser-owned module body returned task-end authority for a different exact Document"
                );
                let activity = match activity {
                    MainParserContinuationBodyActivity::NoPageCodeOrEventDispatch => {
                        PageParserOwnedModuleContinuationBodyActivity::NoPageCodeOrEventDispatch
                    }
                    MainParserContinuationBodyActivity::PageCodeOrEventDispatch => {
                        PageParserOwnedModuleContinuationBodyActivity::PageCodeOrEventDispatch
                    }
                };
                PageParserOwnedModuleContinuationTargetEffect::AppliedToSelectedOwner(activity)
            }
        })
    }

    pub(super) fn finish_selected_page_parser_owned_module_continuation(
        &mut self,
        action: PageParserOwnedModuleContinuationTurnAction,
    ) -> anyhow::Result<()> {
        let selected_owner = action.owner();
        let activity = match action.target_effect() {
            PageParserOwnedModuleContinuationTargetEffect::AppliedToSelectedOwner(activity) => {
                activity
            }
            PageParserOwnedModuleContinuationTargetEffect::CurrentOwnerReservationSpent
            | PageParserOwnedModuleContinuationTargetEffect::DiscardedStaleOwner => return Ok(()),
        };
        match activity {
            PageParserOwnedModuleContinuationBodyActivity::NoPageCodeOrEventDispatch => {
                self.finish_selected_page_task_checkpoint()?;
            }
            PageParserOwnedModuleContinuationBodyActivity::PageCodeOrEventDispatch => {
                self.vm_mut()
                    .finish_main_page_owned_document_script_callback_checkpoint()?;
            }
        }
        self.vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        tracing::debug!(
            ?selected_owner,
            current_document_owner = ?self.vm().current_main_document_task_owner(),
            "completed selected parser-owned module continuation"
        );
        Ok(())
    }
}
