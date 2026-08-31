use serde::Deserialize;

use crate::devtools_runtime::DevToolsTargetId;

use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ActivateTargetParams {
    pub(super) target_id: String,
}

pub(super) fn start_activate_target_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> TargetCommandTaskStep {
    let params: ActivateTargetParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return super::target_command_error(-32602, "InvalidParams");
        }
    };
    let command = build_cdp_activate_target_command(cmd, params);
    super::start_devtools_target_command(
        conn,
        cmd.id,
        cmd.session_id,
        DevToolsCommand::ActivateTarget(command),
    )
}

pub(super) fn build_cdp_activate_target_command(
    cmd: &Cmd<'_>,
    params: ActivateTargetParams,
) -> DevToolsActivateTargetCommand {
    DevToolsActivateTargetCommand {
        context: cmd
            .devtools_command_context(Some(params.target_id.as_str()), Option::<&str>::None),
        target_id: DevToolsTargetId::from(params.target_id),
    }
}

pub(super) async fn complete_activate_target_command_async(
    conn: &mut CdpConnection,
    command: DevToolsActivateTargetCommand,
) -> CommandOutputPlan {
    match execute_devtools_activate_target_command_async(conn, command).await {
        Ok(events) => {
            let mut plan = CommandOutputPlan::default();
            plan.extend_background_events(events);
            plan.push_success();
            plan
        }
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
}

pub(super) async fn execute_devtools_activate_target_command_async(
    conn: &mut CdpConnection,
    command: DevToolsActivateTargetCommand,
) -> Result<Vec<BackgroundProtocolEvent>, DevToolsError> {
    if conn.default_placeholder_is_logically_active(command.target_id.as_str()) {
        return Ok(Vec::new());
    }
    conn.ensure_default_target_live(command.target_id.as_str());
    let target_id = conn
        .primary_page_target_id_for_tab_target_id(command.target_id.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| command.target_id.as_str().to_owned());
    let previously_active_browser_context_id = previously_active_browser_context_id(conn);
    if let Err(message) = select_browser_context_for_target(conn, &target_id) {
        restore_previously_active_browser_context(
            conn,
            previously_active_browser_context_id.as_deref(),
        );
        return Err(DevToolsError::new(DevToolsErrorKind::NoSuchTarget, message));
    }
    let bc = match conn.browser_context.as_ref() {
        Some(bc) => bc,
        None => {
            restore_previously_active_browser_context(
                conn,
                previously_active_browser_context_id.as_deref(),
            );
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "BrowserContextNotLoaded",
            ));
        }
    };
    if bc.has_shared_worker_target(&target_id) {
        restore_previously_active_browser_context(
            conn,
            previously_active_browser_context_id.as_deref(),
        );
        return Ok(Vec::new());
    }
    if bc.has_dedicated_worker_target(&target_id) {
        restore_previously_active_browser_context(
            conn,
            previously_active_browser_context_id.as_deref(),
        );
        return Ok(Vec::new());
    }
    if bc.has_service_worker_target(&target_id) {
        restore_previously_active_browser_context(
            conn,
            previously_active_browser_context_id.as_deref(),
        );
        return Ok(Vec::new());
    }
    if bc.active_target_id().is_none() && bc.has_no_background_targets() {
        restore_previously_active_browser_context(
            conn,
            previously_active_browser_context_id.as_deref(),
        );
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "TargetNotLoaded",
        ));
    }
    let protocol_events = if !matches!(
        bc.active_target_identity(),
        Some((ref active_target_id, _)) if active_target_id == &target_id
    ) && bc.background_target(&target_id).is_some()
    {
        match conn
            .promote_background_target_to_active_for_connection_async(&target_id)
            .await
        {
            Ok(Some(activation)) => activation.into_protocol_events(),
            Ok(None) => {
                restore_previously_active_browser_context(
                    conn,
                    previously_active_browser_context_id.as_deref(),
                );
                return Err(DevToolsError::new(
                    DevToolsErrorKind::NoSuchTarget,
                    "UnknownTargetId",
                ));
            }
            Err(message) => {
                restore_previously_active_browser_context(
                    conn,
                    previously_active_browser_context_id.as_deref(),
                );
                return Err(DevToolsError::new(DevToolsErrorKind::NoSuchTarget, message));
            }
        }
    } else if !matches!(
        bc.active_target_identity(),
        Some((ref active_target_id, _)) if active_target_id == &target_id
    ) {
        restore_previously_active_browser_context(
            conn,
            previously_active_browser_context_id.as_deref(),
        );
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "UnknownTargetId",
        ));
    } else {
        Vec::new()
    };

    restore_previously_active_browser_context(
        conn,
        previously_active_browser_context_id.as_deref(),
    );
    Ok(protocol_events)
}
