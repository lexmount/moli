use anyhow::Result;

use super::{
    CompletedPageCommand, Page, PendingPageCommand, RendererPageCommand, RendererPageReply,
    RendererResourceTextSearchOutcome,
};

impl Page {
    /// Capture current resource records before selecting the immutable source.
    pub fn start_resource_search_snapshot(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::ResourceSearchSnapshot)
    }

    pub fn start_text_search_by_lines(
        &self,
        text: String,
        query: String,
        case_sensitive: bool,
        is_regex: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SearchTextByLines {
            text,
            query,
            case_sensitive,
            is_regex,
        })
    }

    pub fn start_child_frame_resource_search_by_lines(
        &self,
        frame_id: String,
        url: String,
        query: String,
        case_sensitive: bool,
        is_regex: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SearchChildFrameResourceByLines {
            frame_id,
            url,
            query,
            case_sensitive,
            is_regex,
        })
    }

    pub fn finish_resource_search_by_lines(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererResourceTextSearchOutcome> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "resource text search",
            "a resource text search outcome",
            RendererPageReply::ResourceTextSearchOutcome(outcome) => Ok(outcome),
        )
    }
}
