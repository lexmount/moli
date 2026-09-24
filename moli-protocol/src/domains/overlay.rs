use super::command_output::CommandOutputPlan;
use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};
use chromiumoxide_cdp::cdp::browser_protocol::{dom::Rgba, overlay::HighlightNodeParams};
use moli_core::page::{CompletedPageCommand, PendingPageCommand, RendererInspectorOverlayCommand};
use serde::Deserialize;

fn color(value: Option<Rgba>) -> [f32; 4] {
    value.map_or([0.0; 4], |value| {
        [
            value.r.clamp(0, 255) as f32 / 255.0,
            value.g.clamp(0, 255) as f32 / 255.0,
            value.b.clamp(0, 255) as f32 / 255.0,
            value.a.unwrap_or(1.0).clamp(0.0, 1.0) as f32,
        ]
    })
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RectParams {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: Option<Rgba>,
    outline_color: Option<Rgba>,
}

fn unsupported_highlight_option(
    config: &chromiumoxide_cdp::cdp::browser_protocol::overlay::HighlightConfig,
) -> Option<String> {
    let mut fields = serde_json::to_value(config).ok()?.as_object()?.clone();
    for color in ["contentColor", "paddingColor", "borderColor", "marginColor"] {
        fields.remove(color);
    }
    // These values request exactly the protocol defaults and need no painting.
    for flag in ["showInfo", "showStyles", "showRulers", "showExtensionLines"] {
        if fields.get(flag) == Some(&serde_json::Value::Bool(false)) {
            fields.remove(flag);
        }
    }
    if fields.get("showAccessibilityInfo") == Some(&serde_json::Value::Bool(true)) {
        fields.remove("showAccessibilityInfo");
    }
    fields.into_iter().next().map(|(name, _)| name)
}

pub(crate) struct PendingOverlayCommandDispatch {
    command_id: Option<u64>,
    owner: CommandOwnerScope,
    cleanup: bool,
    pending: PendingPageCommand,
}
pub(crate) struct CompletedOverlayCommandDispatch {
    command_id: Option<u64>,
    owner: CommandOwnerScope,
    cleanup: bool,
    completed: Result<CompletedPageCommand, String>,
}
pub(crate) enum OverlayCommandTaskStep {
    Pending(PendingOverlayCommandDispatch),
    Complete(CommandOutputPlan),
}
impl PendingOverlayCommandDispatch {
    pub(crate) async fn wait(self) -> CompletedOverlayCommandDispatch {
        CompletedOverlayCommandDispatch {
            command_id: self.command_id,
            owner: self.owner,
            cleanup: self.cleanup,
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
        }
    }
}
impl CompletedOverlayCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner.session_id()
    }
}

fn parse_command(cmd: &Cmd<'_>) -> Result<RendererInspectorOverlayCommand, CommandOutputPlan> {
    let invalid = |error| CommandOutputPlan::error(-32602, error);
    match cmd.action {
        "enable" => Ok(RendererInspectorOverlayCommand::Enable),
        "disable" => Ok(RendererInspectorOverlayCommand::Disable),
        "hideHighlight" => Ok(RendererInspectorOverlayCommand::Hide),
        "highlightRect" => {
            let p = cmd
                .get_params::<RectParams>()
                .map_err(invalid)?
                .ok_or_else(|| invalid("Missing rectangle"))?;
            if p.width < 0 || p.height < 0 {
                return Err(invalid("Rectangle dimensions must be nonnegative"));
            }
            Ok(RendererInspectorOverlayCommand::Rect {
                rect: [p.x as f32, p.y as f32, p.width as f32, p.height as f32],
                color: color(p.color),
                outline: color(p.outline_color),
            })
        }
        "highlightNode" => {
            let p = cmd
                .get_params::<HighlightNodeParams>()
                .map_err(invalid)?
                .ok_or_else(|| invalid("Missing node"))?;
            if p.selector.is_some() {
                return Err(invalid("selector highlighting is not supported"));
            }
            if p.node_id.is_none() && p.backend_node_id.is_none() && p.object_id.is_none() {
                return Err(invalid("A node reference is required"));
            }
            if let Some(option) = unsupported_highlight_option(&p.highlight_config) {
                return Err(CommandOutputPlan::error(
                    -32602,
                    format!("Highlight option {option} is not supported"),
                ));
            }
            let node_id = p
                .node_id
                .map(|id| u32::try_from(*id.inner()).map_err(|_| invalid("Invalid nodeId")))
                .transpose()?;
            let backend_node_id = p
                .backend_node_id
                .map(|id| u32::try_from(*id.inner()).map_err(|_| invalid("Invalid backendNodeId")))
                .transpose()?;
            let config = p.highlight_config;
            Ok(RendererInspectorOverlayCommand::Node {
                node_id,
                backend_node_id,
                object_id: p.object_id.map(String::from),
                colors: [
                    config.content_color,
                    config.padding_color,
                    config.border_color,
                    config.margin_color,
                ]
                .map(color),
            })
        }
        _ => Err(CommandOutputPlan::error(-32601, "UnknownMethod")),
    }
}

pub(crate) fn try_start_overlay_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> OverlayCommandTaskStep {
    let command = match parse_command(cmd) {
        Ok(command) => command,
        Err(plan) => return OverlayCommandTaskStep::Complete(plan),
    };
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    if conn
        .target_devtools_session_state_for_owner(&owner)
        .is_none()
    {
        return OverlayCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "No target for Overlay command",
        ));
    }
    if matches!(command, RendererInspectorOverlayCommand::Enable) {
        return OverlayCommandTaskStep::Complete(CommandOutputPlan::success());
    }
    let cleanup = matches!(
        command,
        RendererInspectorOverlayCommand::Disable | RendererInspectorOverlayCommand::Hide
    );
    if conn.layout_policy() == moli_core::LayoutPolicy::Mock
        && matches!(
            command,
            RendererInspectorOverlayCommand::Rect { .. }
                | RendererInspectorOverlayCommand::Node { .. }
        )
    {
        return OverlayCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "Inspector highlighting requires layout",
        ));
    }
    let session = conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    let page = if cleanup {
        conn.loaded_page_mut_for_target_configuration_for_owner(&owner)
    } else {
        conn.loaded_page_mut_for_protocol_access_for_owner(&owner)
    };
    if cleanup && page.is_err() {
        return OverlayCommandTaskStep::Complete(CommandOutputPlan::success());
    }
    let result = page.and_then(|page| {
        page.start_set_inspector_overlay(session, command)
            .map_err(|error| error.to_string())
    });
    match result {
        Ok(pending) => OverlayCommandTaskStep::Pending(PendingOverlayCommandDispatch {
            command_id: cmd.id,
            owner,
            cleanup,
            pending,
        }),
        Err(error) => OverlayCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error)),
    }
}

pub(crate) fn complete_pending_overlay_command(
    conn: &mut CdpConnection,
    completed: CompletedOverlayCommandDispatch,
) -> CommandOutputPlan {
    let result = completed.completed.and_then(|completion| {
        let page = if completed.cleanup {
            conn.loaded_page_mut_for_target_configuration_for_owner(&completed.owner)
        } else {
            conn.loaded_page_mut_for_protocol_access_for_owner(&completed.owner)
        };
        match page {
            Ok(page) => page
                .finish_set_inspector_overlay(completion)
                .map_err(|error| error.to_string()),
            Err(_) if completed.cleanup => Ok(()),
            Err(error) => Err(error),
        }
    });
    match result {
        Ok(()) => CommandOutputPlan::success(),
        Err(error) => CommandOutputPlan::error(-32000, error),
    }
}
