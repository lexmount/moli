use anyhow::Result;

use crate::page_task_queue::{
    PageWebSocketTargetEffect, PageWebSocketTurnAction, PageWebSocketTurnOutcome,
    RendererPageWebSocketTask,
};

use super::{IntoPageTaskCompletion, PageTaskCompletion, PageVm};

impl IntoPageTaskCompletion for PageWebSocketTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageWebSocketTargetEffect::CallbackVisibleWorkAppliedToCurrentDocument => {
                PageTaskCompletion::CallbackCompletion
            }
            PageWebSocketTargetEffect::InternalStateAppliedToCurrentDocument => {
                PageTaskCompletion::CheckpointOnly
            }
            PageWebSocketTargetEffect::CurrentDocumentTargetDisappeared
            | PageWebSocketTargetEffect::ParkedForReadableBackpressure
            | PageWebSocketTargetEffect::DiscardedStaleDocument { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

impl PageVm {
    pub(in crate::runtime) fn apply_selected_page_websocket_turn(
        &mut self,
        task: RendererPageWebSocketTask,
    ) -> Result<PageWebSocketTurnOutcome> {
        let owner = task.owner();
        let current_document = self.document_lifecycle.identity().document;
        if owner.root_document() != current_document {
            return Ok(PageWebSocketTurnAction {
                owner,
                target_effect: PageWebSocketTargetEffect::DiscardedStaleDocument {
                    current_document,
                },
            }
            .outcome());
        }

        let body_effect = self
            .vm_mut()
            .apply_current_page_websocket_event_body(task.event())?;
        let target_effect = PageWebSocketTargetEffect::from_current_body(body_effect);
        if matches!(
            target_effect,
            PageWebSocketTargetEffect::ParkedForReadableBackpressure
        ) {
            task.return_backpressured();
        }
        Ok(PageWebSocketTurnAction {
            owner,
            target_effect,
        }
        .outcome())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completion_for(target_effect: PageWebSocketTargetEffect) -> PageTaskCompletion {
        PageWebSocketTurnAction {
            owner: crate::page_task_queue::RendererPageWebSocketOwner::new_for_test(
                crate::runtime::RendererDocumentToken::new_for_testing(
                    crate::PageId::new_for_testing(1),
                    1,
                ),
                7,
            ),
            target_effect,
        }
        .into_page_task_completion()
    }

    #[test]
    fn websocket_target_effects_map_to_exact_task_completion_kinds() {
        assert!(matches!(
            completion_for(PageWebSocketTargetEffect::CallbackVisibleWorkAppliedToCurrentDocument),
            PageTaskCompletion::CallbackCompletion
        ));
        assert!(matches!(
            completion_for(PageWebSocketTargetEffect::InternalStateAppliedToCurrentDocument),
            PageTaskCompletion::CheckpointOnly
        ));
        for target_effect in [
            PageWebSocketTargetEffect::CurrentDocumentTargetDisappeared,
            PageWebSocketTargetEffect::ParkedForReadableBackpressure,
            PageWebSocketTargetEffect::DiscardedStaleDocument {
                current_document: crate::runtime::RendererDocumentToken::new_for_testing(
                    crate::PageId::new_for_testing(1),
                    2,
                ),
            },
        ] {
            assert!(matches!(
                completion_for(target_effect),
                PageTaskCompletion::NoCompletion
            ));
        }
    }
}
