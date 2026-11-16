use anyhow::Result;

use super::{
    CompletedPageCommand, Page, PendingPageCommand, RendererChildFrameNavigation,
    RendererCommandTurnOutput,
};
use crate::renderer::{RendererPageCommand, RendererPageReply};

impl Page {
    pub fn start_child_frame_navigation_identity_snapshot(
        &self,
        frame_id: &str,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::ChildFrameNavigationIdentitySnapshot {
            frame_id: frame_id.to_owned(),
        })
    }

    pub fn finish_child_frame_navigation_identity_snapshot(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<moli_page_types::ChildFrameNavigationIdentitySnapshot>> {
        match self.finish_page_command(completion) {
            RendererPageReply::ChildFrameNavigationIdentitySnapshot(identity) => Ok(identity),
            reply => anyhow::bail!(
                "child navigation identity expected a snapshot, got {}",
                Self::page_reply_kind(&reply)
            ),
        }
    }

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

    pub fn finish_child_frame_navigation_to_url(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<bool> {
        let (completed, _) = self.finish_child_frame_navigation_to_url_command_turn(completion)?;
        Ok(completed.is_some())
    }

    pub fn finish_child_frame_navigation_to_url_command_turn(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<(Option<u64>, RendererCommandTurnOutput)> {
        let (navigation_id, _, output) =
            self.finish_child_frame_navigation_with_identity_command_turn(completion)?;
        Ok((navigation_id, output))
    }

    pub fn finish_child_frame_navigation_with_identity_command_turn(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<(
        Option<u64>,
        Option<RendererChildFrameNavigation>,
        RendererCommandTurnOutput,
    )> {
        let output = self.finish_page_command_turn(completion);
        let RendererPageReply::ChildFrameNavigation {
            navigation_id,
            navigation,
        } = output.completion().reply()
        else {
            anyhow::bail!(
                "child frame navigation expected its admitted identity, got {}",
                Self::page_reply_kind(output.completion().reply())
            );
        };
        Ok((*navigation_id, navigation.clone(), output))
    }
}
