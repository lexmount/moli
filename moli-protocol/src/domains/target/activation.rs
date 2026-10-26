use serde::Deserialize;

use crate::automation::DevToolsTargetId;

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
        AutomationCommand::ActivateTarget(command),
    )
}

pub(super) fn build_cdp_activate_target_command(
    cmd: &Cmd<'_>,
    params: ActivateTargetParams,
) -> DevToolsActivateTargetCommand {
    DevToolsActivateTargetCommand {
        context: cmd.automation_context(Some(params.target_id.as_str()), Option::<&str>::None),
        target_id: DevToolsTargetId::from(params.target_id),
    }
}

pub(crate) struct PreparedTargetActivation {
    updates: Vec<crate::conn::TargetSurfaceUpdate>,
    selection: Option<(String, String)>,
    visibility: Vec<TargetVisibilityChange>,
}

#[derive(Clone)]
pub(crate) struct TargetVisibilityChange {
    pub(crate) target_id: String,
    pub(crate) visible: bool,
    pub(crate) session_ids: Vec<Option<String>>,
}

#[derive(Clone)]
pub(crate) struct TargetActivationOutput {
    visibility: Vec<TargetVisibilityChange>,
}

impl TargetActivationOutput {
    pub(crate) fn finish(self, conn: &mut CdpConnection) -> Vec<BackgroundProtocolEvent> {
        let mut events = Vec::new();
        for change in self.visibility {
            if conn
                .target_session_route_for_target_id(&change.target_id)
                .is_none()
                || conn.target_document_is_visible(&change.target_id) != change.visible
            {
                continue;
            }
            for session in change.session_ids {
                if let Some(session) = session.as_deref()
                    && conn
                        .session_route(Some(session))
                        .as_ref()
                        .and_then(|route| route.target_id())
                        != Some(&change.target_id)
                {
                    continue;
                }
                events.push(BackgroundProtocolEvent::page_screencast_visibility_changed(
                    session.as_deref(),
                    change.visible,
                ));
            }
        }
        events
    }
}

pub(super) fn prepare_target_activation(
    conn: &mut CdpConnection,
    command: DevToolsActivateTargetCommand,
) -> Result<Option<PreparedTargetActivation>, DevToolsError> {
    prepare_target_activation_for_target(conn, command.target_id.as_str())
}

pub(crate) fn prepare_target_activation_for_target(
    conn: &mut CdpConnection,
    requested_target_id: &str,
) -> Result<Option<PreparedTargetActivation>, DevToolsError> {
    if conn.default_placeholder_is_logically_active(requested_target_id) {
        return Ok(None);
    }
    conn.ensure_default_target_live(requested_target_id);
    let target_id = conn
        .primary_page_target_id_for_tab_target_id(requested_target_id)
        .map(str::to_owned)
        .unwrap_or_else(|| requested_target_id.to_owned());
    if conn.page_target_is_closing(&target_id) {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "Target is closing",
        ));
    }
    let restore = previously_active_browser_context_id(conn);
    let prepared = (|| {
        select_browser_context_for_target(conn, &target_id).map_err(activation_error)?;
        let context = conn.browser_context.as_ref().ok_or_else(|| {
            DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "BrowserContextNotLoaded")
        })?;
        if context.has_shared_worker_target(&target_id)
            || context.has_dedicated_worker_target(&target_id)
            || context.has_service_worker_target(&target_id)
        {
            return Ok(None);
        }
        if context.active_target_id().is_none() && context.has_no_background_targets() {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "TargetNotLoaded",
            ));
        }
        let changed = !context.is_active_target(&target_id);
        if changed && context.background_target(&target_id).is_none() {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "UnknownTargetId",
            ));
        }
        let browser_context_id = context.id.clone();
        let previous_target = context.active_target_id_owned();
        let was_visible = conn.target_document_is_visible(&target_id);
        let hidden_sessions = previous_target
            .as_deref()
            .filter(|previous| *previous != target_id)
            .map(|previous| conn.page_screencast_session_ids_for_target(previous))
            .unwrap_or_default();
        conn.select_browser_focus_for_target(&target_id);
        let updates = conn
            .browser_context
            .as_mut()
            .unwrap()
            .start_select_page_target(&target_id)
            .expect("a validated target remains selectable in synchronous preparation");
        let mut visibility = Vec::new();
        if changed {
            if let Some(previous) = previous_target
                && !conn.target_document_is_visible(&previous)
            {
                visibility.push(TargetVisibilityChange {
                    target_id: previous,
                    visible: false,
                    session_ids: hidden_sessions,
                });
            }
            if !was_visible {
                visibility.push(TargetVisibilityChange {
                    target_id: target_id.clone(),
                    visible: true,
                    session_ids: conn.page_screencast_session_ids_for_target(&target_id),
                });
            }
        }
        Ok(Some(PreparedTargetActivation {
            updates,
            selection: changed.then_some((browser_context_id, target_id)),
            visibility,
        }))
    })();
    // Global routing selection is never held across a renderer wait.
    restore_previously_active_browser_context(conn, restore.as_deref());
    prepared
}

pub(crate) fn prepare_target_activation_after_removal(
    conn: &mut CdpConnection,
    target_id: &str,
) -> Result<Option<PreparedTargetActivation>, DevToolsError> {
    let mut prepared = prepare_target_activation_for_target(conn, target_id)?;
    // Removing a selected tab already updates the logical tab selector. Its
    // newly exposed neighbor still needs the renderer/screencast transition.
    if let Some(prepared) = &mut prepared
        && !prepared
            .visibility
            .iter()
            .any(|change| change.target_id == target_id && change.visible)
    {
        prepared.visibility.push(TargetVisibilityChange {
            target_id: target_id.to_owned(),
            visible: true,
            session_ids: conn.page_screencast_session_ids_for_target(target_id),
        });
    }
    Ok(prepared)
}

impl PreparedTargetActivation {
    pub(crate) async fn run(
        self,
        driver: &mut crate::conn::ProtocolWorkflowDriver<'_>,
    ) -> Result<TargetActivationOutput> {
        wait_for_surface_updates(driver, self.updates).await?;
        let required = self.selection.is_some();
        let selection = self.selection;
        let activity = driver
            .with_state(move |state| state.complete_target_activation_selection(selection))
            .await?;
        let activity = match activity {
            Ok(updates) => wait_for_surface_updates(driver, updates).await,
            Err(error) => Err(error),
        };
        match activity {
            Ok(()) => {}
            Err(error) if required => return Err(error),
            Err(error) => tracing::warn!(%error,
                "failed to update document activity after activation"),
        }
        Ok(TargetActivationOutput {
            visibility: self.visibility,
        })
    }
}

pub(crate) async fn wait_for_surface_updates(
    driver: &mut crate::conn::ProtocolWorkflowDriver<'_>,
    updates: Vec<crate::conn::TargetSurfaceUpdate>,
) -> Result<()> {
    for update in updates {
        let completed = update.updates.wait().await;
        let target_id = update.target_id;
        let logging_target = target_id.clone();
        let result = driver
            .with_state(move |state| {
                state.finish_target_surface_update(update.browser_context_id, target_id, completed)
            })
            .await?;
        if let Err(error) = result {
            if update.required {
                return Err(error);
            }
            tracing::warn!(target_id = logging_target, %error,
                "failed to update background document activity");
        }
    }
    Ok(())
}

pub(super) fn activation_result_plan(
    conn: &mut CdpConnection,
    result: Result<TargetActivationOutput>,
) -> CommandOutputPlan {
    match result {
        Ok(output) => {
            let mut plan = CommandOutputPlan::default();
            plan.extend_background_events(output.finish(conn));
            plan.push_success();
            plan
        }
        Err(error) => CommandOutputPlan::from_devtools_error(activation_error(error)),
    }
}

fn activation_error(error: anyhow::Error) -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::NoSuchTarget, format!("{error:#}"))
}

pub(super) async fn execute_devtools_activate_target_command_async(
    conn: &mut CdpConnection,
    command: DevToolsActivateTargetCommand,
) -> Result<Vec<BackgroundProtocolEvent>, DevToolsError> {
    let Some(prepared) = prepare_target_activation(conn, command)? else {
        return Ok(Vec::new());
    };
    let completed = prepared
        .run(&mut crate::conn::ProtocolWorkflowDriver::inline(conn))
        .await
        .map_err(activation_error)?;
    Ok(completed.finish(conn))
}
