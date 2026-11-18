use crate::page::CompletedPageCommand;
use crate::page::Page;
use crate::page::PendingPageCommand;
use crate::renderer::RendererDomNodeReference;
use crate::renderer::RendererPageCommand;
use crate::renderer::RendererPageReply;
use crate::renderer::RendererStyleSheetInventoryUpdate;
use crate::renderer::RendererStyleSheetPayload;
use crate::{RendererDevToolsPageCommand, RendererDevToolsPageReply};
use anyhow::Result;

impl Page {
    pub fn start_set_inline_style_sheet_text_for_style_sheet_id(
        &self,
        style_sheet_id: &str,
        text: &str,
    ) -> Result<PendingPageCommand> {
        self.start_set_inline_style_sheet_text_for_style_sheet_id_and_inspector_session(
            None,
            style_sheet_id,
            text,
        )
    }

    pub fn start_set_inline_style_sheet_text_for_style_sheet_id_and_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        style_sheet_id: &str,
        text: &str,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::SetInlineStyleSheetTextForStyleSheetId {
                inspector_session_id,
                style_sheet_id: style_sheet_id.to_owned(),
                text: text.to_owned(),
            },
        ))
    }

    pub fn start_style_sheet_payload_for_style_sheet_id(
        &self,
        style_sheet_id: &str,
    ) -> Result<PendingPageCommand> {
        self.start_style_sheet_payload_for_style_sheet_id_and_inspector_session(
            None,
            style_sheet_id,
        )
    }

    pub fn start_style_sheet_payload_for_style_sheet_id_and_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        style_sheet_id: &str,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::StyleSheetPayloadForStyleSheetId {
                inspector_session_id,
                style_sheet_id: style_sheet_id.to_owned(),
            },
        ))
    }

    pub fn finish_style_sheet_payload(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<RendererStyleSheetPayload>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "stylesheet payload page command",
            "an optional stylesheet payload",
            RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalStyleSheetPayload(payload)) => Ok(payload),
        )
    }

    pub fn start_style_sheet_inventory_for_document(&self) -> Result<PendingPageCommand> {
        self.start_style_sheet_inventory_for_document_and_inspector_session(None)
    }

    pub fn start_style_sheet_inventory_for_document_and_inspector_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::StyleSheetInventoryForDocument {
                inspector_session_id,
            },
        ))
    }

    pub fn finish_style_sheet_inventory_for_document(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererStyleSheetInventoryUpdate> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "stylesheet inventory page command",
            "stylesheet inventory update",
            RendererPageReply::DevTools(RendererDevToolsPageReply::StyleSheetInventory(update)) => Ok(update),
        )
    }

    pub fn start_reset_css_agent_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::ResetCssAgentSession {
                inspector_session_id,
            },
        ))
    }

    pub fn start_computed_style_properties(
        &self,
        reference: RendererDomNodeReference,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::ComputedStyleProperties { reference },
        ))
    }

    pub fn start_computed_style_properties_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_computed_style_properties(RendererDomNodeReference::BackendNodeId(
            backend_node_id,
        ))
    }

    pub fn start_computed_style_properties_for_object_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        object_id: &str,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::computed_style_properties_for_object_id(
                inspector_session_id,
                object_id.to_owned(),
            ),
        )
    }

    pub fn finish_computed_style_properties(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<Vec<(String, String)>>> {
        expect_page_reply!(
            self.finish_page_command(completion),
            "computed style page command",
            "computed style properties",
            RendererPageReply::DevTools(RendererDevToolsPageReply::ComputedStyleProperties(properties)) => Ok(properties),
        )
    }
}
