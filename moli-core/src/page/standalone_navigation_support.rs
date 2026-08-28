use anyhow::Result;

use super::{Page, RendererTopLevelNavigationRequest};
use crate::renderer::{RendererPageCommand, RendererPageReply};

impl Page {
    /// Applies one already-frozen browser-owner navigation through this
    /// Page's standalone follow state machine.
    ///
    /// This is intentionally crate-local. Protocol-owned Pages use the
    /// NavigationEngine replacement pipeline; the standalone Browser owner
    /// uses this path so auxiliary Pages retain their stable Page/WindowProxy
    /// while carrying POST bodies and request metadata without serializing a
    /// synthetic JavaScript `location` assignment.
    pub(crate) async fn follow_top_level_navigation_in_standalone_adapter_async(
        &mut self,
        request: RendererTopLevelNavigationRequest,
        navigation_history_entry_seed: Option<Box<moli_page_types::NavigationHistoryEntrySeed>>,
    ) -> Result<bool> {
        let reply = self
            .dispatch_page_command_async(
                RendererPageCommand::FollowTopLevelNavigationInStandaloneAdapter {
                    request,
                    navigation_history_entry_seed,
                },
            )
            .await?;
        expect_page_reply!(
            reply,
            "standalone top-level navigation follow",
            "a boolean admission reply",
            RendererPageReply::Bool(admitted) => Ok(admitted),
        )
    }
}
