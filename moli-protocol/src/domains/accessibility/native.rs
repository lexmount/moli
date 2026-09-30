use super::*;
use crate::automation::DevToolsDomNodeReference;
use crate::domains::native::{self, NativeCommandStep, NodeLookupExecution};
use moli_core::{
    RendererNativeOperation as Operation, RendererNativeProtocolResponse as Response,
    RendererPageCommand as Command, RendererPageReply as Reply,
};

pub(crate) fn try_start(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> Option<NativeCommandStep> {
    let action = cmd.parse_action::<AccessibilityAction>()?;
    if !action.queries_tree() {
        return None;
    }
    let operation = prepare(conn, cmd, action);
    Some(match operation {
        Ok(operation) => native::start_operation(conn, cmd, operation),
        Err(error) => {
            NativeCommandStep::Complete(CommandOutputPlan::error(error.code, error.message))
        }
    })
}

type StartError = PendingAccessibilityCommandStartError;

fn prepare(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    action: AccessibilityAction,
) -> Result<Operation, StartError> {
    let top_frame_id = helpers::top_frame_id_for_session(conn, cmd.session_id)
        .ok_or_else(StartError::no_document_loaded)?;
    match action {
        AccessibilityAction::GetFullAxTree => {
            let params = cmd
                .get_params::<helpers::GetFullAxTreeParams>()
                .map_err(|_| StartError::invalid_params())?
                .unwrap_or(helpers::GetFullAxTreeParams {
                    depth: None,
                    frame_id: None,
                });
            let max_depth = match params.depth {
                Some(depth) => {
                    let depth = i32::try_from(depth).map_err(|_| StartError::invalid_params())?;
                    (depth >= 0).then_some(depth)
                }
                None => None,
            };
            Ok(frame_operation(
                top_frame_id,
                params.frame_id.map(|id| id.as_ref().to_owned()),
                max_depth,
                false,
            ))
        }
        AccessibilityAction::GetRootAxNode => {
            let params = cmd
                .get_params::<helpers::FrameScopedParams>()
                .map_err(|_| StartError::invalid_params())?
                .unwrap_or(helpers::FrameScopedParams { frame_id: None });
            Ok(frame_operation(top_frame_id, params.frame_id, None, true))
        }
        AccessibilityAction::GetChildAxNodes => {
            let params = cmd
                .get_params::<helpers::ChildAxNodesParams>()
                .ok()
                .flatten()
                .ok_or_else(StartError::invalid_params)?;
            let backend_node_id = helpers::parse_ax_backend_node_id(params.id.as_ref())
                .ok_or_else(StartError::invalid_params)?;
            let frame_id = params
                .frame_id
                .map(|id| id.as_ref().to_owned())
                .unwrap_or_else(|| top_frame_id.clone());
            Ok(backend_operation(
                backend_node_id,
                frame_id,
                top_frame_id,
                AccessibilityNodeOperation::Children,
            ))
        }
        AccessibilityAction::GetAxNodeAndAncestors => {
            let params = cmd
                .get_params::<helpers::AncestorsParams>()
                .ok()
                .flatten()
                .ok_or_else(StartError::invalid_params)?;
            reference_operation(
                conn,
                cmd,
                params.reference,
                params.frame_id,
                top_frame_id,
                AccessibilityNodeOperation::Ancestors,
            )
        }
        AccessibilityAction::QueryAxTree => {
            let params = cmd
                .get_params::<helpers::QueryAxTreeParams>()
                .ok()
                .flatten()
                .ok_or_else(StartError::invalid_params)?;
            reference_operation(
                conn,
                cmd,
                params.reference,
                params.frame_id,
                top_frame_id,
                AccessibilityNodeOperation::Query {
                    accessible_name: params.accessible_name,
                    role: params.role,
                },
            )
        }
        AccessibilityAction::GetPartialAxTree => {
            let params = cmd
                .get_params::<helpers::PartialAxTreeParams>()
                .ok()
                .flatten()
                .ok_or_else(StartError::invalid_params)?;
            reference_operation(
                conn,
                cmd,
                params.reference,
                params.frame_id,
                top_frame_id,
                AccessibilityNodeOperation::Partial {
                    fetch_relatives: params.fetch_relatives.unwrap_or(true),
                },
            )
        }
        AccessibilityAction::Enable | AccessibilityAction::Disable => unreachable!(),
    }
}

fn frame_operation(
    top_frame_id: String,
    frame_id: Option<String>,
    max_depth: Option<i32>,
    root_only: bool,
) -> Operation {
    let frame_id = frame_id.unwrap_or_else(|| top_frame_id.clone());
    let child = frame_id != top_frame_id;
    let command = match (child, root_only) {
        (false, false) => Command::AccessibilityTreePayloadsForDocument { max_depth },
        (false, true) => Command::AccessibilityNodePayloadForDocument,
        (true, false) => Command::AccessibilityTreePayloadsForChildFrame {
            frame_id,
            max_depth,
        },
        (true, true) => Command::AccessibilityNodePayloadForChildFrame { frame_id },
    };
    Operation::new(command, move |reply| match reply {
        Ok(Reply::OptionalAccessibilityPayload(Some(node))) => {
            Response::success(json!({"node": node}))
        }
        Ok(Reply::OptionalAccessibilityPayloads(Some(nodes))) => {
            Response::success(json!({"nodes": nodes}))
        }
        Ok(Reply::OptionalAccessibilityPayloadsForObjectId(Some(payload))) => {
            match payload.payloads {
                Some(mut nodes) if root_only => {
                    if nodes.is_empty() {
                        node_not_found()
                    } else {
                        Response::success(json!({"node": nodes.remove(0)}))
                    }
                }
                Some(nodes) => Response::success(json!({"nodes": nodes})),
                None => node_not_found(),
            }
        }
        Ok(Reply::OptionalAccessibilityPayloadsForObjectId(None)) => Response::error(
            -32000,
            "Frame with the given id does not belong to the target.",
        ),
        Ok(
            Reply::OptionalAccessibilityPayload(None) | Reply::OptionalAccessibilityPayloads(None),
        ) => Response::error(-32000, "NoDocumentLoaded"),
        Err(error) => {
            let kind = if root_only {
                "root accessibility node"
            } else {
                "accessibility tree"
            };
            let suffix = if child { " for frame" } else { "" };
            Response::error(-32000, format!("Could not build {kind}{suffix}: {error}"))
        }
        _ => unreachable!("frame accessibility reply"),
    })
}

fn reference_operation(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    reference: helpers::NodeReferenceParams,
    frame_id: Option<String>,
    top_frame_id: String,
    operation: AccessibilityNodeOperation,
) -> Result<Operation, StartError> {
    let frame_id = frame_id.unwrap_or_else(|| top_frame_id.clone());
    if let Some(object_id) = reference.object_id {
        let session = cmd.session_id.map(str::to_owned);
        let command = match &operation {
            AccessibilityNodeOperation::Ancestors => {
                Command::accessibility_node_and_ancestor_payloads_for_object_id(session, object_id)
            }
            AccessibilityNodeOperation::Query { .. } => {
                Command::accessibility_tree_payloads_for_object_id(session, object_id)
            }
            AccessibilityNodeOperation::Partial { fetch_relatives } => {
                Command::accessibility_partial_tree_payloads_for_object_id(
                    session,
                    object_id,
                    *fetch_relatives,
                )
            }
            AccessibilityNodeOperation::Children => unreachable!(),
        };
        return Ok(Operation::new(command, move |reply| {
            project_nodes(reply, frame_id, top_frame_id, operation)
        }));
    }
    if let Some(backend_node_id) = renderer_backend_node_id_for_reference(&reference) {
        return Ok(backend_operation(
            backend_node_id,
            frame_id,
            top_frame_id,
            operation,
        ));
    }
    let frontend_node_id = reference.node_id.ok_or_else(StartError::node_not_found)?;
    let inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    Ok(native::with_backend_node(
        inspector_session_id,
        DevToolsDomNodeReference::FrontendNodeId(frontend_node_id),
        NodeLookupExecution::NestedMain,
        move |id| backend_operation(id, frame_id, top_frame_id, operation),
    ))
}

fn backend_operation(
    backend_node_id: u32,
    frame_id: String,
    top_frame_id: String,
    operation: AccessibilityNodeOperation,
) -> Operation {
    let command = match &operation {
        AccessibilityNodeOperation::Children => {
            Command::AccessibilityChildNodePayloadsForBackendNodeId { backend_node_id }
        }
        AccessibilityNodeOperation::Ancestors => {
            Command::AccessibilityNodeAndAncestorPayloadsForBackendNodeId { backend_node_id }
        }
        AccessibilityNodeOperation::Query { .. } => {
            Command::AccessibilityTreePayloadsForBackendNodeId {
                backend_node_id,
                max_depth: None,
            }
        }
        AccessibilityNodeOperation::Partial { fetch_relatives } => {
            Command::AccessibilityPartialTreePayloadsForBackendNodeId {
                backend_node_id,
                fetch_relatives: *fetch_relatives,
            }
        }
    };
    Operation::new(command, move |reply| {
        project_nodes(reply, frame_id, top_frame_id, operation)
    })
}

fn project_nodes(
    reply: anyhow::Result<Reply>,
    frame_id: String,
    top_frame_id: String,
    operation: AccessibilityNodeOperation,
) -> Response {
    let payload = match reply {
        Ok(Reply::OptionalAccessibilityPayloadsForObjectId(Some(payload))) => payload,
        Ok(Reply::OptionalAccessibilityPayloadsForObjectId(None)) => return node_not_found(),
        Err(error) => {
            return Response::error(
                -32000,
                format!("Could not build accessibility payload for node: {error}"),
            );
        }
        _ => unreachable!("accessibility node reply"),
    };
    if payload.frame_id.as_deref().unwrap_or(&top_frame_id) != frame_id {
        return node_not_found();
    }
    let Some(mut nodes) = payload.payloads else {
        return node_not_found();
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
    Response::success(json!({"nodes": nodes}))
}

fn node_not_found() -> Response {
    Response::error(-32000, "Could not find node with given id")
}
