use super::super::*;

/// A borrowed, session-scoped CSS agent capability, without Page access.
pub struct RendererCssInspection<'a> {
    endpoint: &'a RendererInspectionEndpoint,
    attachment: RendererAgentAttachmentId,
    inspector_session_id: Option<String>,
}

impl RendererInspectionEndpoint {
    pub fn css_inspection(
        &self,
        attachment: RendererAgentAttachmentId,
        inspector_session_id: Option<String>,
    ) -> RendererCssInspection<'_> {
        RendererCssInspection {
            endpoint: self,
            attachment,
            inspector_session_id,
        }
    }
}

impl RendererCssInspection<'_> {
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

    pub fn start_set_style_sheet_text(
        &self,
        style_sheet_id: &str,
        text: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::SetInlineStyleSheetTextForStyleSheetId {
                inspector_session_id: self.inspector_session_id.clone(),
                style_sheet_id: style_sheet_id.to_owned(),
                text: text.to_owned(),
            },
        )
    }

    pub fn start_style_sheet_payload(
        &self,
        style_sheet_id: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::StyleSheetPayloadForStyleSheetId {
            inspector_session_id: self.inspector_session_id.clone(),
            style_sheet_id: style_sheet_id.to_owned(),
        })
    }

    pub fn start_style_sheet_inventory(&self) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::StyleSheetInventoryForDocument {
            inspector_session_id: self.inspector_session_id.clone(),
        })
    }

    pub fn start_reset(&self) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::ResetCssAgentSession {
            inspector_session_id: self.inspector_session_id.clone(),
        })
    }

    pub fn start_computed_style_for_backend_node(
        &self,
        backend_node_id: u32,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::ComputedStylePropertiesForBackendNodeId { backend_node_id },
        )
    }

    pub fn start_computed_style_for_object(
        &self,
        object_id: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::computed_style_properties_for_object_id(
                self.inspector_session_id.clone(),
                object_id.to_owned(),
            ),
        )
    }
}
