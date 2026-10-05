use anyhow::Result;

use super::{
    CompletedPageCommand, Page, PendingPageCommand, RendererPageCommand, RendererPageReply,
    RendererResourceSearchRequest, RendererResourceTextSearchOutcome,
};

impl Page {
    /// Resolve and search immutable resource content on its renderer owner.
    pub fn start_resource_search_by_lines(
        &self,
        request: RendererResourceSearchRequest,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SearchResourceByLines(Box::new(
            request,
        )))
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
