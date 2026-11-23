use moli_core::page::RendererDomNodeReference;
use moli_core::{
    RendererDevToolsPageCommand, RendererNativeOperation, RendererOverlayHighlight,
    RendererOverlayNode, RendererPageCommand,
};
use serde::Deserialize;

use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};
use crate::domains::actions::OverlayAction;
use crate::domains::command_output::CommandOutputPlan;
use crate::domains::native::{self, NativeCommandStep, NodeReferenceParams};

#[derive(Deserialize)]
struct Color {
    r: i32,
    g: i32,
    b: i32,
    a: Option<f64>,
}

fn rgba(color: Option<Color>) -> [u8; 4] {
    color.map_or([0; 4], |color| {
        [
            color.r.clamp(0, 255) as u8,
            color.g.clamp(0, 255) as u8,
            color.b.clamp(0, 255) as u8,
            (color.a.unwrap_or(1.0).clamp(0.0, 1.0) * 255.0).round() as u8,
        ]
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RectParams {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: Option<Color>,
    outline_color: Option<Color>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeParams {
    #[serde(flatten)]
    reference: NodeReferenceParams,
    highlight_config: NodeHighlightConfig,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeHighlightConfig {
    content_color: Option<Color>,
    padding_color: Option<Color>,
    border_color: Option<Color>,
    margin_color: Option<Color>,
}

pub(crate) fn try_start(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> Option<NativeCommandStep> {
    let action = match cmd.method {
        "DOM.highlightNode" => OverlayAction::HighlightNode,
        "DOM.highlightRect" => OverlayAction::HighlightRect,
        "DOM.hideHighlight" => OverlayAction::HideHighlight,
        method if method.starts_with("Overlay.") => match cmd.parse_action::<OverlayAction>() {
            Some(action) => action,
            None => {
                return Some(NativeCommandStep::Complete(CommandOutputPlan::error(
                    -32601,
                    "UnknownMethod",
                )));
            }
        },
        _ => return None,
    };
    Some(start(conn, cmd, action))
}

fn start(conn: &mut CdpConnection, cmd: &Cmd<'_>, action: OverlayAction) -> NativeCommandStep {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let Some(state) = conn.target_page_session_state_for_owner(&owner) else {
        return NativeCommandStep::Complete(CommandOutputPlan::error(
            -32601,
            format!("'{}' wasn't found", cmd.method),
        ));
    };
    let enabled = state.overlay_enabled;
    if action == OverlayAction::Enable {
        if !super::dom::dom_agent_enabled_for_owner(conn, &owner) {
            return NativeCommandStep::Complete(CommandOutputPlan::error(
                -32000,
                "DOM should be enabled first",
            ));
        }
        conn.with_target_devtools_session_state_for_session_mut(cmd.session_id, |state| {
            state.page_session_state.overlay_enabled = true;
        });
        return NativeCommandStep::Complete(CommandOutputPlan::success());
    }

    let inspector_session_id = conn.target_renderer_runtime_inspector_session_id_for_owner(&owner);
    let highlight = match action {
        OverlayAction::HighlightNode => node_highlight(cmd, inspector_session_id.clone()).map(Some),
        OverlayAction::HighlightRect => rect_highlight(cmd).map(Some),
        OverlayAction::Disable => {
            conn.with_target_devtools_session_state_for_session_mut(cmd.session_id, |state| {
                state.page_session_state.overlay_enabled = false;
            });
            Ok(None)
        }
        OverlayAction::HideHighlight => Ok(None),
        OverlayAction::Enable => unreachable!(),
    };
    let highlight = match highlight {
        Ok(highlight) => highlight,
        Err(plan) => return NativeCommandStep::Complete(plan),
    };
    if highlight.is_some() && !enabled {
        return NativeCommandStep::Complete(CommandOutputPlan::error(
            -32600,
            "Overlay must be enabled before a tool can be shown",
        ));
    }
    if highlight.is_none()
        && conn
            .target_page_residence_identity_for_owner(&owner)
            .is_none()
    {
        return NativeCommandStep::Complete(CommandOutputPlan::success());
    }
    native::start_operation(
        conn,
        cmd,
        RendererNativeOperation::new(
            RendererPageCommand::DevTools(RendererDevToolsPageCommand::SetOverlayHighlight {
                inspector_session_id,
                highlight: Ok(highlight),
            }),
            native::unit_response,
        ),
    )
}

fn node_highlight(
    cmd: &Cmd<'_>,
    inspector_session_id: Option<String>,
) -> Result<RendererOverlayHighlight, CommandOutputPlan> {
    let params = params::<NodeParams>(cmd)?;
    let NodeReferenceParams {
        node_id,
        backend_node_id,
        object_id,
    } = params.reference;
    let target = match (node_id, backend_node_id, object_id) {
        (Some(frontend_node_id), None, None) => {
            RendererOverlayNode::Node(RendererDomNodeReference::FrontendNodeId {
                inspector_session_id,
                frontend_node_id,
            })
        }
        (None, Some(id), None) => {
            RendererOverlayNode::Node(RendererDomNodeReference::BackendNodeId(id))
        }
        (None, None, Some(id)) => RendererOverlayNode::RuntimeObject(id),
        _ => {
            return Err(CommandOutputPlan::error(
                -32602,
                "Exactly one node reference is required",
            ));
        }
    };
    let config = params.highlight_config;
    Ok(RendererOverlayHighlight::Node {
        target,
        box_colors: [
            rgba(config.content_color),
            rgba(config.padding_color),
            rgba(config.border_color),
            rgba(config.margin_color),
        ],
    })
}

fn rect_highlight(cmd: &Cmd<'_>) -> Result<RendererOverlayHighlight, CommandOutputPlan> {
    let params = params::<RectParams>(cmd)?;
    if params.width < 0 || params.height < 0 {
        return Err(CommandOutputPlan::error(-32602, "Invalid rectangle size"));
    }
    Ok(RendererOverlayHighlight::Rect {
        rect: [params.x, params.y, params.width, params.height],
        fill: rgba(params.color),
        outline: rgba(params.outline_color),
    })
}

fn params<T: serde::de::DeserializeOwned>(cmd: &Cmd<'_>) -> Result<T, CommandOutputPlan> {
    cmd.get_params::<T>()
        .ok()
        .flatten()
        .ok_or_else(|| CommandOutputPlan::error(-32602, "Invalid params"))
}
