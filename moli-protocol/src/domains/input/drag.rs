use super::*;
use crate::conn::build_event;
use moli_core::page::RendererInterceptedDrag;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetInterceptDragsParams {
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
    if conn
        .with_target_devtools_session_state_for_session_mut(cmd.session_id, |state| {
            state.page_session_state.input_intercept_drags_enabled = params.enabled;
        })
        .is_none()
    {
        return CommandOutputPlan::error(-32000, "NoBrowserContext");
    }
    CommandOutputPlan::success()
}

pub(super) fn intercept_drags_enabled(conn: &CdpConnection, owner: &CommandOwnerScope) -> bool {
    conn.target_page_session_state_for_owner(owner)
        .is_some_and(|state| state.input_intercept_drags_enabled)
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
                // Blink serializes DragOperationEvery as -1. Keep the store's
                // supported copy/link/move bits internal to the renderer.
                "dragOperationsMask": if drag.drag_operations_mask == 19 {
                    -1
                } else {
                    drag.drag_operations_mask
                },
            }
        }),
        session_id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_interception_serializes_chromium_masks_without_mutating_renderer_operations() {
        for (mask, wire_mask) in [(19, -1), (-1, -1), (17, 17), (1, 1), (0, 0)] {
            let drag = RendererInterceptedDrag {
                items: Vec::new(),
                drag_operations_mask: mask,
            };
            let event = drag_intercepted_event(Some("drag-session"), &drag)
                .into_wire_message()
                .expect("wire drag event");
            assert_eq!(event["method"], "Input.dragIntercepted");
            assert_eq!(event["sessionId"], "drag-session");
            assert_eq!(event["params"]["data"]["dragOperationsMask"], wire_mask);
            assert_eq!(drag.drag_operations_mask, mask);
        }
    }
}
