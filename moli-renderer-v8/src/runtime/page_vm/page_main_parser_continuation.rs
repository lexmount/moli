use crate::{
    page_task_queue::{
        PageMainParserContinuationTargetEffect, PageMainParserContinuationTurnAction,
        PageMainParserContinuationTurnOutcome, RendererPageMainParserContinuationTask,
    },
    runtime::PageOwnerTurnOutcome,
};

use super::PageVm;

impl PageVm {
    /// Apply one main-parser continuation selected from the shared Networking
    /// FIFO.
    ///
    /// Selection consumes the task even when its owner is stale. A current task
    /// grants exactly one resume admission to the sole phase-one residence; it
    /// does not run or copy the parser while the PageVm is checked out for this
    /// ordinary Page turn.
    pub(in crate::runtime) fn apply_selected_page_main_parser_continuation_turn(
        &mut self,
        task: RendererPageMainParserContinuationTask,
    ) -> PageMainParserContinuationTurnOutcome {
        // Networking dequeue already released coalescing before this executor
        // was entered. A producer fact in the dequeue-to-execution window can
        // therefore queue the next bounded parser opportunity.
        let owner = task.into_owner();
        let root_document_is_current =
            owner.root_document() == self.document_lifecycle.identity().document;
        let target_effect = if root_document_is_current
            && self.vm().current_main_document_task_owner() == Some(owner.document_owner())
            && self
                .vm_mut()
                .document_runtime
                .admit_selected_main_parser_continuation(owner)
        {
            PageMainParserContinuationTargetEffect::AdmittedCurrentParser
        } else {
            PageMainParserContinuationTargetEffect::DiscardedStaleOrInactiveParser
        };
        PageOwnerTurnOutcome::new(PageMainParserContinuationTurnAction {
            owner,
            target_effect,
        })
    }
}
