use super::*;
use crate::devtools_runtime::DevToolsDomNodeReference;
use crate::domains::native::{self, NativeCommandStep, NodeLookupExecution};
use moli_core::{
    RendererNativeOperation as Operation, RendererNativeProtocolResponse as Response,
    RendererPageCommand as Command, RendererPageReply as Reply,
};

pub(crate) fn try_start(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> Option<NativeCommandStep> {
    let action = cmd.parse_action::<CssAction>()?;
    let inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    let frame_id = top_frame_id_for_session(conn, cmd.session_id).unwrap_or_default();
    let invalid_params =
        || NativeCommandStep::Complete(CommandOutputPlan::error(-32602, "InvalidParams"));
    let operation = match action {
        CssAction::Enable => {
            conn.mutate_target_page_state_for_session(cmd.session_id, |state| {
                state.css_enabled = true
            });
            Operation::new(
                Command::StyleSheetInventoryForDocument {
                    inspector_session_id,
                },
                move |reply| match reply {
                    Ok(Reply::StyleSheetInventory(update)) => {
                        let mut response = Response::success(json!({}));
                        for id in update.removed {
                            response.notifications.push(json!({"method": "CSS.styleSheetRemoved", "params": {"styleSheetId": id}}).into());
                        }
                        for header in update.added {
                            response.notifications.push(json!({"method": "CSS.styleSheetAdded", "params": {
                                "header": style_sheets::style_sheet_header_value(&frame_id, &header),
                            }}).into());
                        }
                        response
                    }
                    Err(error) => Response::error(-32000, error.to_string()),
                    _ => unreachable!("CSS inventory reply"),
                },
            )
        }
        CssAction::Disable => {
            conn.mutate_target_page_state_for_session(cmd.session_id, |state| {
                state.css_enabled = false
            });
            Operation::new(
                Command::ResetCssAgentSession {
                    inspector_session_id,
                },
                |reply| match reply {
                    Ok(Reply::Unit) => Response::success(json!({})),
                    Err(error) => Response::error(-32000, error.to_string()),
                    _ => unreachable!("CSS reset reply"),
                },
            )
        }
        CssAction::GetStyleSheet => {
            let params = match cmd.get_params::<StyleSheetIdParams>() {
                Ok(Some(params)) => params,
                _ => return Some(invalid_params()),
            };
            let id = params.style_sheet_id.as_ref().to_owned();
            Operation::new(
                Command::StyleSheetPayloadForStyleSheetId {
                    inspector_session_id,
                    style_sheet_id: id.clone(),
                },
                move |reply| match reply {
                    Ok(Reply::OptionalStyleSheetPayload(Some(payload))) => Response::success(
                        style_sheets::get_style_sheet_result(&id, &frame_id, payload),
                    ),
                    Ok(Reply::OptionalStyleSheetPayload(None)) => {
                        Response::error(-32000, "Could not find stylesheet with given id")
                    }
                    Err(error) => Response::error(-32000, error.to_string()),
                    _ => unreachable!("CSS stylesheet reply"),
                },
            )
        }
        CssAction::SetStyleSheetText => {
            let params = match cmd.get_params::<SetStyleSheetTextParams>() {
                Ok(Some(params)) => params,
                _ => return Some(invalid_params()),
            };
            let id = params.style_sheet_id.as_ref().to_owned();
            Operation::new(
                Command::SetInlineStyleSheetTextForStyleSheetId {
                    inspector_session_id,
                    style_sheet_id: id.clone(),
                    text: params.text,
                },
                move |reply| match reply {
                    Ok(Reply::Bool(true)) => {
                        Response::success(json!({"styleSheetId": id, "sourceMapURL": ""}))
                    }
                    Ok(Reply::Bool(false)) => {
                        Response::error(-32000, "Could not find stylesheet with given id")
                    }
                    Err(error) => Response::error(-32000, error.to_string()),
                    _ => unreachable!("CSS text mutation reply"),
                },
            )
        }
        CssAction::GetComputedStyleForNode
        | CssAction::GetInlineStylesForNode
        | CssAction::GetMatchedStylesForNode => {
            let params = match cmd.get_params::<NodeReferenceParams>() {
                Ok(Some(params)) => params,
                _ => return Some(invalid_params()),
            };
            let query = match action {
                CssAction::GetComputedStyleForNode => StyleQuery::Computed,
                CssAction::GetInlineStylesForNode => {
                    StyleQuery::Inline(InlineStyleQueryKind::InlineStyles)
                }
                CssAction::GetMatchedStylesForNode => {
                    StyleQuery::Inline(InlineStyleQueryKind::MatchedStyles)
                }
                _ => unreachable!(),
            };
            if let Some(object_id) = params.object_id {
                if matches!(query, StyleQuery::Inline(_)) {
                    // The existing native API has no inline-style object-id
                    // lookup. Preserve its explicit unsupported-method result.
                    return None;
                }
                Operation::new(
                    Command::computed_style_properties_for_object_id(
                        cmd.session_id.map(str::to_owned),
                        object_id,
                    ),
                    move |reply| project_style(reply, query),
                )
            } else if let Some(frontend_node_id) = params.node_id {
                native::with_backend_node(
                    inspector_session_id,
                    DevToolsDomNodeReference::FrontendNodeId(frontend_node_id),
                    NodeLookupExecution::NestedMain,
                    move |id| backend_style_operation(id, query),
                )
            } else if let Some(backend_node_id) = params.backend_node_id {
                backend_style_operation(backend_node_id, query)
            } else {
                return Some(NativeCommandStep::Complete(CommandOutputPlan::error(
                    -32000,
                    "Could not find node with given id",
                )));
            }
        }
    };
    Some(native::start_operation(conn, cmd, operation))
}

#[derive(Clone, Copy)]
enum StyleQuery {
    Computed,
    Inline(InlineStyleQueryKind),
}

fn backend_style_operation(backend_node_id: u32, query: StyleQuery) -> Operation {
    let command = match query {
        StyleQuery::Computed => {
            Command::ComputedStylePropertiesForBackendNodeId { backend_node_id }
        }
        StyleQuery::Inline(_) => {
            Command::DocumentNodeAttributesForBackendNodeId { backend_node_id }
        }
    };
    Operation::new(command, move |reply| project_style(reply, query))
}

fn project_style(reply: anyhow::Result<Reply>, query: StyleQuery) -> Response {
    match (query, reply) {
        (StyleQuery::Computed, Ok(Reply::ComputedStyleProperties(Some(properties)))) => {
            Response::success(computed_style_result(properties))
        }
        (StyleQuery::Computed, Ok(Reply::ComputedStyleProperties(None))) => {
            Response::error(-32000, "Could not find node with given id")
        }
        (StyleQuery::Inline(kind), Ok(Reply::DocumentNodeAttributesResolution(attributes))) => {
            match inline_style_result_from_attributes_resolution(attributes, kind) {
                Ok(result) => Response::success(result),
                Err(error) => Response::error(error.code, error.message),
            }
        }
        (StyleQuery::Computed, Err(error)) => {
            Response::error(-32000, format!("failed to compute style: {error}"))
        }
        (_, Err(error)) => Response::error(-32000, error.to_string()),
        _ => unreachable!("CSS native style reply"),
    }
}
