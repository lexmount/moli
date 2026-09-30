use super::*;
use crate::domains::native::{self, NativeCommandStep};
use moli_core::page::{
    RendererDomFrontendNodeBindingResolution, RendererDomSearchResultsResolution,
};
use moli_core::{
    RendererNativeOperation as Operation, RendererNativeOperationStep as Step,
    RendererNativeProtocolNotification as Notification, RendererNativeProtocolResponse as Response,
    RendererPageCommand as Command, RendererPageReply as Reply,
};

mod files;
mod mutation;
mod remote_object;

type StartError = PendingDomCommandStartError;

pub(crate) fn try_start(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> Option<NativeCommandStep> {
    let action = cmd.parse_action::<DomAction>()?;
    if !mutation::handles(action)
        && !matches!(
            action,
            DomAction::ResolveNode
                | DomAction::SetFileInputFiles
                | DomAction::GetDocument
                | DomAction::GetFlattenedDocument
                | DomAction::QuerySelector
                | DomAction::QuerySelectorAll
                | DomAction::RequestChildNodes
                | DomAction::GetFrameOwner
                | DomAction::GetAttributes
                | DomAction::DescribeNode
                | DomAction::GetOuterHtml
                | DomAction::GetBoxModel
                | DomAction::GetContentQuads
                | DomAction::RequestNode
                | DomAction::PushNodesByBackendIdsToFrontend
                | DomAction::PerformSearch
                | DomAction::GetSearchResults
                | DomAction::DiscardSearchResults
        )
    {
        return None;
    }
    if action == DomAction::DiscardSearchResults
        && search::build_cdp_discard_search_results_command(conn, cmd).is_none()
    {
        return None;
    }
    if action == DomAction::Disable {
        if !dom_agent_enabled_for_session(conn, cmd.session_id) {
            return Some(NativeCommandStep::Complete(CommandOutputPlan::error(
                -32000,
                "DOM agent hasn't been enabled",
            )));
        }
        disable_dom_agent_for_session(conn, cmd.session_id);
    }
    if action == DomAction::GetDocument {
        enable_dom_agent_for_session(conn, cmd.session_id, false);
    }
    if action == DomAction::GetFlattenedDocument
        && !dom_agent_enabled_for_session(conn, cmd.session_id)
    {
        return Some(NativeCommandStep::Complete(CommandOutputPlan::error(
            -32000,
            "DOM agent hasn't been enabled",
        )));
    }
    Some(match prepare(conn, cmd, action) {
        Ok(operation) => native::start_operation(conn, cmd, operation),
        Err(error) => {
            NativeCommandStep::Complete(CommandOutputPlan::error(error.code, error.message))
        }
    })
}

fn prepare(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    action: DomAction,
) -> Result<Operation, StartError> {
    if action == DomAction::ResolveNode {
        return remote_object::prepare(conn, cmd);
    }
    if action == DomAction::SetFileInputFiles {
        return files::prepare(conn, cmd).map(Operation::require_owner_turn);
    }
    if mutation::handles(action) {
        return mutation::prepare(conn, cmd, action);
    }
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(&owner);
    let top_frame = top_frame_id_for_owner(conn, &owner);
    let whitespace = dom_agent_includes_whitespace_for_owner(conn, &owner);
    match action {
        DomAction::GetDocument | DomAction::GetFlattenedDocument => {
            let flattened = action == DomAction::GetFlattenedDocument;
            let kind = if flattened {
                PendingDomDocumentSnapshotOperation::GetFlattenedDocument
            } else {
                PendingDomDocumentSnapshotOperation::GetDocument
            };
            let params = build_cdp_get_document_command(conn, cmd, kind)?;
            Ok(Operation::new(
                Command::DocumentNodeSnapshotForDocument {
                    inspector_session_id: session,
                    include_whitespace: whitespace,
                    depth: params.depth.unwrap_or(if flattened { -1 } else { 2 }),
                    pierce: params.pierce,
                },
                move |reply| match reply {
                    Ok(Reply::OptionalDocumentNodeObjectSnapshot(snapshot)) => match *snapshot {
                        Some(snapshot) => {
                            let snapshot = snapshot.snapshot;
                            if flattened {
                                let mut nodes = Vec::new();
                                collect_flattened_node_snapshot(
                                    &snapshot,
                                    snapshot.node_id,
                                    top_frame.as_deref(),
                                    &mut nodes,
                                );
                                Response::success(json!({"nodes": nodes}))
                            } else {
                                node_snapshot_to_cdp(
                                    &snapshot,
                                    Some(snapshot.node_id),
                                    top_frame.as_deref(),
                                )
                                .map(|root| Response::success(json!({"root": root})))
                                .unwrap_or_else(|| {
                                    Response::error(-32000, "No node with given id found")
                                })
                            }
                        }
                        None => Response::error(-32000, "NoDocumentLoaded"),
                    },
                    Err(error) => Response::error(
                        -32000,
                        format!("Could not capture document snapshot: {error}"),
                    ),
                    _ => unreachable!("DOM document snapshot reply"),
                },
            ))
        }
        DomAction::GetAttributes => {
            let params = build_cdp_get_attributes_command(conn, cmd)?;
            Ok(with_backend(session, params.reference, |backend_node_id| {
                Operation::new(
                    Command::DocumentNodeAttributesForBackendNodeId { backend_node_id },
                    |reply| match reply {
                        Ok(Reply::DocumentNodeAttributesResolution(resolution)) => {
                            match attributes_result_from_renderer_resolution(resolution) {
                                Ok(result) => Response::success(
                                    json!({"attributes": result.attributes.into_iter().flat_map(|attr| [attr.name, attr.value]).collect::<Vec<_>>()}),
                                ),
                                Err(error) => Response::error(error.code, error.message),
                            }
                        }
                        Err(error) => Response::error(-32000, error.to_string()),
                        _ => unreachable!("DOM attributes reply"),
                    },
                )
            }))
        }
        DomAction::QuerySelector | DomAction::QuerySelectorAll => {
            let params =
                build_cdp_query_selector_command(conn, cmd, action == DomAction::QuerySelectorAll)?;
            let selector = params.selector;
            let multiple = params.multiple;
            Ok(match params.root {
                None => Operation::new(
                    Command::DocumentQuerySelectorForDocument {
                        inspector_session_id: session,
                        include_whitespace: whitespace,
                        selector,
                        multiple,
                    },
                    move |reply| project_query(reply, multiple, top_frame),
                ),
                Some(reference) => {
                    let frontend = matches!(reference, DevToolsDomNodeReference::FrontendNodeId(_));
                    with_backend(session.clone(), reference, move |root_backend_node_id| {
                        let command = if frontend {
                            Command::DocumentQuerySelectorWithChildNodeSnapshotEventsForBackendNodeId {
                                inspector_session_id: session, include_whitespace: whitespace, root_backend_node_id, selector, multiple,
                            }
                        } else {
                            Command::DocumentQuerySelectorForBackendNodeId {
                                inspector_session_id: session,
                                include_whitespace: whitespace,
                                root_backend_node_id,
                                selector,
                                multiple,
                            }
                        };
                        Operation::new(command, move |reply| {
                            project_query(reply, multiple, top_frame)
                        })
                    })
                }
            })
        }
        DomAction::RequestChildNodes => {
            let params = build_cdp_request_child_nodes_command(conn, cmd)?;
            let depth = if params.depth > 0 {
                params.depth - 1
            } else {
                params.depth
            };
            Ok(with_backend(
                session.clone(),
                params.reference,
                move |backend_node_id| {
                    Operation::new(
                        Command::DocumentChildNodeSnapshotEventsForBackendNodeId {
                            inspector_session_id: session,
                            include_whitespace: whitespace,
                            backend_node_id,
                            depth,
                            pierce: params.pierce,
                        },
                        move |reply| match reply {
                            Ok(Reply::OptionalDocumentChildNodeSnapshotEvents(Some(
                                mut snapshots,
                            ))) => {
                                // requestChildNodes publishes the requested parent's
                                // snapshot; querySelector may publish several ancestors.
                                snapshots.events.truncate(1);
                                let mut response = Response::success(json!({}));
                                response.notifications =
                                    child_notifications(snapshots, top_frame.as_deref());
                                response
                            }
                            Ok(Reply::OptionalDocumentChildNodeSnapshotEvents(None)) => {
                                Response::error(-32000, "InvalidNode")
                            }
                            Err(error) => Response::error(
                                -32000,
                                format!("Could not capture child node snapshots: {error}"),
                            ),
                            _ => unreachable!("DOM child nodes reply"),
                        },
                    )
                },
            ))
        }
        DomAction::GetFrameOwner => {
            let params = build_cdp_get_frame_owner_command(conn, cmd)?;
            Ok(Operation::new(
                Command::ChildFrameOwnerNodeReference {
                    frame_id: params.frame_id.into_string(),
                    inspector_session_id: session,
                },
                |reply| match reply {
                    Ok(Reply::OptionalDocumentNodeReference(Some(node))) => Response::success(
                        json!({"backendNodeId": node.backend_node_id, "nodeId": node.node_id}),
                    ),
                    Ok(Reply::OptionalDocumentNodeReference(None)) => Response::error(
                        -32000,
                        "Frame with the given id does not belong to the target.",
                    ),
                    Err(error) => Response::error(-32000, error.to_string()),
                    _ => unreachable!("DOM frame owner reply"),
                },
            ))
        }
        DomAction::DescribeNode => {
            let params: DescribeNodeParams = cmd
                .get_params()
                .ok()
                .flatten()
                .ok_or_else(StartError::invalid_params)?;
            let cached = params
                .reference
                .object_id
                .as_deref()
                .and_then(|id| cached_dom_remote_object_node_for_owner(conn, &owner, id));
            let project = move |reply| project_node_snapshot(reply, top_frame, cached, false);
            if let Some(object) = params.reference.object_id {
                let object = dom_object_reference_id_for_owner(conn, &owner, &object.into());
                Ok(Operation::new(
                    Command::document_node_snapshot_for_object_id(
                        session,
                        whitespace,
                        object,
                        params.depth,
                        params.pierce,
                    ),
                    project,
                ))
            } else {
                let reference = devtools_node_reference_from_ids(
                    params.reference.node_id,
                    params.reference.backend_node_id,
                )
                .ok_or_else(StartError::node_not_found)?;
                Ok(with_backend(
                    session.clone(),
                    reference,
                    move |backend_node_id| {
                        Operation::new(
                            Command::DocumentNodeSnapshotForBackendNodeIdInInspectorSession {
                                inspector_session_id: session,
                                include_whitespace: whitespace,
                                backend_node_id,
                                depth: params.depth,
                                pierce: params.pierce,
                            },
                            project,
                        )
                    },
                ))
            }
        }
        DomAction::RequestNode => {
            let params: RequestNodeParams = cmd
                .get_params()
                .ok()
                .flatten()
                .ok_or_else(StartError::invalid_params)?;
            Ok(Operation::new(
                Command::document_node_snapshot_for_object_id(
                    session,
                    whitespace,
                    dom_object_reference_id_for_owner(
                        conn,
                        &owner,
                        &params.object_id.inner().to_owned().into(),
                    ),
                    0,
                    false,
                ),
                move |reply| project_node_snapshot(reply, top_frame, None, true),
            ))
        }
        DomAction::GetOuterHtml => {
            let params: GetOuterHtmlParams = cmd
                .get_params()
                .ok()
                .flatten()
                .ok_or_else(StartError::invalid_params)?;
            let project = |reply| match reply {
                Ok(Reply::OptionalString(Some(html))) => {
                    Response::success(json!({"outerHTML": html}))
                }
                Ok(Reply::OptionalString(None)) => node_not_found(),
                Err(error) => Response::error(-32000, format!("Could not get outer HTML: {error}")),
                _ => unreachable!("DOM outer HTML reply"),
            };
            if let Some(object) = params.reference.object_id {
                let object = dom_object_reference_id_for_owner(conn, &owner, &object.into());
                Ok(Operation::new(
                    Command::outer_html_for_object_id(session, object, params.include_shadow_dom),
                    project,
                ))
            } else if let Some(reference) = devtools_node_reference_from_ids(
                params.reference.node_id,
                params.reference.backend_node_id,
            ) {
                Ok(with_backend(session, reference, move |backend_node_id| {
                    Operation::new(
                        Command::OuterHtmlForBackendNodeId {
                            backend_node_id,
                            include_shadow_dom: params.include_shadow_dom,
                        },
                        project,
                    )
                }))
            } else {
                Ok(Operation::new(
                    Command::OuterHtmlForDocument {
                        include_shadow_dom: params.include_shadow_dom,
                    },
                    project,
                ))
            }
        }
        DomAction::GetBoxModel | DomAction::GetContentQuads => {
            let params: NodeReferenceParams = cmd
                .get_params()
                .ok()
                .flatten()
                .ok_or_else(StartError::invalid_params)?;
            let geometry = if action == DomAction::GetBoxModel {
                DevToolsDomGeometryOperation::GetBoxModel
            } else {
                DevToolsDomGeometryOperation::GetContentQuads
            };
            let project = move |reply| match reply {
                Ok(Reply::OptionalDocumentNodeGeometry(Some(value))) => {
                    match devtools_dom_geometry_result_from_renderer(geometry, value) {
                        Ok(result) => {
                            Response::success(devtools_dom_geometry_result_value(&result))
                        }
                        Err(error) => Response::error(-32000, error.message),
                    }
                }
                Ok(Reply::OptionalDocumentNodeGeometry(None)) => node_not_found(),
                Err(error) => {
                    Response::error(-32000, format!("Could not resolve node geometry: {error}"))
                }
                _ => unreachable!("DOM geometry reply"),
            };
            if let Some(object) = params.object_id {
                let object = dom_object_reference_id_for_owner(conn, &owner, &object.into());
                Ok(Operation::new(
                    Command::document_geometry_for_object_id(session, object),
                    project,
                ))
            } else {
                let reference =
                    devtools_node_reference_from_ids(params.node_id, params.backend_node_id)
                        .ok_or_else(StartError::node_not_found)?;
                Ok(with_backend(session, reference, move |backend_node_id| {
                    Operation::new(
                        Command::DocumentGeometryForBackendNodeId { backend_node_id },
                        project,
                    )
                }))
            }
        }
        DomAction::PushNodesByBackendIdsToFrontend => {
            let params = build_cdp_push_nodes_by_backend_ids_command(conn, cmd)?;
            Ok(Operation::new(
                Command::DocumentFrontendNodeIdsForBackendNodeIds {
                    inspector_session_id: session,
                    backend_node_ids: params.backend_node_ids,
                },
                |reply| match reply {
                    Ok(Reply::DocumentFrontendNodeIds(
                        RendererDocumentFrontendNodeIdsResolution::Found(ids),
                    )) => Response::success(
                        json!({"nodeIds": ids.into_iter().map(|id| id.unwrap_or(0)).collect::<Vec<_>>()}),
                    ),
                    Ok(Reply::DocumentFrontendNodeIds(
                        RendererDocumentFrontendNodeIdsResolution::DocumentNotBound,
                    )) => Response::error(-32000, "Document needs to be requested first"),
                    Err(error) => Response::error(
                        -32000,
                        format!("Could not resolve backend node ids: {error}"),
                    ),
                    _ => unreachable!("DOM push nodes reply"),
                },
            ))
        }
        DomAction::PerformSearch => {
            let params = search::build_cdp_perform_search_command(conn, cmd)?;
            Ok(Operation::new(
                Command::DocumentPerformSearch {
                    inspector_session_id: session,
                    query: params.query,
                    include_user_agent_shadow_dom: params.include_user_agent_shadow_dom,
                    include_whitespace: whitespace,
                },
                |reply| match reply {
                    Ok(Reply::DocumentPerformSearch(search)) => Response::success(
                        json!({"searchId": search.search_id, "resultCount": search.result_count}),
                    ),
                    Err(error) => {
                        Response::error(-32000, format!("Could not perform DOM search: {error}"))
                    }
                    _ => unreachable!("DOM search reply"),
                },
            ))
        }
        DomAction::GetSearchResults => {
            let params = search::build_cdp_get_search_results_command(conn, cmd)?;
            Ok(Operation::new(
                Command::DocumentGetSearchResults {
                    inspector_session_id: session,
                    search_id: params.search_id,
                    from_index: params.from_index,
                    to_index: params.to_index,
                },
                |reply| match reply {
                    Ok(Reply::DocumentSearchResults(
                        RendererDomSearchResultsResolution::Found(nodes),
                    )) => Response::success(
                        json!({"nodeIds": nodes.into_iter().map(|node| node.frontend_node_id).collect::<Vec<_>>()}),
                    ),
                    Ok(Reply::DocumentSearchResults(
                        RendererDomSearchResultsResolution::SearchResultNotFound,
                    )) => Response::error(-32000, "No search session with given id found"),
                    Ok(Reply::DocumentSearchResults(_)) => {
                        Response::error(-32000, "Invalid search result range")
                    }
                    Err(error) => Response::error(
                        -32000,
                        format!("Could not get DOM search results: {error}"),
                    ),
                    _ => unreachable!("DOM search results reply"),
                },
            ))
        }
        DomAction::DiscardSearchResults => {
            let params = search::build_cdp_discard_search_results_command(conn, cmd)
                .ok_or_else(StartError::invalid_params)?;
            Ok(Operation::new(
                Command::DocumentDiscardSearchResults {
                    inspector_session_id: session,
                    search_id: params.search_id,
                },
                |reply| match reply {
                    Ok(Reply::DocumentSearchResultsDiscarded) => Response::success(json!({})),
                    Err(error) => Response::error(
                        -32000,
                        format!("Could not discard DOM search results: {error}"),
                    ),
                    _ => unreachable!("DOM discard search reply"),
                },
            ))
        }
        _ => unreachable!("native DOM preparation is selected at admission"),
    }
}

fn with_backend(
    session: Option<String>,
    reference: DevToolsDomNodeReference,
    next: impl FnOnce(u32) -> Operation + Send + 'static,
) -> Operation {
    match reference {
        DevToolsDomNodeReference::BackendNodeId(id) => next(id),
        DevToolsDomNodeReference::FrontendNodeId(frontend_node_id) => {
            Operation::then_on_nested_main(
                Command::DocumentFrontendNodeBinding {
                    inspector_session_id: session,
                    frontend_node_id,
                },
                move |reply| match reply {
                    Ok(Reply::DocumentFrontendNodeBinding(
                        RendererDomFrontendNodeBindingResolution::BackendNodeId(id),
                    )) => Step::Continue(next(id)),
                    Ok(Reply::DocumentFrontendNodeBinding(
                        RendererDomFrontendNodeBindingResolution::NotFound,
                    )) => Step::Complete(node_not_found()),
                    Err(error) => Step::Complete(Response::error(
                        -32000,
                        format!("Could not resolve frontend node binding: {error}"),
                    )),
                    _ => unreachable!("DOM node binding reply"),
                },
            )
        }
    }
}

fn node_not_found() -> Response {
    Response::error(-32000, "Could not find node with given id")
}

fn project_node_snapshot(
    reply: anyhow::Result<Reply>,
    top_frame: Option<String>,
    cached: Option<Value>,
    request: bool,
) -> Response {
    match reply {
        Ok(Reply::OptionalDocumentNodeObjectSnapshot(snapshot)) => match *snapshot {
            Some(snapshot) if request => frontend_node_id_for_snapshot(&snapshot.snapshot)
                .map(|id| Response::success(json!({"nodeId": id})))
                .unwrap_or_else(node_not_found),
            Some(snapshot) => node_for_describe_node_object_snapshot(
                &snapshot.snapshot,
                snapshot.frame_id.as_deref(),
                top_frame.as_deref(),
            )
            .map(|node| Response::success(json!({"node": node})))
            .unwrap_or_else(node_not_found),
            None => cached
                .map(|node| Response::success(json!({"node": node})))
                .unwrap_or_else(node_not_found),
        },
        Err(error) => Response::error(
            -32000,
            format!(
                "Could not {} node object: {error}",
                if request { "request" } else { "describe" }
            ),
        ),
        _ => unreachable!("DOM node snapshot reply"),
    }
}

fn child_notifications(
    snapshots: moli_core::page::RendererDocumentChildNodeSnapshotEvents,
    top_frame: Option<&str>,
) -> Vec<Notification> {
    snapshots
        .events
        .into_iter()
        .map(|event| Notification::DomSetChildNodes {
            parent_node_id: event.parent_frontend_node_id,
            nodes: event
                .snapshots
                .iter()
                .filter_map(|snapshot| {
                    node_snapshot_to_cdp(snapshot, Some(snapshots.top_snapshot_node_id), top_frame)
                })
                .collect(),
        })
        .collect()
}

fn project_query(
    reply: anyhow::Result<Reply>,
    multiple: bool,
    top_frame: Option<String>,
) -> Response {
    let (resolution, snapshots) = match reply {
        Ok(Reply::DocumentQuerySelectorResolution(resolution)) => (resolution, None),
        Ok(Reply::DocumentQuerySelectorWithChildNodeSnapshotEvents(result)) => (
            result.query_selector_resolution,
            result.child_node_snapshot_events,
        ),
        Err(error) => return Response::error(-32000, error.to_string()),
        _ => unreachable!("DOM query selector reply"),
    };
    let mut response = match query_selector_result_from_renderer_resolution(resolution, multiple) {
        Ok(result) if multiple => Response::success(json!({"nodeIds": result.node_ids})),
        Ok(result) => {
            Response::success(json!({"nodeId": result.node_ids.first().copied().unwrap_or(0)}))
        }
        Err(error) => Response::error(error.code, error.message),
    };
    if let Some(snapshots) = snapshots {
        response.notifications = child_notifications(snapshots, top_frame.as_deref());
    }
    response
}
