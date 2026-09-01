use serde::Deserialize;
use serde_json::{Value, json};

use super::node_references::{NodeReferenceParams, devtools_node_reference_from_ids};
use super::*;
use crate::conn::BackgroundProtocolEvent;
use crate::devtools_runtime::{
    DevToolsCommand, DevToolsCommandResult, DevToolsDescribeNodeCommand,
    DevToolsDescribeNodeResult, DevToolsDomAttribute, DevToolsDomBoxModel,
    DevToolsDomGeometryCommand, DevToolsDomGeometryOperation, DevToolsDomGeometryResult,
    DevToolsDomNodeReference, DevToolsDomObjectReferenceCommand,
    DevToolsDomObjectReferenceOperation, DevToolsDomQuad, DevToolsError, DevToolsErrorKind,
    DevToolsFrameId, DevToolsGetAttributesCommand, DevToolsGetAttributesResult,
    DevToolsGetDocumentCommand, DevToolsGetFrameOwnerCommand, DevToolsGetFrameOwnerResult,
    DevToolsGetNodeForLocationCommand, DevToolsGetNodeForLocationResult,
    DevToolsGetOuterHtmlCommand, DevToolsGetOuterHtmlResult, DevToolsGetPropertyCommand,
    DevToolsGetPropertyResult, DevToolsGetTextCommand, DevToolsGetTextResult,
    DevToolsPushNodesByBackendIdsCommand, DevToolsPushNodesByBackendIdsResult,
    DevToolsQuerySelectorCommand, DevToolsQuerySelectorResult, DevToolsRemoteHandleId,
    DevToolsRemoveNodeCommand, DevToolsRequestChildNodesCommand, DevToolsResolveNodeCommand,
    DevToolsResolveNodeResult, DevToolsScrollIntoViewIfNeededCommand,
};
use crate::domains::actions::DomAction;
use crate::domains::command_output::CommandOutputPlan;
use chromiumoxide_cdp::cdp::browser_protocol::dom::{
    GetAttributesParams, GetDocumentParams, GetFrameOwnerParams, GetNodeForLocationParams,
    PushNodesByBackendIdsToFrontendParams, QuerySelectorAllParams, QuerySelectorParams,
    RemoveAttributeParams, RequestChildNodesParams, RequestNodeParams, SetAttributeValueParams,
};
use moli_core::page::{
    CompletedPageCommand, DocumentNodeSnapshot, DomScrollIntoViewRect, PendingPageCommand,
    RendererDocumentFrontendNodeIdsResolution, RendererDocumentNodeAttributesResolution,
    RendererDocumentNodeGeometry, RendererDocumentNodePropertyResolution,
    RendererDocumentNodeReference, RendererDocumentNodeTextResolution,
    RendererDocumentQuerySelectorResolution, RendererDomAttributeMutation,
    RendererDomAttributeMutationOutcome, RendererDomEditOutcome, RendererDomFocusOutcome,
    RendererScrollIntoViewResult, SelectedFile,
};
use moli_page_types::DocumentSnapshotNodeId;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribeNodeParams {
    #[serde(flatten)]
    reference: NodeReferenceParams,
    #[serde(default = "default_describe_depth")]
    depth: i32,
    #[serde(default)]
    pierce: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolveNodeParams {
    #[serde(default)]
    node_id: Option<u32>,
    #[serde(default)]
    backend_node_id: Option<u32>,
    #[serde(default)]
    object_group: Option<String>,
    #[serde(default, alias = "contextId")]
    execution_context_id: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetOuterHtmlParams {
    #[serde(flatten)]
    reference: NodeReferenceParams,
    #[serde(default, rename = "includeShadowDOM")]
    include_shadow_dom: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScrollIntoViewIfNeededParams {
    #[serde(flatten)]
    reference: NodeReferenceParams,
    #[serde(default)]
    rect: Option<ScrollIntoViewRectParams>,
}

#[derive(Deserialize)]
struct ScrollIntoViewRectParams {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl TryFrom<ScrollIntoViewRectParams> for DomScrollIntoViewRect {
    type Error = ();

    fn try_from(rect: ScrollIntoViewRectParams) -> Result<Self, Self::Error> {
        Self::try_new(rect.x, rect.y, rect.width, rect.height).ok_or(())
    }
}

fn validated_scroll_into_view_rect(
    rect: Option<ScrollIntoViewRectParams>,
) -> Result<Option<DomScrollIntoViewRect>, PendingDomCommandStartError> {
    rect.map(DomScrollIntoViewRect::try_from)
        .transpose()
        .map_err(|()| PendingDomCommandStartError::invalid_params())
}

pub(crate) struct PendingDomCommandDispatch {
    pub(super) command_id: Option<u64>,
    pub(super) owner_scope: CommandOwnerScope,
    pub(super) kind: PendingDomCommandKind,
    pub(super) pending: PendingDomCommandWork,
}

pub(crate) struct CompletedDomCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    kind: PendingDomCommandKind,
    completed: Result<CompletedDomCommandWork, String>,
}

impl CompletedDomCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }

    pub(crate) fn renderer_output_predecessor(&self) -> Option<moli_core::RendererOutputFence> {
        match &self.completed {
            Ok(CompletedDomCommandWork::Page(completion)) => {
                completion.renderer_output_predecessor()
            }
            Err(_) => None,
        }
    }
}

pub(crate) enum DomCommandTaskStep {
    Pending(Box<PendingDomCommandDispatch>),
    Complete,
}

pub(super) enum DevToolsDomCommandTaskStep {
    Pending(Box<PendingDomCommandDispatch>),
    Complete(Box<Result<DevToolsCommandResult, DevToolsError>>),
}

pub(super) fn devtools_dom_command_task_complete(
    result: Result<DevToolsCommandResult, DevToolsError>,
) -> DevToolsDomCommandTaskStep {
    DevToolsDomCommandTaskStep::Complete(Box::new(result))
}

fn devtools_dom_node_not_found_error() -> DevToolsError {
    DevToolsError::new(
        DevToolsErrorKind::NoSuchNode,
        "Could not find node with given id",
    )
}

pub(super) struct PendingDomCommandStartError {
    pub(super) code: i32,
    pub(super) message: String,
}

#[derive(Clone)]
pub(super) enum PendingDomCommandKind {
    DiscardDomAgentFrontendBindings,
    RemoveNode,
    RendererBackendNodeClientRect {
        operation: DevToolsDomGeometryOperation,
    },
    GetNodeForLocation {
        top_frame_id: String,
    },
    RendererBackendNodeScrollIntoViewIfNeeded,
    PushNodesByBackendIdsToFrontend {
        backend_node_ids: Vec<u32>,
        node_ids: Vec<u32>,
        renderer_backend_positions: Vec<usize>,
    },
    GetFrameOwner {
        frame_id: String,
    },
    QuerySelectorLive {
        multiple: bool,
    },
    ResolveFrontendNodeForRemoveNode {
        frontend_node_id: u32,
    },
    ResolveFrontendNodeForFocus {
        frontend_node_id: u32,
    },
    ResolveFrontendNodeForMutateAttribute {
        mutation: RendererDomAttributeMutation,
    },
    ResolveFrontendNodeForQuerySelector {
        selector: String,
        multiple: bool,
        top_frame_id: Option<String>,
    },
    ResolveFrontendNodeForResolveNode {
        frontend_node_id: u32,
        requested_execution_context_id: Option<i64>,
        object_group: Option<String>,
        top_frame_id: Option<String>,
    },
    ResolveFrontendNodeForGetAttributes {
        frontend_node_id: u32,
    },
    ResolveFrontendNodeForGetText {
        frontend_node_id: u32,
    },
    ResolveFrontendNodeForGetProperty {
        frontend_node_id: u32,
        name: String,
    },
    ResolveFrontendNodeForDomGeometry {
        frontend_node_id: u32,
        operation: DevToolsDomGeometryOperation,
    },
    ResolveFrontendNodeForDescribeNode {
        frontend_node_id: u32,
        depth: i32,
        pierce: bool,
        top_frame_id: Option<String>,
    },
    ResolveFrontendNodeForRequestChildNodes {
        depth: i32,
        pierce: bool,
        top_frame_id: Option<String>,
    },
    ResolveFrontendNodeForGetOuterHtml {
        frontend_node_id: u32,
        include_shadow_dom: bool,
    },
    ResolveFrontendNodeForScrollIntoViewIfNeeded {
        frontend_node_id: u32,
        rect: Option<DomScrollIntoViewRect>,
    },
    ResolveBidiNodeForSetFileInputFiles {
        object_id: DevToolsRemoteHandleId,
        files: Vec<SelectedFile>,
        append: bool,
    },
    ResolveFrontendNodeForSetFileInputFiles {
        frontend_node_id: u32,
        file_paths: Vec<String>,
        append: bool,
    },
    GetAttributesLive,
    GetTextLive,
    GetPropertyLive,
    RequestNodeObjectReference,
    GetOuterHtmlDocument,
    GetOuterHtmlObjectReference,
    GetOuterHtmlBackendNodeReference,
    ScrollIntoViewIfNeededObjectReference,
    Focus {
        missing_node_message: &'static str,
    },
    MutateAttribute,
    EditDocumentNode,
    SetFileInputFilesObjectReference,
    DescribeNodeObjectReference {
        cached_object_node: Option<Value>,
        top_frame_id: Option<String>,
    },
    SetFileInputFiles,
    SetFileInputFilesPreflight {
        reference: DevToolsDomNodeReference,
        file_paths: Vec<String>,
        append: bool,
    },
    ObjectReferenceLiveClientRect {
        operation: PendingDomObjectReferenceOperation,
    },
    DocumentSnapshot {
        operation: PendingDomDocumentSnapshotOperation,
        top_frame_id: Option<String>,
    },
    SetChildNodesSnapshotForBackendNode {
        after: PendingSetChildNodesAfter,
        top_frame_id: Option<String>,
        missing_node_message: &'static str,
    },
    QuerySelectorSetChildNodesLive {
        multiple: bool,
        top_frame_id: Option<String>,
    },
    PerformSearchLive,
    GetSearchResultsLive,
    DiscardSearchResultsLive,
    SetNodeStackTracesEnabled,
    GetNodeStackTraces,
    ResolveNode {
        object_group: Option<String>,
        cache_top_frame_id: Option<Option<String>>,
    },
    ResolveNodeCacheSnapshot {
        remote_object: Box<Value>,
        object_group: Option<String>,
        cache_object_id: String,
        top_frame_id: Option<String>,
    },
    ResolveNodeExecutionContextFrame {
        reference: DevToolsDomNodeReference,
        execution_context_id: i64,
        object_group: Option<String>,
        top_frame_id: Option<String>,
    },
}

pub(super) fn start_disable_dom_agent_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let Some(page) = loaded_page_mut_for_owner(conn, &owner) else {
        return Ok(None);
    };
    let pending = page
        .start_discard_dom_agent_frontend_bindings(renderer_inspector_session_id)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id: cmd.id,
        owner_scope: owner,
        kind: PendingDomCommandKind::DiscardDomAgentFrontendBindings,
        pending: PendingDomCommandWork::Page(pending),
    }))
}

pub(super) enum PendingDomCommandWork {
    Page(PendingPageCommand),
}

enum CompletedDomCommandWork {
    Page(Box<CompletedPageCommand>),
}

#[derive(Clone)]
pub(super) enum PendingDomObjectReferenceOperation {
    RequestNode,
    Focus,
    GetOuterHtml {
        include_shadow_dom: bool,
    },
    GetBoxModel,
    GetContentQuads,
    ScrollIntoViewIfNeeded {
        rect: Option<DomScrollIntoViewRect>,
    },
    DescribeNode {
        depth: i32,
        pierce: bool,
        cached_object_node: Option<Value>,
        top_frame_id: Option<String>,
    },
}

pub(super) fn dom_object_reference_id_for_owner(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    object_id: &DevToolsRemoteHandleId,
) -> String {
    if let Some(object_id) = conn.runtime_remote_object_alias_for_owner(owner, object_id.as_str()) {
        return object_id;
    }
    object_id.as_str().to_owned()
}

#[derive(Clone, Copy)]
pub(super) enum PendingDomDocumentSnapshotOperation {
    GetDocument,
    GetFlattenedDocument,
}

#[derive(Clone)]
pub(super) enum PendingSetChildNodesAfter {
    EmptyResult,
    QuerySelectorLive {
        resolution: RendererDocumentQuerySelectorResolution,
        multiple: bool,
    },
}

impl PendingDomCommandDispatch {
    pub(crate) async fn wait(self) -> CompletedDomCommandDispatch {
        let completed = match self.pending {
            PendingDomCommandWork::Page(pending) => Box::pin(pending.wait())
                .await
                .map(|completed| CompletedDomCommandWork::Page(Box::new(completed)))
                .map_err(|error| error.to_string()),
        };
        CompletedDomCommandDispatch {
            command_id: self.command_id,
            owner_scope: self.owner_scope,
            kind: self.kind,
            completed,
        }
    }
}

impl PendingDomCommandStartError {
    pub(super) fn invalid_params() -> Self {
        Self {
            code: -32602,
            message: "InvalidParams".to_owned(),
        }
    }

    pub(super) fn no_document_loaded() -> Self {
        Self {
            code: -32000,
            message: "NoDocumentLoaded".to_owned(),
        }
    }

    pub(super) fn node_not_found() -> Self {
        Self {
            code: -32000,
            message: "Could not find node with given id".to_owned(),
        }
    }

    pub(super) fn no_such_target() -> Self {
        Self {
            code: -32000,
            message: "NoSuchTarget".to_owned(),
        }
    }

    pub(super) fn renderer_error(error: impl std::fmt::Display) -> Self {
        Self {
            code: -32000,
            message: error.to_string(),
        }
    }

    pub(super) fn invalid_selector(error: impl std::fmt::Display) -> Self {
        Self {
            code: -32602,
            message: error.to_string(),
        }
    }
}

impl From<PendingDomCommandStartError> for DevToolsError {
    fn from(error: PendingDomCommandStartError) -> Self {
        let kind = match error.code {
            -32602 if matches!(error.message.as_str(), "InvalidParams" | "InvalidParam") => {
                DevToolsErrorKind::InvalidArgument
            }
            -32602 => DevToolsErrorKind::InvalidSelector,
            _ if error.message == "Could not find node with given id" => {
                DevToolsErrorKind::NoSuchNode
            }
            _ if error.message == "NoSuchTarget" => DevToolsErrorKind::NoSuchTarget,
            _ => DevToolsErrorKind::Internal,
        };
        DevToolsError::new(kind, error.message)
    }
}

pub(super) fn default_describe_depth() -> i32 {
    1
}

pub(super) const INVALID_REQUEST_CHILD_NODES_DEPTH_MESSAGE: &str =
    "Please provide a positive integer as a depth or -1 for entire subtree";

fn cdp_id_from_i64(value: i64) -> Option<u32> {
    value.try_into().ok()
}

fn required_backend_node_id_for_reference(
    reference: &DevToolsDomNodeReference,
) -> Result<u32, PendingDomCommandStartError> {
    match reference {
        DevToolsDomNodeReference::BackendNodeId(backend_node_id) => Ok(*backend_node_id),
        DevToolsDomNodeReference::FrontendNodeId(_) => {
            Err(PendingDomCommandStartError::node_not_found())
        }
    }
}

fn start_document_node_attributes_for_reference(
    page: &Page,
    reference: DevToolsDomNodeReference,
) -> Result<PendingPageCommand, PendingDomCommandStartError> {
    let backend_node_id = required_backend_node_id_for_reference(&reference)?;
    page.start_document_node_attributes_for_backend_node_id(backend_node_id)
        .map_err(PendingDomCommandStartError::renderer_error)
}

fn start_document_node_text_for_reference(
    page: &Page,
    reference: DevToolsDomNodeReference,
) -> Result<PendingPageCommand, PendingDomCommandStartError> {
    let backend_node_id = required_backend_node_id_for_reference(&reference)?;
    page.start_document_node_text_for_backend_node_id(backend_node_id)
        .map_err(PendingDomCommandStartError::renderer_error)
}

fn start_document_node_property_for_reference(
    page: &Page,
    reference: DevToolsDomNodeReference,
    name: &str,
) -> Result<PendingPageCommand, PendingDomCommandStartError> {
    let backend_node_id = required_backend_node_id_for_reference(&reference)?;
    page.start_document_node_property_for_backend_node_id(backend_node_id, name)
        .map_err(PendingDomCommandStartError::renderer_error)
}

pub(super) fn start_document_node_snapshot_for_reference(
    page: &Page,
    reference: DevToolsDomNodeReference,
    depth: i32,
    pierce: bool,
) -> Result<PendingPageCommand, PendingDomCommandStartError> {
    let backend_node_id = required_backend_node_id_for_reference(&reference)?;
    page.start_document_node_snapshot_for_backend_node_id(backend_node_id, depth, pierce)
        .map_err(PendingDomCommandStartError::renderer_error)
}

fn start_inspector_document_node_snapshot_for_reference(
    page: &Page,
    renderer_inspector_session_id: Option<String>,
    include_whitespace: bool,
    reference: DevToolsDomNodeReference,
    depth: i32,
    pierce: bool,
) -> Result<PendingPageCommand, PendingDomCommandStartError> {
    let backend_node_id = required_backend_node_id_for_reference(&reference)?;
    page.start_document_node_snapshot_for_backend_node_id_in_inspector_session(
        renderer_inspector_session_id,
        include_whitespace,
        backend_node_id,
        depth,
        pierce,
    )
    .map_err(PendingDomCommandStartError::renderer_error)
}

fn start_outer_html_for_reference(
    page: &Page,
    reference: DevToolsDomNodeReference,
    include_shadow_dom: bool,
) -> Result<(PendingPageCommand, PendingDomCommandKind), PendingDomCommandStartError> {
    let backend_node_id = required_backend_node_id_for_reference(&reference)?;
    let pending = page
        .start_outer_html_for_backend_node_id(backend_node_id, include_shadow_dom)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok((
        pending,
        PendingDomCommandKind::GetOuterHtmlBackendNodeReference,
    ))
}

fn start_client_rect_for_reference(
    page: &Page,
    reference: DevToolsDomNodeReference,
    operation: DevToolsDomGeometryOperation,
) -> Result<(PendingPageCommand, PendingDomCommandKind), PendingDomCommandStartError> {
    let backend_node_id = required_backend_node_id_for_reference(&reference)?;
    let pending = page
        .start_document_geometry_for_backend_node_id(backend_node_id)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok((
        pending,
        PendingDomCommandKind::RendererBackendNodeClientRect { operation },
    ))
}

fn start_scroll_into_view_for_reference(
    page: &Page,
    reference: DevToolsDomNodeReference,
    rect: Option<DomScrollIntoViewRect>,
) -> Result<(PendingPageCommand, PendingDomCommandKind), PendingDomCommandStartError> {
    let backend_node_id = required_backend_node_id_for_reference(&reference)?;
    let pending = page
        .start_scroll_backend_node_into_view_if_needed(backend_node_id, rect)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok((
        pending,
        PendingDomCommandKind::RendererBackendNodeScrollIntoViewIfNeeded,
    ))
}

fn start_query_selector_with_child_node_snapshot_events_for_reference(
    page: &Page,
    renderer_inspector_session_id: Option<String>,
    include_whitespace: bool,
    reference: DevToolsDomNodeReference,
    selector: String,
    multiple: bool,
    top_frame_id: Option<String>,
) -> Result<(PendingPageCommand, PendingDomCommandKind), PendingDomCommandStartError> {
    let root_backend_node_id = required_backend_node_id_for_reference(&reference)?;
    let pending = page
        .start_document_query_selector_with_child_node_snapshot_events_for_backend_node_id(
            renderer_inspector_session_id,
            include_whitespace,
            root_backend_node_id,
            selector,
            multiple,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok((
        pending,
        PendingDomCommandKind::QuerySelectorSetChildNodesLive {
            multiple,
            top_frame_id,
        },
    ))
}

fn start_query_selector_for_reference(
    page: &Page,
    renderer_inspector_session_id: Option<String>,
    include_whitespace: bool,
    reference: DevToolsDomNodeReference,
    selector: String,
    multiple: bool,
) -> Result<(PendingPageCommand, PendingDomCommandKind), PendingDomCommandStartError> {
    let root_backend_node_id = required_backend_node_id_for_reference(&reference)?;
    let pending = page
        .start_document_query_selector_for_backend_node_id_in_inspector_session(
            renderer_inspector_session_id,
            include_whitespace,
            root_backend_node_id,
            selector,
            multiple,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok((
        pending,
        PendingDomCommandKind::QuerySelectorLive { multiple },
    ))
}

fn optional_i64_to_i32(value: Option<i64>) -> Option<Option<i32>> {
    value.map(i32::try_from).transpose().ok()
}

pub(super) fn try_start_pending_dom_command_result(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingDomCommandDispatch>, (i32, String)> {
    start_pending_dom_command(conn, cmd).map_err(|error| (error.code, error.message))
}

pub(super) fn complete_non_pending_dom_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<CommandOutputPlan> {
    match cmd.parse_action::<DomAction>() {
        Some(DomAction::RequestChildNodes) => complete_non_pending_messages(conn, |conn, out| {
            complete_non_pending_request_child_nodes_command(conn, cmd, out)
        }),
        Some(DomAction::QuerySelector) => complete_non_pending_messages(conn, |conn, out| {
            complete_non_pending_query_selector_command(conn, cmd, false, out)
        }),
        Some(DomAction::QuerySelectorAll) => complete_non_pending_messages(conn, |conn, out| {
            complete_non_pending_query_selector_command(conn, cmd, true, out)
        }),
        Some(DomAction::PerformSearch) => complete_non_pending_messages(conn, |conn, out| {
            search::complete_non_pending_perform_search_command(conn, cmd, out)
        }),
        Some(DomAction::DiscardSearchResults) => complete_non_pending_messages(conn, |_, out| {
            search::complete_non_pending_discard_search_results_command(out)
        }),
        Some(DomAction::GetFrameOwner) => complete_non_pending_get_frame_owner_command(conn, cmd),
        Some(DomAction::GetNodeForLocation) => Some(
            complete_non_pending_get_node_for_location_command(conn, cmd),
        ),
        _ => None,
    }
}

fn complete_non_pending_messages(
    conn: &mut CdpConnection,
    complete: impl FnOnce(&mut CdpConnection, &mut DomCommandOutput) -> bool,
) -> Option<CommandOutputPlan> {
    let mut out = DomCommandOutput::default();
    complete(conn, &mut out).then(|| out.into_plan())
}

#[derive(Default)]
pub(super) struct DomCommandOutput {
    plan: CommandOutputPlan,
}

impl DomCommandOutput {
    pub(super) fn push_success(&mut self) {
        self.plan.push_success();
    }

    pub(super) fn push_result(&mut self, value: Value) {
        self.plan.push_result(value);
    }

    pub(super) fn push_error(&mut self, code: i32, message: impl Into<String>) {
        self.plan.push_error(code, message);
    }

    pub(super) fn push_background_event(&mut self, event: BackgroundProtocolEvent) {
        self.plan.push_background_event(event);
    }

    fn set_renderer_output_predecessor(&mut self, predecessor: moli_core::RendererOutputFence) {
        self.plan.set_renderer_output_predecessor(predecessor);
    }

    fn into_plan(self) -> CommandOutputPlan {
        self.plan
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.plan.is_empty()
    }
}

fn complete_non_pending_request_child_nodes_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    out: &mut DomCommandOutput,
) -> bool {
    let command = match build_cdp_request_child_nodes_command(conn, cmd) {
        Ok(command) => command,
        Err(error) => {
            out.push_error(error.code, error.message);
            return true;
        }
    };
    match complete_devtools_request_child_nodes_command(conn, command, out) {
        Ok(()) => {}
        Err(error) => {
            out.push_error(error.code, error.message);
        }
    }
    true
}

fn complete_non_pending_query_selector_command(
    _conn: &mut CdpConnection,
    _cmd: &Cmd<'_>,
    _all: bool,
    out: &mut DomCommandOutput,
) -> bool {
    out.push_error(-32000, "MissingDomCommand");
    true
}

fn complete_non_pending_get_frame_owner_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<CommandOutputPlan> {
    match build_cdp_get_frame_owner_command(conn, cmd) {
        Ok(_) => None,
        Err(error) => Some(CommandOutputPlan::error(error.code, error.message)),
    }
}

fn complete_non_pending_get_node_for_location_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let command = match build_cdp_get_node_for_location_command(conn, cmd) {
        Ok(command) => command,
        Err(error) => {
            return CommandOutputPlan::error(error.code, error.message);
        }
    };
    match complete_devtools_get_node_for_location_command(conn, command) {
        Ok(result) => CommandOutputPlan::result(result),
        Err(error) => CommandOutputPlan::error(error.code, error.message),
    }
}

fn push_set_child_nodes_event(
    out: &mut DomCommandOutput,
    session_id: Option<&str>,
    parent_frontend_node_id: u32,
    nodes: Vec<Value>,
) {
    out.push_background_event(BackgroundProtocolEvent::dom_set_child_nodes(
        session_id,
        parent_frontend_node_id,
        nodes,
    ));
}

fn start_pending_dom_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let Some(action) = cmd.parse_action::<DomAction>() else {
        return Ok(None);
    };
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    if action == DomAction::DiscardSearchResults && !target_owner_exists_for_owner(conn, &owner) {
        return Ok(None);
    }
    if !target_owner_exists_for_owner(conn, &owner) {
        return Err(PendingDomCommandStartError {
            code: -31998,
            message: "BrowserContextNotLoaded".to_owned(),
        });
    }
    if action.requires_document_access()
        && let Err(message) = conn
            .ensure_document_accessible_for_route(owner.session_id(), owner.session_owner_route())
    {
        return Err(PendingDomCommandStartError {
            code: -32000,
            message,
        });
    }
    if action == DomAction::GetDocument {
        let command = build_cdp_get_document_command(
            conn,
            cmd,
            PendingDomDocumentSnapshotOperation::GetDocument,
        )?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::GetDocument(command),
        );
    }
    if action == DomAction::GetFlattenedDocument {
        let command = build_cdp_get_document_command(
            conn,
            cmd,
            PendingDomDocumentSnapshotOperation::GetFlattenedDocument,
        )?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::GetDocument(command),
        );
    }
    if action == DomAction::RequestChildNodes {
        let command = build_cdp_request_child_nodes_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::RequestChildNodes(command),
        );
    }
    if action == DomAction::QuerySelector {
        let command = build_cdp_query_selector_command(conn, cmd, false)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::QuerySelector(command),
        );
    }
    if action == DomAction::QuerySelectorAll {
        let command = build_cdp_query_selector_command(conn, cmd, true)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::QuerySelector(command),
        );
    }
    if action == DomAction::PerformSearch {
        let command = search::build_cdp_perform_search_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::PerformSearch(command),
        );
    }
    if action == DomAction::GetSearchResults {
        let command = search::build_cdp_get_search_results_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::GetSearchResults(command),
        );
    }
    if action == DomAction::DiscardSearchResults {
        let Some(command) = search::build_cdp_discard_search_results_command(conn, cmd) else {
            return Ok(None);
        };
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::DiscardSearchResults(command),
        );
    }
    if action == DomAction::SetNodeStackTracesEnabled {
        return stack_traces::start_set_node_stack_traces_enabled_command(conn, cmd);
    }
    if action == DomAction::GetNodeStackTraces {
        return stack_traces::start_get_node_stack_traces_command(conn, cmd);
    }
    if action == DomAction::ResolveNode {
        let command = build_cdp_resolve_node_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::ResolveNode(command),
        );
    }
    if action == DomAction::GetFrameOwner {
        let command = build_cdp_get_frame_owner_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::GetFrameOwner(command),
        );
    }
    if action == DomAction::GetAttributes {
        let command = build_cdp_get_attributes_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::GetAttributes(command),
        );
    }
    if action == DomAction::GetNodeForLocation {
        let command = build_cdp_get_node_for_location_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::GetNodeForLocation(command),
        );
    }
    if action == DomAction::RequestNode {
        let command = build_cdp_dom_object_reference_command(
            conn,
            cmd,
            DevToolsDomObjectReferenceOperation::RequestNode,
        )?
        .ok_or_else(PendingDomCommandStartError::invalid_params)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::DomObjectReference(command),
        );
    }
    if action == DomAction::DescribeNode {
        let Some(command) = build_cdp_describe_node_command(conn, cmd)? else {
            let params: DescribeNodeParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            let command = build_cdp_dom_object_reference_command(
                conn,
                cmd,
                DevToolsDomObjectReferenceOperation::DescribeNode {
                    depth: params.depth,
                    pierce: params.pierce,
                },
            )?
            .ok_or_else(PendingDomCommandStartError::invalid_params)?;
            return start_devtools_dom_command_for_owner(
                conn,
                cmd.id,
                &owner,
                DevToolsCommand::DomObjectReference(command),
            );
        };
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::DescribeNode(command),
        );
    }
    if action == DomAction::GetOuterHtml {
        let params: GetOuterHtmlParams = match cmd.get_params() {
            Ok(Some(params)) => params,
            _ => return Err(PendingDomCommandStartError::invalid_params()),
        };
        if params.reference.object_id.is_some() {
            let Some(command) = build_cdp_dom_object_reference_command(
                conn,
                cmd,
                DevToolsDomObjectReferenceOperation::GetOuterHtml {
                    include_shadow_dom: params.include_shadow_dom,
                },
            )?
            else {
                return Ok(None);
            };
            return start_devtools_dom_command_for_owner(
                conn,
                cmd.id,
                &owner,
                DevToolsCommand::DomObjectReference(command),
            );
        }
        let Some(command) = build_cdp_get_outer_html_command(conn, cmd)? else {
            return Ok(None);
        };
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::GetOuterHtml(command),
        );
    }
    if action == DomAction::ScrollIntoViewIfNeeded {
        let params: ScrollIntoViewIfNeededParams = match cmd.get_params() {
            Ok(Some(params)) => params,
            _ => return Err(PendingDomCommandStartError::invalid_params()),
        };
        if params.reference.object_id.is_some() {
            let rect = validated_scroll_into_view_rect(params.rect)?;
            let Some(command) = build_cdp_dom_object_reference_command(
                conn,
                cmd,
                DevToolsDomObjectReferenceOperation::ScrollIntoViewIfNeeded { rect },
            )?
            else {
                return Ok(None);
            };
            return start_devtools_dom_command_for_owner(
                conn,
                cmd.id,
                &owner,
                DevToolsCommand::DomObjectReference(command),
            );
        }
        let Some(command) = build_cdp_scroll_into_view_if_needed_command(conn, cmd)? else {
            return Ok(None);
        };
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::ScrollIntoViewIfNeeded(command),
        );
    }
    if action == DomAction::Focus {
        return start_cdp_dom_focus_command(conn, cmd).map(Some);
    }
    if action == DomAction::SetFileInputFiles {
        let Some(command) = set_file_input::build_cdp_set_file_input_files_command(conn, cmd)?
        else {
            return set_file_input::start_cdp_set_file_input_files_by_node_reference(conn, cmd);
        };
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::SetFileInputFiles(command),
        );
    }
    if matches!(
        action,
        DomAction::SetAttributeValue | DomAction::RemoveAttribute
    ) {
        return start_cdp_dom_attribute_mutation_command(conn, cmd, action).map(Some);
    }
    if matches!(
        action,
        DomAction::MoveTo
            | DomAction::SetAttributesAsText
            | DomAction::SetNodeName
            | DomAction::SetNodeValue
            | DomAction::SetOuterHtml
    ) {
        return start_cdp_dom_edit_command(conn, cmd, action).map(Some);
    }
    if action == DomAction::PushNodesByBackendIdsToFrontend {
        let command = build_cdp_push_nodes_by_backend_ids_command(conn, cmd)?;
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::PushNodesByBackendIds(command),
        );
    }
    if matches!(action, DomAction::GetBoxModel | DomAction::GetContentQuads) {
        let operation = match action {
            DomAction::GetBoxModel => DevToolsDomGeometryOperation::GetBoxModel,
            DomAction::GetContentQuads => DevToolsDomGeometryOperation::GetContentQuads,
            _ => unreachable!("guarded by matches! above"),
        };
        let Some(command) = build_cdp_dom_geometry_command(conn, cmd, operation)? else {
            let operation = match action {
                DomAction::GetBoxModel => DevToolsDomObjectReferenceOperation::GetBoxModel,
                DomAction::GetContentQuads => DevToolsDomObjectReferenceOperation::GetContentQuads,
                _ => unreachable!("guarded by matches! above"),
            };
            let command = build_cdp_dom_object_reference_command(conn, cmd, operation)?
                .ok_or_else(PendingDomCommandStartError::invalid_params)?;
            return start_devtools_dom_command_for_owner(
                conn,
                cmd.id,
                &owner,
                DevToolsCommand::DomObjectReference(command),
            );
        };
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::DomGeometry(command),
        );
    }
    if action == DomAction::RemoveNode {
        let Some(command) = build_cdp_remove_node_command(conn, cmd)? else {
            return Ok(None);
        };
        return start_devtools_dom_command_for_owner(
            conn,
            cmd.id,
            &owner,
            DevToolsCommand::RemoveNode(command),
        );
    }

    Ok(None)
}

fn start_cdp_dom_focus_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<PendingDomCommandDispatch, PendingDomCommandStartError> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let params: NodeReferenceParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => NodeReferenceParams::default(),
        Err(_) => return Err(PendingDomCommandStartError::invalid_params()),
    };
    if params.node_id.is_none() && params.backend_node_id.is_none() {
        let object_id = params
            .object_id
            .ok_or_else(|| PendingDomCommandStartError {
                code: -32000,
                message: "Either nodeId, backendNodeId or objectId must be specified".to_owned(),
            })?;
        return start_dom_object_reference_operation(
            conn,
            cmd.id,
            &owner,
            DevToolsRemoteHandleId::from(object_id),
            PendingDomObjectReferenceOperation::Focus,
        )?
        .ok_or_else(PendingDomCommandStartError::node_not_found);
    }

    let reference = devtools_node_reference_from_ids(params.node_id, params.backend_node_id)
        .ok_or_else(PendingDomCommandStartError::invalid_params)?;
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = reference {
        return start_document_frontend_node_binding_command(
            conn,
            cmd.id,
            &owner,
            frontend_node_id,
            PendingDomCommandKind::ResolveFrontendNodeForFocus { frontend_node_id },
        )?
        .ok_or_else(PendingDomCommandStartError::node_not_found);
    }
    let page = loaded_page_mut_for_owner(conn, &owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = start_focus_document_node_for_reference(page, reference)?;
    Ok(PendingDomCommandDispatch {
        command_id: cmd.id,
        owner_scope: owner,
        kind: PendingDomCommandKind::Focus {
            missing_node_message: "No node found for given backend id",
        },
        pending: PendingDomCommandWork::Page(pending),
    })
}

fn start_focus_document_node_for_reference(
    page: &Page,
    reference: DevToolsDomNodeReference,
) -> Result<PendingPageCommand, PendingDomCommandStartError> {
    match reference {
        DevToolsDomNodeReference::FrontendNodeId(_) => {
            Err(PendingDomCommandStartError::node_not_found())
        }
        DevToolsDomNodeReference::BackendNodeId(backend_node_id) => page
            .start_focus_document_backend_node_id(backend_node_id)
            .map_err(PendingDomCommandStartError::renderer_error),
    }
}

fn start_cdp_dom_attribute_mutation_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: DomAction,
) -> Result<PendingDomCommandDispatch, PendingDomCommandStartError> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let (frontend_node_id, mutation) = match action {
        DomAction::SetAttributeValue => {
            let params: SetAttributeValueParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            let Some(frontend_node_id) = cdp_id_from_i64(*params.node_id.inner()) else {
                return Err(PendingDomCommandStartError::invalid_params());
            };
            (
                frontend_node_id,
                RendererDomAttributeMutation::Set {
                    name: params.name,
                    value: params.value,
                },
            )
        }
        DomAction::RemoveAttribute => {
            let params: RemoveAttributeParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            let Some(frontend_node_id) = cdp_id_from_i64(*params.node_id.inner()) else {
                return Err(PendingDomCommandStartError::invalid_params());
            };
            (
                frontend_node_id,
                RendererDomAttributeMutation::Remove { name: params.name },
            )
        }
        _ => unreachable!("attribute mutation command requires an attribute mutation action"),
    };

    start_document_frontend_node_binding_command(
        conn,
        cmd.id,
        &owner,
        frontend_node_id,
        PendingDomCommandKind::ResolveFrontendNodeForMutateAttribute { mutation },
    )?
    .ok_or_else(PendingDomCommandStartError::node_not_found)
}

fn start_cdp_dom_edit_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: DomAction,
) -> Result<PendingDomCommandDispatch, PendingDomCommandStartError> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let edit = super::edit::renderer_dom_edit_from_cdp(cmd, action)?;
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let page = loaded_page_mut_for_owner(conn, &owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_edit_document_node(renderer_inspector_session_id, edit)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(PendingDomCommandDispatch {
        command_id: cmd.id,
        owner_scope: owner,
        kind: PendingDomCommandKind::EditDocumentNode,
        pending: PendingDomCommandWork::Page(pending),
    })
}

fn start_mutate_document_node_for_reference(
    page: &Page,
    reference: DevToolsDomNodeReference,
    mutation: RendererDomAttributeMutation,
) -> Result<(PendingPageCommand, PendingDomCommandKind), PendingDomCommandStartError> {
    match reference {
        DevToolsDomNodeReference::FrontendNodeId(_) => {
            Err(PendingDomCommandStartError::node_not_found())
        }
        DevToolsDomNodeReference::BackendNodeId(backend_node_id) => page
            .start_mutate_document_backend_node_attribute(backend_node_id, mutation)
            .map(|pending| (pending, PendingDomCommandKind::MutateAttribute))
            .map_err(PendingDomCommandStartError::renderer_error),
    }
}

fn start_remove_document_node_for_reference(
    page: &Page,
    reference: DevToolsDomNodeReference,
) -> Result<PendingPageCommand, PendingDomCommandStartError> {
    match reference {
        DevToolsDomNodeReference::FrontendNodeId(_) => {
            Err(PendingDomCommandStartError::node_not_found())
        }
        DevToolsDomNodeReference::BackendNodeId(backend_node_id) => page
            .start_remove_document_backend_node_id(backend_node_id)
            .map_err(PendingDomCommandStartError::renderer_error),
    }
}

fn build_cdp_remove_node_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<DevToolsRemoveNodeCommand>, PendingDomCommandStartError> {
    let params: NodeReferenceParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    if params.object_id.is_some() {
        return Ok(None);
    }
    let reference = devtools_node_reference_from_ids(params.node_id, params.backend_node_id)
        .ok_or_else(PendingDomCommandStartError::invalid_params)?;
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(Some(DevToolsRemoveNodeCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference,
    }))
}

fn start_devtools_remove_node_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsRemoveNodeCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = command.reference {
        return start_document_frontend_node_binding_command(
            conn,
            command_id,
            owner,
            frontend_node_id,
            PendingDomCommandKind::ResolveFrontendNodeForRemoveNode { frontend_node_id },
        );
    }
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = start_remove_document_node_for_reference(page, command.reference)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::RemoveNode,
        pending: PendingDomCommandWork::Page(pending),
    }))
}

fn build_cdp_dom_geometry_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    operation: DevToolsDomGeometryOperation,
) -> Result<Option<DevToolsDomGeometryCommand>, PendingDomCommandStartError> {
    let params: NodeReferenceParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    if params.object_id.is_some() {
        return Ok(None);
    }
    let reference = devtools_node_reference_from_ids(params.node_id, params.backend_node_id)
        .ok_or_else(PendingDomCommandStartError::invalid_params)?;
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(Some(DevToolsDomGeometryCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference,
        operation,
    }))
}

fn start_devtools_dom_geometry_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsDomGeometryCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = command.reference {
        return start_document_frontend_node_binding_command(
            conn,
            command_id,
            owner,
            frontend_node_id,
            PendingDomCommandKind::ResolveFrontendNodeForDomGeometry {
                frontend_node_id,
                operation: command.operation,
            },
        );
    }
    let reference = command.reference;
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let (pending, kind) = start_client_rect_for_reference(page, reference, command.operation)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind,
        pending: PendingDomCommandWork::Page(pending),
    }))
}

fn build_cdp_describe_node_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<DevToolsDescribeNodeCommand>, PendingDomCommandStartError> {
    let params: DescribeNodeParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    if params.reference.object_id.is_some() {
        return Ok(None);
    }
    let reference = devtools_node_reference_from_ids(
        params.reference.node_id,
        params.reference.backend_node_id,
    );
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(Some(DevToolsDescribeNodeCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference,
        depth: params.depth,
        pierce: params.pierce,
    }))
}

fn start_devtools_describe_node_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsDescribeNodeCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let Some(reference) = command.reference else {
        return Err(PendingDomCommandStartError::node_not_found());
    };
    let top_frame_id = top_frame_id_for_owner(conn, owner);
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = reference {
        return start_document_frontend_node_binding_command(
            conn,
            command_id,
            owner,
            frontend_node_id,
            PendingDomCommandKind::ResolveFrontendNodeForDescribeNode {
                frontend_node_id,
                depth: command.depth,
                pierce: command.pierce,
                top_frame_id,
            },
        );
    }
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let include_whitespace = dom_agent_includes_whitespace_for_route(
        conn,
        owner.session_id(),
        owner.session_owner_route(),
    );
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = start_inspector_document_node_snapshot_for_reference(
        page,
        renderer_inspector_session_id,
        include_whitespace,
        reference,
        command.depth,
        command.pierce,
    )?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::DescribeNodeObjectReference {
            cached_object_node: None,
            top_frame_id,
        },
        pending: PendingDomCommandWork::Page(pending),
    }))
}

fn build_cdp_request_child_nodes_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsRequestChildNodesCommand, PendingDomCommandStartError> {
    let params: RequestChildNodesParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let Some(cdp_node_id_value) = cdp_id_from_i64(*params.node_id.inner()) else {
        return Err(PendingDomCommandStartError::invalid_params());
    };
    let Some(depth) = optional_i64_to_i32(params.depth) else {
        return Err(PendingDomCommandStartError::invalid_params());
    };
    let depth = depth.unwrap_or_else(default_describe_depth);
    if depth == 0 {
        return Err(PendingDomCommandStartError {
            code: -32000,
            message: INVALID_REQUEST_CHILD_NODES_DEPTH_MESSAGE.to_owned(),
        });
    }
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsRequestChildNodesCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference: DevToolsDomNodeReference::FrontendNodeId(cdp_node_id_value),
        depth,
        pierce: params.pierce.unwrap_or(false),
    })
}

fn start_devtools_request_child_nodes_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsRequestChildNodesCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let top_frame_id = top_frame_id_for_owner(conn, owner);
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let include_whitespace = dom_agent_includes_whitespace_for_route(
        conn,
        owner.session_id(),
        owner.session_owner_route(),
    );
    let next_depth = if command.depth > 0 {
        command.depth - 1
    } else {
        command.depth
    };
    match command.reference {
        DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) => {
            start_document_frontend_node_binding_command(
                conn,
                command_id,
                owner,
                frontend_node_id,
                PendingDomCommandKind::ResolveFrontendNodeForRequestChildNodes {
                    depth: next_depth,
                    pierce: command.pierce,
                    top_frame_id,
                },
            )
        }
        DevToolsDomNodeReference::BackendNodeId(backend_node_id) => {
            let page = loaded_page_mut_for_owner(conn, owner)
                .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
            let (pending, kind) = start_request_child_nodes_for_reference(
                page,
                renderer_inspector_session_id,
                include_whitespace,
                DevToolsDomNodeReference::BackendNodeId(backend_node_id),
                next_depth,
                command.pierce,
                top_frame_id,
            )?;
            Ok(Some(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind,
                pending: PendingDomCommandWork::Page(pending),
            }))
        }
    }
}

fn start_request_child_nodes_for_reference(
    page: &Page,
    renderer_inspector_session_id: Option<String>,
    include_whitespace: bool,
    reference: DevToolsDomNodeReference,
    depth: i32,
    pierce: bool,
    top_frame_id: Option<String>,
) -> Result<(PendingPageCommand, PendingDomCommandKind), PendingDomCommandStartError> {
    let DevToolsDomNodeReference::BackendNodeId(backend_node_id) = reference else {
        return Err(PendingDomCommandStartError {
            code: -32000,
            message: "InvalidNode".to_owned(),
        });
    };
    let pending = page
        .start_document_child_node_snapshot_events_for_backend_node_id(
            renderer_inspector_session_id,
            include_whitespace,
            backend_node_id,
            depth,
            pierce,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok((
        pending,
        PendingDomCommandKind::SetChildNodesSnapshotForBackendNode {
            after: PendingSetChildNodesAfter::EmptyResult,
            top_frame_id,
            missing_node_message: "InvalidNode",
        },
    ))
}

fn complete_devtools_request_child_nodes_command(
    _conn: &mut CdpConnection,
    _command: DevToolsRequestChildNodesCommand,
    _out: &mut DomCommandOutput,
) -> Result<(), PendingDomCommandStartError> {
    Err(PendingDomCommandStartError {
        code: -32000,
        message: "RequestChildNodesRequiresPendingRendererCapture".to_owned(),
    })
}

fn build_cdp_query_selector_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    multiple: bool,
) -> Result<DevToolsQuerySelectorCommand, PendingDomCommandStartError> {
    let (cdp_node_id_value, selector) = if multiple {
        let params: QuerySelectorAllParams = match cmd.get_params() {
            Ok(Some(params)) => params,
            _ => return Err(PendingDomCommandStartError::invalid_params()),
        };
        let Some(cdp_node_id_value) = cdp_id_from_i64(*params.node_id.inner()) else {
            return Err(PendingDomCommandStartError::invalid_params());
        };
        (cdp_node_id_value, params.selector)
    } else {
        let params: QuerySelectorParams = match cmd.get_params() {
            Ok(Some(params)) => params,
            _ => return Err(PendingDomCommandStartError::invalid_params()),
        };
        let Some(cdp_node_id_value) = cdp_id_from_i64(*params.node_id.inner()) else {
            return Err(PendingDomCommandStartError::invalid_params());
        };
        (cdp_node_id_value, params.selector)
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsQuerySelectorCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        root: Some(DevToolsDomNodeReference::FrontendNodeId(cdp_node_id_value)),
        selector,
        multiple,
    })
}

fn start_devtools_query_selector_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsQuerySelectorCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let top_frame_id = top_frame_id_for_owner(conn, owner);
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let include_whitespace = dom_agent_includes_whitespace_for_route(
        conn,
        owner.session_id(),
        owner.session_owner_route(),
    );
    let Some(reference) = command.root else {
        let page =
            loaded_page_mut_for_owner(conn, owner).ok_or_else(|| PendingDomCommandStartError {
                code: -32000,
                message: "Could not find node with given id".to_owned(),
            })?;
        let pending = page
            .start_document_query_selector_for_document_in_inspector_session(
                renderer_inspector_session_id,
                include_whitespace,
                command.selector,
                command.multiple,
            )
            .map_err(PendingDomCommandStartError::renderer_error)?;
        return Ok(Some(PendingDomCommandDispatch {
            command_id,
            owner_scope: owner.clone(),
            kind: PendingDomCommandKind::QuerySelectorLive {
                multiple: command.multiple,
            },
            pending: PendingDomCommandWork::Page(pending),
        }));
    };

    match reference {
        DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) => {
            if loaded_page_mut_for_owner(conn, owner).is_none() {
                return Err(PendingDomCommandStartError {
                    code: -32000,
                    message: "Could not find node with given id".to_owned(),
                });
            }
            start_document_frontend_node_binding_command(
                conn,
                command_id,
                owner,
                frontend_node_id,
                PendingDomCommandKind::ResolveFrontendNodeForQuerySelector {
                    selector: command.selector,
                    multiple: command.multiple,
                    top_frame_id,
                },
            )
        }
        DevToolsDomNodeReference::BackendNodeId(root_backend_node_id) => {
            let page = loaded_page_mut_for_owner(conn, owner).ok_or_else(|| {
                PendingDomCommandStartError {
                    code: -32000,
                    message: "Could not find node with given id".to_owned(),
                }
            })?;
            let pending = page
                .start_document_query_selector_for_backend_node_id_in_inspector_session(
                    renderer_inspector_session_id,
                    include_whitespace,
                    root_backend_node_id,
                    command.selector,
                    command.multiple,
                )
                .map_err(PendingDomCommandStartError::renderer_error)?;
            Ok(Some(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::QuerySelectorLive {
                    multiple: command.multiple,
                },
                pending: PendingDomCommandWork::Page(pending),
            }))
        }
    }
}

fn build_cdp_get_document_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    operation: PendingDomDocumentSnapshotOperation,
) -> Result<DevToolsGetDocumentCommand, PendingDomCommandStartError> {
    let params: GetDocumentParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => match operation {
            PendingDomDocumentSnapshotOperation::GetDocument => GetDocumentParams::default(),
            PendingDomDocumentSnapshotOperation::GetFlattenedDocument => GetDocumentParams {
                depth: Some(-1),
                pierce: None,
            },
        },
        Err(_) => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let Some(depth) = optional_i64_to_i32(params.depth) else {
        return Err(PendingDomCommandStartError::invalid_params());
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsGetDocumentCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        depth,
        pierce: params.pierce.unwrap_or(false),
        flattened: matches!(
            operation,
            PendingDomDocumentSnapshotOperation::GetFlattenedDocument
        ),
    })
}

fn start_devtools_dom_command_for_owner(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    match command {
        DevToolsCommand::GetFrameOwner(command) => {
            start_devtools_get_frame_owner_command(conn, command_id, owner, command)
        }
        DevToolsCommand::GetAttributes(command) => {
            start_devtools_get_attributes_command(conn, command_id, owner, command)
        }
        DevToolsCommand::GetText(command) => {
            start_devtools_get_text_command(conn, command_id, owner, command)
        }
        DevToolsCommand::GetProperty(command) => {
            start_devtools_get_property_command(conn, command_id, owner, command)
        }
        DevToolsCommand::GetDocument(command) => {
            start_devtools_get_document_command(conn, command_id, owner, command)
        }
        DevToolsCommand::RequestChildNodes(command) => {
            start_devtools_request_child_nodes_command(conn, command_id, owner, command)
        }
        DevToolsCommand::QuerySelector(command) => {
            start_devtools_query_selector_command(conn, command_id, owner, command)
        }
        DevToolsCommand::PerformSearch(command) => {
            search::start_devtools_perform_search_command(conn, command_id, owner, command)
        }
        DevToolsCommand::GetSearchResults(command) => {
            search::start_devtools_get_search_results_command(conn, command_id, owner, command)
        }
        DevToolsCommand::DiscardSearchResults(command) => {
            search::start_devtools_discard_search_results_command(conn, command_id, owner, command)
        }
        DevToolsCommand::GetNodeForLocation(command) => {
            start_devtools_get_node_for_location_command(conn, command_id, owner, command)
        }
        DevToolsCommand::ResolveNode(command) => {
            start_devtools_resolve_node_command(conn, command_id, owner, command)
        }
        DevToolsCommand::DescribeNode(command) => {
            start_devtools_describe_node_command(conn, command_id, owner, command)
        }
        DevToolsCommand::DomObjectReference(command) => {
            start_devtools_dom_object_reference_command(conn, command_id, owner, command)
        }
        DevToolsCommand::SetFileInputFiles(command) => {
            set_file_input::start_devtools_set_file_input_files_command(
                conn, command_id, owner, command,
            )
        }
        DevToolsCommand::PushNodesByBackendIds(command) => {
            start_devtools_push_nodes_by_backend_ids_command(conn, command_id, owner, command)
        }
        DevToolsCommand::GetOuterHtml(command) => {
            start_devtools_get_outer_html_command(conn, command_id, owner, command)
        }
        DevToolsCommand::DomGeometry(command) => {
            start_devtools_dom_geometry_command(conn, command_id, owner, command)
        }
        DevToolsCommand::ScrollIntoViewIfNeeded(command) => {
            start_devtools_scroll_into_view_if_needed_command(conn, command_id, owner, command)
        }
        DevToolsCommand::RemoveNode(command) => {
            start_devtools_remove_node_command(conn, command_id, owner, command)
        }
        _ => Err(PendingDomCommandStartError {
            code: -32000,
            message: "UnsupportedDevToolsCommand".to_owned(),
        }),
    }
}

fn complete_devtools_dom_command(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
) -> Result<Value, PendingDomCommandStartError> {
    match command {
        DevToolsCommand::GetFrameOwner(command) => {
            complete_devtools_get_frame_owner_command(conn, command)
        }
        DevToolsCommand::GetAttributes(_)
        | DevToolsCommand::GetText(_)
        | DevToolsCommand::GetProperty(_) => Err(PendingDomCommandStartError {
            code: -32000,
            message: "MissingDomCommand".to_owned(),
        }),
        DevToolsCommand::PushNodesByBackendIds(_) => Err(PendingDomCommandStartError {
            code: -32000,
            message: "MissingDomCommand".to_owned(),
        }),
        DevToolsCommand::DescribeNode(_)
        | DevToolsCommand::GetOuterHtml(_)
        | DevToolsCommand::ScrollIntoViewIfNeeded(_) => Err(PendingDomCommandStartError {
            code: -32000,
            message: "MissingDomCommand".to_owned(),
        }),
        _ => Err(PendingDomCommandStartError {
            code: -32000,
            message: "UnsupportedDevToolsCommand".to_owned(),
        }),
    }
}

fn start_devtools_get_document_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetDocumentCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let operation = if command.flattened {
        PendingDomDocumentSnapshotOperation::GetFlattenedDocument
    } else {
        PendingDomDocumentSnapshotOperation::GetDocument
    };
    let top_frame_id = top_frame_id_for_owner(conn, owner);
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let include_whitespace = dom_agent_includes_whitespace_for_route(
        conn,
        owner.session_id(),
        owner.session_owner_route(),
    );
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let depth = match operation {
        PendingDomDocumentSnapshotOperation::GetDocument => command.depth.unwrap_or(2),
        PendingDomDocumentSnapshotOperation::GetFlattenedDocument => command.depth.unwrap_or(-1),
    };
    let pending = page
        .start_document_node_snapshot_for_document(
            renderer_inspector_session_id,
            include_whitespace,
            depth,
            command.pierce,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::DocumentSnapshot {
            operation,
            top_frame_id,
        },
        pending: PendingDomCommandWork::Page(pending),
    }))
}

fn build_cdp_get_frame_owner_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsGetFrameOwnerCommand, PendingDomCommandStartError> {
    let params: GetFrameOwnerParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsGetFrameOwnerCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        frame_id: DevToolsFrameId::new(params.frame_id.as_ref()),
    })
}

fn start_devtools_get_frame_owner_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetFrameOwnerCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let page =
        loaded_page_mut_for_owner(conn, owner).ok_or_else(|| PendingDomCommandStartError {
            code: -32000,
            message: "Frame with the given id does not belong to the target.".to_owned(),
        })?;
    let frame_id = command.frame_id.into_string();
    let pending = page
        .start_child_frame_owner_node_reference(&frame_id, renderer_inspector_session_id)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::GetFrameOwner { frame_id },
        pending: PendingDomCommandWork::Page(pending),
    }))
}

fn complete_devtools_get_frame_owner_command(
    _conn: &mut CdpConnection,
    _command: DevToolsGetFrameOwnerCommand,
) -> Result<Value, PendingDomCommandStartError> {
    Err(PendingDomCommandStartError {
        code: -32000,
        message: "GetFrameOwnerRequiresPendingRendererResolution".to_owned(),
    })
}

fn build_cdp_get_node_for_location_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsGetNodeForLocationCommand, PendingDomCommandStartError> {
    let params: GetNodeForLocationParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsGetNodeForLocationCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        x: params.x as f64,
        y: params.y as f64,
        include_user_agent_shadow_dom: params.include_user_agent_shadow_dom.unwrap_or(false),
        ignore_pointer_events_none: params.ignore_pointer_events_none.unwrap_or(false),
    })
}

fn start_devtools_get_node_for_location_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetNodeForLocationCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let top_frame_id = conn
        .target_session_owner_frame_tree_identity_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        )
        .map(|identity| identity.0)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let inspector_session_id = conn.target_renderer_runtime_inspector_session_id_for_route(
        owner.session_id(),
        owner.session_owner_route(),
    );
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_document_hit_test(
            inspector_session_id,
            command.x,
            command.y,
            command.include_user_agent_shadow_dom,
            command.ignore_pointer_events_none,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::GetNodeForLocation { top_frame_id },
        pending: PendingDomCommandWork::Page(pending),
    }))
}

fn complete_devtools_get_node_for_location_command(
    conn: &mut CdpConnection,
    command: DevToolsGetNodeForLocationCommand,
) -> Result<Value, PendingDomCommandStartError> {
    let _ = (conn, command);
    Err(PendingDomCommandStartError {
        code: -32000,
        message: "GetNodeForLocationRequiresPendingRendererHitTest".to_owned(),
    })
}

fn build_cdp_resolve_node_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsResolveNodeCommand, PendingDomCommandStartError> {
    let params: ResolveNodeParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let reference = devtools_node_reference_from_ids(params.node_id, params.backend_node_id)
        .ok_or_else(|| PendingDomCommandStartError {
            code: -32602,
            message: "InvalidParam".to_owned(),
        })?;
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsResolveNodeCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference,
        execution_context_id: params.execution_context_id,
        object_group: params.object_group,
    })
}

struct PendingResolveRuntimeObjectForReference {
    pending: PendingPageCommand,
    cache_top_frame_id: Option<Option<String>>,
}

fn start_resolve_runtime_object_for_reference(
    page: &Page,
    renderer_inspector_session_id: Option<String>,
    reference: DevToolsDomNodeReference,
    execution_context_id: Option<i64>,
    object_group: Option<&str>,
    top_frame_id: Option<String>,
) -> Result<PendingResolveRuntimeObjectForReference, PendingDomCommandStartError> {
    let backend_node_id = required_backend_node_id_for_reference(&reference)?;
    let pending = page
        .start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
            renderer_inspector_session_id,
            backend_node_id,
            execution_context_id,
            object_group,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(PendingResolveRuntimeObjectForReference {
        pending,
        cache_top_frame_id: Some(top_frame_id),
    })
}

fn start_devtools_resolve_node_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsResolveNodeCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let reference = command.reference;
    let top_frame_id = top_frame_id_for_owner(conn, owner);
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = reference {
        return start_document_frontend_node_binding_command(
            conn,
            command_id,
            owner,
            frontend_node_id,
            PendingDomCommandKind::ResolveFrontendNodeForResolveNode {
                frontend_node_id,
                requested_execution_context_id: command.execution_context_id,
                object_group: command.object_group,
                top_frame_id,
            },
        );
    }
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    if let Some(execution_context_id) = command.execution_context_id {
        let pending = page
            .start_child_frame_id_for_default_execution_context_id(execution_context_id)
            .map_err(PendingDomCommandStartError::renderer_error)?;
        return Ok(Some(PendingDomCommandDispatch {
            command_id,
            owner_scope: owner.clone(),
            kind: PendingDomCommandKind::ResolveNodeExecutionContextFrame {
                reference,
                execution_context_id,
                object_group: command.object_group,
                top_frame_id,
            },
            pending: PendingDomCommandWork::Page(pending),
        }));
    }
    let object_group = command.object_group;
    let resolution = start_resolve_runtime_object_for_reference(
        page,
        renderer_inspector_session_id,
        reference,
        None,
        object_group.as_deref(),
        top_frame_id,
    )?;

    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::ResolveNode {
            object_group,
            cache_top_frame_id: resolution.cache_top_frame_id,
        },
        pending: PendingDomCommandWork::Page(resolution.pending),
    }))
}

fn build_cdp_dom_object_reference_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    operation: DevToolsDomObjectReferenceOperation,
) -> Result<Option<DevToolsDomObjectReferenceCommand>, PendingDomCommandStartError> {
    let object_id = match operation.clone() {
        DevToolsDomObjectReferenceOperation::RequestNode => {
            let params: RequestNodeParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            Some(params.object_id.inner().to_owned())
        }
        DevToolsDomObjectReferenceOperation::GetOuterHtml { .. } => {
            let params: GetOuterHtmlParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            params.reference.object_id
        }
        DevToolsDomObjectReferenceOperation::GetBoxModel
        | DevToolsDomObjectReferenceOperation::GetContentQuads => {
            let params: NodeReferenceParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            params.object_id
        }
        DevToolsDomObjectReferenceOperation::ScrollIntoViewIfNeeded { .. } => {
            let params: ScrollIntoViewIfNeededParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            params.reference.object_id
        }
        DevToolsDomObjectReferenceOperation::DescribeNode { .. } => {
            let params: DescribeNodeParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return Err(PendingDomCommandStartError::invalid_params()),
            };
            params.reference.object_id
        }
    };
    let Some(object_id) = object_id else {
        return Ok(None);
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(Some(DevToolsDomObjectReferenceCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        object_id: DevToolsRemoteHandleId::from(object_id),
        operation,
    }))
}

fn pending_dom_object_reference_operation_from_devtools(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    object_id: &str,
    operation: DevToolsDomObjectReferenceOperation,
) -> PendingDomObjectReferenceOperation {
    match operation {
        DevToolsDomObjectReferenceOperation::RequestNode => {
            PendingDomObjectReferenceOperation::RequestNode
        }
        DevToolsDomObjectReferenceOperation::GetOuterHtml { include_shadow_dom } => {
            PendingDomObjectReferenceOperation::GetOuterHtml { include_shadow_dom }
        }
        DevToolsDomObjectReferenceOperation::GetBoxModel => {
            PendingDomObjectReferenceOperation::GetBoxModel
        }
        DevToolsDomObjectReferenceOperation::GetContentQuads => {
            PendingDomObjectReferenceOperation::GetContentQuads
        }
        DevToolsDomObjectReferenceOperation::ScrollIntoViewIfNeeded { rect } => {
            PendingDomObjectReferenceOperation::ScrollIntoViewIfNeeded { rect }
        }
        DevToolsDomObjectReferenceOperation::DescribeNode { depth, pierce } => {
            PendingDomObjectReferenceOperation::DescribeNode {
                depth,
                pierce,
                cached_object_node: cached_dom_remote_object_node_for_owner(conn, owner, object_id),
                top_frame_id: top_frame_id_for_owner(conn, owner),
            }
        }
    }
}

fn start_devtools_dom_object_reference_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsDomObjectReferenceCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let operation = pending_dom_object_reference_operation_from_devtools(
        conn,
        owner,
        command.object_id.as_str(),
        command.operation,
    );
    start_dom_object_reference_operation(conn, command_id, owner, command.object_id, operation)
}

fn start_dom_object_reference_operation(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    object_id: DevToolsRemoteHandleId,
    operation: PendingDomObjectReferenceOperation,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let reference = dom_object_reference_id_for_owner(conn, owner, &object_id);
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let include_whitespace = dom_agent_includes_whitespace_for_route(
        conn,
        owner.session_id(),
        owner.session_owner_route(),
    );
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let object_id = reference;
    match operation {
        PendingDomObjectReferenceOperation::RequestNode => {
            let pending = page
                .start_document_node_snapshot_for_object_id_in_inspector_session(
                    renderer_inspector_session_id,
                    include_whitespace,
                    &object_id,
                    0,
                    false,
                )
                .map_err(PendingDomCommandStartError::renderer_error)?;
            Ok(Some(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::RequestNodeObjectReference,
                pending: PendingDomCommandWork::Page(pending),
            }))
        }
        PendingDomObjectReferenceOperation::Focus => {
            let pending = page
                .start_focus_document_node_for_object_id(renderer_inspector_session_id, object_id)
                .map_err(PendingDomCommandStartError::renderer_error)?;
            Ok(Some(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::Focus {
                    missing_node_message: "Could not find node with given id",
                },
                pending: PendingDomCommandWork::Page(pending),
            }))
        }
        PendingDomObjectReferenceOperation::GetOuterHtml { include_shadow_dom } => {
            let pending = page
                .start_outer_html_for_object_id_in_inspector_session(
                    renderer_inspector_session_id,
                    &object_id,
                    include_shadow_dom,
                )
                .map_err(PendingDomCommandStartError::renderer_error)?;
            Ok(Some(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::GetOuterHtmlObjectReference,
                pending: PendingDomCommandWork::Page(pending),
            }))
        }
        PendingDomObjectReferenceOperation::DescribeNode {
            depth,
            pierce,
            cached_object_node,
            top_frame_id,
        } => {
            let pending = page
                .start_document_node_snapshot_for_object_id_in_inspector_session(
                    renderer_inspector_session_id,
                    include_whitespace,
                    &object_id,
                    depth,
                    pierce,
                )
                .map_err(PendingDomCommandStartError::renderer_error)?;
            Ok(Some(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::DescribeNodeObjectReference {
                    cached_object_node,
                    top_frame_id,
                },
                pending: PendingDomCommandWork::Page(pending),
            }))
        }
        PendingDomObjectReferenceOperation::GetBoxModel
        | PendingDomObjectReferenceOperation::GetContentQuads => {
            let pending = page
                .start_document_geometry_for_object_id_in_inspector_session(
                    renderer_inspector_session_id,
                    &object_id,
                )
                .map_err(PendingDomCommandStartError::renderer_error)?;
            Ok(Some(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::ObjectReferenceLiveClientRect { operation },
                pending: PendingDomCommandWork::Page(pending),
            }))
        }
        PendingDomObjectReferenceOperation::ScrollIntoViewIfNeeded { rect } => {
            let pending = page
                .start_scroll_node_into_view_if_needed_for_object_id_in_inspector_session(
                    renderer_inspector_session_id,
                    &object_id,
                    rect,
                )
                .map_err(PendingDomCommandStartError::renderer_error)?;
            Ok(Some(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::ScrollIntoViewIfNeededObjectReference,
                pending: PendingDomCommandWork::Page(pending),
            }))
        }
    }
}

fn start_devtools_push_nodes_by_backend_ids_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsPushNodesByBackendIdsCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let renderer_runtime_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let renderer_backend_positions = (0..command.backend_node_ids.len()).collect::<Vec<_>>();
    let node_ids = vec![0; command.backend_node_ids.len()];
    let backend_node_ids = command.backend_node_ids.clone();

    let pending = page
        .start_document_frontend_node_ids_for_backend_node_ids(
            renderer_runtime_inspector_session_id,
            command.backend_node_ids,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::PushNodesByBackendIdsToFrontend {
            backend_node_ids,
            node_ids,
            renderer_backend_positions,
        },
        pending: PendingDomCommandWork::Page(pending),
    }))
}

fn start_devtools_get_outer_html_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetOuterHtmlCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let include_shadow_dom = command.include_shadow_dom;
    let Some(reference) = command.reference else {
        let page = loaded_page_mut_for_owner(conn, owner)
            .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
        let pending = page
            .start_outer_html_for_document(include_shadow_dom)
            .map_err(PendingDomCommandStartError::renderer_error)?;
        return Ok(Some(PendingDomCommandDispatch {
            command_id,
            owner_scope: owner.clone(),
            kind: PendingDomCommandKind::GetOuterHtmlDocument,
            pending: PendingDomCommandWork::Page(pending),
        }));
    };
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = reference {
        return start_document_frontend_node_binding_command(
            conn,
            command_id,
            owner,
            frontend_node_id,
            PendingDomCommandKind::ResolveFrontendNodeForGetOuterHtml {
                frontend_node_id,
                include_shadow_dom,
            },
        );
    }
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let (pending, kind) = start_outer_html_for_reference(page, reference, include_shadow_dom)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind,
        pending: PendingDomCommandWork::Page(pending),
    }))
}

fn complete_pending_dom_command(
    conn: &mut CdpConnection,
    completed: CompletedDomCommandDispatch,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let command_id = completed.command_id;
    let owner_scope = completed.owner_scope.clone();
    let session_id = owner_scope.session_id();
    let completed_work = match completed.completed {
        Ok(completion) => completion,
        Err(error) => {
            out.push_error(-32000, error);
            return DomCommandTaskStep::Complete;
        }
    };
    let CompletedDomCommandWork::Page(completion) = completed_work;
    let completion = *completion;
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            session_id,
            owner_scope.session_owner_route(),
        );
    let include_whitespace = dom_agent_includes_whitespace_for_route(
        conn,
        session_id,
        owner_scope.session_owner_route(),
    );

    let kind = match completed.kind {
        PendingDomCommandKind::PerformSearchLive => {
            return search::complete_perform_search_live(conn, &owner_scope, completion, out);
        }
        PendingDomCommandKind::GetSearchResultsLive => {
            return search::complete_get_search_results_live(conn, &owner_scope, completion, out);
        }
        PendingDomCommandKind::DiscardSearchResultsLive => {
            return search::complete_discard_search_results_live(
                conn,
                &owner_scope,
                completion,
                out,
            );
        }
        PendingDomCommandKind::SetNodeStackTracesEnabled => {
            return stack_traces::complete_set_node_stack_traces_enabled_command(
                conn,
                &owner_scope,
                completion,
                out,
            );
        }
        PendingDomCommandKind::GetNodeStackTraces => {
            return stack_traces::complete_get_node_stack_traces_command(
                conn,
                &owner_scope,
                completion,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForGetAttributes { frontend_node_id } => {
            return complete_frontend_node_binding_for_get_attributes(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForGetText { frontend_node_id } => {
            return complete_frontend_node_binding_for_get_text(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForGetProperty {
            frontend_node_id,
            name,
        } => {
            return complete_frontend_node_binding_for_get_property(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                name,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForDomGeometry {
            frontend_node_id,
            operation,
        } => {
            return complete_frontend_node_binding_for_dom_geometry(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                operation,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForDescribeNode {
            frontend_node_id,
            depth,
            pierce,
            top_frame_id,
        } => {
            return complete_frontend_node_binding_for_describe_node(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                depth,
                pierce,
                top_frame_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForRequestChildNodes {
            depth,
            pierce,
            top_frame_id,
        } => {
            return complete_frontend_node_binding_for_request_child_nodes(
                conn,
                command_id,
                &owner_scope,
                completion,
                depth,
                pierce,
                top_frame_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForRemoveNode { frontend_node_id } => {
            return complete_frontend_node_binding_for_remove_node(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForFocus { frontend_node_id } => {
            return complete_frontend_node_binding_for_focus(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForMutateAttribute { mutation } => {
            return complete_frontend_node_binding_for_mutate_attribute(
                conn,
                command_id,
                &owner_scope,
                completion,
                mutation,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForQuerySelector {
            selector,
            multiple,
            top_frame_id,
        } => {
            return complete_frontend_node_binding_for_query_selector(
                conn,
                command_id,
                &owner_scope,
                completion,
                selector,
                multiple,
                top_frame_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForResolveNode {
            frontend_node_id,
            requested_execution_context_id,
            object_group,
            top_frame_id,
        } => {
            return complete_frontend_node_binding_for_resolve_node(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                requested_execution_context_id,
                object_group,
                top_frame_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForGetOuterHtml {
            frontend_node_id,
            include_shadow_dom,
        } => {
            return complete_frontend_node_binding_for_get_outer_html(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                include_shadow_dom,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForScrollIntoViewIfNeeded {
            frontend_node_id,
            rect,
        } => {
            return complete_frontend_node_binding_for_scroll_into_view_if_needed(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                rect,
                out,
            );
        }
        PendingDomCommandKind::ResolveBidiNodeForSetFileInputFiles {
            object_id,
            files,
            append,
        } => {
            return set_file_input::complete_bidi_node_binding_for_set_file_input_files(
                conn,
                command_id,
                &owner_scope,
                completion,
                object_id,
                files,
                append,
                out,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForSetFileInputFiles {
            frontend_node_id,
            file_paths,
            append,
        } => {
            return set_file_input::complete_frontend_node_binding_for_set_file_input_files(
                conn,
                command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                file_paths,
                append,
                out,
            );
        }
        kind => kind,
    };
    let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
        out.push_error(-32000, "NoDocumentLoaded");
        return DomCommandTaskStep::Complete;
    };

    match kind {
        PendingDomCommandKind::DiscardDomAgentFrontendBindings => {
            match page.finish_discard_dom_agent_frontend_bindings(completion) {
                Ok(()) => out.push_success(),
                Err(error) => {
                    out.push_error(-32000, format!("Could not disable DOM agent: {error}"));
                }
            }
        }
        PendingDomCommandKind::RemoveNode => match page.finish_remove_document_node(completion) {
            Ok(true) => out.push_success(),
            Ok(false) => out.push_error(-32000, "Could not remove node"),
            Err(error) => out.push_error(-32000, format!("Could not remove node: {error}")),
        },
        PendingDomCommandKind::SetFileInputFilesPreflight {
            reference,
            file_paths,
            append,
        } => {
            return set_file_input::complete_preflight(
                page,
                command_id,
                &owner_scope,
                completion,
                reference,
                file_paths,
                append,
                out,
            );
        }
        PendingDomCommandKind::Focus {
            missing_node_message,
        } => match page.finish_focus_document_node_id(completion) {
            Ok(RendererDomFocusOutcome::Focused) => out.push_success(),
            Ok(RendererDomFocusOutcome::NodeNotFound) => {
                out.push_error(-32000, missing_node_message);
            }
            Ok(RendererDomFocusOutcome::NodeNotElement) => {
                out.push_error(-32000, "Node is not an Element");
            }
            Ok(RendererDomFocusOutcome::ElementNotFocusable) => {
                out.push_error(-32000, "Element is not focusable");
            }
            Err(error) => out.push_error(-32000, format!("Could not focus node: {error}")),
        },
        PendingDomCommandKind::MutateAttribute => {
            match page.finish_mutate_document_node_attribute(completion) {
                Ok(RendererDomAttributeMutationOutcome::Applied { .. }) => out.push_success(),
                Ok(RendererDomAttributeMutationOutcome::NodeNotFound) => {
                    out.push_error(-32000, "Could not find node with given id");
                }
                Ok(RendererDomAttributeMutationOutcome::NodeNotElement) => {
                    out.push_error(-32000, "Node is not an Element");
                }
                Ok(RendererDomAttributeMutationOutcome::InvalidName { name }) => {
                    out.push_error(
                        -32000,
                        format!("InvalidCharacterError '{name}' is not a valid attribute name."),
                    );
                }
                Err(error) => {
                    out.push_error(-32000, format!("Could not mutate node attribute: {error}"));
                }
            }
        }
        PendingDomCommandKind::EditDocumentNode => {
            match page.finish_edit_document_node(completion) {
                Ok(RendererDomEditOutcome::Applied {
                    result_frontend_node_id: None,
                }) => out.push_success(),
                Ok(RendererDomEditOutcome::Applied {
                    result_frontend_node_id: Some(node_id),
                }) => out.push_result(json!({ "nodeId": node_id })),
                Ok(RendererDomEditOutcome::NodeNotFound) => {
                    out.push_error(-32000, "Could not find node with given id");
                }
                Ok(RendererDomEditOutcome::NodeNotElement) => {
                    out.push_error(-32000, "Node is not an Element");
                }
                Ok(RendererDomEditOutcome::NodeValueUnsupported) => out.push_error(
                    -32000,
                    "Can only set value of text nodes or processing instructions",
                ),
                Ok(RendererDomEditOutcome::MoveIntoSelfOrDescendant) => {
                    out.push_error(-32000, "Unable to move node into self or descendant");
                }
                Ok(RendererDomEditOutcome::AnchorNotChildOfTarget) => {
                    out.push_error(-32000, "Anchor node must be child of the target element")
                }
                Ok(RendererDomEditOutcome::DetachedNode) => {
                    out.push_error(-32000, "Cannot edit detached node");
                }
                Ok(RendererDomEditOutcome::InvalidName { name }) => out.push_error(
                    -32000,
                    format!("InvalidCharacterError '{name}' is not a valid node name."),
                ),
                Ok(RendererDomEditOutcome::CouldNotParseAttributes) => {
                    out.push_error(-32000, "Could not parse value as attributes");
                }
                Ok(RendererDomEditOutcome::MutationFailed) => {
                    out.push_error(-32000, "Could not edit node");
                }
                Err(error) => {
                    out.push_error(-32000, format!("Could not edit node: {error}"));
                }
            }
        }
        PendingDomCommandKind::SetFileInputFiles => {
            return set_file_input::complete_set_file_input_files(page, completion, out);
        }
        PendingDomCommandKind::SetFileInputFilesObjectReference => {
            return set_file_input::complete_set_file_input_files_object_reference(
                page, completion, out,
            );
        }
        PendingDomCommandKind::RendererBackendNodeClientRect { operation } => {
            return complete_renderer_backend_node_client_rect(operation, page, completion, out);
        }
        PendingDomCommandKind::GetNodeForLocation { top_frame_id } => {
            match finish_document_hit_test(page, completion, top_frame_id) {
                Ok(result) => out.push_result(get_node_for_location_result_value(&result)),
                Err(error) => out.push_error(-32000, error.message),
            }
        }
        PendingDomCommandKind::RendererBackendNodeScrollIntoViewIfNeeded => {
            return complete_renderer_backend_node_scroll_into_view_if_needed(
                page, completion, out,
            );
        }
        PendingDomCommandKind::PushNodesByBackendIdsToFrontend {
            backend_node_ids,
            node_ids,
            renderer_backend_positions,
        } => {
            return complete_push_nodes_by_backend_ids_to_frontend(
                page,
                completion,
                session_id,
                backend_node_ids,
                node_ids,
                renderer_backend_positions,
                out,
            );
        }
        PendingDomCommandKind::GetFrameOwner { frame_id } => {
            match page.finish_document_node_reference(completion) {
                Ok(Some(reference)) => {
                    let result = get_frame_owner_result_from_node_reference(reference);
                    out.push_result(get_frame_owner_result_value(&result));
                }
                Ok(None) => out.push_error(
                    -32000,
                    "Frame with the given id does not belong to the target.",
                ),
                Err(error) => out.push_error(
                    -32000,
                    format!("Could not resolve frame owner for {frame_id}: {error}"),
                ),
            }
        }
        PendingDomCommandKind::QuerySelectorLive { multiple } => {
            return complete_query_selector_live(page, completion, multiple, out);
        }
        PendingDomCommandKind::GetAttributesLive => {
            return complete_get_attributes_live(page, completion, out);
        }
        PendingDomCommandKind::GetTextLive => {
            return complete_get_text_live(page, completion, out);
        }
        PendingDomCommandKind::GetPropertyLive => {
            return complete_get_property_live(page, completion, out);
        }
        PendingDomCommandKind::RequestNodeObjectReference => {
            return complete_request_node_object_reference(session_id, page, completion, out);
        }
        PendingDomCommandKind::GetOuterHtmlDocument => {
            return complete_get_outer_html_document(page, completion, out);
        }
        PendingDomCommandKind::GetOuterHtmlObjectReference => {
            return complete_get_outer_html_object_reference(page, completion, out);
        }
        PendingDomCommandKind::GetOuterHtmlBackendNodeReference => {
            return complete_get_outer_html_backend_node_reference(page, completion, out);
        }
        PendingDomCommandKind::ScrollIntoViewIfNeededObjectReference => {
            return complete_scroll_into_view_if_needed_object_reference(page, completion, out);
        }
        PendingDomCommandKind::DescribeNodeObjectReference {
            cached_object_node,
            top_frame_id,
        } => {
            return complete_describe_node_object_reference(
                page,
                completion,
                cached_object_node,
                top_frame_id,
                out,
            );
        }
        PendingDomCommandKind::ObjectReferenceLiveClientRect { operation } => {
            return complete_object_reference_live_client_rect(operation, page, completion, out);
        }
        PendingDomCommandKind::DocumentSnapshot {
            operation,
            top_frame_id,
        } => {
            return complete_document_snapshot(operation, page, completion, top_frame_id, out);
        }
        PendingDomCommandKind::SetChildNodesSnapshotForBackendNode {
            after,
            top_frame_id,
            missing_node_message,
        } => {
            return complete_set_child_nodes_snapshot_for_backend_node(
                session_id,
                page,
                completion,
                after,
                top_frame_id,
                missing_node_message,
                out,
            );
        }
        PendingDomCommandKind::QuerySelectorSetChildNodesLive {
            multiple,
            top_frame_id,
        } => {
            return complete_query_selector_set_child_nodes_live(
                session_id,
                page,
                completion,
                multiple,
                top_frame_id,
                out,
            );
        }
        PendingDomCommandKind::ResolveNode {
            object_group,
            cache_top_frame_id,
        } => {
            let resolution = page.finish_resolve_runtime_object_for_backend_node_id(completion);
            let remote_object = match resolution {
                Ok(DocumentNodeRuntimeObjectResolution::Found(remote_object)) => remote_object,
                Ok(DocumentNodeRuntimeObjectResolution::MissingContext) => {
                    out.push_error(-32000, "ContextNotFound");
                    return DomCommandTaskStep::Complete;
                }
                Ok(DocumentNodeRuntimeObjectResolution::MissingNode) => {
                    out.push_error(-32000, "Could not find node with given id");
                    return DomCommandTaskStep::Complete;
                }
                Err(error) => {
                    out.push_error(
                        -32000,
                        format!("Could not resolve node runtime object: {error}"),
                    );
                    return DomCommandTaskStep::Complete;
                }
            };
            let remote_object = remote_object.into_protocol_value();
            if let Some(top_frame_id) = cache_top_frame_id
                && let Some(cache_object_id) = remote_object
                    .get("objectId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            {
                return match page.start_document_node_snapshot_for_object_id_in_inspector_session(
                    renderer_inspector_session_id.clone(),
                    include_whitespace,
                    &cache_object_id,
                    0,
                    false,
                ) {
                    Ok(pending) => {
                        DomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
                            command_id,
                            owner_scope: owner_scope.clone(),
                            kind: PendingDomCommandKind::ResolveNodeCacheSnapshot {
                                remote_object: Box::new(remote_object),
                                object_group,
                                cache_object_id,
                                top_frame_id,
                            },
                            pending: PendingDomCommandWork::Page(pending),
                        }))
                    }
                    Err(error) => {
                        out.push_error(
                            -32000,
                            format!("Could not snapshot resolved node: {error}"),
                        );
                        DomCommandTaskStep::Complete
                    }
                };
            }
            push_resolve_node_result(conn, &owner_scope, out, remote_object, object_group);
        }
        PendingDomCommandKind::ResolveNodeCacheSnapshot {
            remote_object,
            object_group,
            cache_object_id,
            top_frame_id,
        } => {
            match page.finish_document_node_snapshot_for_object_id(completion) {
                Ok(Some(snapshot)) => cache_resolved_node_snapshot(
                    conn,
                    &owner_scope,
                    cache_object_id,
                    &snapshot.snapshot,
                    top_frame_id.as_deref(),
                ),
                Ok(None) => {}
                Err(error) => {
                    out.push_error(-32000, format!("Could not snapshot resolved node: {error}"));
                    return DomCommandTaskStep::Complete;
                }
            }
            push_resolve_node_result(conn, &owner_scope, out, *remote_object, object_group);
        }
        PendingDomCommandKind::ResolveNodeExecutionContextFrame {
            reference,
            execution_context_id,
            object_group,
            top_frame_id,
        } => {
            let child_frame_id =
                match page.finish_child_frame_id_for_default_execution_context_id(completion) {
                    Ok(frame_id) => frame_id,
                    Err(error) => {
                        out.push_error(
                            -32000,
                            format!("Could not resolve execution context frame: {error}"),
                        );
                        return DomCommandTaskStep::Complete;
                    }
                };
            if child_frame_id.is_some() {
                if let DevToolsDomNodeReference::BackendNodeId(backend_node_id) = reference
                    && is_renderer_backend_node_id(backend_node_id)
                {
                    return match page
                        .start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
                            renderer_inspector_session_id.clone(),
                            backend_node_id,
                            Some(execution_context_id),
                            object_group.as_deref(),
                        ) {
                        Ok(pending) => {
                            DomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
                                command_id,
                                owner_scope: owner_scope.clone(),
                                kind: PendingDomCommandKind::ResolveNode {
                                    object_group,
                                    cache_top_frame_id: None,
                                },
                                pending: PendingDomCommandWork::Page(pending),
                            }))
                        }
                        Err(error) => {
                            out.push_error(
                                -32000,
                                format!("Could not resolve node runtime object: {error}"),
                            );
                            DomCommandTaskStep::Complete
                        }
                    };
                }
                out.push_error(-32000, "Could not find node with given id");
                return DomCommandTaskStep::Complete;
            }
            return match start_resolve_runtime_object_for_reference(
                page,
                renderer_inspector_session_id.clone(),
                reference,
                Some(execution_context_id),
                object_group.as_deref(),
                top_frame_id,
            ) {
                Ok(resolution) => {
                    DomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
                        command_id,
                        owner_scope: owner_scope.clone(),
                        kind: PendingDomCommandKind::ResolveNode {
                            object_group,
                            cache_top_frame_id: resolution.cache_top_frame_id,
                        },
                        pending: PendingDomCommandWork::Page(resolution.pending),
                    }))
                }
                Err(error) => {
                    out.push_error(error.code, error.message);
                    DomCommandTaskStep::Complete
                }
            };
        }
        PendingDomCommandKind::PerformSearchLive
        | PendingDomCommandKind::GetSearchResultsLive
        | PendingDomCommandKind::DiscardSearchResultsLive
        | PendingDomCommandKind::SetNodeStackTracesEnabled
        | PendingDomCommandKind::GetNodeStackTraces
        | PendingDomCommandKind::ResolveFrontendNodeForGetAttributes { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForGetText { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForGetProperty { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForDomGeometry { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForDescribeNode { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForRequestChildNodes { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForRemoveNode { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForFocus { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForMutateAttribute { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForQuerySelector { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForResolveNode { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForGetOuterHtml { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForScrollIntoViewIfNeeded { .. }
        | PendingDomCommandKind::ResolveBidiNodeForSetFileInputFiles { .. }
        | PendingDomCommandKind::ResolveFrontendNodeForSetFileInputFiles { .. } => {
            unreachable!(
                "DOM search/frontend/shared-node lookup pending commands are completed before borrowing page"
            )
        }
    }
    DomCommandTaskStep::Complete
}

pub(crate) async fn complete_pending_dom_command_output_plan(
    conn: &mut CdpConnection,
    completed: CompletedDomCommandDispatch,
) -> (DomCommandTaskStep, CommandOutputPlan) {
    let mut out = DomCommandOutput::default();
    if let Some(predecessor) = completed.renderer_output_predecessor() {
        // A DOM command may consist of several renderer commands (for
        // example, resolve a frontend node and then mutate it). Every segment
        // owns the concrete output it produced. Preserve its exact fence even
        // when this completion starts another pending segment; the outer
        // command context merges same-stream fences and releases the response
        // only after the final frontier has projected.
        out.set_renderer_output_predecessor(predecessor);
    }
    let step = complete_pending_dom_command(conn, completed, &mut out);
    (step, out.into_plan())
}

fn complete_pending_dom_command_result(
    conn: &mut CdpConnection,
    completed: CompletedDomCommandDispatch,
) -> DevToolsDomCommandTaskStep {
    let owner_scope = completed.owner_scope.clone();
    let session_id = owner_scope.session_id();
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            session_id,
            owner_scope.session_owner_route(),
        );
    let include_whitespace = dom_agent_includes_whitespace_for_route(
        conn,
        session_id,
        owner_scope.session_owner_route(),
    );
    let completed_work = match completed.completed {
        Ok(completion) => completion,
        Err(error) => {
            return devtools_dom_command_task_complete(Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                error,
            )));
        }
    };
    let CompletedDomCommandWork::Page(completion) = completed_work;
    let completion = *completion;

    let result = match completed.kind {
        PendingDomCommandKind::ResolveFrontendNodeForGetAttributes { frontend_node_id } => {
            return complete_frontend_node_binding_for_get_attributes_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForGetText { frontend_node_id } => {
            return complete_frontend_node_binding_for_get_text_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForGetProperty {
            frontend_node_id,
            name,
        } => {
            return complete_frontend_node_binding_for_get_property_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                name,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForDomGeometry {
            frontend_node_id,
            operation,
        } => {
            return complete_frontend_node_binding_for_dom_geometry_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                operation,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForDescribeNode {
            frontend_node_id,
            depth,
            pierce,
            top_frame_id,
        } => {
            return complete_frontend_node_binding_for_describe_node_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                depth,
                pierce,
                top_frame_id,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForRemoveNode { frontend_node_id } => {
            return complete_frontend_node_binding_for_remove_node_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForRequestChildNodes {
            depth,
            pierce,
            top_frame_id,
        } => {
            return complete_frontend_node_binding_for_request_child_nodes_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                depth,
                pierce,
                top_frame_id,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForQuerySelector {
            selector, multiple, ..
        } => {
            return complete_frontend_node_binding_for_query_selector_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                selector,
                multiple,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForResolveNode {
            frontend_node_id,
            requested_execution_context_id,
            object_group,
            top_frame_id,
        } => {
            return complete_frontend_node_binding_for_resolve_node_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                requested_execution_context_id,
                object_group,
                top_frame_id,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForGetOuterHtml {
            frontend_node_id,
            include_shadow_dom,
        } => {
            return complete_frontend_node_binding_for_get_outer_html_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                include_shadow_dom,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForScrollIntoViewIfNeeded {
            frontend_node_id,
            rect,
        } => {
            return complete_frontend_node_binding_for_scroll_into_view_if_needed_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                rect,
            );
        }
        PendingDomCommandKind::ResolveBidiNodeForSetFileInputFiles {
            object_id,
            files,
            append,
        } => {
            return set_file_input::complete_bidi_node_binding_for_set_file_input_files_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                object_id,
                files,
                append,
            );
        }
        PendingDomCommandKind::ResolveFrontendNodeForSetFileInputFiles {
            frontend_node_id,
            file_paths,
            append,
        } => {
            return set_file_input::complete_frontend_node_binding_for_set_file_input_files_result(
                conn,
                completed.command_id,
                &owner_scope,
                completion,
                frontend_node_id,
                file_paths,
                append,
            );
        }
        PendingDomCommandKind::SetFileInputFiles => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            set_file_input::complete_set_file_input_files_result(page, completion)
                .map(|()| DevToolsCommandResult::Empty)
        }
        PendingDomCommandKind::SetFileInputFilesObjectReference => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            set_file_input::complete_set_file_input_files_object_reference_result(page, completion)
                .map(|()| DevToolsCommandResult::Empty)
        }
        PendingDomCommandKind::GetNodeForLocation { top_frame_id } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            finish_document_hit_test(page, completion, top_frame_id)
                .map(DevToolsCommandResult::GetNodeForLocation)
        }
        PendingDomCommandKind::RendererBackendNodeClientRect { operation } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_document_node_geometry_result(
                page.finish_document_geometry_for_backend_node_id(completion),
                operation,
                "Could not resolve node geometry",
            )
            .map(DevToolsCommandResult::DomGeometry)
        }
        PendingDomCommandKind::RendererBackendNodeScrollIntoViewIfNeeded => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_renderer_backend_node_scroll_into_view_if_needed_result(page, completion)
                .map(|()| DevToolsCommandResult::Empty)
        }
        PendingDomCommandKind::PushNodesByBackendIdsToFrontend {
            backend_node_ids,
            node_ids,
            renderer_backend_positions,
        } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            match complete_push_nodes_by_backend_ids_to_frontend_result(
                page,
                completion,
                backend_node_ids,
                node_ids,
                renderer_backend_positions,
            ) {
                Ok(result) => Ok(DevToolsCommandResult::PushNodesByBackendIds(result)),
                Err(error) => Err(error),
            }
        }
        PendingDomCommandKind::GetFrameOwner { frame_id } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_get_frame_owner_result(page, completion, &frame_id)
                .map(DevToolsCommandResult::GetFrameOwner)
        }
        PendingDomCommandKind::QuerySelectorLive { multiple } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_query_selector_live_result(page, completion, multiple)
                .map(DevToolsCommandResult::QuerySelector)
        }
        PendingDomCommandKind::QuerySelectorSetChildNodesLive { multiple, .. } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_query_selector_set_child_nodes_live_result(page, completion, multiple)
                .map(DevToolsCommandResult::QuerySelector)
        }
        PendingDomCommandKind::GetAttributesLive => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_get_attributes_live_result(page, completion)
                .map(DevToolsCommandResult::GetAttributes)
        }
        PendingDomCommandKind::GetTextLive => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_get_text_live_result(page, completion).map(DevToolsCommandResult::GetText)
        }
        PendingDomCommandKind::GetPropertyLive => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_get_property_live_result(page, completion)
                .map(DevToolsCommandResult::GetProperty)
        }
        PendingDomCommandKind::ResolveNode {
            object_group,
            cache_top_frame_id,
        } => {
            let remote_object = {
                let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                    return devtools_dom_command_task_complete(Err(DevToolsError::new(
                        DevToolsErrorKind::Internal,
                        "NoDocumentLoaded",
                    )));
                };
                match complete_resolve_node_page_result(page, completion) {
                    Ok(result) => result,
                    Err(error) => return devtools_dom_command_task_complete(Err(error)),
                }
            };
            if let Some(top_frame_id) = cache_top_frame_id
                && let Some(cache_object_id) = remote_object
                    .get("objectId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            {
                let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                    return devtools_dom_command_task_complete(Err(DevToolsError::new(
                        DevToolsErrorKind::Internal,
                        "NoDocumentLoaded",
                    )));
                };
                return match page.start_document_node_snapshot_for_object_id_in_inspector_session(
                    renderer_inspector_session_id.clone(),
                    include_whitespace,
                    &cache_object_id,
                    0,
                    false,
                ) {
                    Ok(pending) => {
                        DevToolsDomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
                            command_id: None,
                            owner_scope: owner_scope.clone(),
                            kind: PendingDomCommandKind::ResolveNodeCacheSnapshot {
                                remote_object: Box::new(remote_object),
                                object_group,
                                cache_object_id,
                                top_frame_id,
                            },
                            pending: PendingDomCommandWork::Page(pending),
                        }))
                    }
                    Err(error) => devtools_dom_command_task_complete(Err(DevToolsError::new(
                        DevToolsErrorKind::Internal,
                        format!("Could not snapshot resolved node: {error}"),
                    ))),
                };
            }
            Ok(DevToolsCommandResult::ResolveNode(
                register_resolve_node_result(conn, &owner_scope, remote_object, object_group),
            ))
        }
        PendingDomCommandKind::ResolveNodeCacheSnapshot {
            remote_object,
            object_group,
            cache_object_id,
            top_frame_id,
        } => {
            {
                let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                    return devtools_dom_command_task_complete(Err(DevToolsError::new(
                        DevToolsErrorKind::Internal,
                        "NoDocumentLoaded",
                    )));
                };
                match page.finish_document_node_snapshot_for_object_id(completion) {
                    Ok(Some(snapshot)) => cache_resolved_node_snapshot(
                        conn,
                        &owner_scope,
                        cache_object_id,
                        &snapshot.snapshot,
                        top_frame_id.as_deref(),
                    ),
                    Ok(None) => {}
                    Err(error) => {
                        return devtools_dom_command_task_complete(Err(DevToolsError::new(
                            DevToolsErrorKind::Internal,
                            format!("Could not snapshot resolved node: {error}"),
                        )));
                    }
                }
            }
            Ok(DevToolsCommandResult::ResolveNode(
                register_resolve_node_result(conn, &owner_scope, *remote_object, object_group),
            ))
        }
        PendingDomCommandKind::ResolveNodeExecutionContextFrame {
            reference,
            execution_context_id,
            object_group,
            top_frame_id,
        } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            return complete_resolve_node_execution_context_frame_result(
                &owner_scope,
                renderer_inspector_session_id.clone(),
                page,
                completion,
                reference,
                execution_context_id,
                object_group,
                top_frame_id,
            );
        }
        PendingDomCommandKind::DocumentSnapshot { .. } => Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "UnsupportedDevToolsCommand",
        )),
        PendingDomCommandKind::ObjectReferenceLiveClientRect { operation } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_object_reference_live_client_rect_result(page, completion, operation)
                .map(DevToolsCommandResult::DomGeometry)
        }
        PendingDomCommandKind::GetOuterHtmlObjectReference => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_get_outer_html_object_reference_result(page, completion)
                .map(DevToolsCommandResult::GetOuterHtml)
        }
        PendingDomCommandKind::GetOuterHtmlDocument => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_get_outer_html_document_result(page, completion)
                .map(DevToolsCommandResult::GetOuterHtml)
        }
        PendingDomCommandKind::GetOuterHtmlBackendNodeReference => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_get_outer_html_backend_node_reference_result(page, completion)
                .map(DevToolsCommandResult::GetOuterHtml)
        }
        PendingDomCommandKind::ScrollIntoViewIfNeededObjectReference => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_scroll_into_view_if_needed_object_reference_result(page, completion)
                .map(|()| DevToolsCommandResult::Empty)
        }
        PendingDomCommandKind::DescribeNodeObjectReference {
            cached_object_node,
            top_frame_id,
        } => {
            let Some(page) = loaded_page_mut_for_owner(conn, &owner_scope) else {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "NoDocumentLoaded",
                )));
            };
            complete_describe_node_object_reference_result(
                page,
                completion,
                cached_object_node,
                top_frame_id,
            )
            .map(DevToolsCommandResult::DescribeNode)
        }
        _ => Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "UnsupportedDevToolsDomPendingCommand",
        )),
    };
    devtools_dom_command_task_complete(result)
}

fn complete_scroll_into_view_result(
    result: Result<RendererScrollIntoViewResult, impl std::fmt::Display>,
) -> Result<(), DevToolsError> {
    match result {
        Ok(RendererScrollIntoViewResult::ScrolledOrAlreadyVisible) => Ok(()),
        Ok(RendererScrollIntoViewResult::NodeNotFound) => Err(devtools_dom_node_not_found_error()),
        Ok(RendererScrollIntoViewResult::NodeDetached) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "Node is detached from document",
        )),
        Ok(RendererScrollIntoViewResult::NodeDoesNotHaveLayoutObject) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "Node does not have a layout object",
        )),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not scroll node into view: {error}"),
        )),
    }
}

fn complete_document_node_geometry_result(
    geometry: Result<Option<RendererDocumentNodeGeometry>, impl std::fmt::Display>,
    operation: DevToolsDomGeometryOperation,
    error_prefix: &str,
) -> Result<DevToolsDomGeometryResult, DevToolsError> {
    match geometry {
        Ok(Some(geometry)) => devtools_dom_geometry_result_from_renderer(operation, geometry),
        Ok(None) => Err(devtools_dom_node_not_found_error()),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("{error_prefix}: {error}"),
        )),
    }
}

fn finish_document_hit_test(
    page: &mut Page,
    completion: CompletedPageCommand,
    top_frame_id: String,
) -> Result<DevToolsGetNodeForLocationResult, DevToolsError> {
    match page.finish_document_hit_test(completion) {
        Ok(Some(hit)) => Ok(DevToolsGetNodeForLocationResult {
            backend_node_id: hit.node.backend_node_id,
            frame_id: DevToolsFrameId::from(hit.frame_id.unwrap_or(top_frame_id)),
            node_id: (hit.node.node_id != 0).then_some(hit.node.node_id),
        }),
        Ok(None) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "No node found at given location",
        )),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not hit test document: {error}"),
        )),
    }
}

fn get_node_for_location_result_value(result: &DevToolsGetNodeForLocationResult) -> Value {
    let mut value = json!({
        "backendNodeId": result.backend_node_id,
        "frameId": result.frame_id.as_str(),
    });
    if let Some(node_id) = result.node_id {
        value["nodeId"] = json!(node_id);
    }
    value
}

fn complete_get_frame_owner_result(
    page: &mut Page,
    completion: CompletedPageCommand,
    frame_id: &str,
) -> Result<DevToolsGetFrameOwnerResult, DevToolsError> {
    match page.finish_document_node_reference(completion) {
        Ok(Some(reference)) => Ok(get_frame_owner_result_from_node_reference(reference)),
        Ok(None) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "Frame with the given id does not belong to the target.",
        )),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not resolve frame owner for {frame_id}: {error}"),
        )),
    }
}

fn get_frame_owner_result_from_node_reference(
    reference: RendererDocumentNodeReference,
) -> DevToolsGetFrameOwnerResult {
    DevToolsGetFrameOwnerResult {
        node_id: reference.node_id,
        backend_node_id: reference.backend_node_id,
    }
}

fn get_frame_owner_result_value(result: &DevToolsGetFrameOwnerResult) -> Value {
    json!({
        "nodeId": result.node_id,
        "backendNodeId": result.backend_node_id,
    })
}

fn live_node_not_element_error() -> PendingDomCommandStartError {
    PendingDomCommandStartError {
        code: -32000,
        message: "Node is not an Element".to_owned(),
    }
}

pub(super) fn attributes_result_from_renderer_resolution(
    resolution: RendererDocumentNodeAttributesResolution,
) -> Result<DevToolsGetAttributesResult, PendingDomCommandStartError> {
    match resolution {
        RendererDocumentNodeAttributesResolution::Found(attributes) => {
            Ok(DevToolsGetAttributesResult {
                attributes: attributes
                    .into_iter()
                    .map(|(name, value)| DevToolsDomAttribute { name, value })
                    .collect(),
            })
        }
        RendererDocumentNodeAttributesResolution::NotElement => Err(live_node_not_element_error()),
        RendererDocumentNodeAttributesResolution::MissingNode => {
            Err(PendingDomCommandStartError::node_not_found())
        }
    }
}

pub(super) fn text_result_from_renderer_resolution(
    resolution: RendererDocumentNodeTextResolution,
) -> Result<DevToolsGetTextResult, PendingDomCommandStartError> {
    match resolution {
        RendererDocumentNodeTextResolution::Found(text) => Ok(DevToolsGetTextResult { text }),
        RendererDocumentNodeTextResolution::MissingNode => {
            Err(PendingDomCommandStartError::node_not_found())
        }
    }
}

pub(super) fn property_result_from_renderer_resolution(
    resolution: RendererDocumentNodePropertyResolution,
) -> Result<DevToolsGetPropertyResult, PendingDomCommandStartError> {
    match resolution {
        RendererDocumentNodePropertyResolution::Found(value) => {
            Ok(DevToolsGetPropertyResult { value })
        }
        RendererDocumentNodePropertyResolution::NotElement => Err(live_node_not_element_error()),
        RendererDocumentNodePropertyResolution::MissingNode => {
            Err(PendingDomCommandStartError::node_not_found())
        }
    }
}

pub(super) fn query_selector_result_from_renderer_resolution(
    resolution: RendererDocumentQuerySelectorResolution,
    multiple: bool,
) -> Result<DevToolsQuerySelectorResult, PendingDomCommandStartError> {
    match resolution {
        RendererDocumentQuerySelectorResolution::Found(nodes) => Ok(DevToolsQuerySelectorResult {
            node_ids: nodes
                .into_iter()
                .map(|node| node.frontend_node_id)
                .collect(),
            multiple,
        }),
        RendererDocumentQuerySelectorResolution::MissingRoot => {
            Err(PendingDomCommandStartError::node_not_found())
        }
        RendererDocumentQuerySelectorResolution::InvalidSelector(message) => {
            Err(PendingDomCommandStartError::invalid_selector(message))
        }
    }
}

fn complete_query_selector_live(
    page: &mut Page,
    completion: CompletedPageCommand,
    multiple: bool,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page
        .finish_document_query_selector(completion)
        .map_err(PendingDomCommandStartError::renderer_error)
        .and_then(|resolution| query_selector_result_from_renderer_resolution(resolution, multiple))
    {
        Ok(result) if result.multiple => {
            out.push_result(json!({ "nodeIds": result.node_ids }));
        }
        Ok(result) => {
            out.push_result(json!({ "nodeId": result.node_ids.first().copied().unwrap_or(0) }));
        }
        Err(error) => out.push_error(error.code, error.message),
    }
    DomCommandTaskStep::Complete
}

fn complete_query_selector_live_result(
    page: &mut Page,
    completion: CompletedPageCommand,
    multiple: bool,
) -> Result<DevToolsQuerySelectorResult, DevToolsError> {
    page.finish_document_query_selector(completion)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))
        .and_then(|resolution| {
            query_selector_result_from_renderer_resolution(resolution, multiple)
                .map_err(DevToolsError::from)
        })
}

fn complete_query_selector_set_child_nodes_live_result(
    page: &mut Page,
    completion: CompletedPageCommand,
    multiple: bool,
) -> Result<DevToolsQuerySelectorResult, DevToolsError> {
    page.finish_document_query_selector_with_child_node_snapshot_events(completion)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))
        .and_then(|result| {
            query_selector_set_child_nodes_result_from_renderer_resolution(
                result.query_selector_resolution,
                multiple,
            )
            .map_err(DevToolsError::from)
        })
}

fn complete_frontend_node_binding_reference(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> Result<DevToolsDomNodeReference, DomCommandTaskStep> {
    let Some(page) = loaded_page_mut_for_owner(conn, owner) else {
        out.push_error(-32000, "NoDocumentLoaded");
        return Err(DomCommandTaskStep::Complete);
    };
    match super::frontend_binding::finish_reference(page, completion) {
        Ok(reference) => Ok(reference),
        Err(message) => {
            out.push_error(-32000, message);
            Err(DomCommandTaskStep::Complete)
        }
    }
}

fn complete_frontend_node_binding_reference_result(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
) -> Result<DevToolsDomNodeReference, DevToolsDomCommandTaskStep> {
    let Some(page) = loaded_page_mut_for_owner(conn, owner) else {
        return Err(devtools_dom_command_task_complete(Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "NoDocumentLoaded",
        ))));
    };
    super::frontend_binding::finish_reference(page, completion).map_err(|message| {
        let kind = if message == "Could not find node with given id" {
            DevToolsErrorKind::NoSuchNode
        } else {
            DevToolsErrorKind::Internal
        };
        devtools_dom_command_task_complete(Err(DevToolsError::new(kind, message)))
    })
}

fn complete_frontend_node_binding_followup<F>(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
    start: F,
) -> DomCommandTaskStep
where
    F: FnOnce(
        &Page,
        DevToolsDomNodeReference,
    )
        -> Result<(PendingPageCommand, PendingDomCommandKind), PendingDomCommandStartError>,
{
    let reference = match complete_frontend_node_binding_reference(conn, owner, completion, out) {
        Ok(reference) => reference,
        Err(step) => return step,
    };
    let Some(page) = loaded_page_mut_for_owner(conn, owner) else {
        out.push_error(-32000, "NoDocumentLoaded");
        return DomCommandTaskStep::Complete;
    };
    match start(page, reference) {
        Ok((pending, kind)) => DomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
            command_id,
            owner_scope: owner.clone(),
            kind,
            pending: PendingDomCommandWork::Page(pending),
        })),
        Err(error) => {
            out.push_error(error.code, error.message);
            DomCommandTaskStep::Complete
        }
    }
}

fn complete_frontend_node_binding_followup_result<F>(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    start: F,
) -> DevToolsDomCommandTaskStep
where
    F: FnOnce(
        &Page,
        DevToolsDomNodeReference,
    )
        -> Result<(PendingPageCommand, PendingDomCommandKind), PendingDomCommandStartError>,
{
    let reference = match complete_frontend_node_binding_reference_result(conn, owner, completion) {
        Ok(reference) => reference,
        Err(step) => return step,
    };
    let Some(page) = loaded_page_mut_for_owner(conn, owner) else {
        return devtools_dom_command_task_complete(Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "NoDocumentLoaded",
        )));
    };
    match start(page, reference) {
        Ok((pending, kind)) => {
            DevToolsDomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
                command_id,
                owner_scope: owner.clone(),
                kind,
                pending: PendingDomCommandWork::Page(pending),
            }))
        }
        Err(error) => devtools_dom_command_task_complete(Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            error.message,
        ))),
    }
}

fn complete_frontend_node_binding_for_get_attributes(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_document_node_attributes_for_reference(page, reference)
                .map(|pending| (pending, PendingDomCommandKind::GetAttributesLive))
        },
    )
}

fn complete_frontend_node_binding_for_get_attributes_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
) -> DevToolsDomCommandTaskStep {
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_document_node_attributes_for_reference(page, reference)
                .map(|pending| (pending, PendingDomCommandKind::GetAttributesLive))
        },
    )
}

fn complete_frontend_node_binding_for_remove_node(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_remove_document_node_for_reference(page, reference)
                .map(|pending| (pending, PendingDomCommandKind::RemoveNode))
        },
    )
}

fn complete_frontend_node_binding_for_remove_node_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
) -> DevToolsDomCommandTaskStep {
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_remove_document_node_for_reference(page, reference)
                .map(|pending| (pending, PendingDomCommandKind::RemoveNode))
        },
    )
}

fn complete_frontend_node_binding_for_focus(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_focus_document_node_for_reference(page, reference).map(|pending| {
                (
                    pending,
                    PendingDomCommandKind::Focus {
                        missing_node_message: "Could not find node with given id",
                    },
                )
            })
        },
    )
}

fn complete_frontend_node_binding_for_mutate_attribute(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    mutation: RendererDomAttributeMutation,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        move |page, reference| start_mutate_document_node_for_reference(page, reference, mutation),
    )
}

fn complete_frontend_node_binding_for_get_text(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_document_node_text_for_reference(page, reference)
                .map(|pending| (pending, PendingDomCommandKind::GetTextLive))
        },
    )
}

fn complete_frontend_node_binding_for_get_text_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
) -> DevToolsDomCommandTaskStep {
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_document_node_text_for_reference(page, reference)
                .map(|pending| (pending, PendingDomCommandKind::GetTextLive))
        },
    )
}

fn complete_frontend_node_binding_for_get_property(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    name: String,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_document_node_property_for_reference(page, reference, &name)
                .map(|pending| (pending, PendingDomCommandKind::GetPropertyLive))
        },
    )
}

fn complete_frontend_node_binding_for_get_property_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    name: String,
) -> DevToolsDomCommandTaskStep {
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_document_node_property_for_reference(page, reference, &name)
                .map(|pending| (pending, PendingDomCommandKind::GetPropertyLive))
        },
    )
}

fn complete_frontend_node_binding_for_dom_geometry(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    operation: DevToolsDomGeometryOperation,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| start_client_rect_for_reference(page, reference, operation),
    )
}

fn complete_frontend_node_binding_for_dom_geometry_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    operation: DevToolsDomGeometryOperation,
) -> DevToolsDomCommandTaskStep {
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| start_client_rect_for_reference(page, reference, operation),
    )
}

fn complete_frontend_node_binding_for_describe_node(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    depth: i32,
    pierce: bool,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let include_whitespace = dom_agent_includes_whitespace_for_route(
        conn,
        owner.session_id(),
        owner.session_owner_route(),
    );
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_inspector_document_node_snapshot_for_reference(
                page,
                renderer_inspector_session_id,
                include_whitespace,
                reference,
                depth,
                pierce,
            )
            .map(|pending| {
                (
                    pending,
                    PendingDomCommandKind::DescribeNodeObjectReference {
                        cached_object_node: None,
                        top_frame_id,
                    },
                )
            })
        },
    )
}

fn complete_frontend_node_binding_for_describe_node_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    depth: i32,
    pierce: bool,
    top_frame_id: Option<String>,
) -> DevToolsDomCommandTaskStep {
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let include_whitespace = dom_agent_includes_whitespace_for_route(
        conn,
        owner.session_id(),
        owner.session_owner_route(),
    );
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_inspector_document_node_snapshot_for_reference(
                page,
                renderer_inspector_session_id,
                include_whitespace,
                reference,
                depth,
                pierce,
            )
            .map(|pending| {
                (
                    pending,
                    PendingDomCommandKind::DescribeNodeObjectReference {
                        cached_object_node: None,
                        top_frame_id,
                    },
                )
            })
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn complete_frontend_node_binding_for_request_child_nodes(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    depth: i32,
    pierce: bool,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let include_whitespace = dom_agent_includes_whitespace_for_route(
        conn,
        owner.session_id(),
        owner.session_owner_route(),
    );
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_request_child_nodes_for_reference(
                page,
                renderer_inspector_session_id,
                include_whitespace,
                reference,
                depth,
                pierce,
                top_frame_id,
            )
        },
    )
}

fn complete_frontend_node_binding_for_request_child_nodes_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    depth: i32,
    pierce: bool,
    top_frame_id: Option<String>,
) -> DevToolsDomCommandTaskStep {
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let include_whitespace = dom_agent_includes_whitespace_for_route(
        conn,
        owner.session_id(),
        owner.session_owner_route(),
    );
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_request_child_nodes_for_reference(
                page,
                renderer_inspector_session_id,
                include_whitespace,
                reference,
                depth,
                pierce,
                top_frame_id,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn complete_frontend_node_binding_for_query_selector(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    selector: String,
    multiple: bool,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let include_whitespace = dom_agent_includes_whitespace_for_route(
        conn,
        owner.session_id(),
        owner.session_owner_route(),
    );
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_query_selector_with_child_node_snapshot_events_for_reference(
                page,
                renderer_inspector_session_id,
                include_whitespace,
                reference,
                selector,
                multiple,
                top_frame_id,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn complete_frontend_node_binding_for_query_selector_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    selector: String,
    multiple: bool,
) -> DevToolsDomCommandTaskStep {
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let include_whitespace = dom_agent_includes_whitespace_for_route(
        conn,
        owner.session_id(),
        owner.session_owner_route(),
    );
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_query_selector_for_reference(
                page,
                renderer_inspector_session_id,
                include_whitespace,
                reference,
                selector,
                multiple,
            )
        },
    )
}

fn start_resolve_node_for_bound_reference(
    page: &Page,
    renderer_inspector_session_id: Option<String>,
    reference: DevToolsDomNodeReference,
    requested_execution_context_id: Option<i64>,
    object_group: Option<String>,
    top_frame_id: Option<String>,
) -> Result<(PendingPageCommand, PendingDomCommandKind), PendingDomCommandStartError> {
    let resolution = start_resolve_runtime_object_for_reference(
        page,
        renderer_inspector_session_id,
        reference,
        requested_execution_context_id,
        object_group.as_deref(),
        top_frame_id,
    )?;
    Ok((
        resolution.pending,
        PendingDomCommandKind::ResolveNode {
            object_group,
            cache_top_frame_id: resolution.cache_top_frame_id,
        },
    ))
}

#[allow(clippy::too_many_arguments)]
fn complete_frontend_node_binding_for_resolve_node(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    requested_execution_context_id: Option<i64>,
    object_group: Option<String>,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| {
            start_resolve_node_for_bound_reference(
                page,
                renderer_inspector_session_id,
                reference,
                requested_execution_context_id,
                object_group,
                top_frame_id,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn complete_frontend_node_binding_for_resolve_node_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    requested_execution_context_id: Option<i64>,
    object_group: Option<String>,
    top_frame_id: Option<String>,
) -> DevToolsDomCommandTaskStep {
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| {
            start_resolve_node_for_bound_reference(
                page,
                renderer_inspector_session_id,
                reference,
                requested_execution_context_id,
                object_group,
                top_frame_id,
            )
        },
    )
}

fn complete_frontend_node_binding_for_get_outer_html(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    include_shadow_dom: bool,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| start_outer_html_for_reference(page, reference, include_shadow_dom),
    )
}

fn complete_frontend_node_binding_for_get_outer_html_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    include_shadow_dom: bool,
) -> DevToolsDomCommandTaskStep {
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| start_outer_html_for_reference(page, reference, include_shadow_dom),
    )
}

fn complete_frontend_node_binding_for_scroll_into_view_if_needed(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    rect: Option<DomScrollIntoViewRect>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    complete_frontend_node_binding_followup(
        conn,
        command_id,
        owner,
        completion,
        out,
        |page, reference| start_scroll_into_view_for_reference(page, reference, rect),
    )
}

fn complete_frontend_node_binding_for_scroll_into_view_if_needed_result(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    _frontend_node_id: u32,
    rect: Option<DomScrollIntoViewRect>,
) -> DevToolsDomCommandTaskStep {
    complete_frontend_node_binding_followup_result(
        conn,
        command_id,
        owner,
        completion,
        |page, reference| start_scroll_into_view_for_reference(page, reference, rect),
    )
}

fn complete_get_attributes_live(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page
        .finish_document_node_attributes(completion)
        .map_err(PendingDomCommandStartError::renderer_error)
        .and_then(attributes_result_from_renderer_resolution)
    {
        Ok(result) => out.push_result(json!({
            "attributes": flatten_dom_attributes(result.attributes),
        })),
        Err(error) => out.push_error(error.code, error.message),
    }
    DomCommandTaskStep::Complete
}

fn complete_get_text_live(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page
        .finish_document_node_text(completion)
        .map_err(PendingDomCommandStartError::renderer_error)
        .and_then(text_result_from_renderer_resolution)
    {
        Ok(result) => out.push_result(json!({ "text": result.text })),
        Err(error) => out.push_error(error.code, error.message),
    }
    DomCommandTaskStep::Complete
}

fn complete_get_property_live(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page
        .finish_document_node_property(completion)
        .map_err(PendingDomCommandStartError::renderer_error)
        .and_then(property_result_from_renderer_resolution)
    {
        Ok(result) => out.push_result(json!({ "value": result.value })),
        Err(error) => out.push_error(error.code, error.message),
    }
    DomCommandTaskStep::Complete
}

fn complete_get_attributes_live_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<DevToolsGetAttributesResult, DevToolsError> {
    page.finish_document_node_attributes(completion)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))
        .and_then(|resolution| {
            attributes_result_from_renderer_resolution(resolution).map_err(DevToolsError::from)
        })
}

fn complete_get_text_live_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<DevToolsGetTextResult, DevToolsError> {
    page.finish_document_node_text(completion)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))
        .and_then(|resolution| {
            text_result_from_renderer_resolution(resolution).map_err(DevToolsError::from)
        })
}

fn complete_get_property_live_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<DevToolsGetPropertyResult, DevToolsError> {
    page.finish_document_node_property(completion)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))
        .and_then(|resolution| {
            property_result_from_renderer_resolution(resolution).map_err(DevToolsError::from)
        })
}

fn complete_resolve_node_page_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<Value, DevToolsError> {
    let resolution = page.finish_resolve_runtime_object_for_backend_node_id(completion);
    let remote_object = match resolution {
        Ok(DocumentNodeRuntimeObjectResolution::Found(remote_object)) => remote_object,
        Ok(DocumentNodeRuntimeObjectResolution::MissingContext) => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                "ContextNotFound",
            ));
        }
        Ok(DocumentNodeRuntimeObjectResolution::MissingNode) => {
            return Err(devtools_dom_node_not_found_error());
        }
        Err(error) => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                format!("Could not resolve node runtime object: {error}"),
            ));
        }
    };
    Ok(remote_object.into_protocol_value())
}

fn complete_resolve_node_execution_context_frame_result(
    owner: &CommandOwnerScope,
    renderer_inspector_session_id: Option<String>,
    page: &mut Page,
    completion: CompletedPageCommand,
    reference: DevToolsDomNodeReference,
    execution_context_id: i64,
    object_group: Option<String>,
    top_frame_id: Option<String>,
) -> DevToolsDomCommandTaskStep {
    let child_frame_id =
        match page.finish_child_frame_id_for_default_execution_context_id(completion) {
            Ok(frame_id) => frame_id,
            Err(error) => {
                return devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    format!("Could not resolve execution context frame: {error}"),
                )));
            }
        };
    if child_frame_id.is_some() {
        if let DevToolsDomNodeReference::BackendNodeId(backend_node_id) = reference
            && is_renderer_backend_node_id(backend_node_id)
        {
            return match page.start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
                renderer_inspector_session_id,
                backend_node_id,
                Some(execution_context_id),
                object_group.as_deref(),
            ) {
                Ok(pending) => {
                    DevToolsDomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
                        command_id: None,
                        owner_scope: owner.clone(),
                        kind: PendingDomCommandKind::ResolveNode {
                            object_group,
                            cache_top_frame_id: None,
                        },
                        pending: PendingDomCommandWork::Page(pending),
                    }))
                }
                Err(error) => devtools_dom_command_task_complete(Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    format!("Could not resolve node runtime object: {error}"),
                ))),
            };
        }
        return devtools_dom_command_task_complete(Err(devtools_dom_node_not_found_error()));
    }
    match start_resolve_runtime_object_for_reference(
        page,
        renderer_inspector_session_id,
        reference,
        Some(execution_context_id),
        object_group.as_deref(),
        top_frame_id,
    ) {
        Ok(resolution) => {
            DevToolsDomCommandTaskStep::Pending(Box::new(PendingDomCommandDispatch {
                command_id: None,
                owner_scope: owner.clone(),
                kind: PendingDomCommandKind::ResolveNode {
                    object_group,
                    cache_top_frame_id: resolution.cache_top_frame_id,
                },
                pending: PendingDomCommandWork::Page(resolution.pending),
            }))
        }
        Err(error) => devtools_dom_command_task_complete(Err(DevToolsError::from(error))),
    }
}

fn complete_object_reference_live_client_rect_result(
    page: &mut Page,
    completion: CompletedPageCommand,
    operation: PendingDomObjectReferenceOperation,
) -> Result<DevToolsDomGeometryResult, DevToolsError> {
    let operation = match operation {
        PendingDomObjectReferenceOperation::GetBoxModel => {
            DevToolsDomGeometryOperation::GetBoxModel
        }
        PendingDomObjectReferenceOperation::GetContentQuads => {
            DevToolsDomGeometryOperation::GetContentQuads
        }
        _ => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Unsupported,
                "UnsupportedDevToolsDomObjectReferenceCommand",
            ));
        }
    };
    match page.finish_document_geometry_for_object_id(completion) {
        Ok(Some(geometry)) => devtools_dom_geometry_result_from_renderer(operation, geometry),
        Ok(None) => Err(devtools_dom_node_not_found_error()),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not resolve node geometry: {error}"),
        )),
    }
}

fn cache_resolved_node_snapshot(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    cache_object_id: String,
    snapshot: &DocumentNodeSnapshot,
    top_frame_id: Option<&str>,
) {
    let top_snapshot_node_id = top_snapshot_node_id_for_live_object_snapshot(snapshot);
    if let Some(cached_node) = node_snapshot_to_cdp(snapshot, top_snapshot_node_id, top_frame_id) {
        cache_dom_remote_object_node_for_owner(conn, owner, cache_object_id, cached_node);
    }
}

fn register_resolve_node_result(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    mut remote_object: Value,
    object_group: Option<String>,
) -> DevToolsResolveNodeResult {
    if let Some(remote_object) = remote_object.as_object_mut() {
        remote_object
            .entry("subtype".to_owned())
            .or_insert_with(|| json!("node"));
    }
    let result = json!({ "object": remote_object.clone() });
    if let Some(object_group) = object_group.as_deref() {
        conn.register_runtime_remote_object_ids_from_value_for_owner_with_group(
            owner,
            &result,
            object_group,
        );
    } else {
        conn.register_runtime_remote_object_ids_from_value_for_owner(owner, &result);
    }
    DevToolsResolveNodeResult {
        object: remote_object,
    }
}

fn complete_set_child_nodes_snapshot_for_backend_node(
    session_id: Option<&str>,
    page: &mut Page,
    completion: CompletedPageCommand,
    after: PendingSetChildNodesAfter,
    top_frame_id: Option<String>,
    missing_node_message: &'static str,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let snapshot_events = match page.finish_document_child_node_snapshot_events(completion) {
        Ok(Some(events)) => events,
        Ok(None) => {
            out.push_error(-32000, missing_node_message);
            return DomCommandTaskStep::Complete;
        }
        Err(error) => {
            out.push_error(
                -32000,
                format!("Could not capture child node snapshots: {error}"),
            );
            return DomCommandTaskStep::Complete;
        }
    };
    let Some(event) = snapshot_events.events.into_iter().next() else {
        complete_set_child_nodes_after(after, out);
        return DomCommandTaskStep::Complete;
    };
    complete_set_child_nodes_from_snapshots(
        session_id,
        after,
        event.parent_frontend_node_id,
        event.snapshots,
        snapshot_events.top_snapshot_node_id,
        top_frame_id,
        out,
    )
}

fn complete_query_selector_set_child_nodes_live(
    session_id: Option<&str>,
    page: &mut Page,
    completion: CompletedPageCommand,
    multiple: bool,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let result =
        match page.finish_document_query_selector_with_child_node_snapshot_events(completion) {
            Ok(result) => result,
            Err(error) => {
                out.push_error(
                    -32000,
                    format!("Could not capture query selector child node snapshots: {error}"),
                );
                return DomCommandTaskStep::Complete;
            }
        };
    let after = PendingSetChildNodesAfter::QuerySelectorLive {
        resolution: result.query_selector_resolution,
        multiple,
    };
    let Some(snapshot_events) = result.child_node_snapshot_events else {
        complete_set_child_nodes_after(after, out);
        return DomCommandTaskStep::Complete;
    };
    for event in snapshot_events.events {
        push_set_child_nodes_event_from_snapshots(
            session_id,
            event.parent_frontend_node_id,
            &event.snapshots,
            snapshot_events.top_snapshot_node_id,
            top_frame_id.as_deref(),
            out,
        );
    }
    complete_set_child_nodes_after(after, out);
    DomCommandTaskStep::Complete
}

fn complete_set_child_nodes_from_snapshots(
    session_id: Option<&str>,
    after: PendingSetChildNodesAfter,
    parent_frontend_node_id: u32,
    snapshots: Vec<DocumentNodeSnapshot>,
    top_snapshot_node_id: DocumentSnapshotNodeId,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    push_set_child_nodes_event_from_snapshots(
        session_id,
        parent_frontend_node_id,
        &snapshots,
        top_snapshot_node_id,
        top_frame_id.as_deref(),
        out,
    );
    complete_set_child_nodes_after(after, out);
    DomCommandTaskStep::Complete
}

fn complete_set_child_nodes_after(after: PendingSetChildNodesAfter, out: &mut DomCommandOutput) {
    match after {
        PendingSetChildNodesAfter::EmptyResult => {
            out.push_success();
        }
        PendingSetChildNodesAfter::QuerySelectorLive {
            resolution,
            multiple,
        } => match query_selector_set_child_nodes_result_from_renderer_resolution(
            resolution, multiple,
        ) {
            Ok(result) if result.multiple => {
                out.push_result(json!({ "nodeIds": result.node_ids }));
            }
            Ok(result) => {
                out.push_result(json!({ "nodeId": result.node_ids[0] }));
            }
            Err(error) => out.push_error(error.code, error.message),
        },
    }
}

fn query_selector_set_child_nodes_result_from_renderer_resolution(
    resolution: RendererDocumentQuerySelectorResolution,
    multiple: bool,
) -> Result<DevToolsQuerySelectorResult, PendingDomCommandStartError> {
    match resolution {
        RendererDocumentQuerySelectorResolution::Found(nodes) => {
            if !multiple && nodes.is_empty() {
                return Ok(DevToolsQuerySelectorResult {
                    node_ids: vec![0],
                    multiple,
                });
            }
            Ok(DevToolsQuerySelectorResult {
                node_ids: nodes
                    .into_iter()
                    .map(|node| node.frontend_node_id)
                    .collect(),
                multiple,
            })
        }
        RendererDocumentQuerySelectorResolution::MissingRoot => {
            Err(PendingDomCommandStartError::node_not_found())
        }
        RendererDocumentQuerySelectorResolution::InvalidSelector(message) => {
            Err(PendingDomCommandStartError::invalid_selector(message))
        }
    }
}

pub(super) fn push_set_child_nodes_event_from_snapshots(
    session_id: Option<&str>,
    parent_frontend_node_id: u32,
    snapshots: &[DocumentNodeSnapshot],
    top_snapshot_node_id: DocumentSnapshotNodeId,
    top_frame_id: Option<&str>,
    out: &mut DomCommandOutput,
) {
    let nodes = snapshots
        .iter()
        .filter_map(|snapshot| {
            node_snapshot_to_cdp(snapshot, Some(top_snapshot_node_id), top_frame_id)
        })
        .collect::<Vec<_>>();
    push_set_child_nodes_event(out, session_id, parent_frontend_node_id, nodes);
}

fn push_resolve_node_result(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    out: &mut DomCommandOutput,
    mut remote_object: Value,
    object_group: Option<String>,
) {
    if let Some(remote_object) = remote_object.as_object_mut() {
        remote_object
            .entry("subtype".to_owned())
            .or_insert_with(|| json!("node"));
    }
    let result = json!({ "object": remote_object });
    if let Some(object_group) = object_group.as_deref() {
        conn.register_runtime_remote_object_ids_from_value_for_owner_with_group(
            owner,
            &result,
            object_group,
        );
    } else {
        conn.register_runtime_remote_object_ids_from_value_for_owner(owner, &result);
    }
    out.push_result(result);
}

fn top_snapshot_node_id_for_live_object_snapshot(
    snapshot: &DocumentNodeSnapshot,
) -> Option<DocumentSnapshotNodeId> {
    if snapshot.parent_id.is_none()
        && (snapshot.node_name == "#document"
            || (snapshot.is_element && snapshot.local_name == "html"))
    {
        Some(snapshot.node_id)
    } else {
        None
    }
}

fn complete_request_node_object_reference(
    _session_id: Option<&str>,
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_document_node_snapshot_for_object_id(completion) {
        Ok(Some(object_snapshot)) => {
            let Some(frontend_node_id) =
                super::frontend_node_id_for_snapshot(&object_snapshot.snapshot)
            else {
                out.push_error(-32000, "Could not find node with given id");
                return DomCommandTaskStep::Complete;
            };
            out.push_result(json!({
                "nodeId": frontend_node_id
            }));
        }
        Ok(None) => out.push_error(-32000, "Could not find node with given id"),
        Err(error) => {
            out.push_error(-32000, format!("Could not request node object: {error}"));
        }
    }
    DomCommandTaskStep::Complete
}

fn complete_get_outer_html_object_reference(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_outer_html_for_object_id(completion) {
        Ok(Some(outer_html)) => out.push_result(json!({ "outerHTML": outer_html })),
        Ok(None) => out.push_error(-32000, "Could not find node with given id"),
        Err(error) => {
            out.push_error(
                -32000,
                format!("Could not get outerHTML for node object: {error}"),
            );
        }
    }
    DomCommandTaskStep::Complete
}

fn complete_get_outer_html_object_reference_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<DevToolsGetOuterHtmlResult, DevToolsError> {
    match page.finish_outer_html_for_object_id(completion) {
        Ok(Some(outer_html)) => Ok(DevToolsGetOuterHtmlResult { outer_html }),
        Ok(None) => Err(devtools_dom_node_not_found_error()),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not get outerHTML for node object: {error}"),
        )),
    }
}

fn complete_get_outer_html_document(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_outer_html_for_document(completion) {
        Ok(outer_html) => out.push_result(json!({ "outerHTML": outer_html })),
        Err(error) => out.push_error(
            -32000,
            format!("Could not serialize document outerHTML: {error}"),
        ),
    }
    DomCommandTaskStep::Complete
}

fn complete_get_outer_html_document_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<DevToolsGetOuterHtmlResult, DevToolsError> {
    match page.finish_outer_html_for_document(completion) {
        Ok(outer_html) => Ok(DevToolsGetOuterHtmlResult { outer_html }),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not serialize document outerHTML: {error}"),
        )),
    }
}

fn complete_get_outer_html_backend_node_reference(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_outer_html_for_backend_node_id(completion) {
        Ok(Some(outer_html)) => out.push_result(json!({ "outerHTML": outer_html })),
        Ok(None) => out.push_error(-32000, "Could not find node with given id"),
        Err(error) => out.push_error(-32000, format!("Could not get outerHTML for node: {error}")),
    }
    DomCommandTaskStep::Complete
}

fn complete_get_outer_html_backend_node_reference_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<DevToolsGetOuterHtmlResult, DevToolsError> {
    match page.finish_outer_html_for_backend_node_id(completion) {
        Ok(Some(outer_html)) => Ok(DevToolsGetOuterHtmlResult { outer_html }),
        Ok(None) => Err(devtools_dom_node_not_found_error()),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not get outerHTML for node: {error}"),
        )),
    }
}

fn complete_scroll_into_view_if_needed_object_reference(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_scroll_node_into_view_if_needed(completion) {
        Ok(RendererScrollIntoViewResult::ScrolledOrAlreadyVisible) => out.push_success(),
        Ok(RendererScrollIntoViewResult::NodeNotFound) => {
            out.push_error(-32000, "Could not find node with given id")
        }
        Ok(RendererScrollIntoViewResult::NodeDetached) => {
            out.push_error(-32000, "Node is detached from document")
        }
        Ok(RendererScrollIntoViewResult::NodeDoesNotHaveLayoutObject) => {
            out.push_error(-32000, "Node does not have a layout object")
        }
        Err(error) => out.push_error(-32000, format!("Could not scroll node into view: {error}")),
    }
    DomCommandTaskStep::Complete
}

fn complete_scroll_into_view_if_needed_object_reference_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<(), DevToolsError> {
    complete_scroll_into_view_result(page.finish_scroll_node_into_view_if_needed(completion))
}

fn complete_renderer_backend_node_scroll_into_view_if_needed(
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_scroll_node_into_view_if_needed(completion) {
        Ok(RendererScrollIntoViewResult::ScrolledOrAlreadyVisible) => out.push_success(),
        Ok(RendererScrollIntoViewResult::NodeNotFound) => {
            out.push_error(-32000, "Could not find node with given id")
        }
        Ok(RendererScrollIntoViewResult::NodeDetached) => {
            out.push_error(-32000, "Node is detached from document")
        }
        Ok(RendererScrollIntoViewResult::NodeDoesNotHaveLayoutObject) => {
            out.push_error(-32000, "Node does not have a layout object")
        }
        Err(error) => out.push_error(-32000, format!("Could not scroll node into view: {error}")),
    }
    DomCommandTaskStep::Complete
}

fn complete_renderer_backend_node_scroll_into_view_if_needed_result(
    page: &mut Page,
    completion: CompletedPageCommand,
) -> Result<(), DevToolsError> {
    complete_scroll_into_view_result(page.finish_scroll_node_into_view_if_needed(completion))
}

fn fill_pushed_frontend_node_ids(
    node_ids: &mut [u32],
    renderer_backend_positions: Vec<usize>,
    renderer_frontend_node_ids: Vec<Option<u32>>,
) {
    for (position, frontend_node_id) in renderer_backend_positions
        .into_iter()
        .zip(renderer_frontend_node_ids)
    {
        if let Some(slot) = node_ids.get_mut(position) {
            *slot = frontend_node_id.unwrap_or(0);
        }
    }
}

fn complete_push_nodes_by_backend_ids_to_frontend(
    page: &mut Page,
    completion: CompletedPageCommand,
    _session_id: Option<&str>,
    _backend_node_ids: Vec<u32>,
    mut node_ids: Vec<u32>,
    renderer_backend_positions: Vec<usize>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_document_frontend_node_ids_for_backend_node_ids(completion) {
        Ok(RendererDocumentFrontendNodeIdsResolution::Found(renderer_frontend_node_ids)) => {
            fill_pushed_frontend_node_ids(
                &mut node_ids,
                renderer_backend_positions,
                renderer_frontend_node_ids,
            );
            out.push_result(json!({ "nodeIds": node_ids }));
        }
        Ok(RendererDocumentFrontendNodeIdsResolution::DocumentNotBound) => {
            out.push_error(-32000, "Document needs to be requested first")
        }
        Err(error) => out.push_error(
            -32000,
            format!("Could not resolve backend node ids: {error}"),
        ),
    }
    DomCommandTaskStep::Complete
}

fn complete_push_nodes_by_backend_ids_to_frontend_result(
    page: &mut Page,
    completion: CompletedPageCommand,
    _backend_node_ids: Vec<u32>,
    mut node_ids: Vec<u32>,
    renderer_backend_positions: Vec<usize>,
) -> Result<DevToolsPushNodesByBackendIdsResult, DevToolsError> {
    match page.finish_document_frontend_node_ids_for_backend_node_ids(completion) {
        Ok(RendererDocumentFrontendNodeIdsResolution::Found(renderer_frontend_node_ids)) => {
            fill_pushed_frontend_node_ids(
                &mut node_ids,
                renderer_backend_positions,
                renderer_frontend_node_ids,
            );
            Ok(DevToolsPushNodesByBackendIdsResult { node_ids })
        }
        Ok(RendererDocumentFrontendNodeIdsResolution::DocumentNotBound) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "Document needs to be requested first",
        )),
        Err(error) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            format!("Could not resolve backend node ids: {error}"),
        )),
    }
}

fn node_for_describe_node_object_snapshot(
    snapshot: &DocumentNodeSnapshot,
    child_frame_id: Option<&str>,
    top_frame_id: Option<&str>,
) -> Option<Value> {
    let top_snapshot_node_id = if child_frame_id.is_some() {
        snapshot.parent_id.or(Some(snapshot.node_id))
    } else {
        top_snapshot_node_id_for_live_object_snapshot(snapshot)
    };
    let frame_id_for_html = if child_frame_id.is_some() {
        None
    } else {
        top_frame_id
    };
    let mut node = node_snapshot_to_cdp(snapshot, top_snapshot_node_id, frame_id_for_html)?;
    if let Some(child_frame_id) = child_frame_id
        && let Some(node) = node.as_object_mut()
    {
        node.insert("frameId".to_owned(), json!(child_frame_id));
    }
    Some(node)
}

fn complete_describe_node_object_reference(
    page: &mut Page,
    completion: CompletedPageCommand,
    cached_object_node: Option<Value>,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let object_snapshot = match page.finish_document_node_snapshot_for_object_id(completion) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            if let Some(node) = cached_object_node {
                out.push_result(json!({ "node": node }));
            } else {
                out.push_error(-32000, "Could not find node with given id");
            }
            return DomCommandTaskStep::Complete;
        }
        Err(error) => {
            out.push_error(-32000, format!("Could not describe node object: {error}"));
            return DomCommandTaskStep::Complete;
        }
    };

    let Some(node) = node_for_describe_node_object_snapshot(
        &object_snapshot.snapshot,
        object_snapshot.frame_id.as_deref(),
        top_frame_id.as_deref(),
    ) else {
        out.push_error(-32000, "Could not find node with given id");
        return DomCommandTaskStep::Complete;
    };
    out.push_result(json!({ "node": node }));
    DomCommandTaskStep::Complete
}

fn complete_describe_node_object_reference_result(
    page: &mut Page,
    completion: CompletedPageCommand,
    cached_object_node: Option<Value>,
    top_frame_id: Option<String>,
) -> Result<DevToolsDescribeNodeResult, DevToolsError> {
    let object_snapshot = match page.finish_document_node_snapshot_for_object_id(completion) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            if let Some(node) = cached_object_node {
                return Ok(DevToolsDescribeNodeResult { node });
            }
            return Err(devtools_dom_node_not_found_error());
        }
        Err(error) => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                format!("Could not describe node object: {error}"),
            ));
        }
    };
    let Some(node) = node_for_describe_node_object_snapshot(
        &object_snapshot.snapshot,
        object_snapshot.frame_id.as_deref(),
        top_frame_id.as_deref(),
    ) else {
        return Err(devtools_dom_node_not_found_error());
    };
    Ok(DevToolsDescribeNodeResult { node })
}

fn complete_document_snapshot(
    operation: PendingDomDocumentSnapshotOperation,
    page: &mut Page,
    completion: CompletedPageCommand,
    top_frame_id: Option<String>,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let snapshot = match page.finish_document_node_snapshot_for_document(completion) {
        Ok(Some(object_snapshot)) => object_snapshot.snapshot,
        Ok(None) => {
            out.push_error(-32000, "NoDocumentLoaded");
            return DomCommandTaskStep::Complete;
        }
        Err(error) => {
            out.push_error(
                -32000,
                format!("Could not capture document snapshot: {error}"),
            );
            return DomCommandTaskStep::Complete;
        }
    };
    let top_snapshot_node_id = snapshot.node_id;
    match operation {
        PendingDomDocumentSnapshotOperation::GetDocument => {
            let Some(root) = node_snapshot_to_cdp(
                &snapshot,
                Some(top_snapshot_node_id),
                top_frame_id.as_deref(),
            ) else {
                out.push_error(-32000, "No node with given id found");
                return DomCommandTaskStep::Complete;
            };
            out.push_result(json!({ "root": root }));
        }
        PendingDomDocumentSnapshotOperation::GetFlattenedDocument => {
            let mut nodes = Vec::new();
            collect_flattened_node_snapshot(
                &snapshot,
                top_snapshot_node_id,
                top_frame_id.as_deref(),
                &mut nodes,
            );
            out.push_result(json!({ "nodes": nodes }));
        }
    }
    DomCommandTaskStep::Complete
}

fn complete_object_reference_live_client_rect(
    operation: PendingDomObjectReferenceOperation,
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_document_geometry_for_object_id(completion) {
        Ok(Some(geometry)) => {
            let geometry_operation = match dom_geometry_operation_for_object_reference(&operation) {
                Ok(operation) => operation,
                Err(message) => {
                    out.push_error(-32000, message);
                    return DomCommandTaskStep::Complete;
                }
            };
            match devtools_dom_geometry_result_from_renderer(geometry_operation, geometry) {
                Ok(result) => push_devtools_dom_geometry_result(&result, out),
                Err(error) => out.push_error(-32000, error.message),
            }
        }
        Ok(None) => out.push_error(-32000, "Could not find node with given id"),
        Err(error) => out.push_error(-32000, format!("Could not resolve node geometry: {error}")),
    }
    DomCommandTaskStep::Complete
}

fn complete_renderer_backend_node_client_rect(
    operation: DevToolsDomGeometryOperation,
    page: &mut Page,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    match page.finish_document_geometry_for_backend_node_id(completion) {
        Ok(Some(geometry)) => {
            match devtools_dom_geometry_result_from_renderer(operation, geometry) {
                Ok(result) => push_devtools_dom_geometry_result(&result, out),
                Err(error) => out.push_error(-32000, error.message),
            }
        }
        Ok(None) => out.push_error(-32000, "Could not find node with given id"),
        Err(error) => out.push_error(-32000, format!("Could not resolve node geometry: {error}")),
    }
    DomCommandTaskStep::Complete
}

fn dom_geometry_operation_for_object_reference(
    operation: &PendingDomObjectReferenceOperation,
) -> Result<DevToolsDomGeometryOperation, &'static str> {
    match operation {
        PendingDomObjectReferenceOperation::GetBoxModel => {
            Ok(DevToolsDomGeometryOperation::GetBoxModel)
        }
        PendingDomObjectReferenceOperation::GetContentQuads => {
            Ok(DevToolsDomGeometryOperation::GetContentQuads)
        }
        PendingDomObjectReferenceOperation::RequestNode
        | PendingDomObjectReferenceOperation::Focus
        | PendingDomObjectReferenceOperation::GetOuterHtml { .. }
        | PendingDomObjectReferenceOperation::ScrollIntoViewIfNeeded { .. }
        | PendingDomObjectReferenceOperation::DescribeNode { .. } => {
            Err("UnsupportedDevToolsDomObjectReferenceCommand")
        }
    }
}

pub(super) fn devtools_dom_geometry_result_from_renderer(
    operation: DevToolsDomGeometryOperation,
    geometry: RendererDocumentNodeGeometry,
) -> Result<DevToolsDomGeometryResult, DevToolsError> {
    match (operation, geometry) {
        (
            DevToolsDomGeometryOperation::GetBoxModel,
            RendererDocumentNodeGeometry::FoundElement { box_model, .. },
        ) => Ok(DevToolsDomGeometryResult {
            box_model: Some(DevToolsDomBoxModel {
                content: devtools_dom_quad(box_model.content.points),
                padding: devtools_dom_quad(box_model.padding.points),
                border: devtools_dom_quad(box_model.border.points),
                margin: devtools_dom_quad(box_model.margin.points),
                width: box_model.width,
                height: box_model.height,
            }),
            quads: Vec::new(),
            width: Some(box_model.width),
            height: Some(box_model.height),
        }),
        (
            DevToolsDomGeometryOperation::GetContentQuads,
            RendererDocumentNodeGeometry::FoundElement { content_quads, .. }
            | RendererDocumentNodeGeometry::FoundNonElement { content_quads },
        ) => Ok(DevToolsDomGeometryResult {
            box_model: None,
            quads: content_quads
                .into_iter()
                .map(|quad| devtools_dom_quad(quad.points))
                .collect(),
            width: None,
            height: None,
        }),
        (_, RendererDocumentNodeGeometry::NoLayoutObject) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "Could not compute node geometry.",
        )),
        (_, RendererDocumentNodeGeometry::FoundNonElement { .. })
        | (_, RendererDocumentNodeGeometry::NotElement) => Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "Node is not an element",
        )),
    }
}

fn devtools_dom_quad(points: [f64; 8]) -> DevToolsDomQuad {
    DevToolsDomQuad {
        points: points.into(),
    }
}

fn push_devtools_dom_geometry_result(
    result: &DevToolsDomGeometryResult,
    out: &mut DomCommandOutput,
) {
    if let Some(model) = result.box_model.as_ref() {
        out.push_result(json!({
            "model": {
                "content": model.content.points,
                "padding": model.padding.points,
                "border": model.border.points,
                "margin": model.margin.points,
                "width": model.width,
                "height": model.height,
            }
        }));
    } else {
        out.push_result(json!({
            "quads": result.quads.iter().map(|quad| &quad.points).collect::<Vec<_>>()
        }));
    }
}

fn build_cdp_get_attributes_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsGetAttributesCommand, PendingDomCommandStartError> {
    let params: GetAttributesParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let Some(cdp_node_id) = cdp_id_from_i64(*params.node_id.inner()) else {
        return Err(PendingDomCommandStartError::invalid_params());
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsGetAttributesCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference: DevToolsDomNodeReference::FrontendNodeId(cdp_node_id),
    })
}

fn start_devtools_get_attributes_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetAttributesCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = command.reference {
        return start_document_frontend_node_binding_command(
            conn,
            command_id,
            owner,
            frontend_node_id,
            PendingDomCommandKind::ResolveFrontendNodeForGetAttributes { frontend_node_id },
        );
    }
    let reference = command.reference;
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = start_document_node_attributes_for_reference(page, reference)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::GetAttributesLive,
        pending: PendingDomCommandWork::Page(pending),
    }))
}

fn flatten_dom_attributes(attributes: Vec<DevToolsDomAttribute>) -> Vec<String> {
    let mut out = Vec::with_capacity(attributes.len() * 2);
    for attribute in attributes {
        out.push(attribute.name);
        out.push(attribute.value);
    }
    out
}

fn start_document_frontend_node_binding_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    frontend_node_id: u32,
    kind: PendingDomCommandKind,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = page
        .start_document_frontend_node_binding(renderer_inspector_session_id, frontend_node_id)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind,
        pending: PendingDomCommandWork::Page(pending),
    }))
}

fn start_devtools_get_text_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetTextCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = command.reference {
        return start_document_frontend_node_binding_command(
            conn,
            command_id,
            owner,
            frontend_node_id,
            PendingDomCommandKind::ResolveFrontendNodeForGetText { frontend_node_id },
        );
    }
    let reference = command.reference;
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = start_document_node_text_for_reference(page, reference)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::GetTextLive,
        pending: PendingDomCommandWork::Page(pending),
    }))
}

fn start_devtools_get_property_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetPropertyCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = command.reference {
        return start_document_frontend_node_binding_command(
            conn,
            command_id,
            owner,
            frontend_node_id,
            PendingDomCommandKind::ResolveFrontendNodeForGetProperty {
                frontend_node_id,
                name: command.name,
            },
        );
    }
    let reference = command.reference;
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let pending = start_document_node_property_for_reference(page, reference, &command.name)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::GetPropertyLive,
        pending: PendingDomCommandWork::Page(pending),
    }))
}

pub(super) fn push_nodes_by_backend_ids_to_frontend(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<CommandOutputPlan> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    if !target_owner_exists_for_owner(conn, &owner) {
        return Some(CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"));
    }
    match build_cdp_push_nodes_by_backend_ids_command(conn, cmd) {
        Ok(_) => None,
        Err(error) => Some(CommandOutputPlan::error(error.code, error.message)),
    }
}

fn build_cdp_push_nodes_by_backend_ids_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsPushNodesByBackendIdsCommand, PendingDomCommandStartError> {
    let params: PushNodesByBackendIdsToFrontendParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let backend_node_ids = params
        .backend_node_ids
        .iter()
        .map(|backend_node_id| cdp_id_from_i64(*backend_node_id.inner()))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(PendingDomCommandStartError::invalid_params)?;
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsPushNodesByBackendIdsCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        backend_node_ids,
    })
}

pub(super) fn get_outer_html(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> Option<CommandOutputPlan> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    if !target_owner_exists_for_owner(conn, &owner) {
        return Some(CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"));
    }
    None
}

fn build_cdp_get_outer_html_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<DevToolsGetOuterHtmlCommand>, PendingDomCommandStartError> {
    let params: GetOuterHtmlParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    if params.reference.object_id.is_some() {
        return Ok(None);
    }
    let reference = devtools_node_reference_from_ids(
        params.reference.node_id,
        params.reference.backend_node_id,
    )
    .ok_or_else(PendingDomCommandStartError::invalid_params)?;
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(Some(DevToolsGetOuterHtmlCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference: Some(reference),
        include_shadow_dom: params.include_shadow_dom,
    }))
}

pub(crate) async fn execute_devtools_dom_command_async(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let (target_id, command_session_id) = devtools_dom_command_route(&command)?;
    let owner = if let Some(target_id) = target_id.as_deref() {
        if conn.target_session_route_for_target_id(target_id).is_none() {
            let result = super::child_frame::execute_devtools_dom_command(conn, target_id, command)
                .await
                .map_err(DevToolsError::from)?;
            return Ok(result);
        }
        let route = conn
            .target_session_route_for_target_id(target_id)
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
        CommandOwnerScope::for_route(route)
    } else {
        CommandOwnerScope::capture(conn, command_session_id.as_deref())
    };

    execute_devtools_dom_command_for_owner(conn, &owner, command).await
}

fn devtools_dom_command_route(
    command: &DevToolsCommand,
) -> Result<(Option<String>, Option<String>), DevToolsError> {
    let context = match command {
        DevToolsCommand::QuerySelector(command) => &command.context,
        DevToolsCommand::GetAttributes(command) => &command.context,
        DevToolsCommand::GetText(command) => &command.context,
        DevToolsCommand::GetProperty(command) => &command.context,
        DevToolsCommand::PushNodesByBackendIds(command) => &command.context,
        DevToolsCommand::GetOuterHtml(command) => &command.context,
        DevToolsCommand::DescribeNode(command) => &command.context,
        DevToolsCommand::GetFrameOwner(command) => &command.context,
        DevToolsCommand::ResolveNode(command) => &command.context,
        DevToolsCommand::ScrollIntoViewIfNeeded(command) => &command.context,
        DevToolsCommand::DomObjectReference(command)
            if matches!(
                command.operation,
                DevToolsDomObjectReferenceOperation::GetBoxModel
                    | DevToolsDomObjectReferenceOperation::GetContentQuads
            ) =>
        {
            &command.context
        }
        DevToolsCommand::SetFileInputFiles(command) => &command.context,
        DevToolsCommand::DomGeometry(command) => &command.context,
        _ => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Unsupported,
                "UnsupportedDevToolsCommand",
            ));
        }
    };
    Ok((
        context
            .target_id
            .as_ref()
            .map(|target_id| target_id.to_string()),
        context
            .session_id
            .as_ref()
            .map(|session_id| session_id.to_string()),
    ))
}

async fn execute_devtools_dom_command_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    command: DevToolsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    match command {
        DevToolsCommand::QuerySelector(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                DevToolsCommand::QuerySelector(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        DevToolsCommand::GetAttributes(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                DevToolsCommand::GetAttributes(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        DevToolsCommand::GetText(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                DevToolsCommand::GetText(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        DevToolsCommand::GetProperty(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                DevToolsCommand::GetProperty(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        DevToolsCommand::PushNodesByBackendIds(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                DevToolsCommand::PushNodesByBackendIds(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        DevToolsCommand::GetOuterHtml(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                DevToolsCommand::GetOuterHtml(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        DevToolsCommand::DescribeNode(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                DevToolsCommand::DescribeNode(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        DevToolsCommand::GetFrameOwner(command) => {
            let immediate_command = command.clone();
            let Some(pending) = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                DevToolsCommand::GetFrameOwner(command),
            )
            .map_err(DevToolsError::from)?
            else {
                let result = complete_devtools_dom_command(
                    conn,
                    DevToolsCommand::GetFrameOwner(immediate_command),
                )
                .map_err(DevToolsError::from)?;
                return devtools_get_frame_owner_result_from_value(&result)
                    .map(DevToolsCommandResult::GetFrameOwner);
            };

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        DevToolsCommand::ResolveNode(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                DevToolsCommand::ResolveNode(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        DevToolsCommand::ScrollIntoViewIfNeeded(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                DevToolsCommand::ScrollIntoViewIfNeeded(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        DevToolsCommand::DomGeometry(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                DevToolsCommand::DomGeometry(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        DevToolsCommand::DomObjectReference(command)
            if matches!(
                command.operation,
                DevToolsDomObjectReferenceOperation::GetBoxModel
                    | DevToolsDomObjectReferenceOperation::GetContentQuads
            ) =>
        {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                DevToolsCommand::DomObjectReference(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        DevToolsCommand::SetFileInputFiles(command) => {
            let pending = start_devtools_dom_command_for_owner(
                conn,
                None,
                owner,
                DevToolsCommand::SetFileInputFiles(command),
            )
            .map_err(DevToolsError::from)?
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingDomCommand"))?;

            await_pending_devtools_dom_command_result(conn, pending).await
        }
        _ => Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "UnsupportedDevToolsCommand",
        )),
    }
}

async fn await_pending_devtools_dom_command_result(
    conn: &mut CdpConnection,
    mut pending: PendingDomCommandDispatch,
) -> Result<DevToolsCommandResult, DevToolsError> {
    loop {
        let completed = Box::pin(pending.wait()).await;
        match complete_pending_dom_command_result(conn, completed) {
            DevToolsDomCommandTaskStep::Pending(next) => {
                pending = *next;
            }
            DevToolsDomCommandTaskStep::Complete(result) => return *result,
        }
    }
}

fn devtools_get_frame_owner_result_from_value(
    result: &Value,
) -> Result<DevToolsGetFrameOwnerResult, DevToolsError> {
    let node_id = result
        .get("nodeId")
        .and_then(Value::as_u64)
        .and_then(|node_id| u32::try_from(node_id).ok())
        .ok_or_else(|| {
            DevToolsError::new(DevToolsErrorKind::Internal, "MissingFrameOwnerNodeId")
        })?;
    let backend_node_id = result
        .get("backendNodeId")
        .and_then(Value::as_u64)
        .and_then(|backend_node_id| u32::try_from(backend_node_id).ok())
        .ok_or_else(|| {
            DevToolsError::new(
                DevToolsErrorKind::Internal,
                "MissingFrameOwnerBackendNodeId",
            )
        })?;
    Ok(DevToolsGetFrameOwnerResult {
        node_id,
        backend_node_id,
    })
}

pub(super) fn renderer_backend_node_id_for_reference(
    reference: &DevToolsDomNodeReference,
) -> Option<u32> {
    match reference {
        DevToolsDomNodeReference::BackendNodeId(backend_node_id)
            if is_renderer_backend_node_id(*backend_node_id) =>
        {
            Some(*backend_node_id)
        }
        DevToolsDomNodeReference::FrontendNodeId(_)
        | DevToolsDomNodeReference::BackendNodeId(_) => None,
    }
}

pub(super) fn scroll_into_view_if_needed(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<CommandOutputPlan> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    if !target_owner_exists_for_owner(conn, &owner) {
        return Some(CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"));
    }
    None
}

fn build_cdp_scroll_into_view_if_needed_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<DevToolsScrollIntoViewIfNeededCommand>, PendingDomCommandStartError> {
    let params: ScrollIntoViewIfNeededParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    if params.reference.object_id.is_some() {
        return Ok(None);
    }
    let rect = validated_scroll_into_view_rect(params.rect)?;
    let reference = devtools_node_reference_from_ids(
        params.reference.node_id,
        params.reference.backend_node_id,
    );
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(Some(DevToolsScrollIntoViewIfNeededCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        reference,
        rect,
    }))
}

fn start_devtools_scroll_into_view_if_needed_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsScrollIntoViewIfNeededCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let Some(reference) = command.reference else {
        return Err(PendingDomCommandStartError::node_not_found());
    };
    if let DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) = reference {
        return start_document_frontend_node_binding_command(
            conn,
            command_id,
            owner,
            frontend_node_id,
            PendingDomCommandKind::ResolveFrontendNodeForScrollIntoViewIfNeeded {
                frontend_node_id,
                rect: command.rect,
            },
        );
    }
    let page = loaded_page_mut_for_owner(conn, owner)
        .ok_or_else(PendingDomCommandStartError::no_document_loaded)?;
    let (pending, kind) = start_scroll_into_view_for_reference(page, reference, command.rect)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind,
        pending: PendingDomCommandWork::Page(pending),
    }))
}

#[cfg(test)]
mod protocol_neutral_tests {
    use crate::devtools_runtime::{
        AutomationEvent, DevToolsCommand, DevToolsDomGeometryOperation, DevToolsDomNodeReference,
        DevToolsDomObjectReferenceOperation, DevToolsErrorKind, DevToolsProtocol,
    };
    use moli_page_types::DocumentSnapshotNodeId;
    use serde_json::{Value, json};

    use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};

    fn start_devtools_dom_command(
        conn: &mut CdpConnection,
        command_id: Option<u64>,
        command_session_id: Option<&str>,
        command: DevToolsCommand,
    ) -> Result<Option<super::PendingDomCommandDispatch>, super::PendingDomCommandStartError> {
        let owner = CommandOwnerScope::capture(conn, command_session_id);
        super::start_devtools_dom_command_for_owner(conn, command_id, &owner, command)
    }

    fn unique_test_file_path(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after UNIX_EPOCH")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "moli-protocol-{name}-{}-{nanos}.txt",
            std::process::id()
        ))
    }

    #[test]
    fn set_child_nodes_output_preserves_typed_automation_sidecar() {
        let mut out = super::DomCommandOutput::default();
        let nodes = vec![json!({
            "nodeId": 8,
            "backendNodeId": 8,
            "nodeType": 1,
            "nodeName": "DIV",
            "localName": "div",
            "nodeValue": "",
        })];

        super::push_set_child_nodes_event(
            &mut out,
            Some("SID-dom"),
            DocumentSnapshotNodeId::new(7).encoded(),
            nodes.clone(),
        );

        let mut events = out.into_plan().into_background_events(Some(42), None);
        assert_eq!(events.len(), 1);
        assert!(
            events[0].protocol_message().is_none(),
            "DOM.setChildNodes should stay typed until wire projection"
        );
        assert_eq!(events[0].protocol_method(), Some("DOM.setChildNodes"));
        assert!(events[0].has_protocol_wire_message());
        let (message, automation_event) = events.remove(0).into_parts();
        assert_eq!(message["method"], json!("DOM.setChildNodes"));
        assert_eq!(message["sessionId"], json!("SID-dom"));
        assert_eq!(message["params"]["parentId"], json!(8));
        assert_eq!(message["params"]["nodes"], json!(nodes));

        let Some(AutomationEvent::DomSetChildNodes(event)) = automation_event else {
            panic!("expected typed DOM.setChildNodes automation sidecar");
        };
        assert_eq!(event.parent_node_id, 8);
        assert_eq!(event.nodes, nodes);
    }

    #[test]
    fn cdp_get_document_builds_protocol_neutral_document_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "depth": 2,
            "pierce": true
        });
        let cmd = Cmd::for_test(
            Some(61),
            "DOM.getDocument",
            &params,
            Some("SID-dom"),
            r#"{"id":61,"method":"DOM.getDocument"}"#,
        );

        let command = super::build_cdp_get_document_command(
            &conn,
            &cmd,
            super::PendingDomDocumentSnapshotOperation::GetDocument,
        );
        let Ok(command) = command else {
            panic!("valid getDocument command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-dom")
        );
        assert_eq!(command.context.target_id, None);
        assert_eq!(command.context.browser_context_id, None);
        assert_eq!(command.depth, Some(2));
        assert!(command.pierce);
        assert!(!command.flattened);
    }

    #[test]
    fn cdp_get_flattened_document_builds_protocol_neutral_document_command() {
        let conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(62),
            "DOM.getFlattenedDocument",
            &params,
            Some("SID-dom"),
            r#"{"id":62,"method":"DOM.getFlattenedDocument"}"#,
        );

        let command = super::build_cdp_get_document_command(
            &conn,
            &cmd,
            super::PendingDomDocumentSnapshotOperation::GetFlattenedDocument,
        );
        let Ok(command) = command else {
            panic!("valid getFlattenedDocument command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(command.depth, Some(-1));
        assert!(!command.pierce);
        assert!(command.flattened);
    }

    #[test]
    fn devtools_dom_entry_routes_document_command_to_dom_owner() {
        let mut conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(63),
            "DOM.getDocument",
            &params,
            Some("SID-dom"),
            r#"{"id":63,"method":"DOM.getDocument"}"#,
        );
        let command = super::build_cdp_get_document_command(
            &conn,
            &cmd,
            super::PendingDomDocumentSnapshotOperation::GetDocument,
        );
        let Ok(command) = command else {
            panic!("valid getDocument command");
        };

        let result = start_devtools_dom_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::GetDocument(command),
        );

        let Err(error) = result else {
            panic!("missing document should surface through the unified DOM entry");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "NoDocumentLoaded");
    }

    #[test]
    fn cdp_get_frame_owner_builds_protocol_neutral_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "frameId": "TID-child"
        });
        let cmd = Cmd::for_test(
            Some(98),
            "DOM.getFrameOwner",
            &params,
            Some("SID-dom"),
            r#"{"id":98,"method":"DOM.getFrameOwner"}"#,
        );

        let command = super::build_cdp_get_frame_owner_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid getFrameOwner command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(command.frame_id.as_str(), "TID-child");
    }

    #[test]
    fn devtools_dom_entry_routes_get_frame_owner_command_to_dom_owner() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "frameId": "TID-child"
        });
        let cmd = Cmd::for_test(
            Some(99),
            "DOM.getFrameOwner",
            &params,
            Some("SID-dom"),
            r#"{"id":99,"method":"DOM.getFrameOwner"}"#,
        );
        let command = super::build_cdp_get_frame_owner_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid getFrameOwner command");
        };

        let result = start_devtools_dom_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::GetFrameOwner(command),
        );

        let Err(error) = result else {
            panic!("missing document should surface through the unified DOM start entry");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(
            error.message,
            "Frame with the given id does not belong to the target."
        );
    }

    #[test]
    fn cdp_request_child_nodes_builds_protocol_neutral_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "nodeId": 24,
            "depth": 3,
            "pierce": true
        });
        let cmd = Cmd::for_test(
            Some(89),
            "DOM.requestChildNodes",
            &params,
            Some("SID-dom"),
            r#"{"id":89,"method":"DOM.requestChildNodes"}"#,
        );

        let command = super::build_cdp_request_child_nodes_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid requestChildNodes command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.reference,
            DevToolsDomNodeReference::FrontendNodeId(24)
        );
        assert_eq!(command.depth, 3);
        assert!(command.pierce);
    }

    #[test]
    fn devtools_dom_entry_routes_request_child_nodes_command_to_dom_owner() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "nodeId": 25
        });
        let cmd = Cmd::for_test(
            Some(90),
            "DOM.requestChildNodes",
            &params,
            Some("SID-dom"),
            r#"{"id":90,"method":"DOM.requestChildNodes"}"#,
        );
        let command = super::build_cdp_request_child_nodes_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid requestChildNodes command");
        };

        let result = start_devtools_dom_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::RequestChildNodes(command),
        );

        let Err(error) = result else {
            panic!("missing document should surface through the unified DOM start entry");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "NoDocumentLoaded");
    }

    #[test]
    fn devtools_dom_complete_routes_request_child_nodes_command_to_dom_owner() {
        let mut conn = CdpConnection::new();
        let mut out = super::DomCommandOutput::default();
        let params = json!({
            "nodeId": 26
        });
        let cmd = Cmd::for_test(
            Some(91),
            "DOM.requestChildNodes",
            &params,
            Some("SID-dom"),
            r#"{"id":91,"method":"DOM.requestChildNodes"}"#,
        );
        let command = super::build_cdp_request_child_nodes_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid requestChildNodes command");
        };

        let result =
            super::complete_devtools_request_child_nodes_command(&mut conn, command, &mut out);

        let Err(error) = result else {
            panic!("requestChildNodes complete helper should require pending renderer capture");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(
            error.message,
            "RequestChildNodesRequiresPendingRendererCapture"
        );
        assert!(out.is_empty());
    }

    #[test]
    fn cdp_get_node_for_location_builds_protocol_neutral_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "x": 12,
            "y": 34,
            "includeUserAgentShadowDOM": true,
            "ignorePointerEventsNone": true
        });
        let cmd = Cmd::for_test(
            Some(95),
            "DOM.getNodeForLocation",
            &params,
            Some("SID-dom"),
            r#"{"id":95,"method":"DOM.getNodeForLocation"}"#,
        );

        let command = super::build_cdp_get_node_for_location_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid getNodeForLocation command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(command.x, 12.0);
        assert_eq!(command.y, 34.0);
        assert!(command.include_user_agent_shadow_dom);
        assert!(command.ignore_pointer_events_none);
    }

    #[test]
    fn devtools_dom_entry_routes_get_node_for_location_command_to_dom_owner() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "x": 10,
            "y": 20
        });
        let cmd = Cmd::for_test(
            Some(96),
            "DOM.getNodeForLocation",
            &params,
            Some("SID-dom"),
            r#"{"id":96,"method":"DOM.getNodeForLocation"}"#,
        );
        let command = super::build_cdp_get_node_for_location_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid getNodeForLocation command");
        };

        let result = start_devtools_dom_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::GetNodeForLocation(command),
        );

        let Err(error) = result else {
            panic!("missing document should surface through the unified DOM start entry");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "NoDocumentLoaded");
    }

    #[test]
    fn devtools_dom_complete_routes_get_node_for_location_command_to_dom_owner() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "x": 10,
            "y": 20
        });
        let cmd = Cmd::for_test(
            Some(97),
            "DOM.getNodeForLocation",
            &params,
            Some("SID-dom"),
            r#"{"id":97,"method":"DOM.getNodeForLocation"}"#,
        );
        let command = super::build_cdp_get_node_for_location_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid getNodeForLocation command");
        };

        let result = super::complete_devtools_get_node_for_location_command(&mut conn, command);

        let Err(error) = result else {
            panic!("getNodeForLocation requires the pending renderer hit-test path");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(
            error.message,
            "GetNodeForLocationRequiresPendingRendererHitTest"
        );
    }

    #[test]
    fn cdp_query_selector_builds_protocol_neutral_dom_query_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "nodeId": 7,
            "selector": "section > p.target"
        });
        let cmd = Cmd::for_test(
            Some(64),
            "DOM.querySelector",
            &params,
            Some("SID-dom"),
            r#"{"id":64,"method":"DOM.querySelector"}"#,
        );

        let command = super::build_cdp_query_selector_command(&conn, &cmd, false);
        let Ok(command) = command else {
            panic!("valid querySelector command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-dom")
        );
        assert_eq!(
            command.root,
            Some(DevToolsDomNodeReference::FrontendNodeId(7))
        );
        assert_eq!(command.selector, "section > p.target");
        assert!(!command.multiple);
    }

    #[test]
    fn cdp_query_selector_all_builds_protocol_neutral_dom_query_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "nodeId": 8,
            "selector": ".item"
        });
        let cmd = Cmd::for_test(
            Some(65),
            "DOM.querySelectorAll",
            &params,
            Some("SID-dom"),
            r#"{"id":65,"method":"DOM.querySelectorAll"}"#,
        );

        let command = super::build_cdp_query_selector_command(&conn, &cmd, true);
        let Ok(command) = command else {
            panic!("valid querySelectorAll command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.root,
            Some(DevToolsDomNodeReference::FrontendNodeId(8))
        );
        assert_eq!(command.selector, ".item");
        assert!(command.multiple);
    }

    #[test]
    fn devtools_dom_entry_routes_query_selector_command_to_dom_owner() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "nodeId": 9,
            "selector": "main"
        });
        let cmd = Cmd::for_test(
            Some(66),
            "DOM.querySelector",
            &params,
            Some("SID-dom"),
            r#"{"id":66,"method":"DOM.querySelector"}"#,
        );
        let command = super::build_cdp_query_selector_command(&conn, &cmd, false);
        let Ok(command) = command else {
            panic!("valid querySelector command");
        };

        let result = start_devtools_dom_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::QuerySelector(command),
        );

        let Err(error) = result else {
            panic!("missing page should surface through the unified DOM entry");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "Could not find node with given id");
    }

    #[test]
    fn cdp_resolve_node_builds_protocol_neutral_dom_resolve_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "nodeId": 10,
            "executionContextId": 42,
            "objectGroup": "webdriver"
        });
        let cmd = Cmd::for_test(
            Some(67),
            "DOM.resolveNode",
            &params,
            Some("SID-dom"),
            r#"{"id":67,"method":"DOM.resolveNode"}"#,
        );

        let command = super::build_cdp_resolve_node_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid resolveNode command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-dom")
        );
        assert_eq!(
            command.reference,
            DevToolsDomNodeReference::FrontendNodeId(10)
        );
        assert_eq!(command.execution_context_id, Some(42));
        assert_eq!(command.object_group.as_deref(), Some("webdriver"));
    }

    #[test]
    fn cdp_resolve_node_preserves_backend_node_reference_source() {
        let conn = CdpConnection::new();
        let params = json!({
            "backendNodeId": 31,
            "objectGroup": "backend"
        });
        let cmd = Cmd::for_test(
            Some(107),
            "DOM.resolveNode",
            &params,
            Some("SID-dom"),
            r#"{"id":107,"method":"DOM.resolveNode"}"#,
        );

        let command = super::build_cdp_resolve_node_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid resolveNode backendNodeId command");
        };

        assert_eq!(
            command.reference,
            DevToolsDomNodeReference::BackendNodeId(31)
        );
        assert_eq!(command.object_group.as_deref(), Some("backend"));
    }

    #[test]
    fn cdp_resolve_node_without_node_reference_keeps_cdp_invalid_param_shape() {
        let conn = CdpConnection::new();
        let params = json!({
            "objectGroup": "webdriver"
        });
        let cmd = Cmd::for_test(
            Some(68),
            "DOM.resolveNode",
            &params,
            Some("SID-dom"),
            r#"{"id":68,"method":"DOM.resolveNode"}"#,
        );

        let result = super::build_cdp_resolve_node_command(&conn, &cmd);

        let Err(error) = result else {
            panic!("resolveNode without nodeId/backendNodeId should be rejected");
        };
        assert_eq!(error.code, -32602);
        assert_eq!(error.message, "InvalidParam");
    }

    #[test]
    fn pending_dom_start_error_preserves_invalid_param_as_invalid_argument() {
        let singular = super::PendingDomCommandStartError {
            code: -32602,
            message: "InvalidParam".to_owned(),
        };
        let plural = super::PendingDomCommandStartError {
            code: -32602,
            message: "InvalidParams".to_owned(),
        };
        let selector = super::PendingDomCommandStartError {
            code: -32602,
            message: "The selector is not a valid selector".to_owned(),
        };

        let singular: crate::devtools_runtime::DevToolsError = singular.into();
        let plural: crate::devtools_runtime::DevToolsError = plural.into();
        let selector: crate::devtools_runtime::DevToolsError = selector.into();

        assert_eq!(singular.kind, DevToolsErrorKind::InvalidArgument);
        assert_eq!(plural.kind, DevToolsErrorKind::InvalidArgument);
        assert_eq!(selector.kind, DevToolsErrorKind::InvalidSelector);
    }

    #[test]
    fn devtools_dom_entry_routes_resolve_node_command_to_dom_owner() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "nodeId": 11
        });
        let cmd = Cmd::for_test(
            Some(69),
            "DOM.resolveNode",
            &params,
            Some("SID-dom"),
            r#"{"id":69,"method":"DOM.resolveNode"}"#,
        );
        let command = super::build_cdp_resolve_node_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid resolveNode command");
        };

        let result = start_devtools_dom_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::ResolveNode(command),
        );

        let Err(error) = result else {
            panic!("missing document should surface through the unified DOM entry");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "NoDocumentLoaded");
    }

    #[test]
    fn cdp_get_attributes_builds_protocol_neutral_dom_attributes_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "nodeId": 12
        });
        let cmd = Cmd::for_test(
            Some(70),
            "DOM.getAttributes",
            &params,
            Some("SID-dom"),
            r#"{"id":70,"method":"DOM.getAttributes"}"#,
        );

        let command = super::build_cdp_get_attributes_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid getAttributes command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-dom")
        );
        assert_eq!(
            command.reference,
            DevToolsDomNodeReference::FrontendNodeId(12)
        );
    }

    #[test]
    fn devtools_dom_complete_entry_requires_pending_get_attributes_command() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "nodeId": 13
        });
        let cmd = Cmd::for_test(
            Some(71),
            "DOM.getAttributes",
            &params,
            Some("SID-dom"),
            r#"{"id":71,"method":"DOM.getAttributes"}"#,
        );
        let command = super::build_cdp_get_attributes_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid getAttributes command");
        };

        let result = super::complete_devtools_dom_command(
            &mut conn,
            DevToolsCommand::GetAttributes(command),
        );

        let Err(error) = result else {
            panic!("getAttributes sync completion should require a pending DOM command");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "MissingDomCommand");
    }

    #[test]
    fn cdp_push_nodes_by_backend_ids_builds_protocol_neutral_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "backendNodeIds": [14, 15, 16]
        });
        let cmd = Cmd::for_test(
            Some(102),
            "DOM.pushNodesByBackendIdsToFrontend",
            &params,
            Some("SID-dom"),
            r#"{"id":102,"method":"DOM.pushNodesByBackendIdsToFrontend"}"#,
        );

        let command = super::build_cdp_push_nodes_by_backend_ids_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid pushNodesByBackendIds command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-dom")
        );
        assert_eq!(command.backend_node_ids, vec![14, 15, 16]);
    }

    #[test]
    fn devtools_dom_complete_entry_requires_pending_push_nodes_by_backend_ids_command() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "backendNodeIds": [17]
        });
        let cmd = Cmd::for_test(
            Some(103),
            "DOM.pushNodesByBackendIdsToFrontend",
            &params,
            Some("SID-dom"),
            r#"{"id":103,"method":"DOM.pushNodesByBackendIdsToFrontend"}"#,
        );
        let command = super::build_cdp_push_nodes_by_backend_ids_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid pushNodesByBackendIds command");
        };

        let result = super::complete_devtools_dom_command(
            &mut conn,
            DevToolsCommand::PushNodesByBackendIds(command),
        );

        let Err(error) = result else {
            panic!("pushNodesByBackendIds sync completion should require a pending DOM command");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "MissingDomCommand");
    }

    #[test]
    fn cdp_request_node_builds_protocol_neutral_object_reference_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "objectId": "remote-object-1"
        });
        let cmd = Cmd::for_test(
            Some(72),
            "DOM.requestNode",
            &params,
            Some("SID-dom"),
            r#"{"id":72,"method":"DOM.requestNode"}"#,
        );

        let command = super::build_cdp_dom_object_reference_command(
            &conn,
            &cmd,
            DevToolsDomObjectReferenceOperation::RequestNode,
        );
        let Ok(Some(command)) = command else {
            panic!("valid requestNode object reference command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-dom")
        );
        assert_eq!(command.object_id.as_str(), "remote-object-1");
        assert_eq!(
            command.operation,
            DevToolsDomObjectReferenceOperation::RequestNode
        );
    }

    #[test]
    fn cdp_get_outer_html_without_object_id_keeps_node_reference_path() {
        let conn = CdpConnection::new();
        let params = json!({
            "nodeId": 14
        });
        let cmd = Cmd::for_test(
            Some(73),
            "DOM.getOuterHTML",
            &params,
            Some("SID-dom"),
            r#"{"id":73,"method":"DOM.getOuterHTML"}"#,
        );

        let command = super::build_cdp_dom_object_reference_command(
            &conn,
            &cmd,
            DevToolsDomObjectReferenceOperation::GetOuterHtml {
                include_shadow_dom: false,
            },
        );

        let Ok(None) = command else {
            panic!("getOuterHTML without objectId should stay on typed node reference path");
        };
    }

    #[test]
    fn cdp_get_outer_html_builds_protocol_neutral_node_reference_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "nodeId": 15,
            "includeShadowDOM": true
        });
        let cmd = Cmd::for_test(
            Some(76),
            "DOM.getOuterHTML",
            &params,
            Some("SID-dom"),
            r#"{"id":76,"method":"DOM.getOuterHTML"}"#,
        );

        let command = super::build_cdp_get_outer_html_command(&conn, &cmd);
        let Ok(Some(command)) = command else {
            panic!("valid getOuterHTML typed node reference command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.reference,
            Some(DevToolsDomNodeReference::FrontendNodeId(15))
        );
        assert!(command.include_shadow_dom);
    }

    #[test]
    fn cdp_get_outer_html_object_id_keeps_object_reference_path() {
        let conn = CdpConnection::new();
        let params = json!({
            "objectId": "remote-object-outer",
            "includeShadowDOM": true
        });
        let cmd = Cmd::for_test(
            Some(77),
            "DOM.getOuterHTML",
            &params,
            Some("SID-dom"),
            r#"{"id":77,"method":"DOM.getOuterHTML"}"#,
        );

        let object_command = super::build_cdp_dom_object_reference_command(
            &conn,
            &cmd,
            DevToolsDomObjectReferenceOperation::GetOuterHtml {
                include_shadow_dom: true,
            },
        );
        let Ok(Some(object_command)) = object_command else {
            panic!("valid getOuterHTML object reference command");
        };
        assert_eq!(object_command.object_id.as_str(), "remote-object-outer");
        assert_eq!(
            object_command.operation,
            DevToolsDomObjectReferenceOperation::GetOuterHtml {
                include_shadow_dom: true,
            }
        );

        let command = super::build_cdp_get_outer_html_command(&conn, &cmd);

        let Ok(None) = command else {
            panic!("getOuterHTML with objectId should stay on object reference path");
        };
    }

    #[test]
    fn devtools_dom_complete_entry_requires_pending_get_outer_html_command() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "nodeId": 16
        });
        let cmd = Cmd::for_test(
            Some(78),
            "DOM.getOuterHTML",
            &params,
            Some("SID-dom"),
            r#"{"id":78,"method":"DOM.getOuterHTML"}"#,
        );
        let command = super::build_cdp_get_outer_html_command(&conn, &cmd);
        let Ok(Some(command)) = command else {
            panic!("valid getOuterHTML typed node reference command");
        };

        let result =
            super::complete_devtools_dom_command(&mut conn, DevToolsCommand::GetOuterHtml(command));

        let Err(error) = result else {
            panic!("getOuterHTML sync completion should require a pending DOM command");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "MissingDomCommand");
    }

    #[test]
    fn cdp_scroll_into_view_builds_protocol_neutral_node_reference_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "backendNodeId": 17,
            "rect": { "x": 1, "y": 2, "width": 3, "height": 4 }
        });
        let cmd = Cmd::for_test(
            Some(79),
            "DOM.scrollIntoViewIfNeeded",
            &params,
            Some("SID-dom"),
            r#"{"id":79,"method":"DOM.scrollIntoViewIfNeeded"}"#,
        );

        let command = super::build_cdp_scroll_into_view_if_needed_command(&conn, &cmd);
        let Ok(Some(command)) = command else {
            panic!("valid scrollIntoViewIfNeeded typed node reference command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.reference,
            Some(DevToolsDomNodeReference::BackendNodeId(17))
        );
        assert_eq!(
            command.rect,
            moli_core::page::DomScrollIntoViewRect::try_new(1.0, 2.0, 3.0, 4.0)
        );
    }

    #[test]
    fn cdp_scroll_into_view_rejects_every_non_finite_rect_component() {
        for rect in [
            super::ScrollIntoViewRectParams {
                x: f64::NAN,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
            super::ScrollIntoViewRectParams {
                x: 0.0,
                y: f64::INFINITY,
                width: 0.0,
                height: 0.0,
            },
            super::ScrollIntoViewRectParams {
                x: 0.0,
                y: 0.0,
                width: f64::NEG_INFINITY,
                height: 0.0,
            },
            super::ScrollIntoViewRectParams {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: f64::NAN,
            },
        ] {
            let Err(error) = super::validated_scroll_into_view_rect(Some(rect)) else {
                panic!("non-finite scroll rect should be rejected");
            };
            assert_eq!(error.code, -32602);
            assert_eq!(error.message, "InvalidParams");
        }
    }

    #[test]
    fn cdp_scroll_into_view_object_id_keeps_object_reference_path() {
        let conn = CdpConnection::new();
        let params = json!({
            "objectId": "remote-object-scroll"
        });
        let cmd = Cmd::for_test(
            Some(80),
            "DOM.scrollIntoViewIfNeeded",
            &params,
            Some("SID-dom"),
            r#"{"id":80,"method":"DOM.scrollIntoViewIfNeeded"}"#,
        );

        let command = super::build_cdp_scroll_into_view_if_needed_command(&conn, &cmd);

        let Ok(None) = command else {
            panic!("scrollIntoViewIfNeeded with objectId should stay on object reference path");
        };
    }

    #[test]
    fn devtools_dom_start_entry_routes_scroll_command_to_renderer_owner() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "nodeId": 18
        });
        let cmd = Cmd::for_test(
            Some(81),
            "DOM.scrollIntoViewIfNeeded",
            &params,
            Some("SID-dom"),
            r#"{"id":81,"method":"DOM.scrollIntoViewIfNeeded"}"#,
        );
        let command = super::build_cdp_scroll_into_view_if_needed_command(&conn, &cmd);
        let Ok(Some(command)) = command else {
            panic!("valid scrollIntoViewIfNeeded typed node reference command");
        };

        let result = start_devtools_dom_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::ScrollIntoViewIfNeeded(command),
        );

        let Err(error) = result else {
            panic!("missing document should surface through the unified DOM start entry");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "NoDocumentLoaded");
    }

    #[test]
    fn cdp_get_box_model_builds_protocol_neutral_geometry_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "backendNodeId": 19
        });
        let cmd = Cmd::for_test(
            Some(82),
            "DOM.getBoxModel",
            &params,
            Some("SID-dom"),
            r#"{"id":82,"method":"DOM.getBoxModel"}"#,
        );

        let command = super::build_cdp_dom_geometry_command(
            &conn,
            &cmd,
            DevToolsDomGeometryOperation::GetBoxModel,
        );
        let Ok(Some(command)) = command else {
            panic!("valid getBoxModel typed node reference command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.reference,
            DevToolsDomNodeReference::BackendNodeId(19)
        );
        assert_eq!(command.operation, DevToolsDomGeometryOperation::GetBoxModel);
    }

    #[test]
    fn cdp_get_content_quads_object_id_keeps_object_reference_path() {
        let conn = CdpConnection::new();
        let params = json!({
            "objectId": "remote-object-geometry"
        });
        let cmd = Cmd::for_test(
            Some(83),
            "DOM.getContentQuads",
            &params,
            Some("SID-dom"),
            r#"{"id":83,"method":"DOM.getContentQuads"}"#,
        );

        let command = super::build_cdp_dom_geometry_command(
            &conn,
            &cmd,
            DevToolsDomGeometryOperation::GetContentQuads,
        );

        let Ok(None) = command else {
            panic!("getContentQuads with objectId should stay on object reference path");
        };
    }

    #[test]
    fn devtools_dom_entry_routes_geometry_command_to_dom_owner() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "nodeId": 20
        });
        let cmd = Cmd::for_test(
            Some(84),
            "DOM.getContentQuads",
            &params,
            Some("SID-dom"),
            r#"{"id":84,"method":"DOM.getContentQuads"}"#,
        );
        let command = super::build_cdp_dom_geometry_command(
            &conn,
            &cmd,
            DevToolsDomGeometryOperation::GetContentQuads,
        );
        let Ok(Some(command)) = command else {
            panic!("valid getContentQuads typed node reference command");
        };

        let result = start_devtools_dom_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::DomGeometry(command),
        );

        let Err(error) = result else {
            panic!("missing document should surface through the unified DOM entry");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "NoDocumentLoaded");
    }

    #[test]
    fn cdp_remove_node_builds_protocol_neutral_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "backendNodeId": 21
        });
        let cmd = Cmd::for_test(
            Some(100),
            "DOM.removeNode",
            &params,
            Some("SID-dom"),
            r#"{"id":100,"method":"DOM.removeNode"}"#,
        );

        let command = super::build_cdp_remove_node_command(&conn, &cmd);
        let Ok(Some(command)) = command else {
            panic!("valid removeNode command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.reference,
            DevToolsDomNodeReference::BackendNodeId(21)
        );
    }

    #[test]
    fn devtools_dom_entry_routes_remove_node_command_to_dom_owner() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "nodeId": 22
        });
        let cmd = Cmd::for_test(
            Some(101),
            "DOM.removeNode",
            &params,
            Some("SID-dom"),
            r#"{"id":101,"method":"DOM.removeNode"}"#,
        );
        let command = super::build_cdp_remove_node_command(&conn, &cmd);
        let Ok(Some(command)) = command else {
            panic!("valid removeNode command");
        };

        let result = start_devtools_dom_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::RemoveNode(command),
        );

        let Err(error) = result else {
            panic!("missing document should surface through the unified DOM start entry");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "NoDocumentLoaded");
    }

    #[test]
    fn cdp_describe_node_builds_protocol_neutral_node_reference_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "backendNodeId": 21,
            "depth": 2,
            "pierce": true
        });
        let cmd = Cmd::for_test(
            Some(85),
            "DOM.describeNode",
            &params,
            Some("SID-dom"),
            r#"{"id":85,"method":"DOM.describeNode"}"#,
        );

        let command = super::build_cdp_describe_node_command(&conn, &cmd);
        let Ok(Some(command)) = command else {
            panic!("valid describeNode typed node reference command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.reference,
            Some(DevToolsDomNodeReference::BackendNodeId(21))
        );
        assert_eq!(command.depth, 2);
        assert!(command.pierce);
    }

    #[test]
    fn cdp_describe_node_object_id_keeps_object_reference_path() {
        let conn = CdpConnection::new();
        let params = json!({
            "objectId": "remote-object-describe"
        });
        let cmd = Cmd::for_test(
            Some(86),
            "DOM.describeNode",
            &params,
            Some("SID-dom"),
            r#"{"id":86,"method":"DOM.describeNode"}"#,
        );

        let command = super::build_cdp_describe_node_command(&conn, &cmd);

        let Ok(None) = command else {
            panic!("describeNode with objectId should stay on object reference path");
        };
    }

    #[test]
    fn devtools_dom_entry_routes_describe_node_command_to_dom_owner() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "nodeId": 22
        });
        let cmd = Cmd::for_test(
            Some(87),
            "DOM.describeNode",
            &params,
            Some("SID-dom"),
            r#"{"id":87,"method":"DOM.describeNode"}"#,
        );
        let command = super::build_cdp_describe_node_command(&conn, &cmd);
        let Ok(Some(command)) = command else {
            panic!("valid describeNode typed node reference command");
        };

        let result = start_devtools_dom_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::DescribeNode(command),
        );

        let Err(error) = result else {
            panic!("missing document should surface through the unified DOM start entry");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "NoDocumentLoaded");
    }

    #[test]
    fn devtools_dom_complete_entry_requires_pending_describe_node_command() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "nodeId": 23
        });
        let cmd = Cmd::for_test(
            Some(88),
            "DOM.describeNode",
            &params,
            Some("SID-dom"),
            r#"{"id":88,"method":"DOM.describeNode"}"#,
        );
        let command = super::build_cdp_describe_node_command(&conn, &cmd);
        let Ok(Some(command)) = command else {
            panic!("valid describeNode typed node reference command");
        };

        let result =
            super::complete_devtools_dom_command(&mut conn, DevToolsCommand::DescribeNode(command));

        let Err(error) = result else {
            panic!("describeNode sync completion should require a pending DOM command");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "MissingDomCommand");
    }

    #[test]
    fn cdp_describe_node_object_id_builds_protocol_neutral_object_reference_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "objectId": "remote-object-3",
            "depth": 2,
            "pierce": true
        });
        let cmd = Cmd::for_test(
            Some(75),
            "DOM.describeNode",
            &params,
            Some("SID-dom"),
            r#"{"id":75,"method":"DOM.describeNode"}"#,
        );

        let command = super::build_cdp_dom_object_reference_command(
            &conn,
            &cmd,
            DevToolsDomObjectReferenceOperation::DescribeNode {
                depth: 2,
                pierce: true,
            },
        );
        let Ok(Some(command)) = command else {
            panic!("valid describeNode object reference command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(command.object_id.as_str(), "remote-object-3");
        assert_eq!(
            command.operation,
            DevToolsDomObjectReferenceOperation::DescribeNode {
                depth: 2,
                pierce: true
            }
        );
    }

    #[test]
    fn devtools_dom_entry_routes_object_reference_command_to_dom_owner() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "objectId": "remote-object-2"
        });
        let cmd = Cmd::for_test(
            Some(74),
            "DOM.requestNode",
            &params,
            Some("SID-dom"),
            r#"{"id":74,"method":"DOM.requestNode"}"#,
        );
        let command = super::build_cdp_dom_object_reference_command(
            &conn,
            &cmd,
            DevToolsDomObjectReferenceOperation::RequestNode,
        );
        let Ok(Some(command)) = command else {
            panic!("valid requestNode object reference command");
        };

        let result = start_devtools_dom_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::DomObjectReference(command),
        );

        let Err(error) = result else {
            panic!("missing document should surface through the unified DOM entry");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "NoDocumentLoaded");
    }

    #[test]
    fn cdp_set_file_input_files_builds_protocol_neutral_object_command() {
        let conn = CdpConnection::new();
        let file_path = unique_test_file_path("set-file-object");
        std::fs::write(&file_path, b"upload bytes").expect("test upload file should be writable");
        let params = json!({
            "objectId": "remote-file-input",
            "files": [file_path.to_string_lossy()]
        });
        let cmd = Cmd::for_test(
            Some(104),
            "DOM.setFileInputFiles",
            &params,
            Some("SID-dom"),
            r#"{"id":104,"method":"DOM.setFileInputFiles"}"#,
        );

        let command =
            super::super::set_file_input::build_cdp_set_file_input_files_command(&conn, &cmd);
        let _ = std::fs::remove_file(&file_path);
        let Ok(Some(command)) = command else {
            panic!("valid setFileInputFiles object command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-dom")
        );
        assert_eq!(command.object_id.as_str(), "remote-file-input");
        assert_eq!(command.files.len(), 1);
        assert_eq!(command.files[0].bytes, b"upload bytes");
        assert!(
            command.files[0]
                .name
                .starts_with("moli-protocol-set-file-object-"),
            "unexpected selected file name: {}",
            command.files[0].name
        );
        assert!(!command.append);
    }

    #[test]
    fn cdp_set_file_input_files_node_reference_falls_back_to_pending_node_reference_path() {
        let conn = CdpConnection::new();
        let params = json!({
            "backendNodeId": 25,
            "files": ["/tmp/upload.txt"]
        });
        let cmd = Cmd::for_test(
            Some(105),
            "DOM.setFileInputFiles",
            &params,
            Some("SID-dom"),
            r#"{"id":105,"method":"DOM.setFileInputFiles"}"#,
        );

        let command =
            super::super::set_file_input::build_cdp_set_file_input_files_command(&conn, &cmd);

        let Ok(None) = command else {
            panic!("setFileInputFiles without objectId should use node-reference pending path");
        };
    }
}
