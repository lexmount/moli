use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};
use crate::domains::actions::CssAction;
use crate::domains::command_output::CommandOutputPlan;
use chromiumoxide_cdp::cdp::browser_protocol::css::{
    GetStyleSheetTextParams as StyleSheetIdParams, SetStyleSheetTextParams,
};
use moli_core::page::{
    CompletedPageCommand, Page, PendingPageCommand, RendererDocumentNodeAttributesResolution,
};
use moli_css_parse::{DeclarationParseOptions, parse_declaration_list};
use serde::Deserialize;
use serde_json::{Value, json};

mod node_references;
mod style_sheets;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeReferenceParams {
    #[serde(default)]
    node_id: Option<u32>,
    #[serde(default)]
    backend_node_id: Option<u32>,
    #[serde(default)]
    object_id: Option<String>,
}

pub(crate) struct PendingCssCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    kind: PendingCssCommandKind,
    pending: PendingPageCommand,
}

pub(crate) struct CompletedCssCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    kind: PendingCssCommandKind,
    completed: Result<CompletedPageCommand, String>,
}

pub(crate) enum CssCommandDispatchStep {
    Pending(PendingCssCommandDispatch),
    Complete(CommandOutputPlan),
}

enum PendingCssCommandKind {
    Enable {
        frame_id: String,
    },
    Disable,
    ResolveFrontendNodeForComputedStyle,
    ResolveFrontendNodeForInlineStyle {
        kind: InlineStyleQueryKind,
    },
    SetStyleSheetText {
        style_sheet_id: String,
    },
    GetStyleSheet {
        style_sheet_id: String,
        frame_id: String,
    },
    GetComputedStyleForNode,
    GetInlineStyleForNode {
        kind: InlineStyleQueryKind,
    },
}

#[derive(Clone, Copy)]
enum InlineStyleQueryKind {
    InlineStyles,
    MatchedStyles,
}

struct PendingCssCommandStartError {
    code: i32,
    message: String,
}

impl PendingCssCommandDispatch {
    fn from_command(
        conn: &CdpConnection,
        cmd: &Cmd<'_>,
        kind: PendingCssCommandKind,
        pending: PendingPageCommand,
    ) -> Self {
        Self {
            command_id: cmd.id,
            owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
            kind,
            pending,
        }
    }

    pub async fn wait(self) -> CompletedCssCommandDispatch {
        CompletedCssCommandDispatch {
            command_id: self.command_id,
            owner_scope: self.owner_scope,
            kind: self.kind,
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
        }
    }
}

impl CompletedCssCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }
}

impl PendingCssCommandStartError {
    fn invalid_params() -> Self {
        Self {
            code: -32602,
            message: "InvalidParams".to_owned(),
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

    fn renderer_error(error: impl std::fmt::Display) -> Self {
        Self {
            code: -32000,
            message: error.to_string(),
        }
    }
}

impl CssAction {
    fn requires_document_access(self) -> bool {
        !matches!(self, Self::Enable | Self::Disable)
    }
}

pub(crate) fn try_start_css_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<CssCommandDispatchStep> {
    let Some(action) = cmd.parse_action::<CssAction>() else {
        return Some(CssCommandDispatchStep::Complete(CommandOutputPlan::error(
            -32601,
            "UnknownMethod",
        )));
    };
    if action.requires_document_access()
        && let Err(message) = conn.ensure_document_accessible_for_session_owner(cmd.session_id)
    {
        return Some(CssCommandDispatchStep::Complete(CommandOutputPlan::error(
            -32000, message,
        )));
    }
    match action {
        CssAction::Enable => match style_sheets::start_pending_enable_command(conn, cmd) {
            Ok(Some(pending)) => Some(CssCommandDispatchStep::Pending(pending)),
            Ok(None) => Some(CssCommandDispatchStep::Complete(
                CommandOutputPlan::success(),
            )),
            Err(error) => Some(CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                error.code,
                error.message,
            ))),
        },
        CssAction::Disable => match style_sheets::start_pending_disable_command(conn, cmd) {
            Ok(Some(pending)) => Some(CssCommandDispatchStep::Pending(pending)),
            Ok(None) => Some(CssCommandDispatchStep::Complete(
                CommandOutputPlan::success(),
            )),
            Err(error) => Some(CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                error.code,
                error.message,
            ))),
        },
        CssAction::GetStyleSheet
        | CssAction::SetStyleSheetText
        | CssAction::GetComputedStyleForNode
        | CssAction::GetInlineStylesForNode
        | CssAction::GetMatchedStylesForNode => match start_pending_css_command(conn, cmd) {
            Ok(Some(pending)) => Some(CssCommandDispatchStep::Pending(pending)),
            Ok(None) => None,
            Err(error) => Some(CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                error.code,
                error.message,
            ))),
        },
    }
}

fn start_pending_css_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<PendingCssCommandDispatch>, PendingCssCommandStartError> {
    let Some(action) = cmd.parse_action::<CssAction>() else {
        return Err(PendingCssCommandStartError {
            code: -32601,
            message: "UnknownMethod".to_owned(),
        });
    };
    if action.requires_document_access()
        && let Err(message) = conn.ensure_document_accessible_for_session_owner(cmd.session_id)
    {
        return Err(PendingCssCommandStartError {
            code: -32000,
            message,
        });
    }
    match action {
        CssAction::GetStyleSheet => start_pending_get_style_sheet_command(conn, cmd).map(Some),
        CssAction::SetStyleSheetText => {
            start_pending_set_style_sheet_text_command(conn, cmd).map(Some)
        }
        CssAction::GetComputedStyleForNode => {
            start_pending_get_computed_style_for_node_command(conn, cmd).map(Some)
        }
        CssAction::GetInlineStylesForNode => {
            start_pending_inline_style_command(conn, cmd, InlineStyleQueryKind::InlineStyles)
        }
        CssAction::GetMatchedStylesForNode => {
            start_pending_inline_style_command(conn, cmd, InlineStyleQueryKind::MatchedStyles)
        }
        CssAction::Enable | CssAction::Disable => Err(PendingCssCommandStartError {
            code: -32601,
            message: "UnknownMethod".to_owned(),
        }),
    }
}

fn start_pending_get_style_sheet_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<PendingCssCommandDispatch, PendingCssCommandStartError> {
    let params: StyleSheetIdParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingCssCommandStartError::invalid_params()),
    };
    let frame_id = top_frame_id_for_session(conn, cmd.session_id).unwrap_or_default();
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    let Some(page) = loaded_page_mut_for_session(conn, cmd.session_id) else {
        return Err(PendingCssCommandStartError::no_document_loaded());
    };
    let style_sheet_id = params.style_sheet_id.as_ref().to_owned();
    let pending = page
        .start_style_sheet_payload_for_style_sheet_id_and_inspector_session(
            renderer_inspector_session_id,
            &style_sheet_id,
        )
        .map_err(PendingCssCommandStartError::renderer_error)?;
    Ok(PendingCssCommandDispatch::from_command(
        conn,
        cmd,
        PendingCssCommandKind::GetStyleSheet {
            style_sheet_id,
            frame_id,
        },
        pending,
    ))
}

fn start_pending_set_style_sheet_text_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<PendingCssCommandDispatch, PendingCssCommandStartError> {
    let params: SetStyleSheetTextParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingCssCommandStartError::invalid_params()),
    };
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    let Some(page) = loaded_page_mut_for_session(conn, cmd.session_id) else {
        return Err(PendingCssCommandStartError::no_document_loaded());
    };
    let style_sheet_id = params.style_sheet_id.as_ref().to_owned();
    let pending = page
        .start_set_inline_style_sheet_text_for_style_sheet_id_and_inspector_session(
            renderer_inspector_session_id,
            &style_sheet_id,
            &params.text,
        )
        .map_err(PendingCssCommandStartError::renderer_error)?;
    Ok(PendingCssCommandDispatch::from_command(
        conn,
        cmd,
        PendingCssCommandKind::SetStyleSheetText { style_sheet_id },
        pending,
    ))
}

fn start_pending_get_computed_style_for_node_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<PendingCssCommandDispatch, PendingCssCommandStartError> {
    let params: NodeReferenceParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingCssCommandStartError::invalid_params()),
    };
    if let Some(object_id) = params.object_id.as_deref() {
        let Some(page) = loaded_page_mut_for_session(conn, cmd.session_id) else {
            return Err(PendingCssCommandStartError::no_document_loaded());
        };
        let pending = page
            .start_computed_style_properties_for_object_id_in_inspector_session(
                cmd.session_id.map(str::to_owned),
                object_id,
            )
            .map_err(PendingCssCommandStartError::renderer_error)?;
        return Ok(PendingCssCommandDispatch::from_command(
            conn,
            cmd,
            PendingCssCommandKind::GetComputedStyleForNode,
            pending,
        ));
    }
    if let Some(cdp_node_id) = params.node_id {
        return node_references::start_frontend_node_binding_for_computed_style(
            conn,
            cmd,
            cdp_node_id,
        );
    }
    let Some(page) = loaded_page_mut_for_session(conn, cmd.session_id) else {
        return Err(PendingCssCommandStartError::no_document_loaded());
    };
    let pending = if let Some(backend_node_id) = params.backend_node_id {
        page.start_computed_style_properties_for_backend_node_id(backend_node_id)
    } else {
        return Err(PendingCssCommandStartError::node_not_found());
    }
    .map_err(PendingCssCommandStartError::renderer_error)?;
    Ok(PendingCssCommandDispatch::from_command(
        conn,
        cmd,
        PendingCssCommandKind::GetComputedStyleForNode,
        pending,
    ))
}

fn start_pending_inline_style_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    kind: InlineStyleQueryKind,
) -> Result<Option<PendingCssCommandDispatch>, PendingCssCommandStartError> {
    let params: NodeReferenceParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingCssCommandStartError::invalid_params()),
    };
    if params.object_id.is_some() {
        return Ok(None);
    }
    if let Some(cdp_node_id) = params.node_id {
        return node_references::start_frontend_node_binding_for_inline_style(
            conn,
            cmd,
            cdp_node_id,
            kind,
        );
    }
    let Some(page) = loaded_page_mut_for_session(conn, cmd.session_id) else {
        return Err(PendingCssCommandStartError::no_document_loaded());
    };
    let pending = if let Some(backend_node_id) = params.backend_node_id {
        page.start_document_node_attributes_for_backend_node_id(backend_node_id)
    } else {
        return Err(PendingCssCommandStartError::node_not_found());
    }
    .map_err(PendingCssCommandStartError::renderer_error)?;
    Ok(Some(PendingCssCommandDispatch::from_command(
        conn,
        cmd,
        PendingCssCommandKind::GetInlineStyleForNode { kind },
        pending,
    )))
}

pub(crate) fn complete_pending_css_command(
    conn: &mut CdpConnection,
    completed: CompletedCssCommandDispatch,
) -> CssCommandDispatchStep {
    let CompletedCssCommandDispatch {
        command_id,
        owner_scope,
        kind,
        completed,
    } = completed;
    let session_id = owner_scope.session_id();
    let Some(page) = conn
        .loaded_page_mut_for_protocol_access_for_route(
            session_id,
            owner_scope.session_owner_route(),
        )
        .ok()
    else {
        return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
            -32000,
            "NoDocumentLoaded",
        ));
    };
    match kind {
        PendingCssCommandKind::Enable { frame_id } => {
            let completion = match completed {
                Ok(completion) => completion,
                Err(error) => {
                    return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                        -32000, error,
                    ));
                }
            };
            CssCommandDispatchStep::Complete(
                match page.finish_style_sheet_inventory_for_document(completion) {
                    Ok(update) => style_sheets::complete_enable_command_output_plan(
                        &frame_id, session_id, update,
                    ),
                    Err(error) => CommandOutputPlan::error(-32000, error.to_string()),
                },
            )
        }
        PendingCssCommandKind::Disable => {
            let completion = match completed {
                Ok(completion) => completion,
                Err(error) => {
                    return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                        -32000, error,
                    ));
                }
            };
            CssCommandDispatchStep::Complete(
                match page.finish_reset_css_agent_session(completion) {
                    Ok(()) => CommandOutputPlan::success(),
                    Err(error) => CommandOutputPlan::error(-32000, error.to_string()),
                },
            )
        }
        PendingCssCommandKind::ResolveFrontendNodeForComputedStyle => {
            let completion = match completed {
                Ok(completion) => completion,
                Err(error) => {
                    return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                        -32000, error,
                    ));
                }
            };
            let backend_node_id = match page.finish_document_frontend_node_binding(completion) {
                Ok(resolution) => {
                    match node_references::backend_node_id_from_frontend_resolution(resolution) {
                        Some(backend_node_id) => backend_node_id,
                        None => {
                            return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                                -32000,
                                "Could not find node with given id",
                            ));
                        }
                    }
                }
                Err(error) => {
                    return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Could not resolve frontend node binding: {error}"),
                    ));
                }
            };
            let pending =
                match page.start_computed_style_properties_for_backend_node_id(backend_node_id) {
                    Ok(pending) => pending,
                    Err(error) => {
                        return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                            -32000,
                            error.to_string(),
                        ));
                    }
                };
            CssCommandDispatchStep::Pending(PendingCssCommandDispatch {
                command_id,
                owner_scope,
                kind: PendingCssCommandKind::GetComputedStyleForNode,
                pending,
            })
        }
        PendingCssCommandKind::ResolveFrontendNodeForInlineStyle { kind } => {
            let completion = match completed {
                Ok(completion) => completion,
                Err(error) => {
                    return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                        -32000, error,
                    ));
                }
            };
            let backend_node_id = match page.finish_document_frontend_node_binding(completion) {
                Ok(resolution) => {
                    match node_references::backend_node_id_from_frontend_resolution(resolution) {
                        Some(backend_node_id) => backend_node_id,
                        None => {
                            return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                                -32000,
                                "Could not find node with given id",
                            ));
                        }
                    }
                }
                Err(error) => {
                    return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Could not resolve frontend node binding: {error}"),
                    ));
                }
            };
            let pending =
                match page.start_document_node_attributes_for_backend_node_id(backend_node_id) {
                    Ok(pending) => pending,
                    Err(error) => {
                        return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                            -32000,
                            error.to_string(),
                        ));
                    }
                };
            CssCommandDispatchStep::Pending(PendingCssCommandDispatch {
                command_id,
                owner_scope,
                kind: PendingCssCommandKind::GetInlineStyleForNode { kind },
                pending,
            })
        }
        PendingCssCommandKind::GetStyleSheet {
            style_sheet_id,
            frame_id,
        } => {
            let completion = match completed {
                Ok(completion) => completion,
                Err(error) => {
                    return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                        -32000, error,
                    ));
                }
            };
            CssCommandDispatchStep::Complete(match page.finish_style_sheet_payload(completion) {
                Ok(Some(payload)) => style_sheets::get_style_sheet_command_output_plan(
                    &style_sheet_id,
                    &frame_id,
                    payload,
                ),
                Ok(None) => {
                    CommandOutputPlan::error(-32000, "Could not find stylesheet with given id")
                }
                Err(error) => CommandOutputPlan::error(-32000, error.to_string()),
            })
        }
        PendingCssCommandKind::SetStyleSheetText { style_sheet_id } => {
            let completion = match completed {
                Ok(completion) => completion,
                Err(error) => {
                    return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                        -32000, error,
                    ));
                }
            };
            CssCommandDispatchStep::Complete(
                match page.finish_set_inline_style_sheet_text(completion) {
                    Ok(true) => CommandOutputPlan::result(json!({
                        "sourceMapURL": "",
                        "styleSheetId": style_sheet_id,
                    })),
                    Ok(false) => {
                        CommandOutputPlan::error(-32000, "Could not find stylesheet with given id")
                    }
                    Err(error) => CommandOutputPlan::error(-32000, error.to_string()),
                },
            )
        }
        PendingCssCommandKind::GetComputedStyleForNode => {
            let completion = match completed {
                Ok(completion) => completion,
                Err(error) => {
                    return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                        -32000, error,
                    ));
                }
            };
            CssCommandDispatchStep::Complete(
                match page.finish_computed_style_properties(completion) {
                    Ok(Some(properties)) => computed_style_command_output_plan(properties),
                    Ok(None) => {
                        CommandOutputPlan::error(-32000, "Could not find node with given id")
                    }
                    Err(error) => CommandOutputPlan::error(
                        -32000,
                        format!("failed to compute style: {error}"),
                    ),
                },
            )
        }
        PendingCssCommandKind::GetInlineStyleForNode { kind } => {
            let completion = match completed {
                Ok(completion) => completion,
                Err(error) => {
                    return CssCommandDispatchStep::Complete(CommandOutputPlan::error(
                        -32000, error,
                    ));
                }
            };
            CssCommandDispatchStep::Complete(
                match page.finish_document_node_attributes(completion) {
                    Ok(resolution) => {
                        inline_style_result_from_attributes_resolution(resolution, kind)
                            .map(CommandOutputPlan::result)
                            .unwrap_or_else(|error| {
                                CommandOutputPlan::error(error.code, error.message)
                            })
                    }
                    Err(error) => CommandOutputPlan::error(-32000, error.to_string()),
                },
            )
        }
    }
}

fn computed_style_command_output_plan(properties: Vec<(String, String)>) -> CommandOutputPlan {
    CommandOutputPlan::result(json!({
                    "computedStyle": properties
                        .into_iter()
                        .map(|(name, value)| json!({ "name": name, "value": value }))
                        .collect::<Vec<_>>()
    }))
}

fn inline_style_result_from_attributes_resolution(
    resolution: RendererDocumentNodeAttributesResolution,
    kind: InlineStyleQueryKind,
) -> Result<Value, PendingCssCommandStartError> {
    let attributes = match resolution {
        RendererDocumentNodeAttributesResolution::Found(attributes) => attributes,
        RendererDocumentNodeAttributesResolution::NotElement => Vec::new(),
        RendererDocumentNodeAttributesResolution::MissingNode => {
            return Err(PendingCssCommandStartError::node_not_found());
        }
    };
    Ok(inline_style_result_for_attributes(&attributes, kind))
}

fn inline_style_result_for_attributes(
    attributes: &[(String, String)],
    kind: InlineStyleQueryKind,
) -> Value {
    let mut result = Value::Object(inline_style_result_object(attributes));
    if matches!(kind, InlineStyleQueryKind::MatchedStyles) {
        add_empty_matched_style_scaffolding(&mut result);
    }
    result
}

fn inline_style_result_object(attributes: &[(String, String)]) -> serde_json::Map<String, Value> {
    let mut result = serde_json::Map::new();
    if let Some(style_text) = style_attribute_text(attributes) {
        result.insert("inlineStyle".to_owned(), cdp_css_style(style_text));
    }
    result
}

fn add_empty_matched_style_scaffolding(result: &mut Value) {
    let Some(result_object) = result.as_object_mut() else {
        *result = json!({});
        return;
    };
    result_object.insert("matchedCSSRules".to_owned(), json!([]));
    result_object.insert("pseudoElements".to_owned(), json!([]));
    result_object.insert("inherited".to_owned(), json!([]));
    result_object.insert("inheritedPseudoElements".to_owned(), json!([]));
    result_object.insert("cssKeyframesRules".to_owned(), json!([]));
}

fn loaded_page_mut_for_session<'a>(
    conn: &'a mut CdpConnection,
    session_id: Option<&str>,
) -> Option<&'a mut Page> {
    conn.loaded_page_mut_for_protocol_access(session_id).ok()
}

fn top_frame_id_for_session(conn: &CdpConnection, session_id: Option<&str>) -> Option<String> {
    conn.target_session_owner_frame_tree_identity(session_id)
        .map(|(frame_id, _, _, _)| frame_id)
}

fn style_attribute_text(attributes: &[(String, String)]) -> Option<&str> {
    attributes
        .iter()
        .find_map(|(name, value)| name.eq_ignore_ascii_case("style").then_some(value.as_str()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssDeclaration {
    name: String,
    value: String,
    important: bool,
}

fn cdp_css_style(css_text: &str) -> Value {
    let css_properties = parse_inline_style_declarations(css_text)
        .into_iter()
        .map(|declaration| {
            let text = if declaration.important {
                format!("{}: {} !important;", declaration.name, declaration.value)
            } else {
                format!("{}: {};", declaration.name, declaration.value)
            };
            json!({
                "name": declaration.name,
                "value": declaration.value,
                "important": declaration.important,
                "implicit": false,
                "text": text,
                "parsedOk": true,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "cssProperties": css_properties,
        "shorthandEntries": [],
        "cssText": css_text,
    })
}

fn parse_inline_style_declarations(style_text: &str) -> Vec<CssDeclaration> {
    parse_declaration_list(
        style_text,
        DeclarationParseOptions {
            canonicalize_property_name: true,
            unescape_value_semicolons: false,
            preserve_empty_values: false,
        },
    )
    .into_iter()
    .map(|declaration| CssDeclaration {
        name: declaration.name,
        value: declaration.value,
        important: declaration.important,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_inline_style_declarations;
    use crate::conn::{BrowserContext, CdpCommandTaskStep, CdpSchedulerEvent, PageTargetHost};
    use crate::domains::page::LOADER_ID;
    use crate::testing::{TestContext, wait_until_renderer_document_load};
    use moli_core::page::{
        CompletedPageCommand, Page, RENDERER_BACKEND_NODE_ID_START, is_renderer_backend_node_id,
    };
    use serde_json::Value;
    use serde_json::json;

    fn take_response_by_id(ctx: &mut TestContext, id: u64) -> serde_json::Value {
        let pos = ctx
            .sent
            .iter()
            .position(|message| message["id"] == json!(id))
            .expect("expected response with requested id");
        ctx.sent.remove(pos)
    }

    async fn complete_pending_command_task_for_test(
        ctx: &mut TestContext,
        mut pending: crate::conn::PendingCdpCommandDispatch,
    ) -> (Vec<Value>, Vec<CdpSchedulerEvent>) {
        loop {
            let completed = pending.wait().await;
            match ctx.conn.complete_pending_command_dispatch(completed).await {
                CdpCommandTaskStep::Pending(next) => pending = *next,
                CdpCommandTaskStep::Complete(outcome) => return outcome.into_parts(),
            }
        }
    }

    async fn with_loaded_document_async(ctx: &mut TestContext, html: &str) {
        let mut bc = crate::conn::BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1");
        ctx.conn.browser_context = Some(bc);
        ctx.install_navigation_fixture_for_session_owner(&format!("data:text/html,{html}"), None)
            .await;
        wait_until_renderer_document_load(ctx, None, "TID-1", LOADER_ID).await;
    }

    fn loaded_page_mut_for_test(ctx: &mut TestContext) -> &mut Page {
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .active_page_state_mut()
            .runtime_slot
            .loaded_page_mut()
            .expect("loaded page")
    }

    async fn append_live_css_target_without_refreshing_page_snapshot(
        ctx: &mut TestContext,
        style: &str,
    ) -> (u32, CompletedPageCommand) {
        with_loaded_document_async(ctx, "<html><body></body></html>").await;
        let cdp_node_id = RENDERER_BACKEND_NODE_ID_START - 1;
        let style_json = serde_json::to_string(style).expect("style should encode as JSON");
        let expression = format!(
            r#"(() => {{
                const target = document.createElement("div");
                target.id = "fresh-css";
                target.setAttribute("style", {style_json});
                document.body.appendChild(target);
                return "done";
            }})()"#
        );
        let completion = {
            let page = loaded_page_mut_for_test(ctx);
            let mutation = json!({
                "id": 910,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": expression,
                    "returnByValue": true
                }
            });
            let pending = page
                .start_runtime_protocol_message(mutation.to_string())
                .expect("runtime mutation should start");
            pending
                .wait()
                .await
                .expect("runtime mutation should complete")
        };
        (cdp_node_id, completion)
    }

    fn start_pending_document_navigation_for_active_session(ctx: &mut TestContext) {
        let browser_context = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should exist");
        browser_context.attach_active_session("SID-1".to_owned());
        browser_context
            .start_document_navigation_for_active_target("PENDING-LOADER".to_owned())
            .expect("active navigation should start");
        ctx.sent.clear();
    }

    async fn query_selector_node_id_async(ctx: &mut TestContext, selector: &str) -> u32 {
        query_selector_node_id_for_session_async(ctx, None, selector).await
    }

    async fn query_selector_node_id_for_session_async(
        ctx: &mut TestContext,
        session_id: Option<&str>,
        selector: &str,
    ) -> u32 {
        let mut get_document = json!({
            "id": 40,
            "method": "DOM.getDocument",
            "params": { "depth": 1 }
        });
        if let Some(session_id) = session_id {
            get_document["sessionId"] = json!(session_id);
        }
        ctx.process_async(get_document).await;
        let root_id = take_response_by_id(ctx, 40)["result"]["root"]["nodeId"]
            .as_u64()
            .expect("root node id") as u32;
        let mut query_selector = json!({
            "id": 41,
            "method": "DOM.querySelector",
            "params": {
                "nodeId": root_id,
                "selector": selector
            }
        });
        if let Some(session_id) = session_id {
            query_selector["sessionId"] = json!(session_id);
        }
        ctx.process_async(query_selector).await;
        take_response_by_id(ctx, 41)["result"]["nodeId"]
            .as_u64()
            .expect("selected node id") as u32
    }

    async fn inline_style_sheet_id_for_session_async(
        ctx: &mut TestContext,
        session_id: Option<&str>,
    ) -> String {
        ctx.sent.clear();
        let mut enable = json!({
            "id": 43,
            "method": "CSS.enable"
        });
        if let Some(session_id) = session_id {
            enable["sessionId"] = json!(session_id);
        }
        ctx.process_async(enable).await;
        let style_sheet_id = ctx
            .sent
            .iter()
            .filter(|message| message["method"] == json!("CSS.styleSheetAdded"))
            .find(|message| message["params"]["header"]["isInline"] == json!(true))
            .and_then(|message| message["params"]["header"]["styleSheetId"].as_str())
            .expect("inline stylesheet id from CSS.enable")
            .to_owned();
        ctx.sent.clear();
        style_sheet_id
    }

    async fn style_sheet_text_for_id_async(
        ctx: &mut TestContext,
        session_id: Option<&str>,
        style_sheet_id: &str,
    ) -> String {
        let mut command = json!({
            "id": 44,
            "method": "CSS.getStyleSheet",
            "params": { "styleSheetId": style_sheet_id }
        });
        if let Some(session_id) = session_id {
            command["sessionId"] = json!(session_id);
        }
        ctx.process_async(command).await;
        take_response_by_id(ctx, 44)["result"]["styleSheet"]["text"]
            .as_str()
            .expect("stylesheet text")
            .to_owned()
    }

    async fn attach_page_session_async(ctx: &mut TestContext, target_id: &str) -> String {
        ctx.process_async(json!({
            "id": 42,
            "method": "Target.attachToTarget",
            "params": { "targetId": target_id, "flatten": true }
        }))
        .await;
        take_response_by_id(ctx, 42)["result"]["sessionId"]
            .as_str()
            .expect("attached Page session id")
            .to_owned()
    }

    async fn resolve_node_object_id_async(ctx: &mut TestContext, node_id: u32) -> String {
        ctx.process_async(json!({
            "id": 39,
            "method": "Runtime.enable"
        }))
        .await;
        let _ = take_response_by_id(ctx, 39);
        ctx.sent.clear();
        ctx.process_async(json!({
            "id": 42,
            "method": "DOM.resolveNode",
            "params": { "nodeId": node_id }
        }))
        .await;
        take_response_by_id(ctx, 42)["result"]["object"]["objectId"]
            .as_str()
            .expect("objectId")
            .to_owned()
    }

    async fn backend_node_id_for_frontend_node_id_async(
        ctx: &mut TestContext,
        command_id: u64,
        frontend_node_id: u32,
    ) -> u32 {
        ctx.process_async(json!({
            "id": command_id,
            "method": "DOM.describeNode",
            "params": { "nodeId": frontend_node_id, "depth": 0 }
        }))
        .await;
        take_response_by_id(ctx, command_id)["result"]["node"]["backendNodeId"]
            .as_u64()
            .and_then(|id| u32::try_from(id).ok())
            .expect("DOM.describeNode should return backendNodeId")
    }

    fn computed_style_property<'a>(
        properties: &'a [serde_json::Value],
        name: &str,
    ) -> Option<&'a str> {
        properties.iter().find_map(|property| {
            if property["name"] == json!(name) {
                property["value"].as_str()
            } else {
                None
            }
        })
    }

    fn css_property<'a>(style: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
        style["cssProperties"]
            .as_array()
            .expect("cssProperties")
            .iter()
            .find(|property| property["name"] == json!(name))
            .unwrap_or_else(|| panic!("expected CSS property {name} in {style}"))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn css_loaded_page_methods_target_background_owner_without_promotion() {
        let mut ctx = TestContext::new();
        let background = PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "about:blank".to_owned(),
        );

        let mut bc = BrowserContext::new("BID-A".to_owned());
        bc.set_active_target_id("TID-active".to_owned());
        bc.attach_active_session("SID-active".to_owned());
        bc.insert_page_target_host(background);
        ctx.conn.browser_context = Some(bc);
        ctx.install_navigation_fixture_for_session_owner(
            "data:text/html,<html><head><style title='owner'>body { color: red; }</style></head><body><div id='target' style='display:flex;width:123px;color:blue'></div></body></html>",
            Some("SID-background"),
        )
        .await;
        ctx.sent.clear();

        let style_sheet_id =
            inline_style_sheet_id_for_session_async(&mut ctx, Some("SID-background")).await;

        ctx.process_async(json!({
            "id": 201,
            "sessionId": "SID-background",
            "method": "CSS.getStyleSheet",
            "params": { "styleSheetId": style_sheet_id }
        }))
        .await;
        let sheet = take_response_by_id(&mut ctx, 201);
        assert_eq!(sheet["sessionId"], "SID-background");
        assert_eq!(sheet["result"]["styleSheet"]["frameId"], "TID-background");
        assert_eq!(sheet["result"]["styleSheet"]["title"], "owner");

        ctx.process_async(json!({
            "id": 202,
            "sessionId": "SID-background",
            "method": "CSS.setStyleSheetText",
            "params": {
                "styleSheetId": style_sheet_id,
                "text": "body { color: green; }"
            }
        }))
        .await;
        ctx.expect_result(
            202,
            json!({
                "sourceMapURL": "",
                "styleSheetId": style_sheet_id,
            }),
            Some("SID-background"),
        );

        ctx.process_async(json!({
            "id": 203,
            "sessionId": "SID-background",
            "method": "CSS.getStyleSheet",
            "params": { "styleSheetId": style_sheet_id }
        }))
        .await;
        let updated_sheet = take_response_by_id(&mut ctx, 203);
        assert!(
            updated_sheet["result"]["styleSheet"]["text"]
                .as_str()
                .is_some_and(|text| text.contains("color: green"))
        );

        let target_node_id =
            query_selector_node_id_for_session_async(&mut ctx, Some("SID-background"), "#target")
                .await;
        ctx.process_async(json!({
            "id": 204,
            "sessionId": "SID-background",
            "method": "CSS.getComputedStyleForNode",
            "params": { "nodeId": target_node_id }
        }))
        .await;
        let computed = take_response_by_id(&mut ctx, 204);
        assert_eq!(computed["sessionId"], "SID-background");
        let computed_properties = computed["result"]["computedStyle"]
            .as_array()
            .expect("computedStyle array");
        assert_eq!(
            computed_style_property(computed_properties, "display"),
            Some("flex")
        );
        assert_eq!(
            computed_style_property(computed_properties, "width"),
            Some("123px")
        );

        ctx.process_async(json!({
            "id": 205,
            "sessionId": "SID-background",
            "method": "CSS.getInlineStylesForNode",
            "params": { "nodeId": target_node_id }
        }))
        .await;
        let inline = take_response_by_id(&mut ctx, 205);
        assert_eq!(inline["sessionId"], "SID-background");
        assert_eq!(
            css_property(&inline["result"]["inlineStyle"], "color")["value"],
            json!("blue")
        );

        ctx.process_async(json!({
            "id": 206,
            "sessionId": "SID-background",
            "method": "CSS.getMatchedStylesForNode",
            "params": { "nodeId": target_node_id }
        }))
        .await;
        let matched = take_response_by_id(&mut ctx, 206);
        assert_eq!(matched["sessionId"], "SID-background");
        assert_eq!(matched["result"]["matchedCSSRules"], json!([]));
        assert_eq!(
            css_property(&matched["result"]["inlineStyle"], "width")["value"],
            json!("123px")
        );

        assert_eq!(
            ctx.conn
                .browser_context
                .as_ref()
                .and_then(BrowserContext::active_target_id),
            Some("TID-active")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn css_loaded_page_methods_target_inactive_owner_without_activation() {
        let mut ctx = TestContext::new();
        let mut active = BrowserContext::new("BID-active".to_owned());
        active.set_active_target_id("TID-active".to_owned());
        active.attach_active_session("SID-active".to_owned());
        ctx.conn.browser_context = Some(active);

        let mut inactive = BrowserContext::new("BID-inactive".to_owned());
        inactive.set_active_target_id("TID-inactive".to_owned());
        inactive.set_target_url("about:blank".to_owned());
        inactive.attach_active_session("SID-inactive".to_owned());
        ctx.conn.inactive_browser_contexts.push(inactive);
        ctx.install_navigation_fixture_for_session_owner(
            "data:text/html,<html><head><style title='inactive'>main { color: red; }</style></head><body><main id='target' style='display:block;height:77px'></main></body></html>",
            Some("SID-inactive"),
        )
        .await;
        ctx.sent.clear();

        let style_sheet_id =
            inline_style_sheet_id_for_session_async(&mut ctx, Some("SID-inactive")).await;
        ctx.process_async(json!({
            "id": 211,
            "sessionId": "SID-inactive",
            "method": "CSS.getStyleSheet",
            "params": { "styleSheetId": style_sheet_id }
        }))
        .await;
        let sheet = take_response_by_id(&mut ctx, 211);
        assert_eq!(sheet["sessionId"], "SID-inactive");
        assert_eq!(sheet["result"]["styleSheet"]["frameId"], "TID-inactive");
        assert_eq!(sheet["result"]["styleSheet"]["title"], "inactive");

        let target_node_id =
            query_selector_node_id_for_session_async(&mut ctx, Some("SID-inactive"), "#target")
                .await;
        ctx.process_async(json!({
            "id": 212,
            "sessionId": "SID-inactive",
            "method": "CSS.getComputedStyleForNode",
            "params": { "nodeId": target_node_id }
        }))
        .await;
        let computed = take_response_by_id(&mut ctx, 212);
        assert_eq!(computed["sessionId"], "SID-inactive");
        let properties = computed["result"]["computedStyle"]
            .as_array()
            .expect("computedStyle array");
        assert_eq!(computed_style_property(properties, "height"), Some("77px"));
        assert_eq!(
            ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
            Some("BID-active")
        );
    }

    #[test]
    fn inline_style_parser_preserves_cssparser_declaration_boundaries() {
        let declarations = parse_inline_style_declarations(
            r#"color: red; content: "a;b"; background-image: url("data:image/svg+xml;a=b");"#,
        );

        assert_eq!(declarations.len(), 3);
        assert_eq!(declarations[1].name, "content");
        assert_eq!(declarations[1].value, r#""a;b""#);
        assert_eq!(declarations[2].name, "background-image");
        assert_eq!(declarations[2].value, r#"url("data:image/svg+xml;a=b")"#);
    }

    #[test]
    fn inline_style_parser_drops_invalid_blocks_and_reads_priority() {
        let declarations = parse_inline_style_declarations(
            "display: block !important; broken { color: red; } width: 10px;",
        );

        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0].name, "display");
        assert_eq!(declarations[0].value, "block");
        assert!(declarations[0].important);
    }

    #[tokio::test]
    async fn css_enable_and_disable_toggle_browser_context_state() {
        let mut ctx = TestContext::new();
        ctx.conn.browser_context = Some(crate::conn::BrowserContext::new_with_page_for_test(
            "BID-1", "TID-1",
        ));

        ctx.process_async(json!({"id": 101, "method": "CSS.enable"}))
            .await;
        assert!(
            ctx.conn
                .browser_context
                .as_ref()
                .unwrap()
                .active_page_state()
                .css_enabled
        );
        ctx.expect_result(101, json!({}), None);

        ctx.process_async(json!({"id": 102, "method": "CSS.disable"}))
            .await;
        assert!(
            !ctx.conn
                .browser_context
                .as_ref()
                .unwrap()
                .active_page_state()
                .css_enabled
        );
        ctx.expect_result(102, json!({}), None);
    }

    #[tokio::test]
    async fn css_enable_emits_live_style_sheet_added_events() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><head><style title='main'>body { color: red; }\n#x { display: flex; }</style><link rel='stylesheet' title='theme' href='data:text/css,body%20%7B%20background%3A%20rgb(1%2C%202%2C%203)%3B%20%7D'></head><body></body></html>",
        )
        .await;
        ctx.sent.clear();

        ctx.process_async(json!({"id": 119, "method": "CSS.enable"}))
            .await;

        let response_pos = ctx
            .sent
            .iter()
            .position(|message| message["id"] == json!(119))
            .expect("CSS.enable response");
        let first_event_pos = ctx
            .sent
            .iter()
            .position(|message| message["method"] == json!("CSS.styleSheetAdded"))
            .expect("CSS.styleSheetAdded event");
        assert!(
            response_pos < first_event_pos,
            "stylesheet event should follow CSS.enable response: {:?}",
            ctx.sent
        );

        let headers = ctx
            .sent
            .iter()
            .filter(|message| message["method"] == json!("CSS.styleSheetAdded"))
            .map(|message| message["params"]["header"].clone())
            .collect::<Vec<_>>();
        assert_eq!(
            headers.len(),
            2,
            "expected inline and linked stylesheet events"
        );

        let inline_header = headers
            .iter()
            .find(|header| header["isInline"] == json!(true))
            .expect("inline stylesheet header");
        assert_eq!(inline_header["frameId"], json!("TID-1"));
        assert_eq!(inline_header["title"], json!("main"));
        assert_eq!(inline_header["sourceURL"], json!(""));
        assert_eq!(inline_header["origin"], json!("regular"));
        let inline_style_sheet_id = inline_header["styleSheetId"]
            .as_str()
            .expect("styleSheetId")
            .to_owned();
        let inline_agent_id = inline_style_sheet_id
            .strip_prefix("stylesheet:")
            .and_then(|raw| raw.parse::<u32>().ok())
            .expect("renderer CSS agent stylesheet id");

        let link_header = headers
            .iter()
            .find(|header| header["isInline"] == json!(false))
            .expect("linked stylesheet header");
        assert_eq!(link_header["frameId"], json!("TID-1"));
        assert_eq!(link_header["title"], json!("theme"));
        assert_eq!(link_header["origin"], json!("regular"));
        assert!(
            link_header["sourceURL"]
                .as_str()
                .is_some_and(|source_url| source_url.starts_with("data:text/css")),
            "linked stylesheet should expose sourceURL: {link_header:?}"
        );
        let link_style_sheet_id = link_header["styleSheetId"]
            .as_str()
            .expect("linked styleSheetId")
            .to_owned();
        let link_agent_id = link_style_sheet_id
            .strip_prefix("stylesheet:")
            .and_then(|raw| raw.parse::<u32>().ok())
            .expect("linked renderer CSS agent stylesheet id");
        assert_ne!(
            inline_agent_id, link_agent_id,
            "CSS.enable should allocate distinct stylesheet ids: {headers:?}"
        );

        ctx.sent.clear();
        ctx.process_async(json!({
            "id": 120,
            "method": "CSS.getStyleSheet",
            "params": { "styleSheetId": inline_style_sheet_id }
        }))
        .await;
        let sheet = take_response_by_id(&mut ctx, 120)["result"]["styleSheet"].clone();
        assert_eq!(sheet["styleSheetId"], json!(inline_style_sheet_id));
        assert_eq!(sheet["title"], json!("main"));
        assert_eq!(sheet["isInline"], json!(true));
        assert!(
            sheet["text"]
                .as_str()
                .is_some_and(|text| text.contains("display: flex"))
        );

        ctx.sent.clear();
        ctx.process_async(json!({
            "id": 121,
            "method": "CSS.getStyleSheet",
            "params": { "styleSheetId": link_style_sheet_id }
        }))
        .await;
        let sheet = take_response_by_id(&mut ctx, 121)["result"]["styleSheet"].clone();
        assert_eq!(sheet["styleSheetId"], json!(link_style_sheet_id));
        assert_eq!(sheet["title"], json!("theme"));
        assert_eq!(sheet["isInline"], json!(false));
        assert!(
            sheet["text"]
                .as_str()
                .is_some_and(|text| text.contains("background: rgb(1, 2, 3)"))
        );
    }

    #[tokio::test]
    async fn css_enable_uses_stylesheet_inventory_diff_and_disable_resets_session() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><head><style title='main'>body { color: red; }</style></head><body></body></html>",
        )
        .await;
        ctx.sent.clear();

        ctx.process_async(json!({"id": 122, "method": "CSS.enable"}))
            .await;
        ctx.expect_result(122, json!({}), None);
        let first_added_count = ctx
            .sent
            .iter()
            .filter(|message| message["method"] == json!("CSS.styleSheetAdded"))
            .count();
        assert_eq!(first_added_count, 1);

        ctx.sent.clear();
        ctx.process_async(json!({"id": 123, "method": "CSS.enable"}))
            .await;
        ctx.expect_result(123, json!({}), None);
        assert!(
            ctx.sent
                .iter()
                .all(|message| message["method"] != json!("CSS.styleSheetAdded")
                    && message["method"] != json!("CSS.styleSheetRemoved")),
            "repeated CSS.enable should not replay unchanged stylesheet events: {:?}",
            ctx.sent
        );

        ctx.sent.clear();
        ctx.process_async(json!({"id": 124, "method": "CSS.disable"}))
            .await;
        ctx.expect_result(124, json!({}), None);

        ctx.sent.clear();
        ctx.process_async(json!({"id": 125, "method": "CSS.enable"}))
            .await;
        ctx.expect_result(125, json!({}), None);
        let reenabling_added_count = ctx
            .sent
            .iter()
            .filter(|message| message["method"] == json!("CSS.styleSheetAdded"))
            .count();
        assert_eq!(reenabling_added_count, 1);
    }

    #[tokio::test]
    async fn css_enable_and_disable_do_not_require_document_access_while_navigation_is_pending() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><body><div id='previous'></div></body></html>",
        )
        .await;
        start_pending_document_navigation_for_active_session(&mut ctx);

        ctx.process_async(json!({
            "id": 116,
            "method": "CSS.enable",
            "sessionId": "SID-1"
        }))
        .await;
        ctx.expect_result(116, json!({}), Some("SID-1"));
        assert!(
            ctx.conn
                .browser_context
                .as_ref()
                .unwrap()
                .active_page_state()
                .css_enabled
        );

        ctx.process_async(json!({
            "id": 117,
            "method": "CSS.disable",
            "sessionId": "SID-1"
        }))
        .await;
        ctx.expect_result(117, json!({}), Some("SID-1"));
        assert!(
            !ctx.conn
                .browser_context
                .as_ref()
                .unwrap()
                .active_page_state()
                .css_enabled
        );
    }

    #[tokio::test]
    async fn css_document_commands_reject_while_main_document_navigation_is_pending() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><body><div id='previous'></div></body></html>",
        )
        .await;
        start_pending_document_navigation_for_active_session(&mut ctx);

        ctx.process_async(json!({
            "id": 118,
            "method": "CSS.getComputedStyleForNode",
            "sessionId": "SID-1",
            "params": { "nodeId": 1 }
        }))
        .await;

        ctx.expect_error(118, -32000, "Navigation is changing the document");
    }

    #[tokio::test]
    async fn get_style_sheet_returns_inline_style_payload() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><head><style title='main'>body { color: red; }\n#x { display: flex; }</style></head><body></body></html>",
        )
        .await;
        let style_sheet_id = inline_style_sheet_id_for_session_async(&mut ctx, None).await;

        ctx.process_async(json!({
            "id": 104,
            "method": "CSS.getStyleSheet",
            "params": { "styleSheetId": style_sheet_id }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, 104);
        let sheet = &response["result"]["styleSheet"];
        assert_eq!(sheet["styleSheetId"], json!(style_sheet_id));
        assert_eq!(sheet["isInline"], json!(true));
        assert_eq!(sheet["origin"], json!("regular"));
        assert_eq!(sheet["title"], json!("main"));
        assert_eq!(sheet["disabled"], json!(false));
        assert_eq!(sheet["frameId"], json!("TID-1"));
        assert!(
            sheet["text"]
                .as_str()
                .expect("style text")
                .contains("display: flex")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_style_sheet_can_complete_through_pending_command_dispatch() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><head><style title='pending'>body { color: red; }</style></head><body></body></html>",
        )
        .await;
        let style_sheet_id = inline_style_sheet_id_for_session_async(&mut ctx, None).await;

        let raw = json!({
            "id": 115,
            "method": "CSS.getStyleSheet",
            "params": { "styleSheetId": style_sheet_id }
        })
        .to_string();
        let pending = ctx
            .conn
            .try_start_pending_command_dispatch(&raw)
            .expect("CSS.getStyleSheet should start as a pending command");
        let (messages, scheduler_events) =
            complete_pending_command_task_for_test(&mut ctx, pending).await;
        assert!(
            scheduler_events.is_empty(),
            "stylesheet payload read should not enqueue scheduler events: {scheduler_events:?}"
        );
        let sheet = &messages
            .iter()
            .find(|message| message["id"] == json!(115))
            .expect("CSS.getStyleSheet response")["result"]["styleSheet"];
        assert_eq!(sheet["styleSheetId"], json!(style_sheet_id));
        assert_eq!(sheet["title"], json!("pending"));
        assert!(
            sheet["text"]
                .as_str()
                .is_some_and(|text| text.contains("color: red"))
        );
    }

    #[tokio::test]
    async fn get_style_sheet_rejects_unknown_id() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
        ctx.process_async(json!({
            "id": 105,
            "method": "CSS.getStyleSheet",
            "params": { "styleSheetId": "stylesheet:9999" }
        }))
        .await;
        ctx.expect_error(105, -32000, "Could not find stylesheet with given id");
    }

    #[tokio::test]
    async fn set_style_sheet_text_updates_inline_style_sheet() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><head><style>body { color: red; }</style></head><body></body></html>",
        )
        .await;
        let style_sheet_id = inline_style_sheet_id_for_session_async(&mut ctx, None).await;

        ctx.process_async(json!({
            "id": 106,
            "method": "CSS.setStyleSheetText",
            "params": {
                "styleSheetId": style_sheet_id,
                "text": "body { color: blue; }\n#x { width: 10px; }"
            }
        }))
        .await;
        ctx.expect_result(
            106,
            json!({
                "sourceMapURL": "",
                "styleSheetId": style_sheet_id,
            }),
            None,
        );

        ctx.process_async(json!({
            "id": 107,
            "method": "CSS.getStyleSheet",
            "params": { "styleSheetId": style_sheet_id }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, 107);
        let text = response["result"]["styleSheet"]["text"]
            .as_str()
            .expect("style text");
        assert!(text.contains("color: blue"));
        assert!(text.contains("width: 10px"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn set_style_sheet_text_can_complete_through_pending_command_dispatch() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><head><style>body { color: red; }</style></head><body></body></html>",
        )
        .await;
        let style_sheet_id = inline_style_sheet_id_for_session_async(&mut ctx, None).await;

        let raw = json!({
            "id": 113,
            "method": "CSS.setStyleSheetText",
            "params": {
                "styleSheetId": style_sheet_id,
                "text": "body { color: purple; }"
            }
        })
        .to_string();
        let pending = ctx
            .conn
            .try_start_pending_command_dispatch(&raw)
            .expect("CSS.setStyleSheetText should start as a pending command");
        let (messages, scheduler_events) =
            complete_pending_command_task_for_test(&mut ctx, pending).await;
        assert!(
            scheduler_events.is_empty(),
            "stylesheet text update should not enqueue scheduler events: {scheduler_events:?}"
        );
        assert_eq!(
            messages
                .iter()
                .find(|message| message["id"] == json!(113))
                .expect("CSS.setStyleSheetText response")["result"],
            json!({
                "sourceMapURL": "",
                "styleSheetId": style_sheet_id,
            })
        );

        ctx.process_async(json!({
            "id": 114,
            "method": "CSS.getStyleSheet",
            "params": { "styleSheetId": style_sheet_id }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, 114);
        assert!(
            response["result"]["styleSheet"]["text"]
                .as_str()
                .is_some_and(|text| text.contains("color: purple"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pending_css_command_keeps_background_owner_route_across_completion() {
        let mut ctx = TestContext::new();
        let active_page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<html><head><style>body { color: red; }</style></head><body></body></html>",
            )
            .await
            .expect("active page should load");
        let background_page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<html><head><style>body { color: green; }</style></head><body></body></html>",
            )
            .await
            .expect("background page should load");

        let mut browser_context = BrowserContext::new("BID-css-owner-route".to_owned());
        browser_context.set_active_target_id("TID-css-active".to_owned());
        browser_context
            .active_page_state_mut()
            .runtime_slot
            .set_loaded_page_for_test(active_page);
        browser_context.stage_background_target(
            "TID-css-background".to_owned(),
            None,
            "data:text/html,<title>background css</title>".to_owned(),
            None,
            None,
        );
        browser_context
            .background_target_mut("TID-css-background")
            .expect("background target")
            .replace_loaded_page(Some(background_page));
        ctx.conn.browser_context = Some(browser_context);

        let background_session = attach_page_session_async(&mut ctx, "TID-css-background").await;
        let active_style_sheet_id = inline_style_sheet_id_for_session_async(&mut ctx, None).await;
        let style_sheet_id =
            inline_style_sheet_id_for_session_async(&mut ctx, Some(&background_session)).await;
        let raw = json!({
            "id": 214,
            "method": "CSS.setStyleSheetText",
            "params": {
                "styleSheetId": style_sheet_id,
                "text": "body { color: purple; }"
            },
            "sessionId": background_session
        })
        .to_string();
        let pending = ctx
            .conn
            .try_start_pending_command_dispatch(&raw)
            .expect("background CSS.setStyleSheetText should start pending");
        let (messages, scheduler_events) =
            complete_pending_command_task_for_test(&mut ctx, pending).await;

        assert!(
            scheduler_events.is_empty(),
            "CSS completion should not enqueue scheduler events: {scheduler_events:?}"
        );
        assert_eq!(
            messages
                .iter()
                .find(|message| message["id"] == json!(214))
                .expect("CSS.setStyleSheetText response")["result"],
            json!({
                "sourceMapURL": "",
                "styleSheetId": style_sheet_id,
            })
        );
        let active_text =
            style_sheet_text_for_id_async(&mut ctx, None, &active_style_sheet_id).await;
        assert!(
            active_text.contains("color: red"),
            "ambient active page must not receive background CSS completion: {active_text}"
        );
        let background_text =
            style_sheet_text_for_id_async(&mut ctx, Some(&background_session), &style_sheet_id)
                .await;
        assert!(
            background_text.contains("color: purple"),
            "background CSS completion should use captured owner route: {background_text}"
        );
    }

    #[tokio::test]
    async fn get_computed_style_for_node_requires_loaded_page() {
        let mut ctx = TestContext::new();
        ctx.process_async(json!({
            "id": 2,
            "method": "CSS.getComputedStyleForNode",
            "params": { "nodeId": 1 }
        }))
        .await;
        ctx.expect_error(2, -32000, "NoDocumentLoaded");
    }

    #[tokio::test]
    async fn get_computed_style_for_node_returns_inline_and_default_properties() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><head><style>#target{animation-timeline:auto;animation-range-start:entry 10%;animation-range-end:exit 20%;background-position-x:25%;column-span:all;column-width:12px;font-variant-alternates:historical-forms;font-variant-emoji:emoji;font-variant-position:super;grid-auto-columns:17px;object-fit:cover;overflow-wrap:anywhere;pointer-events:none;white-space-collapse:preserve;zoom:125%}</style></head><body><div id='target' style='display:flex;width:123px;--token:present'></div></body></html>",
        )
        .await;
        let node_id = query_selector_node_id_async(&mut ctx, "#target").await;

        ctx.process_async(json!({
            "id": 3,
            "method": "CSS.getComputedStyleForNode",
            "params": { "nodeId": node_id }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, 3);
        let properties = response["result"]["computedStyle"]
            .as_array()
            .expect("computedStyle array");
        assert!(
            properties.len() >= 267,
            "computed style should expose the Stylo-derived longhand set"
        );
        let unique_names = properties
            .iter()
            .filter_map(|property| property["name"].as_str())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique_names.len(), properties.len());
        assert_eq!(computed_style_property(properties, "display"), Some("flex"));
        assert_eq!(computed_style_property(properties, "width"), Some("123px"));
        assert!(computed_style_property(properties, "height").is_some());
        for (name, expected) in [
            ("animation-timeline", "auto"),
            ("animation-range-start", "entry 10%"),
            ("animation-range-end", "exit 20%"),
            ("background-position-x", "25%"),
            ("column-span", "all"),
            ("column-width", "12px"),
            ("font-variant-alternates", "historical-forms"),
            ("font-variant-emoji", "emoji"),
            ("font-variant-position", "super"),
            ("grid-auto-columns", "17px"),
            ("object-fit", "cover"),
            ("overflow-wrap", "anywhere"),
            ("pointer-events", "none"),
            ("white-space-collapse", "preserve"),
            ("zoom", "1.25"),
            ("--token", "present"),
        ] {
            assert_eq!(computed_style_property(properties, name), Some(expected));
        }
        for shorthand in ["margin", "mask", "padding-block"] {
            assert_eq!(computed_style_property(properties, shorthand), None);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_computed_style_for_node_can_complete_through_pending_command_dispatch() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><body><div id='target' style='display:grid;width:321px'></div></body></html>",
        )
        .await;
        let node_id = query_selector_node_id_async(&mut ctx, "#target").await;

        let raw = json!({
            "id": 115,
            "method": "CSS.getComputedStyleForNode",
            "params": { "nodeId": node_id }
        })
        .to_string();
        let pending = ctx
            .conn
            .try_start_pending_command_dispatch(&raw)
            .expect("CSS.getComputedStyleForNode should start as a pending command");
        let (messages, scheduler_events) =
            complete_pending_command_task_for_test(&mut ctx, pending).await;
        assert!(
            scheduler_events.is_empty(),
            "computed style lookup should not enqueue scheduler events: {scheduler_events:?}"
        );
        let response = messages
            .iter()
            .find(|message| message["id"] == json!(115))
            .expect("CSS.getComputedStyleForNode response");
        let properties = response["result"]["computedStyle"]
            .as_array()
            .expect("computedStyle array");
        assert_eq!(computed_style_property(properties, "display"), Some("grid"));
        assert_eq!(computed_style_property(properties, "width"), Some("321px"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_computed_style_for_low_node_id_misses_without_frontend_binding() {
        let mut ctx = TestContext::new();
        let (node_id, mutation_completion) =
            append_live_css_target_without_refreshing_page_snapshot(
                &mut ctx,
                "display:grid;width:222px",
            )
            .await;

        let raw = json!({
            "id": 118,
            "method": "CSS.getComputedStyleForNode",
            "params": { "nodeId": node_id }
        })
        .to_string();
        let pending = ctx
            .conn
            .try_start_pending_command_dispatch(&raw)
            .expect("CSS.getComputedStyleForNode should query the renderer for stale low nodeId");
        let (messages, scheduler_events) =
            complete_pending_command_task_for_test(&mut ctx, pending).await;
        assert!(
            scheduler_events.is_empty(),
            "computed style lookup should not enqueue scheduler events: {scheduler_events:?}"
        );
        let response = messages
            .iter()
            .find(|message| message["id"] == json!(118))
            .expect("CSS.getComputedStyleForNode response");
        assert_eq!(
            response["error"],
            json!({
                "code": -32000,
                "message": "Could not find node with given id"
            })
        );

        let page = loaded_page_mut_for_test(&mut ctx);
        let _ = page
            .finish_runtime_protocol_message(mutation_completion)
            .expect("runtime mutation completion should finish");
    }

    #[tokio::test]
    async fn css_computed_style_uses_fresh_initial_document_without_adapter() {
        let mut ctx = TestContext::new();
        ctx.process_async(json!({
            "id": 116,
            "method": "Target.createTarget",
            "params": { "url": "about:blank" }
        }))
        .await;
        ctx.expect_event("Target.targetCreated", None);
        let create_response = take_response_by_id(&mut ctx, 116);
        assert!(
            create_response["result"]["targetId"].as_str().is_some(),
            "Target.createTarget should return target id: {create_response}"
        );
        assert!(
            ctx.conn
                .browser_context
                .as_ref()
                .expect("browser context")
                .has_loaded_page(),
            "Target.createTarget should install initial about:blank page before CSS observation"
        );

        let body_node_id = query_selector_node_id_async(&mut ctx, "body").await;
        ctx.process_async(json!({
            "id": 117,
            "method": "CSS.getComputedStyleForNode",
            "params": { "nodeId": body_node_id }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, 117);
        let properties = response["result"]["computedStyle"]
            .as_array()
            .expect("computedStyle array");
        assert!(computed_style_property(properties, "display").is_some());
    }

    #[tokio::test]
    async fn get_inline_styles_for_node_returns_style_attribute_properties() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><body><div id='target' style=\"display:flex; color: red !important; background-image: url('a;b:c')\"></div></body></html>",
        )
        .await;
        let node_id = query_selector_node_id_async(&mut ctx, "#target").await;

        ctx.process_async(json!({
            "id": 111,
            "method": "CSS.getInlineStylesForNode",
            "params": { "nodeId": node_id }
        }))
        .await;

        let response = take_response_by_id(&mut ctx, 111);
        let inline_style = &response["result"]["inlineStyle"];
        assert_eq!(
            inline_style["cssText"],
            json!("display:flex; color: red !important; background-image: url('a;b:c')")
        );
        assert_eq!(
            css_property(inline_style, "display")["value"],
            json!("flex")
        );
        assert_eq!(css_property(inline_style, "color")["value"], json!("red"));
        assert_eq!(
            css_property(inline_style, "color")["important"],
            json!(true)
        );
        assert_eq!(
            css_property(inline_style, "background-image")["value"],
            json!("url('a;b:c')")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_inline_styles_for_low_backend_node_id_no_longer_decodes_live_node() {
        let mut ctx = TestContext::new();
        let (backend_node_id, mutation_completion) =
            append_live_css_target_without_refreshing_page_snapshot(
                &mut ctx,
                "display:flex; color: green",
            )
            .await;

        let raw = json!({
            "id": 119,
            "method": "CSS.getInlineStylesForNode",
            "params": { "backendNodeId": backend_node_id }
        })
        .to_string();
        let pending = ctx
            .conn
            .try_start_pending_command_dispatch(&raw)
            .expect("CSS.getInlineStylesForNode should still complete through the renderer");
        let (messages, scheduler_events) =
            complete_pending_command_task_for_test(&mut ctx, pending).await;
        assert!(
            scheduler_events.is_empty(),
            "inline style lookup should not enqueue scheduler events: {scheduler_events:?}"
        );
        let response = messages
            .iter()
            .find(|message| message["id"] == json!(119))
            .expect("CSS.getInlineStylesForNode response");
        assert_eq!(
            response["error"],
            json!({
                "code": -32000,
                "message": "Could not find node with given id"
            })
        );

        let page = loaded_page_mut_for_test(&mut ctx);
        let _ = page
            .finish_runtime_protocol_message(mutation_completion)
            .expect("runtime mutation completion should finish");
    }

    #[tokio::test]
    async fn get_matched_styles_for_node_returns_inline_style_and_empty_rule_lists() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><body><main id='target' style='width: 12px; --token: yes'></main></body></html>",
        )
        .await;
        let node_id = query_selector_node_id_async(&mut ctx, "#target").await;

        ctx.process_async(json!({
            "id": 112,
            "method": "CSS.getMatchedStylesForNode",
            "params": { "nodeId": node_id }
        }))
        .await;

        let response = take_response_by_id(&mut ctx, 112);
        let result = &response["result"];
        assert_eq!(
            css_property(&result["inlineStyle"], "width")["value"],
            json!("12px")
        );
        assert_eq!(
            css_property(&result["inlineStyle"], "--token")["value"],
            json!("yes")
        );
        assert_eq!(result["matchedCSSRules"], json!([]));
        assert_eq!(result["pseudoElements"], json!([]));
        assert_eq!(result["inherited"], json!([]));
    }

    #[tokio::test]
    async fn get_inline_and_matched_styles_complete_through_pending_renderer_command() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><body><main id='target' style='width: 12px; --token: yes'></main></body></html>",
        )
        .await;
        let node_id = query_selector_node_id_async(&mut ctx, "#target").await;
        let backend_node_id =
            backend_node_id_for_frontend_node_id_async(&mut ctx, 210, node_id).await;

        let inline_raw = json!({
            "id": 211,
            "method": "CSS.getInlineStylesForNode",
            "params": { "nodeId": node_id }
        })
        .to_string();
        let inline_pending = match ctx.conn.start_command_dispatch(&inline_raw) {
            CdpCommandTaskStep::Pending(pending) => pending,
            CdpCommandTaskStep::Complete(_) => {
                panic!("CSS.getInlineStylesForNode should start as a pending command")
            }
        };
        let (inline_messages, inline_events) =
            complete_pending_command_task_for_test(&mut ctx, *inline_pending).await;
        assert!(
            inline_events.is_empty(),
            "inline style pending lookup should not enqueue scheduler events: {inline_events:?}"
        );
        let inline_response = inline_messages
            .iter()
            .find(|message| message["id"] == json!(211))
            .expect("CSS.getInlineStylesForNode response");
        assert_eq!(
            css_property(&inline_response["result"]["inlineStyle"], "width")["value"],
            json!("12px")
        );

        let matched_raw = json!({
            "id": 212,
            "method": "CSS.getMatchedStylesForNode",
            "params": { "backendNodeId": backend_node_id }
        })
        .to_string();
        let matched_pending = match ctx.conn.start_command_dispatch(&matched_raw) {
            CdpCommandTaskStep::Pending(pending) => pending,
            CdpCommandTaskStep::Complete(_) => {
                panic!("CSS.getMatchedStylesForNode should start as a pending command")
            }
        };
        let (matched_messages, matched_events) =
            complete_pending_command_task_for_test(&mut ctx, *matched_pending).await;
        assert!(
            matched_events.is_empty(),
            "matched style pending lookup should not enqueue scheduler events: {matched_events:?}"
        );
        let matched_response = matched_messages
            .iter()
            .find(|message| message["id"] == json!(212))
            .expect("CSS.getMatchedStylesForNode response");
        let matched_result = &matched_response["result"];
        assert_eq!(
            css_property(&matched_result["inlineStyle"], "--token")["value"],
            json!("yes")
        );
        assert_eq!(matched_result["matchedCSSRules"], json!([]));
        assert_eq!(matched_result["cssKeyframesRules"], json!([]));
    }

    #[tokio::test]
    async fn get_computed_style_for_node_supports_object_and_backend_node_ids() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><body><span id='target' style='display:inline-block;color:red'></span></body></html>",
        )
        .await;
        let node_id = query_selector_node_id_async(&mut ctx, "#target").await;
        let backend_node_id =
            backend_node_id_for_frontend_node_id_async(&mut ctx, 44, node_id).await;
        let object_id = resolve_node_object_id_async(&mut ctx, node_id).await;

        ctx.process_async(json!({
            "id": 4,
            "method": "CSS.getComputedStyleForNode",
            "params": { "objectId": object_id }
        }))
        .await;
        let object_response = take_response_by_id(&mut ctx, 4);
        let object_properties = object_response["result"]["computedStyle"]
            .as_array()
            .expect("computedStyle array");
        assert_eq!(
            computed_style_property(object_properties, "display"),
            Some("inline-block")
        );
        assert!(matches!(
            computed_style_property(object_properties, "color"),
            Some("red") | Some("rgb(255, 0, 0)")
        ));

        ctx.process_async(json!({
            "id": 5,
            "method": "CSS.getComputedStyleForNode",
            "params": { "backendNodeId": backend_node_id }
        }))
        .await;
        let backend_response = take_response_by_id(&mut ctx, 5);
        let backend_properties = backend_response["result"]["computedStyle"]
            .as_array()
            .expect("computedStyle array");
        assert_eq!(
            computed_style_property(backend_properties, "display"),
            Some("inline-block")
        );
    }

    #[tokio::test]
    async fn css_style_queries_support_renderer_backend_node_id() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><body><span id='target' style='display:inline-block;width:44px;color:red'></span></body></html>",
        )
        .await;
        let node_id = query_selector_node_id_async(&mut ctx, "#target").await;

        ctx.process_async(json!({
            "id": 6,
            "method": "DOM.describeNode",
            "params": { "nodeId": node_id, "depth": 0 }
        }))
        .await;
        let described = take_response_by_id(&mut ctx, 6);
        let backend_node_id = described["result"]["node"]["backendNodeId"]
            .as_u64()
            .and_then(|id| u32::try_from(id).ok())
            .expect("DOM.describeNode should return backendNodeId");
        assert!(
            is_renderer_backend_node_id(backend_node_id),
            "DOM.describeNode should assign renderer backend ids: {described}"
        );

        ctx.process_async(json!({
            "id": 7,
            "method": "CSS.getComputedStyleForNode",
            "params": { "backendNodeId": backend_node_id }
        }))
        .await;
        let computed_response = take_response_by_id(&mut ctx, 7);
        let properties = computed_response["result"]["computedStyle"]
            .as_array()
            .expect("computedStyle array");
        assert_eq!(
            computed_style_property(properties, "display"),
            Some("inline-block")
        );
        assert_eq!(computed_style_property(properties, "width"), Some("44px"));

        ctx.process_async(json!({
            "id": 8,
            "method": "CSS.getInlineStylesForNode",
            "params": { "backendNodeId": backend_node_id }
        }))
        .await;
        let inline_response = take_response_by_id(&mut ctx, 8);
        let inline_style = &inline_response["result"]["inlineStyle"];
        assert_eq!(
            css_property(inline_style, "display")["value"],
            json!("inline-block")
        );
        assert_eq!(css_property(inline_style, "width")["value"], json!("44px"));
    }

    #[tokio::test]
    async fn css_node_id_does_not_accept_renderer_backend_node_id() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><body><span id='target' style='display:inline-block;width:44px;color:red'></span></body></html>",
        )
        .await;
        let node_id = query_selector_node_id_async(&mut ctx, "#target").await;
        let backend_node_id =
            backend_node_id_for_frontend_node_id_async(&mut ctx, 216, node_id).await;
        assert!(
            is_renderer_backend_node_id(backend_node_id),
            "test requires a renderer-owned backend id distinct from frontend node ids"
        );

        ctx.process_async(json!({
            "id": 217,
            "method": "CSS.getComputedStyleForNode",
            "params": { "nodeId": backend_node_id }
        }))
        .await;
        ctx.expect_error(217, -32000, "Could not find node with given id");

        ctx.process_async(json!({
            "id": 218,
            "method": "CSS.getInlineStylesForNode",
            "params": { "nodeId": backend_node_id }
        }))
        .await;
        ctx.expect_error(218, -32000, "Could not find node with given id");

        ctx.process_async(json!({
            "id": 219,
            "method": "CSS.getMatchedStylesForNode",
            "params": { "nodeId": backend_node_id }
        }))
        .await;
        ctx.expect_error(219, -32000, "Could not find node with given id");
    }

    #[tokio::test]
    async fn async_dispatch_set_style_sheet_text_updates_inline_style_sheet() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><head><style>body { color: red; }</style></head><body></body></html>",
        )
        .await;
        let style_sheet_id = inline_style_sheet_id_for_session_async(&mut ctx, None).await;

        ctx.process_async(json!({
            "id": 108,
            "method": "CSS.setStyleSheetText",
            "params": {
                "styleSheetId": style_sheet_id,
                "text": "body { color: green; }\n#x { height: 20px; }"
            }
        }))
        .await;
        ctx.expect_result(
            108,
            json!({
                "sourceMapURL": "",
                "styleSheetId": style_sheet_id,
            }),
            None,
        );

        ctx.process_async(json!({
            "id": 109,
            "method": "CSS.getStyleSheet",
            "params": { "styleSheetId": style_sheet_id }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, 109);
        let text = response["result"]["styleSheet"]["text"]
            .as_str()
            .expect("style text");
        assert!(text.contains("color: green"));
        assert!(text.contains("height: 20px"));
    }

    #[tokio::test]
    async fn async_dispatch_get_computed_style_for_node_supports_object_id() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><body><span id='target' style='display:inline-block;color:red'></span></body></html>",
        )
        .await;
        let node_id = query_selector_node_id_async(&mut ctx, "#target").await;
        let object_id = resolve_node_object_id_async(&mut ctx, node_id).await;

        ctx.process_async(json!({
            "id": 110,
            "method": "CSS.getComputedStyleForNode",
            "params": { "objectId": object_id }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, 110);
        let properties = response["result"]["computedStyle"]
            .as_array()
            .expect("computedStyle array");
        assert_eq!(
            computed_style_property(properties, "display"),
            Some("inline-block")
        );
        assert!(matches!(
            computed_style_property(properties, "color"),
            Some("red") | Some("rgb(255, 0, 0)")
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_computed_style_for_object_id_completes_with_single_renderer_command() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<html><body><span id='target' style='display:inline-block;color:green'></span></body></html>",
        )
        .await;
        let node_id = query_selector_node_id_async(&mut ctx, "#target").await;
        let object_id = resolve_node_object_id_async(&mut ctx, node_id).await;

        let raw = json!({
            "id": 111,
            "method": "CSS.getComputedStyleForNode",
            "params": { "objectId": object_id }
        })
        .to_string();
        let pending = ctx
            .conn
            .try_start_pending_command_dispatch(&raw)
            .expect("CSS.getComputedStyleForNode objectId should start as a pending command");
        let completed = pending.wait().await;
        let step = ctx.conn.complete_pending_command_dispatch(completed).await;
        let CdpCommandTaskStep::Complete(outcome) = step else {
            panic!("CSS objectId computed style should complete after one renderer command");
        };
        let (messages, scheduler_events) = outcome.into_parts();
        assert!(
            scheduler_events.is_empty(),
            "computed style objectId lookup should not enqueue scheduler events: {scheduler_events:?}"
        );
        let response = messages
            .iter()
            .find(|message| message["id"] == json!(111))
            .expect("CSS.getComputedStyleForNode response");
        let properties = response["result"]["computedStyle"]
            .as_array()
            .expect("computedStyle array");
        assert_eq!(
            computed_style_property(properties, "display"),
            Some("inline-block")
        );
        assert!(matches!(
            computed_style_property(properties, "color"),
            Some("green") | Some("rgb(0, 128, 0)")
        ));
    }
}
