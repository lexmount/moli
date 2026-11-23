use moli_core::page::RendererSyntheticWheelGesture;
use std::time::Duration;
use tokio::time::Instant;

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
            Ok(ScrollSegment { dx, dy, duration })
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
    let admission_queue = conn
        .synthetic_scroll_admissions_for_owner(&owner)
        .ok_or_else(PendingInputCommandStartError::no_document_loaded)?;
    // Reserve the first turn during synchronous command admission. Repeats
    // reserve a new turn after their delay so other sessions can make progress.
    let first_admission = admission_queue.reserve();
    let x = params.x;
    let y = params.y;
    let access = conn.protocol_state_access();
    Ok(Some(
        crate::conn::PreparedCommandWorkflow::new(move |mut context| async move {
            let mut driver = ProtocolWorkflowDriver::new(access);
            let mut first_admission = Some(first_admission);
            let result = async {
                for repeat in 0..=repeat_count {
                    if repeat != 0 {
                        tokio::time::sleep(repeat_delay).await;
                    }
                    let mut admission = first_admission
                        .take()
                        .unwrap_or_else(|| admission_queue.reserve());
                    admission.wait().await;
                    let gesture = RendererSyntheticWheelGesture::default();
                    let mut first = true;
                    let mut cancelable = true;
                    let mut started = Instant::now();
                    for segment in &segments {
                        let mut sent = (0.0, 0.0);
                        loop {
                            let elapsed = started.elapsed();
                            let tick = (elapsed.as_secs_f64() / WHEEL_INTERVAL.as_secs_f64())
                                .floor()
                                + 1.0;
                            let next = WHEEL_INTERVAL.mul_f64(tick).min(segment.duration);
                            tokio::time::sleep_until(started + next).await;
                            let elapsed = started.elapsed();
                            let position = segment.position_at(elapsed);
                            let wheel = SyntheticWheel {
                                owner: owner.clone(),
                                page: page.clone(),
                                x,
                                y,
                                delta_x: position.0 - sent.0,
                                delta_y: position.1 - sent.1,
                                cancelable,
                                gesture: gesture.clone(),
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
                            if first {
                                // Only an unconsumed Began wheel permits later
                                // wheels to become non-blocking/non-cancelable.
                                cancelable = completed.wheel_canceled;
                                first = false;
                            }
                            sent = position;
                            if elapsed >= segment.duration {
                                break;
                            }
                        }
                        started += segment.duration;
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
    gesture: RendererSyntheticWheelGesture,
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
        let intercept_drags = drag::intercept_drags_enabled(conn, &self.owner);
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
                    intercept_drags,
                    self.gesture,
                )
            },
        )
        .map_err(devtools_error_from_input_start_error)
    }
}

struct ScrollSegment {
    dx: f64,
    dy: f64,
    duration: Duration,
}

impl ScrollSegment {
    fn position_at(&self, elapsed: Duration) -> (f64, f64) {
        if elapsed >= self.duration {
            (self.dx, self.dy)
        } else {
            let progress = elapsed.as_secs_f64() / self.duration.as_secs_f64();
            (self.dx * progress, self.dy * progress)
        }
    }
}

#[cfg(test)]
#[test]
fn scroll_gesture_progress_catches_up_after_processing_and_finishes_exactly() {
    let segment = ScrollSegment {
        dx: 0.0,
        dy: 300.0,
        duration: Duration::from_millis(375),
    };
    let first = segment.position_at(Duration::from_millis(16));
    let next = segment.position_at(Duration::from_millis(80));
    assert!((first.1 - 12.8).abs() < 0.00001);
    assert!((next.1 - first.1 - 51.2).abs() < 0.00001);
    assert_eq!(
        segment.position_at(Duration::from_millis(500)),
        (0.0, 300.0)
    );
    let zero = ScrollSegment {
        dx: 1e-30,
        dy: 0.0,
        duration: Duration::ZERO,
    };
    assert_eq!(zero.position_at(Duration::ZERO), (1e-30, 0.0));
}
