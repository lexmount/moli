use super::super::*;

/// A borrowed, session-scoped Accessibility agent capability, without Page access.
pub struct RendererAccessibilityInspection<'a> {
    endpoint: &'a RendererInspectionEndpoint,
    attachment: RendererAgentAttachmentId,
    inspector_session_id: Option<String>,
}

impl RendererInspectionEndpoint {
    pub fn accessibility_inspection(
        &self,
        attachment: RendererAgentAttachmentId,
        inspector_session_id: Option<String>,
    ) -> RendererAccessibilityInspection<'_> {
        RendererAccessibilityInspection {
            endpoint: self,
            attachment,
            inspector_session_id,
        }
    }
}

impl RendererAccessibilityInspection<'_> {
    fn start_page_command(
        &self,
        command: RendererPageCommand,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.endpoint.enqueue_typed_inspection_command(
            self.attachment,
            self.inspector_session_id.clone(),
            command,
        )
    }

    pub fn start_accessibility_tree_payloads_for_document(
        &self,
        max_depth: Option<i32>,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::AccessibilityTreePayloadsForDocument {
            max_depth,
        })
    }

    pub fn start_accessibility_node_payload_for_document(
        &self,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::AccessibilityNodePayloadForDocument)
    }

    pub fn start_accessibility_tree_payloads_for_backend_node_id(
        &self,
        backend_node_id: u32,
        max_depth: Option<i32>,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::AccessibilityTreePayloadsForBackendNodeId {
                backend_node_id,
                max_depth,
            },
        )
    }

    pub fn start_accessibility_node_and_ancestor_payloads_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::AccessibilityNodeAndAncestorPayloadsForBackendNodeId {
                backend_node_id,
            },
        )
    }

    pub fn start_accessibility_child_node_payloads_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::AccessibilityChildNodePayloadsForBackendNodeId { backend_node_id },
        )
    }

    pub fn start_accessibility_partial_tree_payloads_for_backend_node_id(
        &self,
        backend_node_id: u32,
        fetch_relatives: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::AccessibilityPartialTreePayloadsForBackendNodeId {
                backend_node_id,
                fetch_relatives,
            },
        )
    }

    pub fn start_accessibility_tree_payloads_for_object_id(
        &self,
        object_id: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::accessibility_tree_payloads_for_object_id(
                self.inspector_session_id.clone(),
                object_id.to_owned(),
            ),
        )
    }

    pub fn start_accessibility_node_and_ancestor_payloads_for_object_id(
        &self,
        object_id: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::accessibility_node_and_ancestor_payloads_for_object_id(
                self.inspector_session_id.clone(),
                object_id.to_owned(),
            ),
        )
    }

    pub fn start_accessibility_partial_tree_payloads_for_object_id(
        &self,
        object_id: &str,
        fetch_relatives: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::accessibility_partial_tree_payloads_for_object_id(
                self.inspector_session_id.clone(),
                object_id.to_owned(),
                fetch_relatives,
            ),
        )
    }

    pub fn start_child_frame_accessibility_tree_payloads(
        &self,
        frame_id: &str,
        max_depth: Option<i32>,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::AccessibilityTreePayloadsForChildFrame {
                frame_id: frame_id.to_owned(),
                max_depth,
            },
        )
    }

    pub fn start_child_frame_accessibility_node_payload(
        &self,
        frame_id: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::AccessibilityNodePayloadForChildFrame {
            frame_id: frame_id.to_owned(),
        })
    }
}
