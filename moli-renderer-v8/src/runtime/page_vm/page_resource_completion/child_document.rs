use anyhow::Result;

use crate::{
    page_resource_completion::{
        PageResourceCompletionOutputEffect, PageResourceCompletionTurnAction,
        RendererPageResourceCompletionOwner,
    },
    runtime::RendererOwnerResourceActivitySource,
    types::{ChildBlockingStylesheetLoadCompletion, ChildClassicScriptLoadCompletion},
};

use super::super::PageVm;

impl PageVm {
    pub(super) fn apply_child_classic_script_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: ChildClassicScriptLoadCompletion,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            return Ok(PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                PageResourceCompletionOutputEffect::None,
            ));
        }

        self.vm_mut()
            .apply_child_classic_script_load_completion_from_page_turn(completion)?;
        Ok(PageResourceCompletionTurnAction::applied(
            source,
            owner,
            PageResourceCompletionOutputEffect::CaptureRequired,
        ))
    }

    pub(super) fn apply_child_blocking_stylesheet_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: ChildBlockingStylesheetLoadCompletion,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            // Native request stages were already published by the physical load.
            return Ok(PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                PageResourceCompletionOutputEffect::CaptureRequired,
            ));
        }

        self.vm_mut()
            .apply_child_blocking_stylesheet_load_completion_from_page_turn(completion)?;
        Ok(PageResourceCompletionTurnAction::applied(
            source,
            owner,
            PageResourceCompletionOutputEffect::CaptureRequired,
        ))
    }
}
