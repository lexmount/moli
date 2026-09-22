use super::command_output::CommandOutputPlan;
use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};
use moli_core::page::{CompletedPageCommand, PendingPageCommand, RendererInspectorOverlayCommand};
use serde::Deserialize;

#[derive(Clone, Copy, Deserialize)]
struct Color {
    r: i32,
    g: i32,
    b: i32,
    a: Option<f32>,
}
impl Color {
    fn components(self) -> [f32; 4] {
        [
            self.r.clamp(0, 255) as f32 / 255.0,
            self.g.clamp(0, 255) as f32 / 255.0,
            self.b.clamp(0, 255) as f32 / 255.0,
            self.a.unwrap_or(1.0).clamp(0.0, 1.0),
        ]
    }
}
fn color(value: Option<Color>) -> [f32; 4] {
    value.map_or([0.0; 4], Color::components)
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HighlightConfig {
    content_color: Option<Color>,
    padding_color: Option<Color>,
    border_color: Option<Color>,
    margin_color: Option<Color>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NodeParams {
    node_id: Option<u32>,
    backend_node_id: Option<u32>,
    object_id: Option<String>,
    highlight_config: HighlightConfig,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RectParams {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: Option<Color>,
    outline_color: Option<Color>,
}

pub(crate) struct PendingOverlayCommandDispatch {
    command_id: Option<u64>,
    owner: CommandOwnerScope,
    pending: PendingPageCommand,
}
pub(crate) struct CompletedOverlayCommandDispatch {
    command_id: Option<u64>,
    owner: CommandOwnerScope,
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
                .get_params::<NodeParams>()
                .map_err(invalid)?
                .ok_or_else(|| invalid("Missing node"))?;
            if p.node_id.is_none() && p.backend_node_id.is_none() && p.object_id.is_none() {
                return Err(invalid("A node reference is required"));
            }
            let config = p.highlight_config;
            Ok(RendererInspectorOverlayCommand::Node {
                node_id: p.node_id,
                backend_node_id: p.backend_node_id,
                object_id: p.object_id,
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
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let session = conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    let result = conn
        .loaded_page_mut_for_protocol_access_for_owner(&owner)
        .and_then(|page| {
            page.start_set_inspector_overlay(session, command)
                .map_err(|error| error.to_string())
        });
    match result {
        Ok(pending) => OverlayCommandTaskStep::Pending(PendingOverlayCommandDispatch {
            command_id: cmd.id,
            owner,
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
        conn.loaded_page_mut_for_protocol_access_for_owner(&completed.owner)
            .and_then(|page| {
                page.finish_set_inspector_overlay(completion)
                    .map_err(|error| error.to_string())
            })
    });
    match result {
        Ok(()) => CommandOutputPlan::success(),
        Err(error) => CommandOutputPlan::error(-32000, error),
    }
}
