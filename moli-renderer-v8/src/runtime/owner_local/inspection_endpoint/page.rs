use super::super::*;

use anyhow::Result;

/// A session-scoped native Page agent capability borrowed from an inspection
/// binding. It exposes only Page queries that can run without entering V8.
pub struct RendererPageInspection<'a> {
    endpoint: &'a RendererInspectionEndpoint,
    attachment: RendererAgentAttachmentId,
    inspector_session_id: Option<String>,
}

impl RendererInspectionEndpoint {
    pub fn page_inspection(
        &self,
        attachment: RendererAgentAttachmentId,
        inspector_session_id: Option<String>,
    ) -> RendererPageInspection<'_> {
        RendererPageInspection {
            endpoint: self,
            attachment,
            inspector_session_id,
        }
    }
}

impl RendererPageInspection<'_> {
    pub fn start_child_frame_tree_snapshot(
        &self,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::ChildFrameTreeSnapshot)
    }

    pub fn start_layout_metrics(&self) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::LayoutMetrics)
    }

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
}
