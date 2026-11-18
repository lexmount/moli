use crate::page::CompletedPageCommand;
use crate::page::DocumentNodeClientRectResolution;
use crate::page::Page;
use crate::page::PendingPageCommand;
use crate::renderer::RendererAutofillTriggerOutcome;
use crate::renderer::RendererAutofillTriggerRequest;
use crate::renderer::RendererDocumentChildNodeSnapshotEvents;
use crate::renderer::RendererDocumentFrontendNodeIdsResolution;
use crate::renderer::RendererDocumentHitTestResult;
use crate::renderer::RendererDocumentNodeAttributesResolution;
use crate::renderer::RendererDocumentNodeClientRect;
use crate::renderer::RendererDocumentNodeGeometry;
use crate::renderer::RendererDocumentNodePropertyResolution;
use crate::renderer::RendererDocumentNodeReference;
use crate::renderer::RendererDocumentNodeTextResolution;
use crate::renderer::RendererDocumentQuerySelectorResolution;
use crate::renderer::RendererDocumentQuerySelectorWithChildNodeSnapshotEvents;
use crate::renderer::RendererDomAttributeMutation;
use crate::renderer::RendererDomAttributeMutationOutcome;
use crate::renderer::RendererDomBidiNodeBindingResolution;
use crate::renderer::RendererDomBidiNodeSharedIdResolution;
use crate::renderer::RendererDomEdit;
use crate::renderer::RendererDomEditOutcome;
use crate::renderer::RendererDomFocusOutcome;
use crate::renderer::RendererDomFrontendNodeBindingResolution;
use crate::renderer::RendererDomNodeReference;
use crate::renderer::RendererDomNodeStackTraceResolution;
use crate::renderer::RendererDomSearchRegistration;
use crate::renderer::RendererDomSearchResultsResolution;
use crate::renderer::RendererDomSnapshotCaptureOptions;
use crate::renderer::RendererDomSnapshotCapturePayload;
use crate::renderer::RendererPageCommand;
use crate::renderer::RendererPageReply;
use crate::{RendererDevToolsPageCommand, RendererDevToolsPageReply};
use anyhow::Result;

impl Page {
    pub fn start_node_has_geometry_for_object_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        object_id: &str,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::node_has_geometry_for_object_id(
            inspector_session_id,
            object_id.to_owned(),
        ))
    }

    pub fn start_scroll_node_into_view_if_needed_for_object_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        object_id: &str,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::scroll_object_node_into_view_if_needed(
            inspector_session_id,
            object_id.to_owned(),
            rect,
        ))
    }

    pub fn start_scroll_node_into_view_if_needed(
        &self,
        reference: RendererDomNodeReference,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::ScrollNodeIntoViewIfNeeded { reference, rect },
        ))
    }

    pub fn start_scroll_backend_node_into_view_if_needed(
        &self,
        backend_node_id: u32,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    ) -> Result<PendingPageCommand> {
        self.start_scroll_node_into_view_if_needed(
            RendererDomNodeReference::BackendNodeId(backend_node_id),
            rect,
        )
    }

    pub fn finish_node_has_geometry_for_object_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<bool>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "node has geometry object id page command",
            "an optional bool reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalBool(has_geometry)) => Ok(has_geometry),
        )
    }

    pub fn start_node_has_geometry_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::NodeHasGeometryForBackendNodeId { backend_node_id },
        ))
    }

    pub fn finish_node_has_geometry_for_backend_node_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<bool>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "node has geometry backend node id page command",
            "an optional bool reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalBool(has_geometry)) => Ok(has_geometry),
        )
    }

    pub fn finish_scroll_node_into_view_if_needed(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<crate::page::RendererScrollIntoViewResult> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "scroll document node into view page command",
            "a scroll-into-view reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::ScrollIntoViewResult(result)) => Ok(result),
        )
    }

    pub fn start_client_rect_for_object_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        object_id: &str,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::client_rect_for_object_id(
            inspector_session_id,
            object_id.to_owned(),
        ))
    }

    pub fn finish_client_rect_for_object_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<DocumentNodeClientRectResolution>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "client rect object id page command",
            "an optional document node client rect reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalDocumentNodeClientRect(rect)) => Ok(rect.map(|rect| match rect {
                RendererDocumentNodeClientRect::Found(rect) => {
                    DocumentNodeClientRectResolution::Found(rect.into())
                }
                RendererDocumentNodeClientRect::FoundNonElement(rect) => {
                    DocumentNodeClientRectResolution::FoundNonElement(rect.into())
                }
                RendererDocumentNodeClientRect::NotElement => {
                    DocumentNodeClientRectResolution::NotElement
                }
            })),
        )
    }

    pub fn start_client_rect_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::ClientRectForBackendNodeId { backend_node_id },
        ))
    }

    pub fn finish_client_rect_for_backend_node_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<DocumentNodeClientRectResolution>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "client rect backend node id page command",
            "an optional document node client rect reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalDocumentNodeClientRect(rect)) => Ok(rect.map(|rect| match rect {
                RendererDocumentNodeClientRect::Found(rect) => {
                    DocumentNodeClientRectResolution::Found(rect.into())
                }
                RendererDocumentNodeClientRect::FoundNonElement(rect) => {
                    DocumentNodeClientRectResolution::FoundNonElement(rect.into())
                }
                RendererDocumentNodeClientRect::NotElement => {
                    DocumentNodeClientRectResolution::NotElement
                }
            })),
        )
    }

    pub fn start_document_geometry_for_object_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        object_id: &str,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::document_geometry_for_object_id(
            inspector_session_id,
            object_id.to_owned(),
        ))
    }

    pub fn finish_document_geometry_for_object_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<RendererDocumentNodeGeometry>> {
        self.finish_document_geometry(completion, "document geometry object id page command")
    }

    pub fn start_document_geometry_for_node(
        &self,
        reference: RendererDomNodeReference,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentGeometryForNode { reference },
        ))
    }

    pub fn start_document_geometry_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_document_geometry_for_node(RendererDomNodeReference::BackendNodeId(
            backend_node_id,
        ))
    }

    pub fn finish_document_geometry_for_backend_node_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<RendererDocumentNodeGeometry>> {
        self.finish_document_geometry_for_node(completion)
    }

    pub fn finish_document_geometry_for_node(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<RendererDocumentNodeGeometry>> {
        self.finish_document_geometry(completion, "document geometry node page command")
    }

    fn finish_document_geometry(
        &mut self,
        completion: CompletedPageCommand,
        operation: &str,
    ) -> Result<Option<RendererDocumentNodeGeometry>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            operation,
            "an optional document node geometry reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalDocumentNodeGeometry(geometry)) => Ok(geometry),
        )
    }

    pub fn start_document_hit_test(
        &self,
        inspector_session_id: Option<String>,
        x: f64,
        y: f64,
        include_user_agent_shadow_dom: bool,
        ignore_pointer_events_none: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentHitTest {
                inspector_session_id,
                x,
                y,
                include_user_agent_shadow_dom,
                ignore_pointer_events_none,
            },
        ))
    }

    pub fn finish_document_hit_test(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<RendererDocumentHitTestResult>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document hit-test page command",
            "an optional document hit-test reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalDocumentHitTest(hit)) => Ok(hit),
        )
    }

    pub fn start_remove_document_node(
        &self,
        reference: RendererDomNodeReference,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::RemoveDocumentNode { reference },
        ))
    }

    pub fn start_remove_document_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_remove_document_node(RendererDomNodeReference::BackendNodeId(backend_node_id))
    }

    pub fn finish_remove_document_node(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<bool> {
        self.finish_remove_document_node_reference(completion)
            .map(|removed| removed.unwrap_or(false))
    }

    pub fn finish_remove_document_node_reference(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<bool>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "remove document node page command",
            "an optional bool reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalBool(value)) => Ok(value),
        )
    }

    pub fn start_mutate_document_node_attribute(
        &self,
        reference: RendererDomNodeReference,
        mutation: RendererDomAttributeMutation,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::MutateDocumentNodeAttribute {
                reference,
                mutation,
            },
        ))
    }

    pub fn start_mutate_document_backend_node_attribute(
        &self,
        backend_node_id: u32,
        mutation: RendererDomAttributeMutation,
    ) -> Result<PendingPageCommand> {
        self.start_mutate_document_node_attribute(
            RendererDomNodeReference::BackendNodeId(backend_node_id),
            mutation,
        )
    }

    pub fn finish_mutate_document_node_attribute(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDomAttributeMutationOutcome> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "mutate document node attribute page command",
            "a DOM attribute mutation outcome reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DomAttributeMutationOutcome(outcome)) => Ok(outcome),
        )
    }

    pub fn start_edit_document_node(
        &self,
        inspector_session_id: Option<String>,
        edit: RendererDomEdit,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::EditDocumentNode {
                inspector_session_id,
                edit,
            },
        ))
    }

    pub fn finish_edit_document_node(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDomEditOutcome> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "edit document node page command",
            "a DOM edit outcome reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DomEditOutcome(outcome)) => Ok(outcome),
        )
    }

    pub fn start_focus_document_node(
        &self,
        reference: RendererDomNodeReference,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::FocusDocumentNode { reference },
        ))
    }

    pub fn start_focus_document_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_focus_document_node(RendererDomNodeReference::BackendNodeId(backend_node_id))
    }

    pub fn finish_focus_document_node_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDomFocusOutcome> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "focus document node page command",
            "a DOM focus outcome reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DomFocusOutcome(outcome)) => Ok(outcome),
        )
    }

    pub fn start_focus_document_node_for_object_id(
        &self,
        inspector_session_id: Option<String>,
        object_id: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::focus_document_node_for_object_id(
            inspector_session_id,
            object_id,
        ))
    }

    pub fn start_autofill_trigger(
        &self,
        request: RendererAutofillTriggerRequest,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::TriggerAutofill(request),
        ))
    }

    pub fn finish_autofill_trigger(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererAutofillTriggerOutcome> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "autofill trigger page command",
            "an Autofill trigger outcome reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::AutofillTriggerOutcome(outcome)) => Ok(outcome),
        )
    }

    pub fn start_set_file_input_files_for_backend_node_id(
        &self,
        backend_node_id: u32,
        files: Vec<crate::page::SelectedFile>,
        append: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::SetFileInputFilesForBackendNodeId {
                backend_node_id,
                files,
                append,
            },
        ))
    }

    pub fn finish_set_file_input_files(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<bool>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set file input files page command",
            "an optional bool reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalBool(value)) => Ok(value),
        )
    }

    pub fn start_set_file_input_files_for_object_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        object_id: &str,
        files: Vec<crate::page::SelectedFile>,
        append: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::set_file_input_files_for_object_id(
            inspector_session_id,
            object_id.to_owned(),
            files,
            append,
        ))
    }

    pub fn finish_set_file_input_files_for_object_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<bool>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set file input files object id page command",
            "an optional bool reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalBool(value)) => Ok(value),
        )
    }

    pub fn start_document_node_snapshot_for_object_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        object_id: &str,
        depth: i32,
        pierce: bool,
        bind_frontend_nodes: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::document_node_snapshot_for_object_id(
            inspector_session_id,
            include_whitespace,
            object_id.to_owned(),
            depth,
            pierce,
            bind_frontend_nodes,
        ))
    }

    pub fn start_request_document_node_for_object_id(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        object_id: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::request_document_node_for_object_id(
            inspector_session_id,
            include_whitespace,
            object_id,
        ))
    }

    pub fn start_document_node_exists_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentNodeExistsForBackendNodeId { backend_node_id },
        ))
    }

    pub fn start_document_node_snapshot_for_backend_node_id(
        &self,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentNodeSnapshotForBackendNodeId {
                backend_node_id,
                depth,
                pierce,
            },
        ))
    }

    pub fn start_document_node_snapshot_for_node_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        reference: RendererDomNodeReference,
        depth: i32,
        pierce: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentNodeSnapshotForNodeInInspectorSession {
                inspector_session_id,
                include_whitespace,
                reference,
                depth,
                pierce,
            },
        ))
    }

    pub fn start_document_node_snapshot_for_backend_node_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<PendingPageCommand> {
        self.start_document_node_snapshot_for_node_in_inspector_session(
            inspector_session_id,
            include_whitespace,
            RendererDomNodeReference::BackendNodeId(backend_node_id),
            depth,
            pierce,
        )
    }

    pub fn start_document_node_snapshot_for_document(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        depth: i32,
        pierce: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentNodeSnapshotForDocument {
                inspector_session_id,
                include_whitespace,
                depth,
                pierce,
            },
        ))
    }

    pub fn start_discard_dom_agent_frontend_bindings(
        &self,
        inspector_session_id: Option<String>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DiscardDomAgentFrontendBindings {
                inspector_session_id,
            },
        ))
    }

    pub fn start_dom_snapshot_capture(
        &self,
        top_frame_id: String,
        options: RendererDomSnapshotCaptureOptions,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DomSnapshotCapture {
                top_frame_id,
                options,
            },
        ))
    }

    pub fn finish_dom_snapshot_capture(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<RendererDomSnapshotCapturePayload>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "DOMSnapshot capture page command",
            "an optional DOMSnapshot capture payload",
            RendererPageReply::DevTools(RendererDevToolsPageReply::OptionalDomSnapshotCapturePayload(payload)) => Ok(payload),
        )
    }

    pub fn start_document_child_node_snapshot_events_for_node(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        reference: RendererDomNodeReference,
        depth: i32,
        pierce: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentChildNodeSnapshotEventsForNode {
                inspector_session_id,
                include_whitespace,
                reference,
                depth,
                pierce,
            },
        ))
    }

    pub fn start_document_child_node_snapshot_events_for_backend_node_id(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<PendingPageCommand> {
        self.start_document_child_node_snapshot_events_for_node(
            inspector_session_id,
            include_whitespace,
            RendererDomNodeReference::BackendNodeId(backend_node_id),
            depth,
            pierce,
        )
    }

    pub fn start_document_query_selector_for_document(
        &self,
        selector: String,
        multiple: bool,
    ) -> Result<PendingPageCommand> {
        self.start_document_query_selector_for_document_in_inspector_session(
            None, false, selector, multiple,
        )
    }

    pub fn start_document_query_selector_for_document_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        selector: String,
        multiple: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentQuerySelectorForDocument {
                inspector_session_id,
                include_whitespace,
                selector,
                multiple,
            },
        ))
    }

    pub fn start_document_query_selector_for_backend_node_id(
        &self,
        root_backend_node_id: u32,
        selector: String,
        multiple: bool,
    ) -> Result<PendingPageCommand> {
        self.start_document_query_selector_for_backend_node_id_in_inspector_session(
            None,
            false,
            root_backend_node_id,
            selector,
            multiple,
        )
    }

    pub fn start_document_query_selector_for_node_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        root: RendererDomNodeReference,
        selector: String,
        multiple: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentQuerySelectorForNode {
                inspector_session_id,
                include_whitespace,
                root,
                selector,
                multiple,
            },
        ))
    }

    pub fn start_document_query_selector_for_backend_node_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        root_backend_node_id: u32,
        selector: String,
        multiple: bool,
    ) -> Result<PendingPageCommand> {
        self.start_document_query_selector_for_node_in_inspector_session(
            inspector_session_id,
            include_whitespace,
            RendererDomNodeReference::BackendNodeId(root_backend_node_id),
            selector,
            multiple,
        )
    }

    pub fn start_child_frame_document_query_selector_for_node(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        frame_id: String,
        root: RendererDomNodeReference,
        selector: String,
        multiple: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentQuerySelectorForChildFrameNode {
                inspector_session_id,
                include_whitespace,
                frame_id,
                root,
                selector,
                multiple,
            },
        ))
    }

    pub fn start_child_frame_document_query_selector_for_backend_node_id(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        frame_id: String,
        root_backend_node_id: u32,
        selector: String,
        multiple: bool,
    ) -> Result<PendingPageCommand> {
        self.start_child_frame_document_query_selector_for_node(
            inspector_session_id,
            include_whitespace,
            frame_id,
            RendererDomNodeReference::BackendNodeId(root_backend_node_id),
            selector,
            multiple,
        )
    }

    pub fn finish_document_query_selector(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDocumentQuerySelectorResolution> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document query selector page command",
            "a document query selector resolution",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentQuerySelectorResolution(resolution)) => Ok(resolution),
        )
    }

    pub fn start_document_query_selector_with_child_node_snapshot_events_for_node(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        root: RendererDomNodeReference,
        selector: String,
        multiple: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentQuerySelectorWithChildNodeSnapshotEventsForNode {
                inspector_session_id,
                include_whitespace,
                root,
                selector,
                multiple,
            },
        ))
    }

    pub fn start_document_query_selector_with_child_node_snapshot_events_for_backend_node_id(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        root_backend_node_id: u32,
        selector: String,
        multiple: bool,
    ) -> Result<PendingPageCommand> {
        self.start_document_query_selector_with_child_node_snapshot_events_for_node(
            inspector_session_id,
            include_whitespace,
            RendererDomNodeReference::BackendNodeId(root_backend_node_id),
            selector,
            multiple,
        )
    }

    pub fn finish_document_query_selector_with_child_node_snapshot_events(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDocumentQuerySelectorWithChildNodeSnapshotEvents> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document query selector with child node snapshot events page command",
            "a document query selector with child node snapshot events reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentQuerySelectorWithChildNodeSnapshotEvents(result)) => Ok(result),
        )
    }

    pub fn start_document_perform_search(
        &self,
        inspector_session_id: Option<String>,
        query: String,
        include_user_agent_shadow_dom: bool,
        include_whitespace: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentPerformSearch {
                inspector_session_id,
                query,
                include_user_agent_shadow_dom,
                include_whitespace,
            },
        ))
    }

    pub fn finish_document_perform_search(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDomSearchRegistration> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document perform search page command",
            "a document search registration reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentPerformSearch(result)) => Ok(result),
        )
    }

    pub fn start_document_search_results(
        &self,
        inspector_session_id: Option<String>,
        search_id: String,
        from_index: usize,
        to_index: usize,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentGetSearchResults {
                inspector_session_id,
                search_id,
                from_index,
                to_index,
            },
        ))
    }

    pub fn finish_document_search_results(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<(
        RendererDomSearchResultsResolution,
        Option<RendererDocumentChildNodeSnapshotEvents>,
    )> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document search results page command",
            "a document search results resolution",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentSearchResults { resolution, child_node_snapshot_events }) => {
                Ok((resolution, child_node_snapshot_events))
            },
        )
    }

    pub fn start_discard_document_search_results(
        &self,
        inspector_session_id: Option<String>,
        search_id: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentDiscardSearchResults {
                inspector_session_id,
                search_id,
            },
        ))
    }

    pub fn finish_discard_document_search_results(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document discard search results page command",
            "a document search results discarded reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentSearchResultsDiscarded) => Ok(()),
        )
    }

    pub fn start_set_document_node_stack_traces_enabled(
        &self,
        inspector_session_id: Option<String>,
        enabled: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentSetNodeStackTracesEnabled {
                inspector_session_id,
                enabled,
            },
        ))
    }

    pub fn finish_set_document_node_stack_traces_enabled(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set document node stack traces enabled page command",
            "a document node stack traces enabled reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentNodeStackTracesEnabled) => Ok(()),
        )
    }

    pub fn start_document_node_stack_trace(
        &self,
        inspector_session_id: Option<String>,
        frontend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentNodeStackTrace {
                inspector_session_id,
                frontend_node_id,
            },
        ))
    }

    pub fn finish_document_node_stack_trace(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDomNodeStackTraceResolution> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document node stack trace page command",
            "a document node stack trace resolution",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentNodeStackTrace(result)) => Ok(result),
        )
    }

    pub fn start_document_frontend_node_binding(
        &self,
        inspector_session_id: Option<String>,
        frontend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentFrontendNodeBinding {
                reference: RendererDomNodeReference::FrontendNodeId {
                    inspector_session_id,
                    frontend_node_id,
                },
            },
        ))
    }

    /// Resolve an internal automation binding without treating it as a CDP input.
    pub fn start_document_internal_frontend_node_binding(
        &self,
        inspector_session_id: Option<String>,
        frontend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentFrontendNodeBinding {
                reference: RendererDomNodeReference::InternalFrontendNodeBinding {
                    inspector_session_id,
                    frontend_node_id,
                },
            },
        ))
    }

    pub fn finish_document_frontend_node_binding(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDomFrontendNodeBindingResolution> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document frontend node binding page command",
            "a document frontend node binding resolution",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentFrontendNodeBinding(result)) => Ok(result),
        )
    }

    pub fn start_register_document_bidi_node_binding(
        &self,
        inspector_session_id: Option<String>,
        shared_id: String,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::RegisterDocumentBidiNodeBinding {
                inspector_session_id,
                shared_id,
                backend_node_id,
            },
        ))
    }

    pub fn finish_register_document_bidi_node_binding(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document BiDi node binding registration page command",
            "a document BiDi node binding registered reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentBidiNodeBindingRegistered) => Ok(()),
        )
    }

    pub fn start_document_bidi_node_binding(
        &self,
        inspector_session_id: Option<String>,
        shared_id: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentBidiNodeBinding {
                inspector_session_id,
                shared_id,
            },
        ))
    }

    pub fn finish_document_bidi_node_binding(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDomBidiNodeBindingResolution> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document BiDi node binding page command",
            "a document BiDi node binding resolution",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentBidiNodeBinding(result)) => Ok(result),
        )
    }

    pub fn start_document_bidi_node_shared_id_for_backend_node_id(
        &self,
        inspector_session_id: Option<String>,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentBidiNodeSharedIdForBackendNodeId {
                inspector_session_id,
                backend_node_id,
            },
        ))
    }

    pub fn finish_document_bidi_node_shared_id_for_backend_node_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDomBidiNodeSharedIdResolution> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document BiDi node shared id page command",
            "a document BiDi node shared id resolution",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentBidiNodeSharedId(result)) => Ok(result),
        )
    }

    /// Resolve the native reference and read attributes in one renderer turn.
    pub fn start_document_node_attributes(
        &self,
        reference: RendererDomNodeReference,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentNodeAttributes { reference },
        ))
    }

    pub fn start_document_node_attributes_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_document_node_attributes(RendererDomNodeReference::BackendNodeId(
            backend_node_id,
        ))
    }

    pub fn finish_document_node_attributes(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDocumentNodeAttributesResolution> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document node attributes page command",
            "a document node attributes resolution",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentNodeAttributesResolution(resolution)) => Ok(resolution),
        )
    }

    pub fn start_document_node_text(
        &self,
        reference: RendererDomNodeReference,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentNodeText { reference },
        ))
    }

    pub fn start_document_node_text_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_document_node_text(RendererDomNodeReference::BackendNodeId(backend_node_id))
    }

    pub fn finish_document_node_text(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDocumentNodeTextResolution> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document node text page command",
            "a document node text resolution",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentNodeTextResolution(resolution)) => Ok(resolution),
        )
    }

    pub fn start_document_node_property(
        &self,
        reference: RendererDomNodeReference,
        name: &str,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentNodeProperty {
                reference,
                name: name.to_owned(),
            },
        ))
    }

    pub fn start_document_node_property_for_backend_node_id(
        &self,
        backend_node_id: u32,
        name: &str,
    ) -> Result<PendingPageCommand> {
        self.start_document_node_property(
            RendererDomNodeReference::BackendNodeId(backend_node_id),
            name,
        )
    }

    pub fn finish_document_node_property(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDocumentNodePropertyResolution> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document node property page command",
            "a document node property resolution",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentNodePropertyResolution(resolution)) => Ok(resolution),
        )
    }

    pub fn start_outer_html_for_object_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        object_id: &str,
        include_shadow_dom: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::outer_html_for_object_id(
            inspector_session_id,
            object_id.to_owned(),
            include_shadow_dom,
        ))
    }

    pub fn start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        backend_node_id: u32,
        execution_context_id: Option<i64>,
        object_group: Option<&str>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::resolve_runtime_object_for_backend_node_id(
                inspector_session_id,
                backend_node_id,
                execution_context_id,
                object_group.map(str::to_owned),
            ),
        )
    }

    pub fn start_resolve_blob_object_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        object_id: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::resolve_blob_object(
            inspector_session_id,
            object_id,
        ))
    }

    pub fn start_document_frontend_node_ids_for_backend_node_ids(
        &self,
        inspector_session_id: Option<String>,
        backend_node_ids: Vec<u32>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::DocumentFrontendNodeIdsForBackendNodeIds {
                inspector_session_id,
                backend_node_ids,
            },
        ))
    }

    pub fn finish_document_frontend_node_ids_for_backend_node_ids(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDocumentFrontendNodeIdsResolution> {
        expect_page_reply!(
            self.finish_page_command(completion),
            "document frontend node ids for backend node ids page command",
            "frontend node ids resolution reply",
            RendererPageReply::DevTools(RendererDevToolsPageReply::DocumentFrontendNodeIds(resolution)) => Ok(resolution),
        )
    }

    pub async fn child_frame_owner_node_reference_async(
        &mut self,
        frame_id: &str,
        inspector_session_id: Option<String>,
    ) -> Result<Option<RendererDocumentNodeReference>> {
        let pending =
            self.start_child_frame_owner_node_reference(frame_id, inspector_session_id)?;
        let completion = pending.wait().await?;
        self.finish_document_node_reference(completion)
    }

    pub fn start_child_frame_owner_node_reference(
        &self,
        frame_id: &str,
        inspector_session_id: Option<String>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::ChildFrameOwnerNodeReference {
                inspector_session_id,
                frame_id: frame_id.to_owned(),
            },
        ))
    }

    pub async fn child_frame_document_root_node_reference_async(
        &mut self,
        frame_id: &str,
        inspector_session_id: Option<String>,
    ) -> Result<Option<RendererDocumentNodeReference>> {
        let pending =
            self.start_child_frame_document_root_node_reference(frame_id, inspector_session_id)?;
        let completion = pending.wait().await?;
        self.finish_document_node_reference(completion)
    }

    pub fn start_child_frame_document_root_node_reference(
        &self,
        frame_id: &str,
        inspector_session_id: Option<String>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DevTools(
            RendererDevToolsPageCommand::ChildFrameDocumentRootNodeReference {
                inspector_session_id,
                frame_id: frame_id.to_owned(),
            },
        ))
    }
}
