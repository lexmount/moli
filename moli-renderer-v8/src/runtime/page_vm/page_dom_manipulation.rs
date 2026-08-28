use crate::page_task_queue::{
    PageDomManipulationTurnAction, PageDomManipulationTurnOutcome, RendererPageDomManipulationTask,
};

use super::PageVm;

impl PageVm {
    pub(in crate::runtime) fn apply_selected_page_dom_manipulation_turn(
        &mut self,
        task: RendererPageDomManipulationTask,
    ) -> anyhow::Result<PageDomManipulationTurnOutcome> {
        match task {
            RendererPageDomManipulationTask::BroadcastChannel(task) => self
                .apply_selected_page_broadcast_channel_delivery_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::BroadcastChannel)),
            RendererPageDomManipulationTask::StorageEvent(task) => self
                .apply_selected_page_storage_event_delivery_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::StorageEvent)),
            RendererPageDomManipulationTask::HashChange(task) => self
                .apply_selected_page_hash_change_delivery_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::HashChange)),
            RendererPageDomManipulationTask::ElementToggle(task) => self
                .apply_selected_page_element_toggle_event_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::ElementToggle)),
            RendererPageDomManipulationTask::FileEntryFileCallback(task) => self
                .apply_selected_page_file_entry_file_callback_turn(task)
                .map(|outcome| {
                    outcome.map_action(PageDomManipulationTurnAction::FileEntryFileCallback)
                }),
            RendererPageDomManipulationTask::ImageLoadEvent(task) => self
                .apply_selected_page_image_load_event_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::ImageLoadEvent)),
            RendererPageDomManipulationTask::ConnectedStyleEvent(task) => self
                .apply_selected_page_connected_style_event_turn(task)
                .map(|outcome| {
                    outcome.map_action(PageDomManipulationTurnAction::ConnectedStyleEvent)
                }),
            RendererPageDomManipulationTask::TextTrackDefaultMode(task) => self
                .apply_selected_page_text_track_default_mode_turn(task)
                .map(|outcome| {
                    outcome.map_action(PageDomManipulationTurnAction::TextTrackDefaultMode)
                }),
            RendererPageDomManipulationTask::TextTrackLoad(task) => self
                .apply_selected_page_text_track_load_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::TextTrackLoad)),
            RendererPageDomManipulationTask::ViewTransitionUpdate(task) => self
                .apply_selected_page_view_transition_update_turn(task)
                .map(|outcome| {
                    outcome.map_action(PageDomManipulationTurnAction::ViewTransitionUpdate)
                }),
        }
    }
}
