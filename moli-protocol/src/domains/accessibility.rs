use crate::conn::{CdpConnection, Cmd, CommandOwnerScope, RendererDispatchLane};
use crate::domains::actions::AccessibilityAction;
use crate::domains::command_output::CommandOutputPlan;
use moli_core::page::{
    CompletedPageCommand, PendingPageCommand, RendererDomFrontendNodeBindingResolution,
};
use serde_json::{Value, json};

mod helpers;
#[cfg(test)]
mod tests;

pub(crate) struct PendingAccessibilityCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    kind: PendingAccessibilityCommandKind,
    pending: PendingPageCommand,
}

pub(crate) struct CompletedAccessibilityCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    kind: PendingAccessibilityCommandKind,
    completed: Result<Box<CompletedPageCommand>, String>,
}

pub(crate) enum AccessibilityCommandDispatchStep {
    Pending(PendingAccessibilityCommandDispatch),
    Complete(CommandOutputPlan),
}

pub(crate) fn command_waits_for_document_projection(cmd: &Cmd<'_>) -> bool {
    cmd.parse_action::<AccessibilityAction>()
        .is_some_and(AccessibilityAction::queries_tree)
}

enum PendingAccessibilityCommandKind {
    TopFrameFullTree {
        max_depth: Option<i32>,
    },
    TopFrameRoot,
    ChildFrameFullTree {
        max_depth: Option<i32>,
    },
    ChildFrameRoot,
    ObjectAccessibilityPayloads {
        frame_id: String,
        top_frame_id: String,
        operation: AccessibilityNodeOperation,
    },
    BackendAccessibilityPayloads {
        frame_id: String,
        top_frame_id: String,
        operation: AccessibilityNodeOperation,
    },
    FrontendAccessibilityPayloads {
        frame_id: String,
        top_frame_id: String,
        operation: AccessibilityNodeOperation,
    },
}

#[derive(Clone)]
enum AccessibilityNodeOperation {
    Children,
    Ancestors,
    Query {
        accessible_name: Option<String>,
        role: Option<String>,
    },
    Partial {
        fetch_relatives: bool,
    },
}

struct PendingAccessibilityCommandStartError {
    code: i32,
    message: String,
}

impl PendingAccessibilityCommandDispatch {
    pub(crate) fn renderer_dispatch_lane(&self) -> Option<RendererDispatchLane> {
        self.pending
            .renderer_agent_attachment_id()
            .map(|_| RendererDispatchLane::Main)
    }

    fn from_command(
        conn: &CdpConnection,
        cmd: &Cmd<'_>,
        kind: PendingAccessibilityCommandKind,
        pending: PendingPageCommand,
    ) -> Self {
        Self {
            command_id: cmd.id,
            owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
            kind,
            pending,
        }
    }

    pub async fn wait(self) -> CompletedAccessibilityCommandDispatch {
        let completed = Box::pin(self.pending.wait())
            .await
            .map(Box::new)
            .map_err(|error| error.to_string());
        CompletedAccessibilityCommandDispatch {
            command_id: self.command_id,
            owner_scope: self.owner_scope,
            kind: self.kind,
            completed,
        }
    }
}

impl CompletedAccessibilityCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }
}

impl PendingAccessibilityCommandStartError {
    fn invalid_params() -> Self {
        Self {
            code: -32602,
            message: "InvalidParams".to_owned(),
        }
    }

    fn browser_context_not_loaded() -> Self {
        Self {
            code: -31998,
            message: "BrowserContextNotLoaded".to_owned(),
        }
    }

    fn no_document_loaded() -> Self {
        Self {
            code: -32000,
            message: "NoDocumentLoaded".to_owned(),
        }
    }

    fn node_not_found() -> Self {
        Self {
            code: -32000,
            message: "Could not find node with given id".to_owned(),
        }
    }

    fn document_access_error(message: impl Into<String>) -> Self {
        Self {
            code: -32000,
            message: message.into(),
        }
    }

    fn renderer_error(error: impl std::fmt::Display) -> Self {
        Self {
            code: -32000,
            message: error.to_string(),
        }
    }
}

pub(crate) fn try_start_accessibility_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<AccessibilityCommandDispatchStep> {
    match cmd.parse_action::<AccessibilityAction>() {
        Some(AccessibilityAction::Enable | AccessibilityAction::Disable) => Some(
            AccessibilityCommandDispatchStep::Complete(CommandOutputPlan::success()),
        ),
        Some(action) if action.queries_tree() => {
            match start_pending_accessibility_command(conn, cmd, action) {
                Ok(Some(pending)) => Some(AccessibilityCommandDispatchStep::Pending(pending)),
                Ok(None) => Some(AccessibilityCommandDispatchStep::Complete(
                    CommandOutputPlan::error(-32000, "AccessibilityCommandDidNotStart"),
                )),
                Err(error) => Some(AccessibilityCommandDispatchStep::Complete(
                    CommandOutputPlan::error(error.code, error.message),
                )),
            }
        }
        None => Some(AccessibilityCommandDispatchStep::Complete(
            CommandOutputPlan::error(-32601, "UnknownMethod"),
        )),
        Some(_) => Some(AccessibilityCommandDispatchStep::Complete(
            CommandOutputPlan::success(),
        )),
    }
}

fn start_pending_accessibility_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: AccessibilityAction,
) -> Result<Option<PendingAccessibilityCommandDispatch>, PendingAccessibilityCommandStartError> {
    match action {
        AccessibilityAction::GetFullAxTree => {
            start_pending_child_frame_full_tree_command(conn, cmd)
        }
        AccessibilityAction::GetRootAxNode => start_pending_child_frame_root_command(conn, cmd),
        AccessibilityAction::GetChildAxNodes => {
            start_pending_child_frame_children_command(conn, cmd)
        }
        AccessibilityAction::GetAxNodeAndAncestors => {
            start_pending_child_frame_ancestors_command(conn, cmd)
        }
        AccessibilityAction::QueryAxTree => start_pending_child_frame_query_command(conn, cmd),
        AccessibilityAction::GetPartialAxTree => {
            start_pending_child_frame_partial_command(conn, cmd)
        }
        AccessibilityAction::Enable | AccessibilityAction::Disable => Ok(None),
    }
}

fn start_pending_child_frame_full_tree_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingAccessibilityCommandDispatch>, PendingAccessibilityCommandStartError> {
    let params: helpers::GetFullAxTreeParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => helpers::GetFullAxTreeParams {
            depth: None,
            frame_id: None,
        },
        Err(_) => return Err(PendingAccessibilityCommandStartError::invalid_params()),
    };
    let max_depth = match params.depth {
        Some(depth) => match i32::try_from(depth) {
            Ok(depth) => (depth >= 0).then_some(depth),
            Err(_) => return Err(PendingAccessibilityCommandStartError::invalid_params()),
        },
        None => None,
    };
    conn.ensure_document_accessible_for_session_owner(cmd.session_id)
        .map_err(PendingAccessibilityCommandStartError::document_access_error)?;
    start_pending_frame_scoped_accessibility_command(
        conn,
        cmd,
        params.frame_id.as_ref().map(AsRef::as_ref),
        PendingAccessibilityCommandKind::TopFrameFullTree { max_depth },
        PendingAccessibilityCommandKind::ChildFrameFullTree { max_depth },
    )
}

fn start_pending_child_frame_root_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingAccessibilityCommandDispatch>, PendingAccessibilityCommandStartError> {
    let params: helpers::FrameScopedParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => helpers::FrameScopedParams { frame_id: None },
        Err(_) => return Err(PendingAccessibilityCommandStartError::invalid_params()),
    };
    start_pending_frame_scoped_accessibility_command(
        conn,
        cmd,
        params.frame_id.as_deref(),
        PendingAccessibilityCommandKind::TopFrameRoot,
        PendingAccessibilityCommandKind::ChildFrameRoot,
    )
}

fn start_pending_child_frame_children_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingAccessibilityCommandDispatch>, PendingAccessibilityCommandStartError> {
    let params: helpers::ChildAxNodesParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingAccessibilityCommandStartError::invalid_params()),
    };
    let Some(backend_node_id) = helpers::parse_ax_backend_node_id(params.id.as_ref()) else {
        return Err(PendingAccessibilityCommandStartError::invalid_params());
    };
    start_pending_backend_reference_command(
        conn,
        cmd,
        params.frame_id.as_ref().map(AsRef::as_ref),
        backend_node_id,
        AccessibilityNodeOperation::Children,
    )
}

fn start_pending_child_frame_ancestors_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingAccessibilityCommandDispatch>, PendingAccessibilityCommandStartError> {
    let params: helpers::AncestorsParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingAccessibilityCommandStartError::invalid_params()),
    };
    if params.reference.object_id.is_some() {
        return start_pending_object_reference_command(
            conn,
            cmd,
            params.frame_id.as_deref(),
            params.reference,
            AccessibilityNodeOperation::Ancestors,
        );
    }
    if let Some(backend_node_id) = renderer_backend_node_id_for_reference(&params.reference) {
        return start_pending_backend_reference_command(
            conn,
            cmd,
            params.frame_id.as_deref(),
            backend_node_id,
            AccessibilityNodeOperation::Ancestors,
        );
    }
    let Some(frontend_node_id) = params.reference.node_id else {
        return Err(PendingAccessibilityCommandStartError::node_not_found());
    };
    start_pending_dom_node_reference_command(
        conn,
        cmd,
        params.frame_id.as_deref(),
        frontend_node_id,
        AccessibilityNodeOperation::Ancestors,
    )
}

fn start_pending_child_frame_query_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingAccessibilityCommandDispatch>, PendingAccessibilityCommandStartError> {
    let params: helpers::QueryAxTreeParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingAccessibilityCommandStartError::invalid_params()),
    };
    if params.reference.object_id.is_some() {
        return start_pending_object_reference_command(
            conn,
            cmd,
            params.frame_id.as_deref(),
            params.reference,
            AccessibilityNodeOperation::Query {
                accessible_name: params.accessible_name,
                role: params.role,
            },
        );
    }
    if let Some(backend_node_id) = renderer_backend_node_id_for_reference(&params.reference) {
        return start_pending_backend_reference_command(
            conn,
            cmd,
            params.frame_id.as_deref(),
            backend_node_id,
            AccessibilityNodeOperation::Query {
                accessible_name: params.accessible_name,
                role: params.role,
            },
        );
    }
    let Some(frontend_node_id) = params.reference.node_id else {
        return Err(PendingAccessibilityCommandStartError::node_not_found());
    };
    start_pending_dom_node_reference_command(
        conn,
        cmd,
        params.frame_id.as_deref(),
        frontend_node_id,
        AccessibilityNodeOperation::Query {
            accessible_name: params.accessible_name.clone(),
            role: params.role.clone(),
        },
    )
}

fn start_pending_child_frame_partial_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingAccessibilityCommandDispatch>, PendingAccessibilityCommandStartError> {
    let params: helpers::PartialAxTreeParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingAccessibilityCommandStartError::invalid_params()),
    };
    let fetch_relatives = params.fetch_relatives.unwrap_or(true);
    if params.reference.object_id.is_some() {
        return start_pending_object_reference_command(
            conn,
            cmd,
            params.frame_id.as_deref(),
            params.reference,
            AccessibilityNodeOperation::Partial { fetch_relatives },
        );
    }
    if let Some(backend_node_id) = renderer_backend_node_id_for_reference(&params.reference) {
        return start_pending_backend_reference_command(
            conn,
            cmd,
            params.frame_id.as_deref(),
            backend_node_id,
            AccessibilityNodeOperation::Partial { fetch_relatives },
        );
    }
    let Some(frontend_node_id) = params.reference.node_id else {
        return Err(PendingAccessibilityCommandStartError::node_not_found());
    };
    start_pending_dom_node_reference_command(
        conn,
        cmd,
        params.frame_id.as_deref(),
        frontend_node_id,
        AccessibilityNodeOperation::Partial { fetch_relatives },
    )
}

fn accessibility_inspection_for_owner<'a>(
    conn: &'a CdpConnection,
    owner: &CommandOwnerScope,
) -> Option<moli_renderer_v8::RendererAccessibilityInspection<'a>> {
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
    conn.renderer_inspection_binding_for_owner(
        owner,
        moli_core::page::RendererInspectorCommandRoute::MainThread,
    )
    .ok()
    .map(|binding| binding.accessibility_inspection(session))
}

fn start_pending_object_reference_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    frame_id: Option<&str>,
    reference: helpers::NodeReferenceParams,
    operation: AccessibilityNodeOperation,
) -> Result<Option<PendingAccessibilityCommandDispatch>, PendingAccessibilityCommandStartError> {
    if conn
        .target_owner_identity_for_session(cmd.session_id)
        .is_none()
        && conn.browser_context.is_none()
    {
        return Err(PendingAccessibilityCommandStartError::browser_context_not_loaded());
    }
    conn.ensure_document_accessible_for_session_owner(cmd.session_id)
        .map_err(PendingAccessibilityCommandStartError::document_access_error)?;
    let Some(top_frame_id) = helpers::top_frame_id_for_session(conn, cmd.session_id) else {
        return Err(PendingAccessibilityCommandStartError::no_document_loaded());
    };
    let resolved_frame_id = frame_id.unwrap_or(top_frame_id.as_str()).to_owned();
    let Some(object_id) = reference.object_id.as_deref() else {
        return Err(PendingAccessibilityCommandStartError {
            code: -32000,
            message: "Could not find node with given id".to_owned(),
        });
    };
    let Some(inspection) =
        accessibility_inspection_for_owner(conn, &CommandOwnerScope::capture(conn, cmd.session_id))
    else {
        return Err(PendingAccessibilityCommandStartError::no_document_loaded());
    };
    let pending = start_accessibility_object_inspection(&inspection, object_id, &operation)
        .map_err(PendingAccessibilityCommandStartError::renderer_error)?;
    Ok(Some(PendingAccessibilityCommandDispatch::from_command(
        conn,
        cmd,
        PendingAccessibilityCommandKind::ObjectAccessibilityPayloads {
            frame_id: resolved_frame_id,
            top_frame_id,
            operation,
        },
        pending,
    )))
}

fn renderer_backend_node_id_for_reference(reference: &helpers::NodeReferenceParams) -> Option<u32> {
    reference.backend_node_id
}

fn start_pending_dom_node_reference_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    frame_id: Option<&str>,
    frontend_node_id: u32,
    operation: AccessibilityNodeOperation,
) -> Result<Option<PendingAccessibilityCommandDispatch>, PendingAccessibilityCommandStartError> {
    if conn
        .target_owner_identity_for_session(cmd.session_id)
        .is_none()
        && conn.browser_context.is_none()
    {
        return Err(PendingAccessibilityCommandStartError::browser_context_not_loaded());
    }
    conn.ensure_document_accessible_for_session_owner(cmd.session_id)
        .map_err(PendingAccessibilityCommandStartError::document_access_error)?;
    let Some(top_frame_id) = helpers::top_frame_id_for_session(conn, cmd.session_id) else {
        return Err(PendingAccessibilityCommandStartError::no_document_loaded());
    };
    let resolved_frame_id = frame_id.unwrap_or(top_frame_id.as_str()).to_owned();
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let Some(inspection) = crate::domains::dom::dom_inspection_for_owner(conn, &owner) else {
        return Err(PendingAccessibilityCommandStartError::no_document_loaded());
    };
    let pending = inspection
        .start_document_frontend_node_binding(frontend_node_id)
        .map(PendingPageCommand::from_inspector_main_route)
        .map_err(PendingAccessibilityCommandStartError::renderer_error)?;
    Ok(Some(PendingAccessibilityCommandDispatch::from_command(
        conn,
        cmd,
        PendingAccessibilityCommandKind::FrontendAccessibilityPayloads {
            frame_id: resolved_frame_id,
            top_frame_id,
            operation,
        },
        pending,
    )))
}

fn start_accessibility_object_inspection(
    inspection: &moli_renderer_v8::RendererAccessibilityInspection<'_>,
    object_id: &str,
    operation: &AccessibilityNodeOperation,
) -> anyhow::Result<PendingPageCommand> {
    match operation {
        AccessibilityNodeOperation::Children => Err(anyhow::anyhow!(
            "children accessibility operation requires an AXNodeId"
        )),
        AccessibilityNodeOperation::Ancestors => {
            inspection.start_accessibility_node_and_ancestor_payloads_for_object_id(object_id)
        }
        AccessibilityNodeOperation::Query { .. } => {
            inspection.start_accessibility_tree_payloads_for_object_id(object_id)
        }
        AccessibilityNodeOperation::Partial { fetch_relatives } => inspection
            .start_accessibility_partial_tree_payloads_for_object_id(object_id, *fetch_relatives),
    }
    .map(PendingPageCommand::from_inspector_main_route)
}

fn start_pending_backend_reference_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    frame_id: Option<&str>,
    backend_node_id: u32,
    operation: AccessibilityNodeOperation,
) -> Result<Option<PendingAccessibilityCommandDispatch>, PendingAccessibilityCommandStartError> {
    if conn
        .target_owner_identity_for_session(cmd.session_id)
        .is_none()
        && conn.browser_context.is_none()
    {
        return Err(PendingAccessibilityCommandStartError::browser_context_not_loaded());
    }
    conn.ensure_document_accessible_for_session_owner(cmd.session_id)
        .map_err(PendingAccessibilityCommandStartError::document_access_error)?;
    let Some(top_frame_id) = helpers::top_frame_id_for_session(conn, cmd.session_id) else {
        return Err(PendingAccessibilityCommandStartError::no_document_loaded());
    };
    let resolved_frame_id = frame_id.unwrap_or(top_frame_id.as_str()).to_owned();
    let Some(inspection) =
        accessibility_inspection_for_owner(conn, &CommandOwnerScope::capture(conn, cmd.session_id))
    else {
        return Err(PendingAccessibilityCommandStartError::no_document_loaded());
    };
    let pending = start_accessibility_backend_inspection(&inspection, backend_node_id, &operation)
        .map_err(PendingAccessibilityCommandStartError::renderer_error)?;
    Ok(Some(PendingAccessibilityCommandDispatch::from_command(
        conn,
        cmd,
        PendingAccessibilityCommandKind::BackendAccessibilityPayloads {
            frame_id: resolved_frame_id,
            top_frame_id,
            operation,
        },
        pending,
    )))
}

fn start_accessibility_backend_inspection(
    inspection: &moli_renderer_v8::RendererAccessibilityInspection<'_>,
    backend_node_id: u32,
    operation: &AccessibilityNodeOperation,
) -> anyhow::Result<PendingPageCommand> {
    match operation {
        AccessibilityNodeOperation::Children => {
            inspection.start_accessibility_child_node_payloads_for_backend_node_id(backend_node_id)
        }
        AccessibilityNodeOperation::Ancestors => inspection
            .start_accessibility_node_and_ancestor_payloads_for_backend_node_id(backend_node_id),
        AccessibilityNodeOperation::Query { .. } => {
            inspection.start_accessibility_tree_payloads_for_backend_node_id(backend_node_id, None)
        }
        AccessibilityNodeOperation::Partial { fetch_relatives } => inspection
            .start_accessibility_partial_tree_payloads_for_backend_node_id(
                backend_node_id,
                *fetch_relatives,
            ),
    }
    .map(PendingPageCommand::from_inspector_main_route)
}

fn start_top_frame_accessibility_inspection(
    inspection: &moli_renderer_v8::RendererAccessibilityInspection<'_>,
    kind: &PendingAccessibilityCommandKind,
) -> anyhow::Result<PendingPageCommand> {
    match kind {
        PendingAccessibilityCommandKind::TopFrameFullTree { max_depth } => {
            inspection.start_accessibility_tree_payloads_for_document(*max_depth)
        }
        PendingAccessibilityCommandKind::TopFrameRoot => {
            inspection.start_accessibility_node_payload_for_document()
        }
        _ => unreachable!("top-frame accessibility kind must use a top-frame variant"),
    }
    .map(PendingPageCommand::from_inspector_main_route)
}

fn start_pending_frame_scoped_accessibility_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    frame_id: Option<&str>,
    top_frame_kind: PendingAccessibilityCommandKind,
    child_frame_kind: PendingAccessibilityCommandKind,
) -> Result<Option<PendingAccessibilityCommandDispatch>, PendingAccessibilityCommandStartError> {
    if conn
        .target_owner_identity_for_session(cmd.session_id)
        .is_none()
        && conn.browser_context.is_none()
    {
        return Err(PendingAccessibilityCommandStartError::browser_context_not_loaded());
    }
    conn.ensure_document_accessible_for_session_owner(cmd.session_id)
        .map_err(PendingAccessibilityCommandStartError::document_access_error)?;
    let Some(top_frame_id) = helpers::top_frame_id_for_session(conn, cmd.session_id) else {
        return Err(PendingAccessibilityCommandStartError::no_document_loaded());
    };
    let resolved_frame_id = frame_id.unwrap_or(top_frame_id.as_str());
    let Some(inspection) =
        accessibility_inspection_for_owner(conn, &CommandOwnerScope::capture(conn, cmd.session_id))
    else {
        return Err(PendingAccessibilityCommandStartError::no_document_loaded());
    };
    let (kind, pending) = if resolved_frame_id == top_frame_id {
        let pending = start_top_frame_accessibility_inspection(&inspection, &top_frame_kind)
            .map_err(PendingAccessibilityCommandStartError::renderer_error)?;
        (top_frame_kind, pending)
    } else {
        let pending = start_child_frame_accessibility_inspection(
            &inspection,
            resolved_frame_id,
            &child_frame_kind,
        )
        .map_err(PendingAccessibilityCommandStartError::renderer_error)?;
        (child_frame_kind, pending)
    };
    Ok(Some(PendingAccessibilityCommandDispatch::from_command(
        conn, cmd, kind, pending,
    )))
}

fn start_child_frame_accessibility_inspection(
    inspection: &moli_renderer_v8::RendererAccessibilityInspection<'_>,
    frame_id: &str,
    kind: &PendingAccessibilityCommandKind,
) -> anyhow::Result<PendingPageCommand> {
    match kind {
        PendingAccessibilityCommandKind::ChildFrameFullTree { max_depth } => {
            inspection.start_child_frame_accessibility_tree_payloads(frame_id, *max_depth)
        }
        PendingAccessibilityCommandKind::ChildFrameRoot => {
            inspection.start_child_frame_accessibility_node_payload(frame_id)
        }
        _ => unreachable!("child-frame accessibility dispatch only accepts child-frame commands"),
    }
    .map(PendingPageCommand::from_inspector_main_route)
}

pub(crate) async fn complete_pending_accessibility_command(
    conn: &mut CdpConnection,
    completed: CompletedAccessibilityCommandDispatch,
) -> AccessibilityCommandDispatchStep {
    let CompletedAccessibilityCommandDispatch {
        command_id,
        owner_scope,
        kind,
        completed,
    } = completed;
    if let Ok(completion) = &completed
        && let Err(error) = conn.observe_renderer_inspection_completion(&owner_scope, completion)
    {
        return AccessibilityCommandDispatchStep::Complete(CommandOutputPlan::error(-32000, error));
    }

    if let Err(message) = conn.ensure_document_accessible_for_owner(&owner_scope) {
        return AccessibilityCommandDispatchStep::Complete(CommandOutputPlan::error(
            -32000, message,
        ));
    }
    let completion = match completed {
        Ok(completion) => *completion,
        Err(error) => {
            return AccessibilityCommandDispatchStep::Complete(CommandOutputPlan::error(
                -32000, error,
            ));
        }
    };
    let plan = match kind {
        PendingAccessibilityCommandKind::TopFrameFullTree { .. } => {
            match completion.finish_accessibility_tree_payloads_optional() {
                Ok(Some(nodes)) => CommandOutputPlan::result(json!({ "nodes": nodes })),
                Ok(None) => CommandOutputPlan::error(-32000, "NoDocumentLoaded"),
                Err(error) => CommandOutputPlan::error(
                    -32000,
                    format!("Could not build accessibility tree: {error}"),
                ),
            }
        }
        PendingAccessibilityCommandKind::TopFrameRoot => {
            match completion.finish_accessibility_node_payload() {
                Ok(Some(node)) => CommandOutputPlan::result(json!({ "node": node })),
                Ok(None) => CommandOutputPlan::error(-32000, "NoDocumentLoaded"),
                Err(error) => CommandOutputPlan::error(
                    -32000,
                    format!("Could not build root accessibility node: {error}"),
                ),
            }
        }
        PendingAccessibilityCommandKind::ChildFrameFullTree { .. } => {
            match finish_child_frame_accessibility_nodes_for_protocol(
                completion,
                "Could not build accessibility tree for frame",
            ) {
                Ok(nodes) => CommandOutputPlan::result(json!({ "nodes": nodes })),
                Err(plan) => plan,
            }
        }
        PendingAccessibilityCommandKind::ChildFrameRoot => {
            match finish_child_frame_accessibility_nodes_for_protocol(
                completion,
                "Could not build root accessibility node for frame",
            ) {
                Ok(mut nodes) => match nodes.is_empty() {
                    false => CommandOutputPlan::result(json!({ "node": nodes.remove(0) })),
                    true => CommandOutputPlan::error(-32000, "Could not find node with given id"),
                },
                Err(plan) => plan,
            }
        }
        PendingAccessibilityCommandKind::ObjectAccessibilityPayloads {
            frame_id,
            top_frame_id,
            operation,
        } => {
            return AccessibilityCommandDispatchStep::Complete(
                complete_accessibility_payloads_command(
                    completion.finish_accessibility_payloads_for_object_id(),
                    frame_id,
                    top_frame_id,
                    operation,
                ),
            );
        }
        PendingAccessibilityCommandKind::FrontendAccessibilityPayloads {
            frame_id,
            top_frame_id,
            operation,
        } => {
            let backend_node_id = match completion.finish_document_frontend_node_binding() {
                Ok(RendererDomFrontendNodeBindingResolution::BackendNodeId(backend_node_id)) => {
                    backend_node_id
                }
                Ok(RendererDomFrontendNodeBindingResolution::NotFound) => {
                    return AccessibilityCommandDispatchStep::Complete(CommandOutputPlan::error(
                        -32000,
                        "Could not find node with given id",
                    ));
                }
                Err(error) => {
                    return AccessibilityCommandDispatchStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Could not resolve frontend node binding: {error}"),
                    ));
                }
            };
            let Some(inspection) = accessibility_inspection_for_owner(conn, &owner_scope) else {
                return AccessibilityCommandDispatchStep::Complete(CommandOutputPlan::error(
                    -32000,
                    "NoDocumentLoaded",
                ));
            };
            let pending = match start_accessibility_backend_inspection(
                &inspection,
                backend_node_id,
                &operation,
            ) {
                Ok(pending) => pending,
                Err(error) => {
                    return AccessibilityCommandDispatchStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Could not build accessibility payload for node: {error}"),
                    ));
                }
            };
            return AccessibilityCommandDispatchStep::Pending(
                PendingAccessibilityCommandDispatch {
                    command_id,
                    owner_scope,
                    kind: PendingAccessibilityCommandKind::BackendAccessibilityPayloads {
                        frame_id,
                        top_frame_id,
                        operation,
                    },
                    pending,
                },
            );
        }
        PendingAccessibilityCommandKind::BackendAccessibilityPayloads {
            frame_id,
            top_frame_id,
            operation,
        } => {
            return AccessibilityCommandDispatchStep::Complete(
                complete_accessibility_payloads_command(
                    completion.finish_accessibility_payloads_for_backend_node_id(),
                    frame_id,
                    top_frame_id,
                    operation,
                ),
            );
        }
    };
    AccessibilityCommandDispatchStep::Complete(plan)
}

fn complete_accessibility_payloads_command(
    payloads: anyhow::Result<Option<moli_renderer_v8::RendererAccessibilityPayloadsForObjectId>>,
    frame_id: String,
    top_frame_id: String,
    operation: AccessibilityNodeOperation,
) -> CommandOutputPlan {
    let payloads = match payloads {
        Ok(Some(payloads)) => payloads,
        Ok(None) => {
            return CommandOutputPlan::error(-32000, "Could not find node with given id");
        }
        Err(error) => {
            return CommandOutputPlan::error(
                -32000,
                format!("Could not build accessibility payload for node: {error}"),
            );
        }
    };
    let object_frame_id = payloads
        .frame_id
        .as_deref()
        .unwrap_or(top_frame_id.as_str());
    if object_frame_id != frame_id {
        return CommandOutputPlan::error(-32000, "Could not find node with given id");
    }
    let Some(mut nodes) = payloads.payloads else {
        return CommandOutputPlan::error(-32000, "Could not find node with given id");
    };
    if let AccessibilityNodeOperation::Query {
        accessible_name,
        role,
    } = operation
    {
        retain_matching_accessibility_nodes(
            &mut nodes,
            accessible_name.as_deref(),
            role.as_deref(),
        );
    }
    CommandOutputPlan::result(json!({ "nodes": nodes }))
}

fn finish_child_frame_accessibility_nodes_for_protocol(
    completion: CompletedPageCommand,
    error_prefix: &str,
) -> Result<Vec<Value>, CommandOutputPlan> {
    let payloads = match completion.finish_child_frame_accessibility_payloads() {
        Ok(Some(payloads)) => payloads,
        Ok(None) => {
            return Err(CommandOutputPlan::error(
                -32000,
                "Frame with the given id does not belong to the target.",
            ));
        }
        Err(error) => {
            return Err(CommandOutputPlan::error(
                -32000,
                format!("{error_prefix}: {error}"),
            ));
        }
    };
    payloads
        .payloads
        .ok_or_else(|| CommandOutputPlan::error(-32000, "Could not find node with given id"))
}

fn retain_matching_accessibility_nodes(
    nodes: &mut Vec<Value>,
    accessible_name: Option<&str>,
    role: Option<&str>,
) {
    nodes.retain(|node| {
        let node_role = node["role"]["value"].as_str().unwrap_or_default();
        let node_name = node["name"]["value"].as_str().unwrap_or_default();
        role.is_none_or(|expected| expected == node_role)
            && accessible_name.is_none_or(|expected| expected == node_name)
    });
}
