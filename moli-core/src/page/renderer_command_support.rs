use super::CompletedPageCommand;
use super::Page;
use super::PendingPageCommand;
use super::dom_protocol_support::DocumentNodeObjectSnapshot;
use super::protocol_support::ChildFrameTreeSnapshot;
use super::protocol_support::ScriptExecutionReport;
use super::protocol_support::ScriptObservableOutputItem;
use super::protocol_support::SubresourceNetworkRecord;
use super::protocol_support::WebSocketLifecycleEvent;
use super::protocol_support::WebSocketNetworkEvent;
use crate::renderer::RendererCaptureScreenshotReply;
use crate::renderer::RendererCaptureScreenshotRequest;
use crate::renderer::RendererDocumentChildNodeSnapshotEvents;
use crate::renderer::RendererDocumentNodeReference;
use crate::renderer::RendererDomNodeReference;
use crate::renderer::RendererHtmlSerializeOptions;
use crate::renderer::RendererLayoutMetrics;
use crate::renderer::RendererMarkdownOptions;
use crate::renderer::RendererPageCommand;
use crate::renderer::RendererPageReply;
use crate::renderer::RendererPageState;
use crate::renderer::RendererRuntimeRemoteObject;
use anyhow::Result;
use std::sync::Arc;
use url::Url;

/// URL and HTML captured in the same synchronous renderer command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializedHtmlDocument {
    pub url: String,
    pub html: String,
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

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
// PageObservableOutputUpdate / TestingOutcome impls
// ---------------------------------------------------------------------------

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

    pub fn document_isolate_identity_for_diagnostics(&self) -> usize {
        self.page_state.state().document_isolate_identity
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

    pub fn headers(&self) -> &[(String, Vec<u8>)] {
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
    pub fn start_refresh_script_execution_report(&self) -> Result<PendingPageCommand> {
        self.start_full_page_command(RendererPageCommand::RefreshFullPageState)
    }

    pub async fn refresh_script_execution_report_async(&mut self) -> Result<()> {
        let pending = self.start_refresh_script_execution_report()?;
        self.finish_unit_runtime_page_command(
            pending.wait().await?,
            "refresh script execution report",
        )
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

    pub fn take_observable_output_update(&mut self) -> PageObservableOutputUpdate<'_> {
        let observable_output_items = self.page_state.script_execution().observable_output_items();
        PageObservableOutputUpdate::append(observable_output_items)
    }

    pub(super) fn replace_page_state(&mut self, page_state: Arc<RendererPageState>) {
        self.page_state.replace(page_state);
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
}

impl Page {
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

    pub fn document_title(&self) -> String {
        self.page_state.document_title().to_owned()
    }
}

impl Page {
    pub fn document_content_type(&self) -> &str {
        &self.page_state.state().document_content_type
    }

    pub fn finish_request_document_node_for_object_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<(Option<u32>, Option<RendererDocumentChildNodeSnapshotEvents>)> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "request document node page command",
            "a requested document node reply",
            RendererPageReply::DocumentNodeRequested { node_id, child_node_snapshot_events } =>
                Ok((node_id, child_node_snapshot_events)),
        )
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

    pub fn finish_document_node_exists(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<bool> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(reply, "document node presence page command", "a bool reply", RendererPageReply::Bool(exists) => Ok(exists),)
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
}

impl Page {
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

    pub fn start_outer_html_for_node(
        &self,
        reference: RendererDomNodeReference,
        include_shadow_dom: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::OuterHtmlForNode {
            reference,
            include_shadow_dom,
        })
    }

    pub fn start_outer_html_for_backend_node_id(
        &self,
        backend_node_id: u32,
        include_shadow_dom: bool,
    ) -> Result<PendingPageCommand> {
        self.start_outer_html_for_node(
            RendererDomNodeReference::BackendNodeId(backend_node_id),
            include_shadow_dom,
        )
    }

    pub fn finish_outer_html_for_node(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<String>> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "outerHTML node page command",
            "an optional string reply",
            RendererPageReply::OptionalString(outer_html) => Ok(outer_html),
        )
    }

    pub fn finish_outer_html_for_backend_node_id(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<Option<String>> {
        self.finish_outer_html_for_node(completion)
    }

    pub fn start_serialize_html(&self) -> Result<PendingPageCommand> {
        self.start_serialize_html_with_options(RendererHtmlSerializeOptions::default())
    }

    pub fn start_serialize_html_with_options(
        &self,
        options: RendererHtmlSerializeOptions,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SerializeDocument { options })
    }

    /// Publishes current layout for subsequent geometry reads without painting.
    ///
    /// This is an explicit refresh, used by CLI `fetch --layout --eval` after
    /// readiness. Ordinary script evaluation and geometry reads do not call it.
    /// When real layout is disabled, this is a no-op.
    pub async fn publish_layout_async(&mut self) -> Result<()> {
        self.dispatch_unit_page_command_async(RendererPageCommand::PublishLayout, "publish layout")
            .await
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

    pub fn start_print_to_pdf(
        &self,
        request: super::RendererPrintToPdfRequest,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::PrintToPdf(request))
    }

    pub fn finish_print_to_pdf(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<super::RendererPrintToPdfReply> {
        anyhow::ensure!(
            completion.renderer_agent_attachment_id() == self.renderer_agent_attachment_id(),
            "print to PDF completed for a stale renderer attachment"
        );
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "print to PDF page command",
            "a print to PDF reply",
            RendererPageReply::PrintToPdf(result) => Ok(result),
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

    pub fn start_render_markdown(
        &self,
        options: RendererMarkdownOptions,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::RenderMarkdown { options })
    }

    pub fn finish_render_markdown(&mut self, completion: CompletedPageCommand) -> Result<String> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "render Markdown page command",
            "a string reply",
            RendererPageReply::OptionalString(Some(rendered)) => Ok(rendered),
        )
    }

    pub async fn render_markdown_async(
        &mut self,
        options: RendererMarkdownOptions,
    ) -> Result<String> {
        let pending = self.start_render_markdown(options)?;
        let completion = pending.wait().await?;
        self.finish_render_markdown(completion)
    }

    pub fn finish_serialized_document(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<SerializedHtmlDocument> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(reply, "serialize HTML page command", "a serialized document reply",
            RendererPageReply::SerializedDocument { url, html } => Ok(SerializedHtmlDocument { url, html }),
        )
    }

    pub fn finish_serialize_html(&mut self, completion: CompletedPageCommand) -> Result<String> {
        self.finish_serialized_document(completion)
            .map(|document| document.html)
    }

    pub async fn serialize_html_async(&mut self) -> Result<String> {
        self.serialize_document_async(RendererHtmlSerializeOptions::default())
            .await
            .map(|document| document.html)
    }

    pub async fn serialize_document_async(
        &mut self,
        options: RendererHtmlSerializeOptions,
    ) -> Result<SerializedHtmlDocument> {
        let pending = self.start_serialize_html_with_options(options)?;
        let completion = pending.wait().await?;
        self.finish_serialized_document(completion)
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
