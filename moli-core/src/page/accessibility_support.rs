use super::Page;
use crate::renderer::RendererPageCommand;
use crate::renderer::RendererPageReply;

impl Page {
    /// Observe the same live AX tree as the protocol without materializing a
    /// JSON object for every node. `frame_id` selects a child Document.
    pub async fn accessibility_tree_nodes_async(
        &mut self,
        frame_id: Option<String>,
    ) -> anyhow::Result<Option<Vec<moli_dom::accessibility::AccessibilityNode>>> {
        let pending =
            self.start_page_command(RendererPageCommand::AccessibilityTreeNodes { frame_id })?;
        let completion = pending.wait().await?;
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "accessibility tree nodes page command",
            "optional accessibility nodes",
            RendererPageReply::OptionalAccessibilityNodes(nodes) => Ok(nodes),
        )
    }
}
