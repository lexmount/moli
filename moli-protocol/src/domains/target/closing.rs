use serde::Deserialize;

use crate::devtools_runtime::{DevToolsCloseTargetResult, DevToolsTargetId};

use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CloseTargetParams {
    pub(super) target_id: String,
}

pub(super) fn start_close_target_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> TargetCommandTaskStep {
    let params: CloseTargetParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        _ => {
            return super::target_command_error(-32602, "InvalidParams");
        }
    };
    if !conn.target_handler_may_close_target(cmd.session_id, &params.target_id) {
        return super::target_command_error(-32000, "Not allowed");
    }
    let command = build_cdp_close_target_command(cmd, params);
    super::start_devtools_target_command(
        conn,
        cmd.id,
        cmd.session_id,
        DevToolsCommand::CloseTarget(command),
    )
}

pub(super) fn build_cdp_close_target_command(
    cmd: &Cmd<'_>,
    params: CloseTargetParams,
) -> DevToolsCloseTargetCommand {
    DevToolsCloseTargetCommand {
        context: cmd.devtools_command_context(Some(params.target_id.as_str()), None::<&str>),
        target_id: DevToolsTargetId::from(params.target_id),
    }
}

pub(super) fn start_devtools_close_target_command(
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsCloseTargetCommand,
) -> TargetCommandTaskStep {
    pending_close_target_command(command_id, command_session_id, command)
}

pub(super) async fn complete_close_target_command_async(
    conn: &mut CdpConnection,
    command: DevToolsCloseTargetCommand,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> CommandOutputPlan {
    let mut side_effects = events::TargetProtocolSideEffects::default();
    match execute_devtools_close_target_command_async(
        conn,
        command,
        &mut side_effects,
        command_context,
    )
    .await
    {
        Ok(result) => {
            let mut plan =
                CommandOutputPlan::from_devtools_result(DevToolsCommandResult::CloseTarget(result));
            plan.extend(side_effects.into_plan());
            plan
        }
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
}

pub(super) async fn execute_devtools_close_target_command_async(
    conn: &mut CdpConnection,
    command: DevToolsCloseTargetCommand,
    out: &mut events::TargetProtocolSideEffects,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> Result<DevToolsCloseTargetResult, DevToolsError> {
    let closes_default_target = matches!(
        command.target_id.as_str(),
        crate::DEFAULT_CDP_PAGE_TARGET_ID | crate::DEFAULT_CDP_TAB_TARGET_ID
    );
    let target_id = command.target_id.into_string();
    if let Some(plan) = conn.close_default_target_placeholder(&target_id) {
        out.extend_background_events(plan.into_background_events());
        return Ok(DevToolsCloseTargetResult { success: true });
    }
    let restore_browser_context_id = previously_active_browser_context_id(conn);
    let result = close_target_inner_async(conn, out, command_context, target_id).await;
    if result.is_ok() && closes_default_target {
        conn.mark_default_browser_target_closed();
    }
    restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
    result
}

async fn close_target_inner_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    command_context: &mut crate::conn::CommandDispatchContext,
    target_id: String,
) -> Result<DevToolsCloseTargetResult, DevToolsError> {
    let target_id = conn
        .primary_page_target_id_for_tab_target_id(&target_id)
        .map(str::to_owned)
        .unwrap_or(target_id);
    if let Err(message) = select_browser_context_for_target(conn, &target_id) {
        return Err(DevToolsError::new(DevToolsErrorKind::NoSuchTarget, message));
    }
    if conn
        .browser_context
        .as_ref()
        .is_some_and(|bc| bc.has_shared_worker_target(&target_id))
    {
        if !worker_target::close_shared_worker_target_for_target_close_async(
            conn,
            &target_id,
            command_context,
        )
        .await
        {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "UnknownTargetId",
            ));
        }
        return Ok(DevToolsCloseTargetResult { success: true });
    }
    if conn
        .browser_context
        .as_ref()
        .is_some_and(|bc| bc.has_dedicated_worker_target(&target_id))
    {
        if !worker_target::close_dedicated_worker_target_for_target_close_async(
            conn,
            &target_id,
            command_context,
        )
        .await
        {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "UnknownTargetId",
            ));
        }
        return Ok(DevToolsCloseTargetResult { success: true });
    }
    if conn
        .browser_context
        .as_ref()
        .is_some_and(|bc| bc.has_service_worker_target(&target_id))
    {
        return Ok(DevToolsCloseTargetResult { success: true });
    }
    if conn.browser_context.as_ref().is_some_and(|bc| {
        !matches!(
            bc.active_target_identity(),
            Some((ref active_target_id, _)) if active_target_id == &target_id
        )
    }) {
        let target_route = conn
            .target_session_route_for_target_id(&target_id)
            .ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "UnknownTargetId")
            })?;
        let session_id = conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.background_target(&target_id))
            .and_then(|target| target.session_id().map(str::to_owned));
        let owner_scope = session_id
            .as_deref()
            .map(crate::conn::CommandOwnerScope::for_session)
            .unwrap_or_else(|| {
                crate::conn::CommandOwnerScope::for_implicit_route(Some(target_route))
            });
        let renderer_output_predecessor =
            events::fail_pending_fetch_state_for_target_background_events_async(
                conn,
                out.background_events_mut(),
                session_id.as_deref(),
                "Target closed",
            )
            .await;
        settle_target_close_after_pending_fetches_async(
            conn,
            out,
            command_context,
            renderer_output_predecessor,
            owner_scope,
            target_id,
        )
        .await;
        return Ok(DevToolsCloseTargetResult { success: true });
    }

    let session_id = match conn.browser_context.as_ref() {
        Some(bc) => {
            if bc.active_target_id().is_none() && bc.has_no_background_targets() {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::NoSuchTarget,
                    "TargetNotLoaded",
                ));
            }
            match bc.active_target_identity() {
                Some((_active_target_id, session_id)) => session_id,
                None => {
                    return Err(DevToolsError::new(
                        DevToolsErrorKind::NoSuchTarget,
                        "TargetNotLoaded",
                    ));
                }
            }
        }
        None => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "BrowserContextNotLoaded",
            ));
        }
    };

    conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
        out.background_events_mut(),
        command_context.protocol_events_mut(),
        session_id.as_deref(),
        "Target closed",
    );
    let renderer_output_predecessor =
        events::fail_pending_fetch_state_for_target_background_events_async(
            conn,
            out.background_events_mut(),
            session_id.as_deref(),
            "Target closed",
        )
        .await;
    let owner_scope = crate::conn::CommandOwnerScope::capture(conn, session_id.as_deref());
    settle_target_close_after_pending_fetches_async(
        conn,
        out,
        command_context,
        renderer_output_predecessor,
        owner_scope,
        target_id,
    )
    .await;
    Ok(DevToolsCloseTargetResult { success: true })
}

/// Preserves the final renderer publication without changing the ordinary
/// `Target.closeTarget` transaction boundary.
///
/// Most target closes produce no renderer output. Those closes still complete
/// synchronously and return their detach/destroy side effects with the command,
/// matching Chromium's target-domain behavior. A paused request can first
/// produce a terminal renderer record, however. Only that case must defer
/// target retirement until the command's exact cursor has crossed ordered
/// ingress; otherwise retiring the route would discard that final record.
async fn settle_target_close_after_pending_fetches_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    command_context: &mut crate::conn::CommandDispatchContext,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
    owner_scope: crate::conn::CommandOwnerScope,
    target_id: String,
) {
    let action = crate::domains::page::PageTargetTerminationOwnerAction::new(
        owner_scope,
        target_id,
        crate::domains::page::PageTargetTerminationKind::TargetClose,
    );
    if let Some(predecessor) = renderer_output_predecessor {
        command_context.set_renderer_output_predecessor(predecessor);
        conn.publish_page_target_termination_owner_action(action);
        return;
    }

    let outcome =
        crate::domains::page::complete_page_target_termination_owner_action_async(conn, action)
            .await;
    let (events, scheduler_events) = outcome.into_protocol_event_parts();
    out.extend_background_events(events);
    conn.extend_scheduler_events(scheduler_events);
}
