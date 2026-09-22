use anyhow::Result;

use super::{CompletedPageCommand, Page, PendingPageCommand, RendererPageCommand};

impl Page {
    pub fn navigation_history(&self) -> moli_renderer_v8::RendererNavigationHistory {
        self.page_state.state().navigation_history.clone()
    }

    pub fn start_reset_navigation_history(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::ResetNavigationHistory)
    }

    pub fn finish_reset_navigation_history(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<bool> {
        let reply = self.finish_page_command(completion);
        Self::decode_bool_page_reply(reply, "reset navigation history")
    }

    pub fn start_top_level_history_traversal_by_delta(
        &self,
        delta: i64,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::QueueTopLevelHistoryTraversalByDelta(
            delta,
        ))
    }

    pub fn finish_top_level_history_traversal_by_delta(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<bool> {
        let reply = self.finish_page_command(completion);
        Self::decode_bool_page_reply(reply, "top-level history traversal")
    }
}
