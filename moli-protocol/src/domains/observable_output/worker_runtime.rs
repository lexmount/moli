use crate::conn::{
    BackgroundProtocolEvent, ServiceWorkerRuntimeExceptionSnapshot, monotonic_timestamp_seconds,
};
use crate::devtools_runtime::DevToolsTargetId;
use moli_core::page::RuntimeConsoleMessageSnapshot;

use super::{
    runtime_console_api_called_background_event, runtime_console_message_type_and_text,
    runtime_exception_thrown_background_event,
};

/// Worker output needs its captured target as well as its wire session: a
/// frontend may apply visibility checks before interpreting session routing.
pub(in crate::domains) fn runtime_console_api_called_events(
    session_id: &str,
    target_id: &str,
    messages: &[RuntimeConsoleMessageSnapshot],
) -> Vec<BackgroundProtocolEvent> {
    let target_id = DevToolsTargetId::from(target_id);
    let base_timestamp = monotonic_timestamp_seconds();
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let (console_type, text) = runtime_console_message_type_and_text(&message.message);
            runtime_console_api_called_background_event(
                Some(session_id),
                Some(&target_id),
                console_type,
                text,
                &message.args,
                message.stack.as_deref(),
                message.execution_context_id,
                base_timestamp + ((index + 1) as f64 * 0.000_001),
            )
        })
        .collect()
}

pub(in crate::domains) fn runtime_exception_thrown_events(
    session_id: &str,
    target_id: &str,
    messages: &[ServiceWorkerRuntimeExceptionSnapshot],
    exception_start: usize,
) -> Vec<BackgroundProtocolEvent> {
    let target_id = DevToolsTargetId::from(target_id);
    let base_timestamp = monotonic_timestamp_seconds();
    messages
        .iter()
        .enumerate()
        .map(|(offset, message)| {
            let exception_index = exception_start + offset;
            runtime_exception_thrown_background_event(
                Some(session_id),
                Some(&target_id),
                &message.message.message,
                &message.message.filename,
                message.execution_context_id,
                exception_index,
                base_timestamp + ((offset + 1) as f64 * 0.000_001),
                Some(u64::from(message.message.lineno.saturating_sub(1))),
                Some(u64::from(message.message.colno.saturating_sub(1))),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_console_keeps_the_captured_target_for_frontend_visibility() {
        let messages = [RuntimeConsoleMessageSnapshot {
            execution_context_id: 42,
            message: "log: startup".into(),
            args: Vec::new(),
            stack: None,
        }];
        let events = runtime_console_api_called_events("SID-worker", "TID-worker", &messages);
        let (wire, automation) = events.into_iter().next().unwrap().into_parts();
        assert_eq!(wire["sessionId"], "SID-worker");
        assert_eq!(wire["params"]["executionContextId"], 42);
        let Some(crate::devtools_runtime::AutomationEvent::RuntimeConsoleApiCalled(event)) =
            automation
        else {
            panic!("console output must retain its typed automation source");
        };
        assert_eq!(event.target_id, Some(DevToolsTargetId::from("TID-worker")));
        assert_eq!(event.text, "startup");
        assert_eq!(event.execution_context_id, Some(42));
    }
}
