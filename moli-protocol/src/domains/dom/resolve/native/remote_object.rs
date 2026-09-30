//! Resolve the remote object and its fallback node snapshot in one handler.
//! Registration is an ordered Browser state update carried with the terminal,
//! so later commands cannot observe an unregistered object after its response.

use super::*;
use moli_core::RendererNativeProtocolStateUpdate as Update;
use moli_core::RendererRuntimeRemoteObjectResolution as Resolution;

pub(super) fn prepare(conn: &CdpConnection, cmd: &Cmd<'_>) -> Result<Operation, StartError> {
    let params = build_cdp_resolve_node_command(conn, cmd)?;
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(&owner);
    let top_frame = top_frame_id_for_owner(conn, &owner);
    let whitespace = dom_agent_includes_whitespace_for_owner(conn, &owner);
    Ok(
        with_backend(session.clone(), params.reference, move |backend_node_id| {
            let prepare = move |cache| {
                resolve(
                    session,
                    backend_node_id,
                    params.execution_context_id,
                    params.object_group,
                    top_frame,
                    whitespace,
                    cache,
                )
            };
            if let Some(context) = params.execution_context_id {
                Operation::then(
                    Command::ChildFrameIdForDefaultExecutionContextId(context),
                    move |reply| match reply {
                        Ok(Reply::OptionalString(frame)) => {
                            if frame.is_some() && !is_renderer_backend_node_id(backend_node_id) {
                                Step::Complete(node_not_found())
                            } else {
                                Step::Continue(prepare(frame.is_none()))
                            }
                        }
                        Err(error) => Step::Complete(Response::error(
                            -32000,
                            format!("Could not resolve execution context frame: {error}"),
                        )),
                        _ => unreachable!("DOM execution context frame reply"),
                    },
                )
            } else {
                prepare(true)
            }
        })
        .require_owner_turn(),
    )
}

fn resolve(
    session: Option<String>,
    backend_node_id: u32,
    execution_context_id: Option<i64>,
    group: Option<String>,
    top_frame: Option<String>,
    whitespace: bool,
    cache: bool,
) -> Operation {
    Operation::then(
        Command::resolve_runtime_object_for_backend_node_id(
            session.clone(),
            backend_node_id,
            execution_context_id,
            group.clone(),
        ),
        move |reply| {
            let mut object = match reply {
                Ok(Reply::RuntimeRemoteObjectResolution(Resolution::Found(object))) => {
                    object.into_protocol_value()
                }
                Ok(Reply::RuntimeRemoteObjectResolution(Resolution::MissingContext)) => {
                    return Step::Complete(Response::error(-32000, "ContextNotFound"));
                }
                Ok(Reply::RuntimeRemoteObjectResolution(Resolution::MissingNode)) => {
                    return Step::Complete(node_not_found());
                }
                Err(error) => {
                    return Step::Complete(Response::error(
                        -32000,
                        format!("Could not resolve node runtime object: {error}"),
                    ));
                }
                _ => unreachable!("DOM resolved runtime object reply"),
            };
            if let Some(object) = object.as_object_mut() {
                object
                    .entry("subtype".to_owned())
                    .or_insert_with(|| json!("node"));
            }
            let result = json!({"object": object});
            if cache && let Some(id) = result["object"]["objectId"].as_str().map(str::to_owned) {
                Step::Continue(Operation::new(
                    Command::document_node_snapshot_for_object_id(
                        session,
                        whitespace,
                        id.clone(),
                        0,
                        false,
                    ),
                    move |reply| match reply {
                        Ok(Reply::OptionalDocumentNodeObjectSnapshot(snapshot)) => {
                            let cached = snapshot
                                .as_ref()
                                .as_ref()
                                .and_then(|snapshot| {
                                    node_snapshot_to_cdp(
                                        &snapshot.snapshot,
                                        top_snapshot_node_id_for_live_object_snapshot(
                                            &snapshot.snapshot,
                                        ),
                                        top_frame.as_deref(),
                                    )
                                })
                                .map(|node| (id, node));
                            registered_response(result, group, cached)
                        }
                        Err(error) => Response::error(
                            -32000,
                            format!("Could not snapshot resolved node: {error}"),
                        ),
                        _ => unreachable!("DOM resolved object snapshot reply"),
                    },
                ))
            } else {
                Step::Complete(registered_response(result, group, None))
            }
        },
    )
}

fn registered_response(
    result: Value,
    group: Option<String>,
    cached: Option<(String, Value)>,
) -> Response {
    let mut response = Response::success(result);
    response.state_updates.push(Update::RemoteObjects {
        object_group: group,
    });
    if let Some((object_id, node)) = cached {
        response
            .state_updates
            .push(Update::DomRemoteObjectNode { object_id, node });
    }
    response
}
