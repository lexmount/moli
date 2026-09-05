use super::{CompletedPageCommand, Page};
use crate::renderer::{
    RendererAccessibilityPayloadsForObjectId, RendererPageCommand, RendererPageReply,
};
use serde_json::Value;

impl Page {
    // Browser-owned semantic reads used by CLI. They do not acquire a DevTools session.
    pub async fn accessibility_tree_payloads_for_document_async(
        &mut self,
        max_depth: Option<i32>,
    ) -> anyhow::Result<Vec<Value>> {
        let pending =
            self.start_page_command(RendererPageCommand::AccessibilityTreePayloadsForDocument {
                max_depth,
            })?;
        let completion = pending.wait().await?;
        self.observe_renderer_page_state(completion.page_state());
        Ok(completion
            .finish_accessibility_tree_payloads_optional()?
            .unwrap_or_default())
    }

    pub async fn child_frame_accessibility_tree_payloads_async(
        &mut self,
        frame_id: &str,
        max_depth: Option<i32>,
    ) -> anyhow::Result<Option<Vec<Value>>> {
        let pending = self.start_page_command(
            RendererPageCommand::AccessibilityTreePayloadsForChildFrame {
                frame_id: frame_id.to_owned(),
                max_depth,
            },
        )?;
        let completion = pending.wait().await?;
        self.observe_renderer_page_state(completion.page_state());
        Ok(completion
            .finish_child_frame_accessibility_payloads()?
            .and_then(|payloads| payloads.payloads))
    }

    pub async fn accessibility_node_payload_for_backend_node_id_async(
        &mut self,
        backend_node_id: u32,
    ) -> anyhow::Result<Option<Value>> {
        let pending = self.start_page_command(
            RendererPageCommand::AccessibilityNodePayloadForBackendNodeId { backend_node_id },
        )?;
        let completion = pending.wait().await?;
        self.observe_renderer_page_state(completion.page_state());
        Ok(completion
            .finish_accessibility_payloads_for_backend_node_id()?
            .and_then(|payloads| payloads.payloads)
            .and_then(|payloads| payloads.into_iter().next()))
    }
}

impl CompletedPageCommand {
    pub fn finish_accessibility_tree_payloads_optional(self) -> anyhow::Result<Option<Vec<Value>>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "accessibility tree payloads page command",
            "optional accessibility payloads",
            RendererPageReply::OptionalAccessibilityPayloads(payloads) => Ok(payloads),
        )
    }

    pub fn finish_accessibility_node_payload(self) -> anyhow::Result<Option<Value>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "accessibility node payload page command",
            "an optional accessibility payload",
            RendererPageReply::OptionalAccessibilityPayload(payload) => Ok(payload),
        )
    }

    pub fn finish_accessibility_payloads_for_backend_node_id(
        self,
    ) -> anyhow::Result<Option<RendererAccessibilityPayloadsForObjectId>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "accessibility payloads for backend node id page command",
            "optional accessibility payloads for backend node id",
            RendererPageReply::OptionalAccessibilityPayloadsForObjectId(payloads) => Ok(payloads),
        )
    }

    pub fn finish_accessibility_payloads_for_object_id(
        self,
    ) -> anyhow::Result<Option<RendererAccessibilityPayloadsForObjectId>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "accessibility payloads for object id page command",
            "optional accessibility payloads for object id",
            RendererPageReply::OptionalAccessibilityPayloadsForObjectId(payloads) => Ok(payloads),
        )
    }

    pub fn finish_child_frame_accessibility_payloads(
        self,
    ) -> anyhow::Result<Option<RendererAccessibilityPayloadsForObjectId>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "child-frame accessibility payloads page command",
            "optional child-frame accessibility payloads",
            RendererPageReply::OptionalAccessibilityPayloadsForObjectId(payloads) => Ok(payloads),
        )
    }
}
