use super::*;
use crate::conn::build_event;
use moli_core::page::RendererInterceptedDrag;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetInterceptDragsParams {
    #[serde(default)]
    enabled: bool,
}

/// Chromium reports a drag that starts while interception is on through
/// `Input.dragIntercepted` and leaves the rest of the drag to the client, which
/// replays it with `Input.dispatchDragEvent`. The flag set here travels with
/// every mouse event to the renderer, which decides whether `dragstart` ran.
pub(super) fn set_intercept_drags_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params: SetInterceptDragsParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };
    if !conn.mutate_target_page_state_for_session(cmd.session_id, |state| {
        state.input_intercept_drags_enabled = params.enabled;
    }) {
        return CommandOutputPlan::error(-32000, "NoBrowserContext");
    }
    CommandOutputPlan::success()
}

pub(super) fn intercept_drags_enabled(conn: &mut CdpConnection, session_id: Option<&str>) -> bool {
    let mut enabled = false;
    conn.mutate_target_page_state_for_session(session_id, |state| {
        enabled = state.input_intercept_drags_enabled;
    });
    enabled
}

/// Builds the `Input.dragIntercepted` event for a drag the renderer started.
pub(super) fn drag_intercepted_event(
    session_id: Option<&str>,
    drag: &RendererInterceptedDrag,
) -> BackgroundProtocolEvent {
    let items: Vec<Value> = drag
        .items
        .iter()
        .map(|item| {
            let mut value = json!({ "mimeType": item.mime_type, "data": item.data });
            if let Some(title) = &item.title {
                value["title"] = json!(title);
            }
            if let Some(base_url) = &item.base_url {
                value["baseURL"] = json!(base_url);
            }
            value
        })
        .collect();
    BackgroundProtocolEvent::immediate(build_event(
        "Input.dragIntercepted",
        json!({
            "data": {
                "items": items,
                "dragOperationsMask": drag.drag_operations_mask,
            }
        }),
        session_id,
    ))
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
