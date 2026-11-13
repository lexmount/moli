use super::*;
use crate::automation::DevToolsDomNodeReference;
use crate::domains::native::{self, NativeCommandStep};
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
                        response.notifications.extend(
                            style_sheets::style_sheet_inventory_notifications(&frame_id, update)
                                .map(|(method, params)| {
                                    json!({ "method": method.method(), "params": params }).into()
                                }),
                        );
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
                native::unit_response,
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
            } else {
                let reference = if let Some(id) = params.node_id {
                    DevToolsDomNodeReference::FrontendNodeId(id)
                } else if let Some(id) = params.backend_node_id {
                    DevToolsDomNodeReference::BackendNodeId(id)
                } else {
                    return Some(NativeCommandStep::Complete(CommandOutputPlan::error(
                        -32000,
                        "Could not find node with given id",
                    )));
                };
                match query {
                    StyleQuery::Inline(_) => Operation::new(
                        Command::DocumentNodeAttributes {
                            reference: reference.into_renderer_reference(inspector_session_id),
                        },
                        move |reply| project_style(reply, query),
                    ),
                    StyleQuery::Computed => Operation::new(
                        Command::ComputedStyleProperties {
                            reference: reference.into_renderer_reference(inspector_session_id),
                        },
                        move |reply| project_style(reply, query),
                    ),
                }
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
