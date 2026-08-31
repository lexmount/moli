use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetInterceptDragsParams {
    #[serde(default)]
    enabled: bool,
}

pub(super) fn set_intercept_drags_command_output_plan(
    _conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params: SetInterceptDragsParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };
    let _ = params.enabled;
    CommandOutputPlan::error(-32000, SET_INTERCEPT_DRAGS_UNSUPPORTED_MESSAGE)
}

pub(super) fn cancel_dragging_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    if !conn.mutate_target_page_state_for_session(cmd.session_id, |state| {
        state.input_drag_intercepted = false;
    }) {
        return CommandOutputPlan::error(-32000, "NoBrowserContext");
    }
    CommandOutputPlan::success()
}
