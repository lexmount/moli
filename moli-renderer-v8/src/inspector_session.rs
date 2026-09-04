//! Synchronous Inspector session commands shared by Page and Worker owners.
//!
//! V8 enables Runtime with a 200-frame exception capture limit. Use a ten-frame
//! default to bound the extra cost of constructing Error objects, while leaving
//! explicit frontend limits and restored session settings under V8's control.

use serde::Deserialize;
use serde_json::{Value, json};

const STACK_CAPTURE_CALL_ID: i32 = -1;
const DEFAULT_RUNTIME_STACK_CAPTURE_DEPTH: i32 = 10;

#[cfg(test)]
pub(crate) mod tests;

/// Adapts the owner's response routing without changing its notification stream.
pub(crate) trait InspectorSessionOutput {
    /// Only for synchronous, non-reentrant Inspector settings. Capture this
    /// dispatch's response before frontend routing, even if a frontend uses the
    /// same ID. Preserve queued responses, callbacks, and notifications.
    fn capture_internal_response(&self, call_id: i32, dispatch: impl FnOnce()) -> Option<Value>;
}

/// The caller establishes its usual Inspector isolate/microtask scope.
pub(crate) fn dispatch_with_runtime_defaults(
    session: &v8::inspector::V8InspectorSession,
    raw_json: &str,
    output: &impl InspectorSessionOutput,
) -> Result<(), String> {
    let enabling_runtime = serde_json::from_str::<Value>(raw_json).is_ok_and(|message| {
        message.get("method").and_then(Value::as_str) == Some("Runtime.enable")
    });
    let runtime_was_disabled = enabling_runtime && !runtime_enabled(session)?;
    session.dispatch_protocol_message(v8::inspector::StringView::from(raw_json.as_bytes()));

    // V8 owns enable/restore state. Repeated or failed enables must not reset
    // explicit frontend limits. The setter runs synchronously without JS or
    // microtasks, so a scoped response capture can safely reuse a fixed ID.
    if runtime_was_disabled && runtime_enabled(session)? {
        let request = json!({
            "id": STACK_CAPTURE_CALL_ID,
            "method": "Runtime.setMaxCallStackSizeToCapture",
            "params": {"size": DEFAULT_RUNTIME_STACK_CAPTURE_DEPTH}
        })
        .to_string();
        let response = output
            .capture_internal_response(STACK_CAPTURE_CALL_ID, || {
                session
                    .dispatch_protocol_message(v8::inspector::StringView::from(request.as_bytes()));
            })
            .ok_or("Runtime stack-capture default produced no Inspector response")?;
        if response != json!({"id": STACK_CAPTURE_CALL_ID, "result": {}}) {
            return Err(format!(
                "Runtime stack-capture default returned an unexpected response: {response}"
            ));
        }
    }
    Ok(())
}

fn runtime_enabled(session: &v8::inspector::V8InspectorSession) -> Result<bool, String> {
    // V8 owns this serialized state and uses these same fields on reconnect.
    // Decode it only for Runtime.enable, never on the Runtime.evaluate hot path.
    #[derive(Deserialize)]
    struct SessionState {
        #[serde(rename = "Runtime")]
        runtime: RuntimeState,
    }
    #[derive(Deserialize)]
    struct RuntimeState {
        #[serde(default, rename = "runtimeEnabled")]
        enabled: bool,
    }

    let state = v8::crdtp::cbor_to_json(&session.state())
        .ok_or("Inspector session state is not valid CBOR")?;
    let state: SessionState = serde_json::from_slice(&state)
        .map_err(|error| format!("invalid Inspector Runtime state: {error}"))?;
    Ok(state.runtime.enabled)
}
