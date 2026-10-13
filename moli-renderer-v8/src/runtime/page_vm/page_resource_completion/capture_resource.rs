use crate::{
    page_resource_completion::{
        CaptureResourceCompletion, PageResourceCompletionOutputEffect,
        PageResourceCompletionTurnAction, RendererPageResourceCompletionOwner,
    },
    runtime::RendererOwnerResourceActivitySource,
};

use super::super::PageVm;

impl PageVm {
    pub(super) fn apply_capture_resource_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: CaptureResourceCompletion,
    ) -> PageResourceCompletionTurnAction {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            return PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                PageResourceCompletionOutputEffect::None,
            );
        }
        // Delivery changes only a job-owned mailbox, not live style, font,
        // image, or layout state. No V8 callback or nested task is entered.
        if completion
            .endpoint
            .deliver(completion.entry_id, completion.result)
        {
            PageResourceCompletionTurnAction::applied(
                source,
                owner,
                PageResourceCompletionOutputEffect::None,
            )
        } else {
            PageResourceCompletionTurnAction::current_owner_without_payload(
                source,
                owner,
                PageResourceCompletionOutputEffect::None,
            )
        }
    }
}
