use super::{CompletedPageCommand, Page, PendingPageCommand};
use crate::renderer::{
    RendererAccessibilityPayloadsForObjectId, RendererPageCommand, RendererPageReply,
};
use serde_json::Value;

impl Page {
    pub fn start_accessibility_tree_payloads_for_document(
        &self,
        max_depth: Option<i32>,
    ) -> anyhow::Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::AccessibilityTreePayloadsForDocument {
            max_depth,
        })
    }

    pub fn finish_accessibility_tree_payloads(
        &mut self,
        completion: CompletedPageCommand,
    ) -> anyhow::Result<Vec<Value>> {
        Ok(self
            .finish_accessibility_tree_payloads_optional(completion)?
            .unwrap_or_default())
    }

    pub fn finish_accessibility_tree_payloads_optional(
        &mut self,
        completion: CompletedPageCommand,
    ) -> anyhow::Result<Option<Vec<Value>>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "accessibility tree payloads page command",
            "optional accessibility payloads",
            RendererPageReply::OptionalAccessibilityPayloads(payloads) => Ok(payloads),
        )
    }

    pub async fn accessibility_tree_payloads_for_document_async(
        &mut self,
        max_depth: Option<i32>,
    ) -> anyhow::Result<Vec<Value>> {
        let pending = self.start_accessibility_tree_payloads_for_document(max_depth)?;
        let completion = pending.wait().await?;
        self.finish_accessibility_tree_payloads(completion)
    }

    pub fn start_accessibility_node_payload_for_document(
        &self,
    ) -> anyhow::Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::AccessibilityNodePayloadForDocument)
    }

    pub fn finish_accessibility_node_payload(
        &mut self,
        completion: CompletedPageCommand,
    ) -> anyhow::Result<Option<Value>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "accessibility node payload page command",
            "an optional accessibility payload",
            RendererPageReply::OptionalAccessibilityPayload(payload) => Ok(payload),
        )
    }

    pub fn start_accessibility_tree_payloads_for_backend_node_id(
        &self,
        backend_node_id: u32,
        max_depth: Option<i32>,
    ) -> anyhow::Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::AccessibilityTreePayloadsForBackendNodeId {
                backend_node_id,
                max_depth,
            },
        )
    }

    pub fn start_accessibility_node_payload_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> anyhow::Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::AccessibilityNodePayloadForBackendNodeId { backend_node_id },
        )
    }

    pub fn start_accessibility_node_and_ancestor_payloads_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> anyhow::Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::AccessibilityNodeAndAncestorPayloadsForBackendNodeId {
                backend_node_id,
            },
        )
    }

    pub fn start_accessibility_child_node_payloads_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> anyhow::Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::AccessibilityChildNodePayloadsForBackendNodeId { backend_node_id },
        )
    }

    pub fn start_accessibility_partial_tree_payloads_for_backend_node_id(
        &self,
        backend_node_id: u32,
        fetch_relatives: bool,
    ) -> anyhow::Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::AccessibilityPartialTreePayloadsForBackendNodeId {
                backend_node_id,
                fetch_relatives,
            },
        )
    }

    pub fn finish_accessibility_payloads_for_backend_node_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> anyhow::Result<Option<RendererAccessibilityPayloadsForObjectId>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "accessibility payloads for backend node id page command",
            "optional accessibility payloads for backend node id",
            RendererPageReply::OptionalAccessibilityPayloadsForObjectId(payloads) => Ok(payloads),
        )
    }

    pub fn start_accessibility_tree_payloads_for_object_id(
        &self,
        inspector_session_id: Option<String>,
        object_id: &str,
    ) -> anyhow::Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::accessibility_tree_payloads_for_object_id(
                inspector_session_id,
                object_id.to_owned(),
            ),
        )
    }

    pub fn start_accessibility_node_and_ancestor_payloads_for_object_id(
        &self,
        inspector_session_id: Option<String>,
        object_id: &str,
    ) -> anyhow::Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::accessibility_node_and_ancestor_payloads_for_object_id(
                inspector_session_id,
                object_id.to_owned(),
            ),
        )
    }

    pub fn start_accessibility_partial_tree_payloads_for_object_id(
        &self,
        inspector_session_id: Option<String>,
        object_id: &str,
        fetch_relatives: bool,
    ) -> anyhow::Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::accessibility_partial_tree_payloads_for_object_id(
                inspector_session_id,
                object_id.to_owned(),
                fetch_relatives,
            ),
        )
    }

    pub fn finish_accessibility_payloads_for_object_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> anyhow::Result<Option<RendererAccessibilityPayloadsForObjectId>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "accessibility payloads for object id page command",
            "optional accessibility payloads for object id",
            RendererPageReply::OptionalAccessibilityPayloadsForObjectId(payloads) => Ok(payloads),
        )
    }

    pub async fn child_frame_accessibility_tree_payloads_async(
        &mut self,
        frame_id: &str,
        max_depth: Option<i32>,
    ) -> anyhow::Result<Option<Vec<Value>>> {
        let pending = self.start_child_frame_accessibility_tree_payloads(frame_id, max_depth)?;
        let completion = pending.wait().await?;
        self.finish_child_frame_accessibility_tree_payloads(completion, max_depth)
    }

    pub fn start_child_frame_accessibility_tree_payloads(
        &self,
        frame_id: &str,
        max_depth: Option<i32>,
    ) -> anyhow::Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::AccessibilityTreePayloadsForChildFrame {
                frame_id: frame_id.to_owned(),
                max_depth,
            },
        )
    }

    pub fn finish_child_frame_accessibility_tree_payloads(
        &mut self,
        completion: CompletedPageCommand,
        _max_depth: Option<i32>,
    ) -> anyhow::Result<Option<Vec<Value>>> {
        Ok(self
            .finish_child_frame_accessibility_payloads(completion)?
            .and_then(|payloads| payloads.payloads))
    }

    pub fn finish_child_frame_accessibility_payloads(
        &mut self,
        completion: CompletedPageCommand,
    ) -> anyhow::Result<Option<RendererAccessibilityPayloadsForObjectId>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "child-frame accessibility payloads page command",
            "optional child-frame accessibility payloads",
            RendererPageReply::OptionalAccessibilityPayloadsForObjectId(payloads) => Ok(payloads),
        )
    }

    pub fn start_child_frame_accessibility_node_payload(
        &self,
        frame_id: &str,
    ) -> anyhow::Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::AccessibilityNodePayloadForChildFrame {
            frame_id: frame_id.to_owned(),
        })
    }

    pub fn finish_child_frame_accessibility_node_payload(
        &mut self,
        completion: CompletedPageCommand,
    ) -> anyhow::Result<Option<Value>> {
        Ok(self
            .finish_child_frame_accessibility_payloads(completion)?
            .and_then(|payloads| payloads.payloads)
            .and_then(|mut payloads| {
                if payloads.is_empty() {
                    None
                } else {
                    Some(payloads.remove(0))
                }
            }))
    }
}
