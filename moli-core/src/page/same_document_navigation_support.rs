use anyhow::Result;
#[cfg(feature = "devtools")]
use anyhow::bail;

#[cfg(feature = "devtools")]
use super::{CompletedPageCommand, RendererCommandTurnOutput};
use super::{Page, PendingPageCommand};
use crate::renderer::RendererPageCommand;
#[cfg(feature = "devtools")]
use crate::renderer::RendererPageReply;

impl Page {
    pub fn start_top_level_same_document_navigation(
        &self,
        url: String,
    ) -> Result<PendingPageCommand> {
        self.start_top_level_same_document_navigation_with_history_replacement(url, false)
    }

    pub fn start_top_level_same_document_navigation_with_history_replacement(
        &self,
        url: String,
        replace_current: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::NavigateTopLevelSameDocument {
            url,
            replace_current,
        })
    }

    #[cfg(feature = "devtools")]
    pub fn finish_top_level_same_document_navigation_command_turn(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<(bool, RendererCommandTurnOutput)> {
        let output = self.finish_page_command_turn(completion);
        let RendererPageReply::Bool(completed) = output.completion().reply() else {
            bail!(
                "top-level same-document navigation page command expected a bool reply, got {}",
                Self::page_reply_kind(output.completion().reply())
            );
        };
        Ok((*completed, output))
    }
}
