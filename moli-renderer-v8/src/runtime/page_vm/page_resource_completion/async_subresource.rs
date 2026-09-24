use anyhow::Result;

use crate::{
    page_resource_completion::{
        PageResourceCompletionOutputEffect, PageResourceCompletionTurnAction,
        RendererPageResourceCompletionOwner,
    },
    runtime::RendererOwnerResourceActivitySource,
    types::{AsyncSubresourceFetchEvent, AsyncSubresourceFetchEventTarget},
};

use super::super::PageVm;

impl PageVm {
    pub(super) fn apply_async_subresource_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        event: AsyncSubresourceFetchEvent,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            // Native receipts are producer-captured historical Network facts;
            // they remain observable after Document replacement but never
            // acquire authority over the replacement Document's request state.
            let output_effect = if matches!(
                event.target(),
                AsyncSubresourceFetchEventTarget::NativeNetwork
            ) {
                let _ = self
                    .vm_mut()
                    .complete_async_subresource_fetch_event_body(event)?;
                PageResourceCompletionOutputEffect::CaptureRequired
            } else {
                PageResourceCompletionOutputEffect::None
            };
            return Ok(PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                output_effect,
            ));
        }

        let activity = self
            .vm_mut()
            .complete_async_subresource_fetch_event_body(event)?;
        let output_effect = PageResourceCompletionOutputEffect::CaptureRequired;
        Ok(match activity {
            crate::script_vm::AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered => {
                PageResourceCompletionTurnAction::applied(source, owner, output_effect)
            }
            crate::script_vm::AsyncSubresourceFetchBodyActivity::WindowRealmEntered => {
                PageResourceCompletionTurnAction::applied_after_page_code(
                    source,
                    owner,
                    output_effect,
                )
            }
        })
    }
}
