use anyhow::{Result, bail};

use super::{CompletedPageCommand, Page, PendingPageCommand, RendererCommandTurnOutput};
use crate::renderer::{RendererPageCommand, RendererPageReply};

impl Page {
    pub fn start_browser_page_close_request(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::RequestBrowserPageClose)
    }

    pub fn finish_browser_page_close_request(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<(bool, RendererCommandTurnOutput)> {
        decode_close_bool_reply(
            self.finish_page_command_turn(completion),
            "browser Page.close preflight",
        )
    }

    pub async fn dispatch_browser_page_close_unload_async(
        &mut self,
        source: crate::RendererTopLevelCloseSource,
    ) -> Result<(bool, RendererCommandTurnOutput)> {
        let pending =
            self.start_page_command(RendererPageCommand::DispatchPageCloseUnload(source))?;
        let completion = pending.wait().await?;
        decode_close_bool_reply(
            self.finish_page_command_turn(completion),
            "browser Page close unload",
        )
    }

    pub async fn acknowledge_browser_page_close_network_drained_async(
        &mut self,
        source: crate::RendererTopLevelCloseSource,
    ) -> Result<(bool, RendererCommandTurnOutput)> {
        let pending = self.start_page_command(
            RendererPageCommand::AcknowledgeBrowserPageCloseNetworkDrained(source),
        )?;
        let completion = pending.wait().await?;
        decode_close_bool_reply(
            self.finish_page_command_turn(completion),
            "browser Page close network-drained ACK",
        )
    }
}

fn decode_close_bool_reply(
    output: RendererCommandTurnOutput,
    operation: &str,
) -> Result<(bool, RendererCommandTurnOutput)> {
    let RendererPageReply::Bool(completed) = output.completion().reply() else {
        bail!(
            "{operation} expected a bool reply, got {}",
            Page::page_reply_kind(output.completion().reply())
        );
    };
    Ok((*completed, output))
}
