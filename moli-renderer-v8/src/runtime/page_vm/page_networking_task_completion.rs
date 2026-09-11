//! Completion boundary for migrated tasks in the shared Networking source.
//!
//! Text-track loads, stylesheet terminals, Worker host-bridge records and
//! resource terminals submit their typed completion here. A shared FIFO does
//! not imply a shared checkpoint policy: each family first produces its own
//! exact-owner action, then maps that post-execution fact to task completion.

use anyhow::Result;

use crate::page_task_queue::{PageNetworkingTurnAction, PageStylesheetNetworkingTargetEffect};

use super::{IntoPageTaskCompletion, PageVm};

impl PageVm {
    pub(super) async fn finish_selected_page_networking_task(
        &mut self,
        action: PageNetworkingTurnAction,
        loader: &crate::network::ResourceRequestClient,
    ) -> Result<()> {
        match action {
            PageNetworkingTurnAction::ResourceCompletion(action) => {
                self.finish_selected_page_resource_completion_task(action)?;
            }
            PageNetworkingTurnAction::StyleElementEvent(action) => {
                self.finish_selected_page_connected_style_event_task(action, loader)
                    .await?;
            }
            PageNetworkingTurnAction::TextTrackLoad(action) => {
                self.finish_selected_page_task_completion(
                    action.into_page_task_completion(),
                    loader,
                )
                .await?;
            }
            PageNetworkingTurnAction::StylesheetCompletion(action) => {
                let completion_owner = action.owner;
                let applied_to_current_owner = matches!(
                    action.target_effect,
                    PageStylesheetNetworkingTargetEffect::AppliedToCurrentOwner
                );
                self.finish_selected_page_task_completion(
                    action.into_page_task_completion(),
                    loader,
                )
                .await?;
                let completion_owner_is_still_current = self.vm().stylesheet_task_owner_is_current(
                    self.document_lifecycle.identity().document,
                    completion_owner,
                );
                // Phase one consumes its selected, exact-owner continuation
                // as the sole parser-resume authority. The direct fallback is
                // only for a document.write()/Page.setDocumentContent parser
                // created after that driver retired. A stale completion, or a
                // checkpoint that replaced its Document, must never touch the
                // replacement parser.
                if applied_to_current_owner
                    && completion_owner_is_still_current
                    && self
                        .vm()
                        .document_runtime
                        .main_parser_continuation_producer()
                        .is_none()
                {
                    self.run_ready_document_write_stylesheet_blocked_script()
                        .await?;
                }
            }
            PageNetworkingTurnAction::WorkerHostBridge(action) => {
                self.finish_selected_page_task_completion(
                    action.into_page_task_completion(),
                    loader,
                )
                .await?;
            }
            PageNetworkingTurnAction::MainParserContinuation(_) => {}
        }
        Ok(())
    }
}
