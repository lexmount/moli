//! Completion boundary for the shared DOM-manipulation source.
//!
//! Every current source variant has completed its P5 migration. Each domain
//! maps its typed, post-execution action to `PageTaskCompletion` in the module
//! that owns that action; this family coordinator only submits the resulting
//! boundary. Source membership therefore cannot become checkpoint policy.

use anyhow::Result;

use crate::page_task_queue::PageDomManipulationTurnAction;

use super::{IntoPageTaskCompletion, PageVm};

impl PageVm {
    pub(super) async fn finish_selected_page_dom_manipulation_task(
        &mut self,
        action: PageDomManipulationTurnAction,
        loader: &crate::network::ResourceRequestClient,
    ) -> Result<()> {
        let completion = match action {
            PageDomManipulationTurnAction::BroadcastChannel(action) => {
                action.into_page_task_completion()
            }
            PageDomManipulationTurnAction::StorageEvent(action) => {
                action.into_page_task_completion()
            }
            PageDomManipulationTurnAction::HashChange(action) => action.into_page_task_completion(),
            PageDomManipulationTurnAction::ElementToggle(action) => {
                action.into_page_task_completion()
            }
            PageDomManipulationTurnAction::FileEntryFileCallback(action) => {
                action.into_page_task_completion()
            }
            PageDomManipulationTurnAction::ImageLoadEvent(action) => {
                action.into_page_task_completion()
            }
            PageDomManipulationTurnAction::PopupLoadEvent(action) => {
                action.into_page_task_completion()
            }
            PageDomManipulationTurnAction::PopupClose(action) => action.into_page_task_completion(),
            PageDomManipulationTurnAction::ConnectedStyleEvent(action) => {
                return self
                    .finish_selected_page_connected_style_event_task(action, loader)
                    .await;
            }
            PageDomManipulationTurnAction::TextTrackDefaultMode(action) => {
                action.into_page_task_completion()
            }
            PageDomManipulationTurnAction::TextTrackLoad(action) => {
                action.into_page_task_completion()
            }
            PageDomManipulationTurnAction::ViewTransitionUpdate(action) => {
                action.into_page_task_completion()
            }
        };
        self.finish_selected_page_task_completion(completion, loader)
            .await
    }
}
