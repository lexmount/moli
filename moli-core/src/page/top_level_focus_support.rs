use anyhow::{Result, bail};

use super::{CompletedPageCommand, Page, PendingPageCommand, RendererCommandTurnOutput};
use crate::renderer::{RendererPageCommand, RendererPageReply};

impl Page {
    pub fn start_set_top_level_page_focus(
        &self,
        active: bool,
        focused: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetTopLevelPageFocus { active, focused })
    }

    pub fn finish_set_top_level_page_focus(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<(bool, RendererCommandTurnOutput)> {
        let output = self.finish_page_command_turn(completion);
        let RendererPageReply::Bool(changed) = output.completion().reply() else {
            bail!(
                "top-level Page focus expected a bool reply, got {}",
                Page::page_reply_kind(output.completion().reply())
            );
        };
        Ok((*changed, output))
    }

    pub async fn set_top_level_page_focus_async(
        &mut self,
        active: bool,
        focused: bool,
    ) -> Result<(bool, RendererCommandTurnOutput)> {
        let pending = self.start_set_top_level_page_focus(active, focused)?;
        let completion = pending.wait().await?;
        self.finish_set_top_level_page_focus(completion)
    }
}
