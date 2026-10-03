use crate::{
    conn::{CdpConnection, Cmd, CommandOwnerScope},
    domains::{actions::WebMcpAction, command_output::CommandOutputPlan},
};
use moli_page_types::{DevToolsSessionKey, RendererWebMcpCommand, RendererWebMcpError};
use serde::Deserialize;
use serde_json::{Value, json};

mod output;
#[cfg(test)]
mod tests;
pub(in crate::domains) use output::navigation_failed;
pub(in crate::domains) use output::{WebMcpPreparedOutputSlot, append_observation, project_async};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InvokeParams {
    frame_id: String,
    tool_name: String,
    input: serde_json::Map<String, Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelParams {
    invocation_id: String,
}

fn request(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<RendererWebMcpCommand, CommandOutputPlan> {
    match cmd.parse_action::<WebMcpAction>() {
        Some(WebMcpAction::Enable) => Ok(RendererWebMcpCommand::Enable),
        Some(WebMcpAction::Disable) => Ok(RendererWebMcpCommand::Disable),
        Some(WebMcpAction::InvokeTool) => {
            let params = cmd
                .get_params::<InvokeParams>()
                .ok()
                .flatten()
                .ok_or_else(|| CommandOutputPlan::error(-32602, "Invalid parameters"))?;
            let root = conn
                .target_session_owner_frame_tree_identity(cmd.session_id)
                .map(|identity| identity.0);
            Ok(RendererWebMcpCommand::InvokeTool {
                frame_id: Some(params.frame_id).filter(|frame| Some(frame) != root.as_ref()),
                name: params.tool_name,
                input: serde_json::to_string(&params.input).expect("JSON input"),
            })
        }
        Some(WebMcpAction::CancelInvocation) => {
            let params = cmd
                .get_params::<CancelParams>()
                .ok()
                .flatten()
                .ok_or_else(|| CommandOutputPlan::error(-32602, "Invalid parameters"))?;
            let invocation_id = params
                .invocation_id
                .parse()
                .ok()
                .filter(|id| *id > 0)
                .ok_or_else(|| CommandOutputPlan::error(-32602, "Invalid invocation id"))?;
            Ok(RendererWebMcpCommand::CancelInvocation { invocation_id })
        }
        None => Err(CommandOutputPlan::error(-32601, "UnknownMethod")),
    }
}

pub(crate) fn try_start_native_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<super::native::NativeCommandStep> {
    use moli_core::{
        RendererNativeProtocolResponse as Response, RendererNativeProtocolStateUpdate as Update,
        RendererPageCommand as Command, RendererPageReply as Reply,
    };
    let command = match request(conn, cmd) {
        Ok(command) => command,
        Err(plan) => return Some(super::native::NativeCommandStep::Complete(plan)),
    };
    let enabled = match &command {
        RendererWebMcpCommand::Enable => Some(true),
        RendererWebMcpCommand::Disable => Some(false),
        _ => None,
    };
    let session = DevToolsSessionKey::from_wire_session_id(
        conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id)
            .as_deref(),
    );
    Some(super::native::start(
        conn,
        cmd,
        Command::WebMcp { session, command },
        move |reply| match reply {
            Ok(Reply::WebMcp(Ok(id))) => {
                let mut response = Response::success(
                    id.map_or_else(|| json!({}), |id| json!({"invocationId": id.to_string()})),
                );
                if let Some(enabled) = enabled {
                    response.state_updates.push(Update::WebMcpEnabled(enabled));
                }
                response
            }
            Ok(Reply::WebMcp(Err(error))) => match error {
                RendererWebMcpError::NotEnabled => {
                    Response::error(-32000, "WebMCP domain is not enabled")
                }
                RendererWebMcpError::InvalidParams(message) => Response::error(-32602, message),
                RendererWebMcpError::SchedulingFailed => {
                    Response::error(-32000, "Tool invocation could not be scheduled")
                }
            },
            Err(error) => Response::error(-32000, error.to_string()),
            _ => unreachable!("WebMCP command reply"),
        },
    ))
}

// Pre-document configuration is replayed when the renderer attaches.
pub(crate) fn command_output_plan(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    match cmd.parse_action::<WebMcpAction>() {
        Some(action @ (WebMcpAction::Enable | WebMcpAction::Disable)) => {
            let owner = CommandOwnerScope::capture(conn, cmd.session_id);
            conn.with_target_devtools_session_state_for_owner_mut(&owner, |state| {
                state.web_mcp_enabled = action == WebMcpAction::Enable
            });
            CommandOutputPlan::success()
        }
        Some(WebMcpAction::InvokeTool | WebMcpAction::CancelInvocation) => {
            CommandOutputPlan::error(-32000, "NoDocumentLoaded")
        }
        None => CommandOutputPlan::error(-32601, "UnknownMethod"),
    }
}
