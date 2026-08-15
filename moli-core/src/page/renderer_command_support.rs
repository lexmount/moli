use std::sync::Arc;

use anyhow::Result;
use url::Url;

use super::dom_protocol_support::DocumentNodeObjectSnapshot;
use super::protocol_support::{
    ChildFrameTreeSnapshot, ScriptExecutionReport, ScriptNetworkOutputItem,
    ScriptObservableOutputItem, SubresourceNetworkRecord, WebSocketLifecycleEvent,
    WebSocketNetworkEvent,
};
use super::{CompletedPageCommand, Page, PendingPageCommand};
use crate::renderer::{
    RendererAutofillTriggerOutcome, RendererAutofillTriggerRequest, RendererCaptureScreenshotReply,
    RendererCaptureScreenshotRequest, RendererDocumentChildNodeSnapshotEvents,
    RendererDocumentFrontendNodeIdsResolution, RendererDocumentHitTestResult,
    RendererDocumentNodeAttributesResolution, RendererDocumentNodeClientRect,
    RendererDocumentNodeGeometry, RendererDocumentNodePropertyResolution,
    RendererDocumentNodeReference, RendererDocumentNodeTextResolution,
    RendererDocumentQuerySelectorResolution,
    RendererDocumentQuerySelectorWithChildNodeSnapshotEvents, RendererDomAttributeMutation,
    RendererDomAttributeMutationOutcome, RendererDomBidiNodeBindingResolution,
    RendererDomBidiNodeSharedIdResolution, RendererDomEdit, RendererDomEditOutcome,
    RendererDomFocusOutcome, RendererDomFrontendNodeBindingResolution,
    RendererDomNodeStackTraceResolution, RendererDomSearchRegistration,
    RendererDomSearchResultsResolution, RendererDomSnapshotCaptureOptions,
    RendererDomSnapshotCapturePayload, RendererLayoutMetrics, RendererPageCommand,
    RendererPageDumpOptions, RendererPageReply, RendererPageState, RendererRuntimeRemoteObject,
    RendererStyleSheetInventoryUpdate, RendererStyleSheetPayload,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct PageNetworkOutputUpdate<'a> {
    network_output_items: &'a [ScriptNetworkOutputItem],
}

#[derive(Debug)]
pub struct PageObservableOutputUpdate<'a> {
    observable_output_items: &'a [ScriptObservableOutputItem],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestingOutcome {
    pub(super) observations: usize,
    pub(super) harness_failures: Vec<String>,
    pub(super) pending_async: usize,
    pub(super) script_failures: Vec<String>,
    pub(super) lifecycle_errors: Vec<String>,
}

pub enum DocumentNodeRuntimeObjectResolution {
    Found(RendererRuntimeRemoteObject),
    MissingContext,
    MissingNode,
}

pub enum DocumentNodeClientRectResolution {
    Found(super::ClientRect),
    FoundNonElement(super::ClientRect),
    NotElement,
}

// ---------------------------------------------------------------------------
// PageNetworkOutputUpdate / PageObservableOutputUpdate / TestingOutcome impls
// ---------------------------------------------------------------------------

impl<'a> PageNetworkOutputUpdate<'a> {
    pub fn append(network_output_items: &'a [ScriptNetworkOutputItem]) -> Self {
        Self {
            network_output_items,
        }
    }

    pub fn network_output_items(&self) -> &'a [ScriptNetworkOutputItem] {
        self.network_output_items
    }
}

impl<'a> PageObservableOutputUpdate<'a> {
    pub fn append(observable_output_items: &'a [ScriptObservableOutputItem]) -> Self {
        Self {
            observable_output_items,
        }
    }

    pub fn observable_output_items(&self) -> &'a [ScriptObservableOutputItem] {
        self.observable_output_items
    }
}

impl TestingOutcome {
    pub fn observations(&self) -> usize {
        self.observations
    }

    pub fn harness_failures(&self) -> &[String] {
        &self.harness_failures
    }

    pub fn pending_async(&self) -> usize {
        self.pending_async
    }

    pub fn script_failures(&self) -> &[String] {
        &self.script_failures
    }

    pub fn lifecycle_errors(&self) -> &[String] {
        &self.lifecycle_errors
    }

    pub fn passed(&self) -> bool {
        self.observations > 0
            && self.pending_async == 0
            && self.harness_failures.is_empty()
            && self.script_failures.is_empty()
            && self.lifecycle_errors.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Page state accessor methods
// ---------------------------------------------------------------------------

impl Page {
    pub fn requested_url(&self) -> &Url {
        self.page_state.requested_url()
    }

    pub fn page_id(&self) -> u64 {
        self.handle.page_id()
    }

    pub fn renderer_page_id(&self) -> moli_renderer_v8::PageId {
        self.handle.renderer_page_id()
    }

    pub fn renderer_owner_local_host_id(&self) -> moli_renderer_v8::RendererOwnerLocalHostId {
        self.handle.owner_local_host_id()
    }

    pub fn service_worker_client_id(&self) -> u64 {
        self.page_state.state().service_worker_client_id
    }

    pub fn dedicated_worker_running_worker_isolate_count_for_diagnostics(&self) -> usize {
        self.page_state
            .state()
            .dedicated_worker_running_worker_isolate_count
    }

    pub fn navigation_initiator_url(&self) -> Option<&Url> {
        self.page_state.navigation_initiator_url()
    }

    pub fn navigation_redirected(&self) -> bool {
        self.page_state.navigation_redirected()
    }

    pub fn navigation_redirect_count(&self) -> usize {
        self.page_state.navigation_redirect_count()
    }

    pub fn final_url(&self) -> &Url {
        self.page_state.final_url()
    }

    pub fn idle_override(&self) -> Option<super::EmulatedIdleOverride> {
        self.page_state.state().idle_override()
    }

    pub fn status(&self) -> u16 {
        self.page_state.status()
    }

    pub fn headers(&self) -> &[(String, String)] {
        self.page_state.headers()
    }

    pub fn script_execution(&self) -> &ScriptExecutionReport {
        self.page_state.script_execution()
    }

    /// Refreshes the complete script report on the renderer owner lane.
    ///
    /// Protocol command completion keeps observable/network state current but
    /// intentionally marks an enabled own-globals projection dirty. In a
    /// `test-support` build, call this before reading `fresh_globals()` when a
    /// current diagnostic snapshot is required. Normal builds do not capture
    /// a realm baseline, so this leaves globals `Uncaptured`.
    pub async fn refresh_script_execution_report_async(&mut self) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::RefreshFullPageState,
            "refresh script execution report",
        )
        .await
    }

    pub fn subresource_network_records(&self) -> &[SubresourceNetworkRecord] {
        self.page_state
            .script_execution()
            .subresource_network_records()
    }

    pub fn websocket_network_events(&self) -> &[WebSocketNetworkEvent] {
        self.page_state
            .script_execution()
            .websocket_network_events()
    }

    pub fn websocket_lifecycle_events(&self) -> &[WebSocketLifecycleEvent] {
        self.page_state
            .script_execution()
            .websocket_lifecycle_events()
    }

    pub fn network_output_counts(&self) -> (usize, usize) {
        (
            self.subresource_network_records().len(),
            self.websocket_network_events().len(),
        )
    }

    pub fn take_network_output_update(&mut self) -> PageNetworkOutputUpdate<'_> {
        let network_output_items = self.page_state.script_execution().network_output_items();
        PageNetworkOutputUpdate::append(network_output_items)
    }

    pub fn take_observable_output_update(&mut self) -> PageObservableOutputUpdate<'_> {
        let observable_output_items = self.page_state.script_execution().observable_output_items();
        PageObservableOutputUpdate::append(observable_output_items)
    }

    pub(super) fn replace_page_state(&mut self, page_state: Arc<RendererPageState>) {
        self.page_state.replace(page_state);
    }

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
        self.start_page_command(
            RendererPageCommand::SetInlineStyleSheetTextForStyleSheetId {
                inspector_session_id,
                style_sheet_id: style_sheet_id.to_owned(),
                text: text.to_owned(),
            },
        )
    }

    pub fn finish_set_inline_style_sheet_text(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<bool> {
        Self::decode_bool_page_reply(
            self.finish_page_command(completion),
            "set inline stylesheet text page command",
        )
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
        self.start_page_command(RendererPageCommand::StyleSheetPayloadForStyleSheetId {
            inspector_session_id,
            style_sheet_id: style_sheet_id.to_owned(),
        })
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
            RendererPageReply::OptionalStyleSheetPayload(payload) => Ok(payload),
        )
    }

    pub fn start_style_sheet_inventory_for_document(&self) -> Result<PendingPageCommand> {
        self.start_style_sheet_inventory_for_document_and_inspector_session(None)
    }

    pub fn start_style_sheet_inventory_for_document_and_inspector_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::StyleSheetInventoryForDocument {
            inspector_session_id,
        })
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
            RendererPageReply::StyleSheetInventory(update) => Ok(update),
        )
    }

    pub fn start_reset_css_agent_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::ResetCssAgentSession {
            inspector_session_id,
        })
    }

    pub fn finish_reset_css_agent_session(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "CSS agent reset page command",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub fn start_computed_style_properties_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::ComputedStylePropertiesForBackendNodeId { backend_node_id },
        )
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
            RendererPageReply::ComputedStyleProperties(properties) => Ok(properties),
        )
    }

    pub fn document_title(&self) -> String {
        self.page_state.document_title().to_owned()
    }

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

    pub fn start_scroll_backend_node_into_view_if_needed(
        &self,
        backend_node_id: u32,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::ScrollBackendNodeIntoViewIfNeeded {
            backend_node_id,
            rect,
        })
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
            RendererPageReply::OptionalBool(has_geometry) => Ok(has_geometry),
        )
    }

    pub fn start_node_has_geometry_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::NodeHasGeometryForBackendNodeId {
            backend_node_id,
        })
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
            RendererPageReply::OptionalBool(has_geometry) => Ok(has_geometry),
        )
    }

    pub fn finish_scroll_node_into_view_if_needed(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<super::RendererScrollIntoViewResult> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "scroll document node into view page command",
            "a scroll-into-view reply",
            RendererPageReply::ScrollIntoViewResult(result) => Ok(result),
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
            RendererPageReply::OptionalDocumentNodeClientRect(rect) => Ok(rect.map(|rect| match rect {
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
        self.start_page_command(RendererPageCommand::ClientRectForBackendNodeId { backend_node_id })
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
            RendererPageReply::OptionalDocumentNodeClientRect(rect) => Ok(rect.map(|rect| match rect {
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

    pub fn start_document_geometry_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentGeometryForBackendNodeId {
            backend_node_id,
        })
    }

    pub fn finish_document_geometry_for_backend_node_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<RendererDocumentNodeGeometry>> {
        self.finish_document_geometry(completion, "document geometry backend node id page command")
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
            RendererPageReply::OptionalDocumentNodeGeometry(geometry) => Ok(geometry),
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
        self.start_page_command(RendererPageCommand::DocumentHitTest {
            inspector_session_id,
            x,
            y,
            include_user_agent_shadow_dom,
            ignore_pointer_events_none,
        })
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
            RendererPageReply::OptionalDocumentHitTest(hit) => Ok(hit),
        )
    }

    pub fn start_remove_document_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::RemoveDocumentBackendNodeId {
            backend_node_id,
        })
    }

    pub fn finish_remove_document_node(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<bool> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "remove document node page command",
            "a bool reply",
            RendererPageReply::Bool(value) => Ok(value),
        )
    }

    pub fn start_mutate_document_backend_node_attribute(
        &self,
        backend_node_id: u32,
        mutation: RendererDomAttributeMutation,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::MutateDocumentBackendNodeAttribute {
            backend_node_id,
            mutation,
        })
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
            RendererPageReply::DomAttributeMutationOutcome(outcome) => Ok(outcome),
        )
    }

    pub fn start_edit_document_node(
        &self,
        inspector_session_id: Option<String>,
        edit: RendererDomEdit,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::EditDocumentNode {
            inspector_session_id,
            edit,
        })
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
            RendererPageReply::DomEditOutcome(outcome) => Ok(outcome),
        )
    }

    pub fn start_focus_document_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::FocusDocumentBackendNode { backend_node_id })
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
            RendererPageReply::DomFocusOutcome(outcome) => Ok(outcome),
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
        self.start_page_command(RendererPageCommand::TriggerAutofill(request))
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
            RendererPageReply::AutofillTriggerOutcome(outcome) => Ok(outcome),
        )
    }

    pub fn start_set_file_input_files_for_backend_node_id(
        &self,
        backend_node_id: u32,
        files: Vec<super::SelectedFile>,
        append: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetFileInputFilesForBackendNodeId {
            backend_node_id,
            files,
            append,
        })
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
            RendererPageReply::OptionalBool(value) => Ok(value),
        )
    }

    pub fn start_set_file_input_files_for_object_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        object_id: &str,
        files: Vec<super::SelectedFile>,
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
            RendererPageReply::OptionalBool(value) => Ok(value),
        )
    }

    pub fn start_document_node_snapshot_for_object_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        object_id: &str,
        depth: i32,
        pierce: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::document_node_snapshot_for_object_id(
            inspector_session_id,
            include_whitespace,
            object_id.to_owned(),
            depth,
            pierce,
        ))
    }

    pub fn finish_document_node_snapshot_for_object_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<DocumentNodeObjectSnapshot>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "describe node object id page command",
            "an optional document node object snapshot reply",
            RendererPageReply::OptionalDocumentNodeObjectSnapshot(snapshot) => Ok(*snapshot),
        )
    }

    pub fn start_document_node_snapshot_for_backend_node_id(
        &self,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentNodeSnapshotForBackendNodeId {
            backend_node_id,
            depth,
            pierce,
        })
    }

    pub fn start_document_node_snapshot_for_backend_node_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::DocumentNodeSnapshotForBackendNodeIdInInspectorSession {
                inspector_session_id,
                include_whitespace,
                backend_node_id,
                depth,
                pierce,
            },
        )
    }

    pub fn finish_document_node_snapshot_for_backend_node_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<DocumentNodeObjectSnapshot>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "backend document node snapshot page command",
            "an optional document node object snapshot reply",
            RendererPageReply::OptionalDocumentNodeObjectSnapshot(snapshot) => Ok(*snapshot),
        )
    }

    pub fn start_document_node_snapshot_for_document(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        depth: i32,
        pierce: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentNodeSnapshotForDocument {
            inspector_session_id,
            include_whitespace,
            depth,
            pierce,
        })
    }

    pub fn finish_document_node_snapshot_for_document(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<DocumentNodeObjectSnapshot>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document snapshot page command",
            "an optional document node object snapshot reply",
            RendererPageReply::OptionalDocumentNodeObjectSnapshot(snapshot) => Ok(*snapshot),
        )
    }

    pub fn start_discard_dom_agent_frontend_bindings(
        &self,
        inspector_session_id: Option<String>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DiscardDomAgentFrontendBindings {
            inspector_session_id,
        })
    }

    pub fn finish_discard_dom_agent_frontend_bindings(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "discard DOM agent frontend bindings page command",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub fn start_dom_snapshot_capture(
        &self,
        top_frame_id: String,
        options: RendererDomSnapshotCaptureOptions,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DomSnapshotCapture {
            top_frame_id,
            options,
        })
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
            RendererPageReply::OptionalDomSnapshotCapturePayload(payload) => Ok(payload),
        )
    }

    pub fn start_document_child_node_snapshot_events_for_backend_node_id(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::DocumentChildNodeSnapshotEventsForBackendNodeId {
                inspector_session_id,
                include_whitespace,
                backend_node_id,
                depth,
                pierce,
            },
        )
    }

    pub fn finish_document_child_node_snapshot_events(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<RendererDocumentChildNodeSnapshotEvents>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document child node snapshot events page command",
            "optional document child node snapshot events",
            RendererPageReply::OptionalDocumentChildNodeSnapshotEvents(events) => Ok(events),
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
        self.start_page_command(RendererPageCommand::DocumentQuerySelectorForDocument {
            inspector_session_id,
            include_whitespace,
            selector,
            multiple,
        })
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

    pub fn start_document_query_selector_for_backend_node_id_in_inspector_session(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        root_backend_node_id: u32,
        selector: String,
        multiple: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentQuerySelectorForBackendNodeId {
            inspector_session_id,
            include_whitespace,
            root_backend_node_id,
            selector,
            multiple,
        })
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
        self.start_page_command(
            RendererPageCommand::DocumentQuerySelectorForChildFrameBackendNodeId {
                inspector_session_id,
                include_whitespace,
                frame_id,
                root_backend_node_id,
                selector,
                multiple,
            },
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
            RendererPageReply::DocumentQuerySelectorResolution(resolution) => Ok(resolution),
        )
    }

    pub fn start_document_query_selector_with_child_node_snapshot_events_for_backend_node_id(
        &self,
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        root_backend_node_id: u32,
        selector: String,
        multiple: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::DocumentQuerySelectorWithChildNodeSnapshotEventsForBackendNodeId {
                inspector_session_id,
                include_whitespace,
                root_backend_node_id,
                selector,
                multiple,
            },
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
            RendererPageReply::DocumentQuerySelectorWithChildNodeSnapshotEvents(result) => Ok(result),
        )
    }

    pub fn start_document_perform_search(
        &self,
        inspector_session_id: Option<String>,
        query: String,
        include_user_agent_shadow_dom: bool,
        include_whitespace: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentPerformSearch {
            inspector_session_id,
            query,
            include_user_agent_shadow_dom,
            include_whitespace,
        })
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
            RendererPageReply::DocumentPerformSearch(result) => Ok(result),
        )
    }

    pub fn start_document_search_results(
        &self,
        inspector_session_id: Option<String>,
        search_id: String,
        from_index: usize,
        to_index: usize,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentGetSearchResults {
            inspector_session_id,
            search_id,
            from_index,
            to_index,
        })
    }

    pub fn finish_document_search_results(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDomSearchResultsResolution> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "document search results page command",
            "a document search results resolution",
            RendererPageReply::DocumentSearchResults(result) => Ok(result),
        )
    }

    pub fn start_discard_document_search_results(
        &self,
        inspector_session_id: Option<String>,
        search_id: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentDiscardSearchResults {
            inspector_session_id,
            search_id,
        })
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
            RendererPageReply::DocumentSearchResultsDiscarded => Ok(()),
        )
    }

    pub fn start_set_document_node_stack_traces_enabled(
        &self,
        inspector_session_id: Option<String>,
        enabled: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentSetNodeStackTracesEnabled {
            inspector_session_id,
            enabled,
        })
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
            RendererPageReply::DocumentNodeStackTracesEnabled => Ok(()),
        )
    }

    pub fn start_document_node_stack_trace(
        &self,
        inspector_session_id: Option<String>,
        frontend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentNodeStackTrace {
            inspector_session_id,
            frontend_node_id,
        })
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
            RendererPageReply::DocumentNodeStackTrace(result) => Ok(result),
        )
    }

    pub fn start_document_frontend_node_binding(
        &self,
        inspector_session_id: Option<String>,
        frontend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentFrontendNodeBinding {
            inspector_session_id,
            frontend_node_id,
        })
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
            RendererPageReply::DocumentFrontendNodeBinding(result) => Ok(result),
        )
    }

    pub fn start_register_document_bidi_node_binding(
        &self,
        inspector_session_id: Option<String>,
        shared_id: String,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::RegisterDocumentBidiNodeBinding {
            inspector_session_id,
            shared_id,
            backend_node_id,
        })
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
            RendererPageReply::DocumentBidiNodeBindingRegistered => Ok(()),
        )
    }

    pub fn start_document_bidi_node_binding(
        &self,
        inspector_session_id: Option<String>,
        shared_id: String,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentBidiNodeBinding {
            inspector_session_id,
            shared_id,
        })
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
            RendererPageReply::DocumentBidiNodeBinding(result) => Ok(result),
        )
    }

    pub fn start_document_bidi_node_shared_id_for_backend_node_id(
        &self,
        inspector_session_id: Option<String>,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::DocumentBidiNodeSharedIdForBackendNodeId {
                inspector_session_id,
                backend_node_id,
            },
        )
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
            RendererPageReply::DocumentBidiNodeSharedId(result) => Ok(result),
        )
    }

    pub fn start_document_node_attributes_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::DocumentNodeAttributesForBackendNodeId { backend_node_id },
        )
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
            RendererPageReply::DocumentNodeAttributesResolution(resolution) => Ok(resolution),
        )
    }

    pub fn start_document_node_text_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentNodeTextForBackendNodeId {
            backend_node_id,
        })
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
            RendererPageReply::DocumentNodeTextResolution(resolution) => Ok(resolution),
        )
    }

    pub fn start_document_node_property_for_backend_node_id(
        &self,
        backend_node_id: u32,
        name: &str,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentNodePropertyForBackendNodeId {
            backend_node_id,
            name: name.to_owned(),
        })
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
            RendererPageReply::DocumentNodePropertyResolution(resolution) => Ok(resolution),
        )
    }

    pub fn start_outer_html_for_document(
        &self,
        include_shadow_dom: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::OuterHtmlForDocument { include_shadow_dom })
    }

    pub fn finish_outer_html_for_document(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<String> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "outerHTML document page command",
            "a string reply",
            RendererPageReply::OptionalString(Some(outer_html)) => Ok(outer_html),
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

    pub fn finish_outer_html_for_object_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<String>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "outerHTML object id page command",
            "an optional string reply",
            RendererPageReply::OptionalString(outer_html) => Ok(outer_html),
        )
    }

    pub fn start_outer_html_for_backend_node_id(
        &self,
        backend_node_id: u32,
        include_shadow_dom: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::OuterHtmlForBackendNodeId {
            backend_node_id,
            include_shadow_dom,
        })
    }

    pub fn finish_outer_html_for_backend_node_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<String>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "outerHTML backend node id page command",
            "an optional string reply",
            RendererPageReply::OptionalString(outer_html) => Ok(outer_html),
        )
    }

    pub fn start_serialize_html(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SerializeHtml)
    }

    pub fn start_layout_metrics(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::LayoutMetrics)
    }

    pub fn finish_layout_metrics(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererLayoutMetrics> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "layout metrics page command",
            "a layout metrics reply",
            RendererPageReply::LayoutMetrics(metrics) => Ok(metrics),
        )
    }

    pub fn start_capture_screenshot(&self) -> Result<PendingPageCommand> {
        self.start_capture_screenshot_with_request(RendererCaptureScreenshotRequest::viewport_png())
    }

    pub fn start_capture_screenshot_with_request(
        &self,
        request: RendererCaptureScreenshotRequest,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::CaptureScreenshot(request))
    }

    pub fn finish_capture_screenshot(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererCaptureScreenshotReply> {
        anyhow::ensure!(
            completion.renderer_agent_attachment_id() == self.renderer_agent_attachment_id(),
            "capture screenshot completed for a stale renderer attachment"
        );
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "capture screenshot page command",
            "a capture screenshot reply",
            RendererPageReply::CaptureScreenshot(result) => Ok(result),
        )
    }

    pub fn start_render_page_dump(
        &self,
        options: RendererPageDumpOptions,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::RenderPageDump { options })
    }

    pub fn finish_render_page_dump(&mut self, completion: CompletedPageCommand) -> Result<String> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "render page dump page command",
            "a string reply",
            RendererPageReply::OptionalString(Some(rendered)) => Ok(rendered),
        )
    }

    pub async fn render_page_dump_async(
        &mut self,
        options: RendererPageDumpOptions,
    ) -> Result<String> {
        let pending = self.start_render_page_dump(options)?;
        let completion = pending.wait().await?;
        self.finish_render_page_dump(completion)
    }

    pub fn finish_serialize_html(&mut self, completion: CompletedPageCommand) -> Result<String> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "serialize HTML page command",
            "a string reply",
            RendererPageReply::OptionalString(Some(html)) => Ok(html),
        )
    }

    pub async fn serialize_html_async(&self) -> Result<String> {
        let pending = self.start_serialize_html()?;
        let completion = pending.wait().await?;
        let (completion, _renderer_output_predecessor) =
            completion.into_output().into_completion_and_predecessor();
        let (reply, _, _) = completion.into_parts();
        expect_page_reply!(
            reply,
            "serialize HTML page command",
            "a string reply",
            RendererPageReply::OptionalString(Some(html)) => Ok(html),
        )
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

    pub fn finish_resolve_runtime_object_for_backend_node_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<DocumentNodeRuntimeObjectResolution> {
        self.finish_runtime_remote_object_resolution(
            completion,
            "resolve backend node runtime object page command",
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

    pub fn finish_resolve_blob_object(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<String> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "resolve Blob object page command",
            "a Blob UUID reply",
            RendererPageReply::BlobUuid(uuid) => Ok(uuid),
        )
    }

    pub fn start_blob_bytes_for_uuid(&self, uuid: String) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::BlobBytesForUuid { uuid })
    }

    pub fn finish_blob_bytes_for_uuid(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<Arc<[u8]>>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "read Blob backing page command",
            "optional Blob bytes",
            RendererPageReply::OptionalBlobBytes(bytes) => Ok(bytes),
        )
    }

    fn finish_runtime_remote_object_resolution(
        &mut self,
        completion: CompletedPageCommand,
        command_name: &'static str,
    ) -> Result<DocumentNodeRuntimeObjectResolution> {
        let reply = self.finish_page_command(completion);
        let resolution = expect_page_reply!(
            reply,
            command_name,
            "a runtime remote object resolution reply",
            RendererPageReply::RuntimeRemoteObjectResolution(resolution) => Ok(resolution),
        )?;
        match resolution {
            crate::renderer::RendererRuntimeRemoteObjectResolution::Found(remote_object) => {
                Ok(DocumentNodeRuntimeObjectResolution::Found(remote_object))
            }
            crate::renderer::RendererRuntimeRemoteObjectResolution::MissingContext => {
                Ok(DocumentNodeRuntimeObjectResolution::MissingContext)
            }
            crate::renderer::RendererRuntimeRemoteObjectResolution::MissingNode => {
                Ok(DocumentNodeRuntimeObjectResolution::MissingNode)
            }
        }
    }

    pub fn start_document_frontend_node_ids_for_backend_node_ids(
        &self,
        inspector_session_id: Option<String>,
        backend_node_ids: Vec<u32>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(
            RendererPageCommand::DocumentFrontendNodeIdsForBackendNodeIds {
                inspector_session_id,
                backend_node_ids,
            },
        )
    }

    pub fn finish_document_frontend_node_ids_for_backend_node_ids(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererDocumentFrontendNodeIdsResolution> {
        expect_page_reply!(
            self.finish_page_command(completion),
            "document frontend node ids for backend node ids page command",
            "frontend node ids resolution reply",
            RendererPageReply::DocumentFrontendNodeIds(resolution) => Ok(resolution),
        )
    }

    pub async fn child_frame_tree_snapshot_async(&mut self) -> Result<Vec<ChildFrameTreeSnapshot>> {
        let pending = self.start_child_frame_tree_snapshot()?;
        let completion = pending.wait().await?;
        self.finish_child_frame_tree_snapshot(completion)
    }

    pub async fn document_storage_key_snapshot_async(&mut self) -> Result<String> {
        let pending = self.start_document_storage_key_snapshot()?;
        self.finish_document_storage_key_snapshot(pending.wait().await?)
    }

    pub fn start_document_storage_key_snapshot(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentStorageKeySnapshot)
    }

    pub fn finish_document_storage_key_snapshot(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<String> {
        expect_page_reply!(
            self.finish_page_command(completion),
            "document storage key snapshot page command",
            "a document storage key reply",
            RendererPageReply::DocumentStorageKey(storage_key) => Ok(storage_key),
        )
    }

    pub fn start_child_frame_tree_snapshot(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::ChildFrameTreeSnapshot)
    }

    pub fn finish_child_frame_tree_snapshot(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Vec<ChildFrameTreeSnapshot>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "child frame tree snapshot page command",
            "child frame tree snapshots",
            RendererPageReply::ChildFrameTreeSnapshots(snapshots) => Ok(snapshots),
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
        self.start_page_command(RendererPageCommand::ChildFrameOwnerNodeReference {
            inspector_session_id,
            frame_id: frame_id.to_owned(),
        })
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
        self.start_page_command(RendererPageCommand::ChildFrameDocumentRootNodeReference {
            inspector_session_id,
            frame_id: frame_id.to_owned(),
        })
    }

    pub fn finish_document_node_reference(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<RendererDocumentNodeReference>> {
        expect_page_reply!(
            self.finish_page_command(completion),
            "document node reference page command",
            "an optional document node reference reply",
            RendererPageReply::OptionalDocumentNodeReference(reference) => Ok(reference),
        )
    }
}
