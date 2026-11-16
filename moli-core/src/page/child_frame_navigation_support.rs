use anyhow::Result;

#[cfg(feature = "devtools")]
use super::{CompletedPageCommand, RendererCommandTurnOutput};
use super::{Page, PendingPageCommand};
use crate::renderer::RendererPageCommand;
#[cfg(feature = "devtools")]
use crate::renderer::RendererPageReply;

impl Page {
    pub fn start_child_frame_navigation_to_url(
        &self,
        frame_id: &str,
        url: &str,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::NavigateChildFrame {
            frame_id: frame_id.to_owned(),
            url: url.to_owned(),
        })
    }

    #[cfg(feature = "devtools")]
    pub fn finish_child_frame_navigation_to_url(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<bool> {
        let (completed, _) = self.finish_child_frame_navigation_to_url_command_turn(completion)?;
        Ok(completed)
    }

    #[cfg(feature = "devtools")]
    pub fn finish_child_frame_navigation_to_url_command_turn(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<(bool, RendererCommandTurnOutput)> {
        let output = self.finish_page_command_turn(completion);
        let RendererPageReply::Bool(completed) = output.completion().reply() else {
            anyhow::bail!(
                "child frame navigation page command expected a bool reply, got {}",
                Self::page_reply_kind(output.completion().reply())
            );
        };
        Ok((*completed, output))
    }
}
