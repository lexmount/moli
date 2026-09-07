use std::sync::Arc;

use anyhow::Result;
use url::Url;

use super::dom_protocol_support::DocumentNodeObjectSnapshot;
use super::protocol_support::{
    ChildFrameTreeSnapshot, ScriptExecutionReport, SubresourceNetworkRecord,
    WebSocketLifecycleEvent, WebSocketNetworkEvent,
};
use super::{CompletedPageCommand, Page, PendingPageCommand};
use crate::renderer::{
    RendererAutofillTriggerOutcome, RendererAutofillTriggerRequest,
    RendererCaptureScreencastFrameReply, RendererCaptureScreencastFrameRequest,
    RendererCaptureScreenshotReply, RendererCaptureScreenshotRequest,
    RendererDocumentChildNodeSnapshotEvents, RendererDocumentFrontendNodeIdsResolution,
    RendererDocumentHitTestResult, RendererDocumentNodeAttributesResolution,
    RendererDocumentNodeClientRect, RendererDocumentNodeGeometry,
    RendererDocumentNodePropertyResolution, RendererDocumentNodeReference,
    RendererDocumentNodeTextResolution, RendererDocumentQuerySelectorResolution,
    RendererDocumentQuerySelectorWithChildNodeSnapshotEvents, RendererDomAttributeMutationOutcome,
    RendererDomBidiNodeBindingResolution, RendererDomBidiNodeSharedIdResolution,
    RendererDomEditOutcome, RendererDomFocusOutcome, RendererDomFrontendNodeBindingResolution,
    RendererDomNodeStackTraceResolution, RendererDomSearchRegistration,
    RendererDomSearchResultsResolution, RendererDomSnapshotCapturePayload, RendererLayoutMetrics,
    RendererPageCommand, RendererPageDumpOptions, RendererPageReply, RendererPageState,
    RendererRuntimeRemoteObject, RendererStyleSheetInventoryUpdate, RendererStyleSheetPayload,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

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
// TestingOutcome impls
// ---------------------------------------------------------------------------

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

impl CompletedPageCommand {
    /// Decodes a native Page-agent reply already frozen on an exact DevTools
    /// attachment. The caller validates that attachment before consuming it.
    pub fn finish_layout_metrics(self) -> Result<RendererLayoutMetrics> {
        expect_page_reply!(
            self.into_reply(),
            "layout metrics inspection command",
            "a layout metrics reply",
            RendererPageReply::LayoutMetrics(metrics) => Ok(metrics),
        )
    }

    /// Decodes a native Page-agent reply already frozen on an exact DevTools
    /// attachment. The caller validates that attachment before consuming it.
    pub fn finish_child_frame_tree_snapshot(self) -> Result<Vec<ChildFrameTreeSnapshot>> {
        expect_page_reply!(
            self.into_reply(),
            "child frame tree inspection command",
            "child frame tree snapshots",
            RendererPageReply::ChildFrameTreeSnapshots(snapshots) => Ok(snapshots),
        )
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

    pub fn navigation_redirect_chain(&self) -> &[super::NavigationRedirect] {
        self.page_state.navigation_redirect_chain()
    }

    pub fn final_url(&self) -> &Url {
        self.page_state.final_url()
    }

    pub fn idle_override(&self) -> Option<super::EmulatedIdleOverride> {
        self.idle_override
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

    /// Observes a renderer snapshot without consuming its command reply or output fence.
    /// Foreign Page/owner captures and older Page-view revisions leave the cache unchanged.
    pub fn observe_renderer_page_state(&mut self, page_state: &Arc<RendererPageState>) -> bool {
        self.page_state.observe(page_state)
    }

    pub fn document_title(&self) -> String {
        self.page_state.document_title().to_owned()
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

    pub fn start_serialize_html(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SerializeHtml)
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
            completion.is_from_page(self),
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

    pub fn start_capture_screencast_frame(
        &self,
        request: RendererCaptureScreencastFrameRequest,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::CaptureScreencastFrame(request))
    }

    pub fn finish_capture_screencast_frame(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererCaptureScreencastFrameReply> {
        anyhow::ensure!(
            completion.is_from_page(self),
            "capture screencast frame completed for a stale renderer attachment"
        );
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "capture screencast frame page command",
            "a capture screencast frame reply",
            RendererPageReply::CaptureScreencastFrame(result) => Ok(result),
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

    /// An owned Browser/CLI DOM read, independent of DevTools session binding.
    pub async fn document_node_snapshot_for_backend_node_id_async(
        &mut self,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<Option<DocumentNodeObjectSnapshot>> {
        let pending =
            self.start_page_command(RendererPageCommand::DocumentNodeSnapshotForBackendNodeId {
                backend_node_id,
                depth,
                pierce,
            })?;
        let completion = pending.wait().await?;
        self.observe_renderer_page_state(completion.page_state());
        completion.finish_document_node_snapshot_for_backend_node_id()
    }

    pub async fn child_frame_owner_node_reference_async(
        &mut self,
        frame_id: &str,
    ) -> Result<Option<RendererDocumentNodeReference>> {
        // Browser/CLI semantic reads do not choose a DevTools session.
        let pending =
            self.start_page_command(RendererPageCommand::ChildFrameOwnerNodeReference {
                frame_id: frame_id.to_owned(),
                inspector_session_id: None,
            })?;
        let completion = pending.wait().await?;
        self.observe_renderer_page_state(completion.page_state());
        completion.finish_document_node_reference()
    }
}

// Decoding a frozen DOM reply requires no Browser Page residence.
impl CompletedPageCommand {
    pub fn finish_resolve_blob_object(self) -> Result<String> {
        expect_page_reply!(
            self.into_reply(),
            "resolve Blob object page command",
            "a Blob UUID reply",
            RendererPageReply::BlobUuid(uuid) => Ok(uuid),
        )
    }

    pub fn finish_set_inline_style_sheet_text(self) -> Result<bool> {
        expect_page_reply!(
            self.into_reply(),
            "set inline stylesheet text page command",
            "a bool reply",
            RendererPageReply::Bool(value) => Ok(value),
        )
    }

    pub fn finish_style_sheet_payload(self) -> Result<Option<RendererStyleSheetPayload>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "stylesheet payload page command",
            "an optional stylesheet payload",
            RendererPageReply::OptionalStyleSheetPayload(payload) => Ok(payload),
        )
    }

    pub fn finish_style_sheet_inventory_for_document(
        self,
    ) -> Result<RendererStyleSheetInventoryUpdate> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "stylesheet inventory page command",
            "stylesheet inventory update",
            RendererPageReply::StyleSheetInventory(update) => Ok(update),
        )
    }

    pub fn finish_reset_css_agent_session(self) -> Result<()> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "CSS agent reset page command",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub fn finish_computed_style_properties(self) -> Result<Option<Vec<(String, String)>>> {
        expect_page_reply!(
            self.into_reply(),
            "computed style page command",
            "computed style properties",
            RendererPageReply::ComputedStyleProperties(properties) => Ok(properties),
        )
    }

    pub fn finish_dom_snapshot_capture(self) -> Result<Option<RendererDomSnapshotCapturePayload>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "DOMSnapshot capture page command",
            "an optional DOMSnapshot capture payload",
            RendererPageReply::OptionalDomSnapshotCapturePayload(payload) => Ok(payload),
        )
    }

    pub fn finish_discard_document_search_results(self) -> Result<()> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document discard search results page command",
            "a document search results discarded reply",
            RendererPageReply::DocumentSearchResultsDiscarded => Ok(()),
        )
    }

    pub fn finish_discard_dom_agent_frontend_bindings(self) -> Result<()> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "discard DOM agent frontend bindings page command",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub fn finish_document_bidi_node_binding(self) -> Result<RendererDomBidiNodeBindingResolution> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document BiDi node binding page command",
            "a document BiDi node binding resolution",
            RendererPageReply::DocumentBidiNodeBinding(result) => Ok(result),
        )
    }

    pub fn finish_document_child_node_snapshot_events(
        self,
    ) -> Result<Option<RendererDocumentChildNodeSnapshotEvents>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document child node snapshot events page command",
            "optional document child node snapshot events",
            RendererPageReply::OptionalDocumentChildNodeSnapshotEvents(events) => Ok(events),
        )
    }

    pub fn finish_document_frontend_node_binding(
        self,
    ) -> Result<RendererDomFrontendNodeBindingResolution> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document frontend node binding page command",
            "a document frontend node binding resolution",
            RendererPageReply::DocumentFrontendNodeBinding(result) => Ok(result),
        )
    }

    pub fn finish_document_frontend_node_ids_for_backend_node_ids(
        self,
    ) -> Result<RendererDocumentFrontendNodeIdsResolution> {
        expect_page_reply!(
            self.into_reply(),
            "document frontend node ids for backend node ids page command",
            "frontend node ids resolution reply",
            RendererPageReply::DocumentFrontendNodeIds(resolution) => Ok(resolution),
        )
    }

    pub fn finish_document_geometry_for_backend_node_id(
        self,
    ) -> Result<Option<RendererDocumentNodeGeometry>> {
        self.finish_document_geometry("document geometry backend node id page command")
    }

    pub fn finish_document_geometry_for_object_id(
        self,
    ) -> Result<Option<RendererDocumentNodeGeometry>> {
        self.finish_document_geometry("document geometry object id page command")
    }

    pub fn finish_document_hit_test(self) -> Result<Option<RendererDocumentHitTestResult>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document hit-test page command",
            "an optional document hit-test reply",
            RendererPageReply::OptionalDocumentHitTest(hit) => Ok(hit),
        )
    }

    pub fn finish_document_node_attributes(
        self,
    ) -> Result<RendererDocumentNodeAttributesResolution> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document node attributes page command",
            "a document node attributes resolution",
            RendererPageReply::DocumentNodeAttributesResolution(resolution) => Ok(resolution),
        )
    }

    pub fn finish_document_node_property(self) -> Result<RendererDocumentNodePropertyResolution> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document node property page command",
            "a document node property resolution",
            RendererPageReply::DocumentNodePropertyResolution(resolution) => Ok(resolution),
        )
    }

    pub fn finish_document_node_reference(self) -> Result<Option<RendererDocumentNodeReference>> {
        expect_page_reply!(
            self.into_reply(),
            "document node reference page command",
            "an optional document node reference reply",
            RendererPageReply::OptionalDocumentNodeReference(reference) => Ok(reference),
        )
    }

    pub fn finish_document_node_snapshot_for_backend_node_id(
        self,
    ) -> Result<Option<DocumentNodeObjectSnapshot>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "backend document node snapshot page command",
            "an optional document node object snapshot reply",
            RendererPageReply::OptionalDocumentNodeObjectSnapshot(snapshot) => Ok(*snapshot),
        )
    }

    pub fn finish_document_node_snapshot_for_document(
        self,
    ) -> Result<Option<DocumentNodeObjectSnapshot>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document snapshot page command",
            "an optional document node object snapshot reply",
            RendererPageReply::OptionalDocumentNodeObjectSnapshot(snapshot) => Ok(*snapshot),
        )
    }

    pub fn finish_document_node_snapshot_for_object_id(
        self,
    ) -> Result<Option<DocumentNodeObjectSnapshot>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "describe node object id page command",
            "an optional document node object snapshot reply",
            RendererPageReply::OptionalDocumentNodeObjectSnapshot(snapshot) => Ok(*snapshot),
        )
    }

    pub fn finish_document_node_stack_trace(self) -> Result<RendererDomNodeStackTraceResolution> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document node stack trace page command",
            "a document node stack trace resolution",
            RendererPageReply::DocumentNodeStackTrace(result) => Ok(result),
        )
    }

    pub fn finish_document_node_text(self) -> Result<RendererDocumentNodeTextResolution> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document node text page command",
            "a document node text resolution",
            RendererPageReply::DocumentNodeTextResolution(resolution) => Ok(resolution),
        )
    }

    pub fn finish_document_perform_search(self) -> Result<RendererDomSearchRegistration> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document perform search page command",
            "a document search registration reply",
            RendererPageReply::DocumentPerformSearch(result) => Ok(result),
        )
    }

    pub fn finish_document_query_selector(self) -> Result<RendererDocumentQuerySelectorResolution> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document query selector page command",
            "a document query selector resolution",
            RendererPageReply::DocumentQuerySelectorResolution(resolution) => Ok(resolution),
        )
    }

    pub fn finish_document_query_selector_with_child_node_snapshot_events(
        self,
    ) -> Result<RendererDocumentQuerySelectorWithChildNodeSnapshotEvents> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document query selector with child node snapshot events page command",
            "a document query selector with child node snapshot events reply",
            RendererPageReply::DocumentQuerySelectorWithChildNodeSnapshotEvents(result) => Ok(result),
        )
    }

    pub fn finish_document_search_results(self) -> Result<RendererDomSearchResultsResolution> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document search results page command",
            "a document search results resolution",
            RendererPageReply::DocumentSearchResults(result) => Ok(result),
        )
    }

    pub fn finish_edit_document_node(self) -> Result<RendererDomEditOutcome> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "edit document node page command",
            "a DOM edit outcome reply",
            RendererPageReply::DomEditOutcome(outcome) => Ok(outcome),
        )
    }

    pub fn finish_focus_document_node_id(self) -> Result<RendererDomFocusOutcome> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "focus document node page command",
            "a DOM focus outcome reply",
            RendererPageReply::DomFocusOutcome(outcome) => Ok(outcome),
        )
    }

    pub fn finish_mutate_document_node_attribute(
        self,
    ) -> Result<RendererDomAttributeMutationOutcome> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "mutate document node attribute page command",
            "a DOM attribute mutation outcome reply",
            RendererPageReply::DomAttributeMutationOutcome(outcome) => Ok(outcome),
        )
    }

    pub fn finish_outer_html_for_backend_node_id(self) -> Result<Option<String>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "outerHTML backend node id page command",
            "an optional string reply",
            RendererPageReply::OptionalString(outer_html) => Ok(outer_html),
        )
    }

    pub fn finish_outer_html_for_document(self) -> Result<String> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "outerHTML document page command",
            "a string reply",
            RendererPageReply::OptionalString(Some(outer_html)) => Ok(outer_html),
        )
    }

    pub fn finish_outer_html_for_object_id(self) -> Result<Option<String>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "outerHTML object id page command",
            "an optional string reply",
            RendererPageReply::OptionalString(outer_html) => Ok(outer_html),
        )
    }

    pub fn finish_remove_document_node(self) -> Result<bool> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "remove document node page command",
            "a bool reply",
            RendererPageReply::Bool(value) => Ok(value),
        )
    }

    pub fn finish_resolve_runtime_object_for_backend_node_id(
        self,
    ) -> Result<DocumentNodeRuntimeObjectResolution> {
        self.finish_runtime_remote_object_resolution(
            "resolve backend node runtime object page command",
        )
    }

    pub fn finish_scroll_node_into_view_if_needed(
        self,
    ) -> Result<super::RendererScrollIntoViewResult> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "scroll document node into view page command",
            "a scroll-into-view reply",
            RendererPageReply::ScrollIntoViewResult(result) => Ok(result),
        )
    }

    pub fn finish_set_document_node_stack_traces_enabled(self) -> Result<()> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "set document node stack traces enabled page command",
            "a document node stack traces enabled reply",
            RendererPageReply::DocumentNodeStackTracesEnabled => Ok(()),
        )
    }

    pub fn finish_set_file_input_files(self) -> Result<Option<bool>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "set file input files page command",
            "an optional bool reply",
            RendererPageReply::OptionalBool(value) => Ok(value),
        )
    }

    pub fn finish_set_file_input_files_for_object_id(self) -> Result<Option<bool>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "set file input files object id page command",
            "an optional bool reply",
            RendererPageReply::OptionalBool(value) => Ok(value),
        )
    }

    fn finish_document_geometry(
        self,
        operation: &str,
    ) -> Result<Option<RendererDocumentNodeGeometry>> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            operation,
            "an optional document node geometry reply",
            RendererPageReply::OptionalDocumentNodeGeometry(geometry) => Ok(geometry),
        )
    }

    fn finish_runtime_remote_object_resolution(
        self,
        command_name: &'static str,
    ) -> Result<DocumentNodeRuntimeObjectResolution> {
        let reply = self.into_reply();
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
}

impl CompletedPageCommand {
    pub fn finish_register_document_bidi_node_binding(self) -> Result<()> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document BiDi node binding registration page command",
            "a document BiDi node binding registered reply",
            RendererPageReply::DocumentBidiNodeBindingRegistered => Ok(()),
        )
    }
}

impl CompletedPageCommand {
    pub fn finish_document_bidi_node_shared_id_for_backend_node_id(
        self,
    ) -> Result<RendererDomBidiNodeSharedIdResolution> {
        let reply = self.into_reply();
        expect_page_reply!(
            reply,
            "document BiDi node shared id page command",
            "a document BiDi node shared id resolution",
            RendererPageReply::DocumentBidiNodeSharedId(result) => Ok(result),
        )
    }
}
