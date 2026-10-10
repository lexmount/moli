use crate::page_task_queue::{
    PageNetworkingTurnAction, PageNetworkingTurnOutcome, RendererPageNetworkingTask,
};

use super::PageVm;

impl PageVm {
    pub(in crate::runtime) fn apply_selected_page_networking_turn(
        &mut self,
        task: RendererPageNetworkingTask,
    ) -> anyhow::Result<PageNetworkingTurnOutcome> {
        match task {
            RendererPageNetworkingTask::WebRtc(task) => self
                .apply_selected_page_webrtc_turn(task)
                .map(|outcome| outcome.map_action(PageNetworkingTurnAction::WebRtc)),
            RendererPageNetworkingTask::ResourceCompletion(completion) => self
                .apply_selected_page_resource_completion_turn(*completion)
                .map(|outcome| outcome.map_action(PageNetworkingTurnAction::ResourceCompletion)),
            RendererPageNetworkingTask::MainParserContinuation(task) => Ok(self
                .apply_selected_page_main_parser_continuation_turn(task)
                .map_action(PageNetworkingTurnAction::MainParserContinuation)),
            RendererPageNetworkingTask::StyleElementEvent(task) => self
                .apply_selected_page_connected_style_event_turn(task)
                .map(|outcome| outcome.map_action(PageNetworkingTurnAction::StyleElementEvent)),
            RendererPageNetworkingTask::TextTrackLoad(task) => self
                .apply_selected_page_text_track_load_turn(task)
                .map(|outcome| outcome.map_action(PageNetworkingTurnAction::TextTrackLoad)),
            RendererPageNetworkingTask::WorkerHostBridge(task) => self
                .apply_selected_page_worker_host_bridge_turn(task)
                .map(|outcome| outcome.map_action(PageNetworkingTurnAction::WorkerHostBridge)),
            RendererPageNetworkingTask::StylesheetCompletion(task) => self
                .apply_selected_page_stylesheet_networking_turn(task)
                .map(|outcome| outcome.map_action(PageNetworkingTurnAction::StylesheetCompletion)),
        }
    }
}
