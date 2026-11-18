use crate::page::CompletedPageCommand;
use crate::page::Page;
use crate::page::PendingPageCommand;
use crate::page::RendererNetworkResourceLoadPreparation;
use crate::page::RendererPageCommand;
use crate::page::RendererPageReply;
use crate::page::RendererResourceSearchRequest;
use crate::page::RendererResourceTextSearchOutcome;
use crate::{RendererDevToolsPageCommand, RendererDevToolsPageReply};
use anyhow::Result;
use url::Url;

impl Page {
    pub fn start_prepare_network_resource_load(
        &self,
        frame_id: String,
        url: Url,
        disable_cache: bool,
        include_credentials: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::PrepareNetworkResourceLoad {
                frame_id,
                url,
                disable_cache,
                include_credentials,
            },
        ))
    }

    pub fn finish_prepare_network_resource_load(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererNetworkResourceLoadPreparation> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "prepare DevTools network resource load",
            "a network resource load preparation",
            RendererPageReply::DevTools(RendererDevToolsPageReply::NetworkResourceLoadPreparation(preparation)) => Ok(preparation),
        )
    }
}

impl Page {
    /// Resolve and search immutable resource content on its renderer owner.
    pub fn start_resource_search_by_lines(
        &self,
        request: RendererResourceSearchRequest,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::SearchResourceByLines(Box::new(request)),
        ))
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
            RendererPageReply::DevTools(RendererDevToolsPageReply::ResourceTextSearchOutcome(outcome)) => Ok(outcome),
        )
    }
}
