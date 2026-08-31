use serde::Deserialize;

use crate::devtools_runtime::{
    DevToolsGetTargetInfoCommand, DevToolsGetTargetInfoResult, DevToolsTargetId,
};

use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetTargetInfoParams {
    target_id: Option<String>,
}

pub(super) fn get_target_info(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params: GetTargetInfoParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        Ok(None) => GetTargetInfoParams { target_id: None },
        Err(e) => {
            return CommandOutputPlan::error_without_session(-32602, e);
        }
    };
    if !conn.target_handler_may_get_target_info(cmd.session_id, params.target_id.as_deref()) {
        return CommandOutputPlan::error(-32000, "Not allowed");
    }
    let command = build_cdp_get_target_info_command(cmd, params);
    match super::start_devtools_target_command(
        conn,
        cmd.id,
        cmd.session_id,
        DevToolsCommand::GetTargetInfo(command),
    ) {
        TargetCommandTaskStep::Complete(plan) => plan,
        TargetCommandTaskStep::Pending(_) => {
            CommandOutputPlan::error_without_session(-32000, "UnexpectedPending")
        }
    }
}

fn build_cdp_get_target_info_command(
    cmd: &Cmd<'_>,
    params: GetTargetInfoParams,
) -> DevToolsGetTargetInfoCommand {
    DevToolsGetTargetInfoCommand {
        context: cmd.devtools_command_context(params.target_id.as_deref(), None::<&str>),
        target_id: params.target_id.map(DevToolsTargetId::from),
    }
}

pub(super) fn start_devtools_get_target_info_command(
    conn: &CdpConnection,
    command: DevToolsGetTargetInfoCommand,
) -> CommandOutputPlan {
    match execute_devtools_get_target_info_command(conn, command) {
        Ok(result) => {
            CommandOutputPlan::from_devtools_result(DevToolsCommandResult::GetTargetInfo(result))
        }
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
}

pub(super) fn execute_devtools_get_target_info_command(
    conn: &CdpConnection,
    command: DevToolsGetTargetInfoCommand,
) -> Result<DevToolsGetTargetInfoResult, DevToolsError> {
    let owner_target_id = command
        .context
        .target_id
        .as_ref()
        .map(|target_id| target_id.as_str().to_owned())
        .or_else(|| {
            conn.non_browser_target_id_for_session(
                command
                    .context
                    .session_id
                    .as_ref()
                    .map(|session_id| session_id.as_str()),
            )
        });
    let wanted = command
        .target_id
        .as_ref()
        .map(|target_id| target_id.as_str())
        .or(owner_target_id.as_deref());
    let Some(wanted) = wanted else {
        return Ok(DevToolsGetTargetInfoResult {
            target_info: super::browser_context::devtools_browser_target_info(),
        });
    };
    if wanted == super::browser_context::DEVTOOLS_BROWSER_TARGET_ID {
        return Ok(DevToolsGetTargetInfoResult {
            target_info: super::browser_context::devtools_browser_target_info(),
        });
    }
    if let Some(target_info) = conn.devtools_target_info(wanted) {
        if let Some(message) =
            super::transient_no_page_devtools_target_info_error(conn, &target_info)
        {
            return Err(DevToolsError::new(DevToolsErrorKind::Internal, message));
        }
        return Ok(DevToolsGetTargetInfoResult { target_info });
    }
    if conn.browser_context.is_none() && conn.inactive_browser_contexts.is_empty() {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "BrowserContextNotLoaded",
        ));
    }
    let target_exists = conn.browser_contexts().any(|bc| {
        bc.active_target_id().is_some()
            || !bc.has_no_background_targets()
            || bc.has_any_shared_worker_targets()
            || bc.has_any_dedicated_worker_targets()
            || bc.has_any_service_worker_targets()
    });
    if !target_exists {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "TargetNotLoaded",
        ));
    }
    Err(DevToolsError::new(
        DevToolsErrorKind::NoSuchTarget,
        "UnknownTargetId",
    ))
}
