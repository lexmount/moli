pub(super) mod native;

use serde::Deserialize;
use serde_json::{Value, json};

use super::node_references::{NodeReferenceParams, devtools_node_reference_from_ids};
use super::*;
use crate::automation::{
    AutomationCommand, AutomationResult, DevToolsDescribeNodeCommand, DevToolsDescribeNodeResult,
    DevToolsDomAttribute, DevToolsDomBoxModel, DevToolsDomGeometryCommand,
    DevToolsDomGeometryOperation, DevToolsDomGeometryResult, DevToolsDomNodeReference,
    DevToolsDomObjectReferenceCommand, DevToolsDomObjectReferenceOperation, DevToolsDomQuad,
    DevToolsError, DevToolsErrorKind, DevToolsFrameId, DevToolsGetAttributesCommand,
    DevToolsGetAttributesResult, DevToolsGetDocumentCommand, DevToolsGetFrameOwnerCommand,
    DevToolsGetFrameOwnerResult, DevToolsGetNodeForLocationCommand,
    DevToolsGetNodeForLocationResult, DevToolsGetOuterHtmlCommand, DevToolsGetOuterHtmlResult,
    DevToolsGetPropertyCommand, DevToolsGetPropertyResult, DevToolsGetTextCommand,
    DevToolsGetTextResult, DevToolsPushNodesByBackendIdsCommand,
    DevToolsPushNodesByBackendIdsResult, DevToolsQuerySelectorCommand, DevToolsQuerySelectorResult,
    DevToolsRemoteHandleId, DevToolsRemoveNodeCommand, DevToolsRequestChildNodesCommand,
    DevToolsResolveNodeCommand, DevToolsResolveNodeResult, DevToolsScrollIntoViewIfNeededCommand,
};
use crate::conn::BackgroundProtocolEvent;
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

mod command_start;
use command_start::*;
mod pending_command;
use pending_command::*;
mod node_completion;
use node_completion::*;
mod protocol_dispatch;
pub(crate) use node_completion::complete_pending_dom_command_output_plan;
pub(super) use node_completion::{
    attributes_result_from_renderer_resolution, devtools_dom_geometry_result_from_renderer,
    property_result_from_renderer_resolution, query_selector_result_from_renderer_resolution,
    text_result_from_renderer_resolution,
};
pub(crate) use protocol_dispatch::execute_devtools_dom_command_async;
use protocol_dispatch::*;
pub(super) use protocol_dispatch::{
    get_outer_html, push_nodes_by_backend_ids_to_frontend, renderer_backend_node_id_for_reference,
    scroll_into_view_if_needed,
};

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
    pub(super) pending: PendingPageCommand,
}

pub(crate) struct CompletedDomCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    kind: PendingDomCommandKind,
    completed: Result<Box<CompletedPageCommand>, String>,
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
            Ok(completion) => completion.renderer_output_predecessor(),
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
    Complete(Box<Result<AutomationResult, DevToolsError>>),
}

pub(super) fn devtools_dom_command_task_complete(
    result: Result<AutomationResult, DevToolsError>,
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
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_owner(&owner);
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
        pending,
    }))
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
        let completed = Box::pin(self.pending.wait())
            .await
            .map(Box::new)
            .map_err(|error| error.to_string());
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

struct PendingResolveRuntimeObjectForReference {
    pending: PendingPageCommand,
    cache_top_frame_id: Option<Option<String>>,
}

#[cfg(test)]
mod protocol_neutral_tests;
