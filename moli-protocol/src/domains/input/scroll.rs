use std::time::Duration;

use chromiumoxide_cdp::cdp::browser_protocol::input::{
    GestureSourceType, SynthesizeScrollGestureParams,
};

use super::*;

const WHEEL_INTERVAL: Duration = Duration::from_nanos(16_666_667);

pub(super) fn start(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> InputCommandDispatchStep {
    match prepare(conn, cmd) {
        Ok(Some(workflow)) => InputCommandDispatchStep::Workflow(workflow),
        Ok(None) => InputCommandDispatchStep::Complete(CommandOutputPlan::success()),
        Err(error) => InputCommandDispatchStep::Complete(CommandOutputPlan::error(
            error.code,
            format!("{error:#}"),
        )),
    }
}

fn prepare(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Option<crate::conn::PreparedCommandWorkflow>, PendingInputCommandStartError> {
    let invalid = PendingInputCommandStartError::invalid_params;
    if !optional_enum_param_matches(cmd, "gestureSourceType", &["default", "mouse", "touch"]) {
        return Err(invalid());
    }
    let params = cmd
        .get_params::<SynthesizeScrollGestureParams>()
        .map_err(|_| invalid())?
        .ok_or_else(invalid)?;
    let speed = params.speed.unwrap_or(800);
    let repeat_count = u32::try_from(params.repeat_count.unwrap_or(0)).map_err(|_| invalid())?;
    let repeat_delay = Duration::from_millis(
        u64::try_from(params.repeat_delay_ms.unwrap_or(250)).map_err(|_| invalid())?,
    );
    if speed <= 0 || !params.x.is_finite() || !params.y.is_finite() {
        return Err(invalid());
    }
    if params.gesture_source_type == Some(GestureSourceType::Touch) {
        return Err(PendingInputCommandStartError::unsupported(
            "Input.synthesizeScrollGesture does not support touch scrolling.",
        ));
    }
    if conn.layout_policy() == moli_core::LayoutPolicy::Mock {
        return Err(PendingInputCommandStartError::unsupported(
            "Input.synthesizeScrollGesture requires layout hit testing.",
        ));
    }
    let parts = [
        (
            -params.x_distance.unwrap_or(0.0),
            -params.y_distance.unwrap_or(0.0),
        ),
        (
            params.x_overscroll.unwrap_or(0.0),
            params.y_overscroll.unwrap_or(0.0),
        ),
    ];
    let segments = parts
        .into_iter()
        .filter(|&(dx, dy)| dx != 0.0 || dy != 0.0)
        .map(|(dx, dy)| {
            let length = dx.hypot(dy);
            let duration =
                Duration::try_from_secs_f64(length / speed as f64).map_err(|_| invalid())?;
            let ticks = (duration.as_secs_f64() / WHEEL_INTERVAL.as_secs_f64()).ceil();
            if ticks > f64::from(u32::MAX) {
                return Err(invalid());
            }
            Ok((dx, dy, (ticks as u32).max(1)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    conn.ensure_document_accessible_for_owner(&owner)
        .map_err(PendingInputCommandStartError::renderer_error)?;
    if input_events_ignored_for_owner(conn, &owner) || segments.is_empty() {
        return Ok(None);
    }
    let page = conn
        .target_page_residence_identity_for_owner(&owner)
        .ok_or_else(PendingInputCommandStartError::no_document_loaded)?;
    let x = params.x;
    let y = params.y;
    let access = conn.protocol_state_access();
    Ok(Some(
        crate::conn::PreparedCommandWorkflow::new(move |mut context| async move {
            let mut driver = ProtocolWorkflowDriver::new(access);
            let result = async {
                for repeat in 0..=repeat_count {
                    if repeat != 0 {
                        tokio::time::sleep(repeat_delay).await;
                    }
                    let mut first = true;
                    'gesture: for &(dx, dy, ticks) in &segments {
                        for _ in 0..ticks {
                            if !first {
                                tokio::time::sleep(WHEEL_INTERVAL).await;
                            }
                            let wheel = SyntheticWheel {
                                owner: owner.clone(),
                                page: page.clone(),
                                x,
                                y,
                                delta_x: dx / f64::from(ticks),
                                delta_y: dy / f64::from(ticks),
                                cancelable: first,
                            };
                            let pending = driver
                                .with_state(move |state| state.prepare_synthetic_wheel(wheel))
                                .map_err(input_workflow_error)??;
                            let Some(pending) = pending else {
                                return Ok(());
                            };
                            let completed = complete_input_workflow(
                                &mut driver,
                                pending.wait().await,
                                &mut context,
                            )
                            .await;
                            context
                                .context_mut()
                                .protocol_events_mut()
                                .extend(completed.protocol_events);
                            completed.result?;
                            first = false;
                            if completed.wheel_canceled {
                                break 'gesture;
                            }
                        }
                    }
                }
                Ok(())
            }
            .await;
            context.complete(match result {
                Ok(()) => CommandOutputPlan::success(),
                Err(error) => CommandOutputPlan::from_devtools_error(error),
            })
        })
        .cancel_on_waiter_drop(),
    ))
}

pub(crate) struct SyntheticWheel {
    owner: CommandOwnerScope,
    page: TargetPageResidenceIdentity,
    x: f64,
    y: f64,
    delta_x: f64,
    delta_y: f64,
    cancelable: bool,
}

impl SyntheticWheel {
    pub(crate) fn prepare(
        self,
        conn: &mut CdpConnection,
    ) -> Result<Option<PendingInputCommandDispatch>, DevToolsError> {
        if conn
            .target_page_residence_identity_for_owner(&self.owner)
            .as_ref()
            != Some(&self.page)
            || input_events_ignored_for_owner(conn, &self.owner)
        {
            return Ok(None);
        }
        start_page_input_command(
            conn,
            None,
            &self.owner,
            PendingInputCommandKind::DispatchMouseEvent,
            |page| {
                page.start_dispatch_synthetic_wheel(
                    self.x,
                    self.y,
                    self.delta_x,
                    self.delta_y,
                    self.cancelable,
                )
            },
        )
        .map_err(devtools_error_from_input_start_error)
    }
}
