//! Renderer-local DOM edits finish through the same channel as DOM queries.
//! Node-id lookups are continuations of the handler, not Browser round trips.

use super::*;

pub(super) fn handles(action: DomAction) -> bool {
    matches!(
        action,
        DomAction::Disable
            | DomAction::RemoveNode
            | DomAction::Focus
            | DomAction::SetAttributeValue
            | DomAction::RemoveAttribute
            | DomAction::MoveTo
            | DomAction::SetAttributesAsText
            | DomAction::SetNodeName
            | DomAction::SetNodeValue
            | DomAction::SetOuterHtml
            | DomAction::ScrollIntoViewIfNeeded
            | DomAction::GetNodeForLocation
            | DomAction::SetNodeStackTracesEnabled
            | DomAction::GetNodeStackTraces
    )
}

pub(super) fn prepare(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    action: DomAction,
) -> Result<Operation, StartError> {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(&owner);
    match action {
        DomAction::Disable => Ok(Operation::new(
            Command::DiscardDomAgentFrontendBindings {
                inspector_session_id: session,
            },
            |reply| unit(reply, "Could not disable DOM agent"),
        )),
        DomAction::RemoveNode => {
            let params =
                build_cdp_remove_node_command(conn, cmd)?.ok_or_else(StartError::invalid_params)?;
            // The first step only resolves a native node binding, but removal
            // enters V8. Keep the complete chain on its owner turn; agent-only
            // operations below inherit their backend entry requirement.
            Ok(with_backend(
                session,
                params.reference,
                OwnerTurn,
                |backend_node_id| {
                    Operation::new(
                        Command::RemoveDocumentBackendNodeId { backend_node_id },
                        |reply| match reply {
                            Ok(Reply::Bool(true)) => Response::success(json!({})),
                            Ok(Reply::Bool(false)) => {
                                Response::error(-32000, "Could not remove node")
                            }
                            Err(error) => {
                                Response::error(-32000, format!("Could not remove node: {error}"))
                            }
                            _ => unreachable!("DOM remove node reply"),
                        },
                    )
                },
            ))
        }
        DomAction::Focus => {
            let params: NodeReferenceParams = cmd
                .get_params()
                .map_err(|_| StartError::invalid_params())?
                .unwrap_or_default();
            if params.node_id.is_none() && params.backend_node_id.is_none() {
                let object_id = params.object_id.ok_or_else(|| StartError {
                    code: -32000,
                    message: "Either nodeId, backendNodeId or objectId must be specified"
                        .to_owned(),
                })?;
                let object_id = dom_object_reference_id_for_owner(conn, &owner, &object_id.into());
                Ok(Operation::new(
                    Command::focus_document_node_for_object_id(session, object_id),
                    |reply| focus(reply, "Could not find node with given id"),
                ))
            } else {
                let reference =
                    devtools_node_reference_from_ids(params.node_id, params.backend_node_id)
                        .ok_or_else(StartError::invalid_params)?;
                let missing = if matches!(reference, DevToolsDomNodeReference::FrontendNodeId(_)) {
                    "Could not find node with given id"
                } else {
                    "No node found for given backend id"
                };
                Ok(with_backend(
                    session,
                    reference,
                    OwnerTurn,
                    move |backend_node_id| {
                        Operation::new(
                            Command::FocusDocumentBackendNode { backend_node_id },
                            move |reply| focus(reply, missing),
                        )
                    },
                ))
            }
        }
        DomAction::SetAttributeValue | DomAction::RemoveAttribute => {
            let (id, mutation) = if action == DomAction::SetAttributeValue {
                let params: SetAttributeValueParams = cmd
                    .get_params()
                    .ok()
                    .flatten()
                    .ok_or_else(StartError::invalid_params)?;
                (
                    cdp_id_from_i64(*params.node_id.inner()),
                    RendererDomAttributeMutation::Set {
                        name: params.name,
                        value: params.value,
                    },
                )
            } else {
                let params: RemoveAttributeParams = cmd
                    .get_params()
                    .ok()
                    .flatten()
                    .ok_or_else(StartError::invalid_params)?;
                (
                    cdp_id_from_i64(*params.node_id.inner()),
                    RendererDomAttributeMutation::Remove { name: params.name },
                )
            };
            let id = id.ok_or_else(StartError::invalid_params)?;
            Ok(with_backend(
                session,
                DevToolsDomNodeReference::FrontendNodeId(id),
                OwnerTurn,
                move |backend_node_id| {
                    Operation::new(
                        Command::MutateDocumentBackendNodeAttribute {
                            backend_node_id,
                            mutation,
                        },
                        attribute,
                    )
                },
            ))
        }
        DomAction::MoveTo
        | DomAction::SetAttributesAsText
        | DomAction::SetNodeName
        | DomAction::SetNodeValue
        | DomAction::SetOuterHtml => {
            let edit = crate::domains::dom::edit::renderer_dom_edit_from_cdp(cmd, action)?;
            Ok(Operation::new(
                Command::EditDocumentNode {
                    inspector_session_id: session,
                    edit,
                },
                project_edit,
            ))
        }
        DomAction::ScrollIntoViewIfNeeded => {
            let params: ScrollIntoViewIfNeededParams = cmd
                .get_params()
                .ok()
                .flatten()
                .ok_or_else(StartError::invalid_params)?;
            let rect = validated_scroll_into_view_rect(params.rect)?;
            if let Some(object_id) = params.reference.object_id {
                let object_id = dom_object_reference_id_for_owner(conn, &owner, &object_id.into());
                Ok(Operation::new(
                    Command::scroll_object_node_into_view_if_needed(session, object_id, rect),
                    scroll,
                ))
            } else {
                let reference = devtools_node_reference_from_ids(
                    params.reference.node_id,
                    params.reference.backend_node_id,
                )
                .ok_or_else(StartError::node_not_found)?;
                Ok(with_backend(
                    session,
                    reference,
                    OwnerTurn,
                    move |backend_node_id| {
                        Operation::new(
                            Command::ScrollBackendNodeIntoViewIfNeeded {
                                backend_node_id,
                                rect,
                            },
                            scroll,
                        )
                    },
                ))
            }
        }
        DomAction::GetNodeForLocation => {
            let params = build_cdp_get_node_for_location_command(conn, cmd)?;
            let top_frame_id =
                top_frame_id_for_owner(conn, &owner).ok_or_else(StartError::no_document_loaded)?;
            Ok(Operation::new(
                Command::DocumentHitTest {
                    inspector_session_id: session,
                    x: params.x,
                    y: params.y,
                    include_user_agent_shadow_dom: params.include_user_agent_shadow_dom,
                    ignore_pointer_events_none: params.ignore_pointer_events_none,
                },
                move |reply| match reply {
                    Ok(Reply::OptionalDocumentHitTest(Some(hit))) => Response::success(
                        get_node_for_location_result_value(&DevToolsGetNodeForLocationResult {
                            backend_node_id: hit.node.backend_node_id,
                            frame_id: DevToolsFrameId::from(hit.frame_id.unwrap_or(top_frame_id)),
                            node_id: (hit.node.node_id != 0).then_some(hit.node.node_id),
                        }),
                    ),
                    Ok(Reply::OptionalDocumentHitTest(None)) => {
                        Response::error(-32000, "No node found at given location")
                    }
                    Err(error) => {
                        Response::error(-32000, format!("Could not hit test document: {error}"))
                    }
                    _ => unreachable!("DOM hit test reply"),
                },
            ))
        }
        DomAction::SetNodeStackTracesEnabled => {
            #[derive(serde::Deserialize)]
            struct Params {
                enable: bool,
            }
            let params: Params = cmd
                .get_params()
                .ok()
                .flatten()
                .ok_or_else(StartError::invalid_params)?;
            Ok(Operation::new(
                Command::DocumentSetNodeStackTracesEnabled {
                    inspector_session_id: session,
                    enabled: params.enable,
                },
                |reply| match reply {
                    Ok(Reply::DocumentNodeStackTracesEnabled) => Response::success(json!({})),
                    Err(error) => Response::error(
                        -32000,
                        format!("Could not configure DOM node stack traces: {error}"),
                    ),
                    _ => unreachable!("DOM stack traces enable reply"),
                },
            ))
        }
        DomAction::GetNodeStackTraces => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Params {
                node_id: u32,
            }
            let params: Params = cmd
                .get_params()
                .ok()
                .flatten()
                .ok_or_else(StartError::invalid_params)?;
            Ok(Operation::new(
                Command::DocumentNodeStackTrace {
                    inspector_session_id: session,
                    frontend_node_id: params.node_id,
                },
                |reply| {
                    use moli_core::page::RendererDomNodeStackTraceResolution as Trace;
                    match reply {
                        Ok(Reply::DocumentNodeStackTrace(Trace::Found(Some(trace)))) => {
                            Response::success(
                                json!({"creation": {"callFrames": trace.call_frames.into_iter().map(|frame| json!({
                        "functionName": frame.function_name, "scriptId": frame.script_id, "url": frame.url, "lineNumber": frame.line_number, "columnNumber": frame.column_number,
                    })).collect::<Vec<_>>()}}),
                            )
                        }
                        Ok(Reply::DocumentNodeStackTrace(Trace::Found(None))) => {
                            Response::success(json!({}))
                        }
                        Ok(Reply::DocumentNodeStackTrace(Trace::MissingNode)) => node_not_found(),
                        Err(error) => Response::error(
                            -32000,
                            format!("Could not get DOM node stack traces: {error}"),
                        ),
                        _ => unreachable!("DOM stack trace reply"),
                    }
                },
            ))
        }
        _ => unreachable!("DOM mutation preparation only handles its admitted actions"),
    }
}

fn unit(reply: anyhow::Result<Reply>, prefix: &str) -> Response {
    match reply {
        Ok(Reply::Unit) => Response::success(json!({})),
        Err(error) => Response::error(-32000, format!("{prefix}: {error}")),
        _ => unreachable!("DOM configuration unit reply"),
    }
}

fn focus(reply: anyhow::Result<Reply>, missing: &str) -> Response {
    match reply {
        Ok(Reply::DomFocusOutcome(RendererDomFocusOutcome::Focused)) => {
            Response::success(json!({}))
        }
        Ok(Reply::DomFocusOutcome(RendererDomFocusOutcome::NodeNotFound)) => {
            Response::error(-32000, missing)
        }
        Ok(Reply::DomFocusOutcome(RendererDomFocusOutcome::NodeNotElement)) => {
            Response::error(-32000, "Node is not an Element")
        }
        Ok(Reply::DomFocusOutcome(RendererDomFocusOutcome::ElementNotFocusable)) => {
            Response::error(-32000, "Element is not focusable")
        }
        Err(error) => Response::error(-32000, format!("Could not focus node: {error}")),
        _ => unreachable!("DOM focus reply"),
    }
}

fn attribute(reply: anyhow::Result<Reply>) -> Response {
    match reply {
        Ok(Reply::DomAttributeMutationOutcome(RendererDomAttributeMutationOutcome::Applied {
            ..
        })) => Response::success(json!({})),
        Ok(Reply::DomAttributeMutationOutcome(
            RendererDomAttributeMutationOutcome::NodeNotFound,
        )) => node_not_found(),
        Ok(Reply::DomAttributeMutationOutcome(
            RendererDomAttributeMutationOutcome::NodeNotElement,
        )) => Response::error(-32000, "Node is not an Element"),
        Ok(Reply::DomAttributeMutationOutcome(
            RendererDomAttributeMutationOutcome::InvalidName { name },
        )) => Response::error(
            -32000,
            format!("InvalidCharacterError '{name}' is not a valid attribute name."),
        ),
        Err(error) => Response::error(-32000, format!("Could not mutate node attribute: {error}")),
        _ => unreachable!("DOM attribute mutation reply"),
    }
}

fn project_edit(reply: anyhow::Result<Reply>) -> Response {
    use RendererDomEditOutcome as Edit;
    match reply {
        Ok(Reply::DomEditOutcome(Edit::Applied {
            result_frontend_node_id: None,
        })) => Response::success(json!({})),
        Ok(Reply::DomEditOutcome(Edit::Applied {
            result_frontend_node_id: Some(node_id),
        })) => Response::success(json!({"nodeId": node_id})),
        Ok(Reply::DomEditOutcome(Edit::NodeNotFound)) => node_not_found(),
        Ok(Reply::DomEditOutcome(Edit::NodeNotElement)) => {
            Response::error(-32000, "Node is not an Element")
        }
        Ok(Reply::DomEditOutcome(Edit::NodeValueUnsupported)) => Response::error(
            -32000,
            "Can only set value of text nodes or processing instructions",
        ),
        Ok(Reply::DomEditOutcome(Edit::MoveIntoSelfOrDescendant)) => {
            Response::error(-32000, "Unable to move node into self or descendant")
        }
        Ok(Reply::DomEditOutcome(Edit::AnchorNotChildOfTarget)) => {
            Response::error(-32000, "Anchor node must be child of the target element")
        }
        Ok(Reply::DomEditOutcome(Edit::DetachedNode)) => {
            Response::error(-32000, "Cannot edit detached node")
        }
        Ok(Reply::DomEditOutcome(Edit::InvalidName { name })) => Response::error(
            -32000,
            format!("InvalidCharacterError '{name}' is not a valid node name."),
        ),
        Ok(Reply::DomEditOutcome(Edit::CouldNotParseAttributes)) => {
            Response::error(-32000, "Could not parse value as attributes")
        }
        Ok(Reply::DomEditOutcome(Edit::MutationFailed)) => {
            Response::error(-32000, "Could not edit node")
        }
        Err(error) => Response::error(-32000, format!("Could not edit node: {error}")),
        _ => unreachable!("DOM edit reply"),
    }
}

fn scroll(reply: anyhow::Result<Reply>) -> Response {
    let result = match reply {
        Ok(Reply::ScrollIntoViewResult(outcome)) => Ok(outcome),
        Err(error) => Err(error),
        _ => unreachable!("DOM scroll reply"),
    };
    match complete_scroll_into_view_result(result) {
        Ok(()) => Response::success(json!({})),
        Err(error) => Response::error(-32000, error.message),
    }
}
