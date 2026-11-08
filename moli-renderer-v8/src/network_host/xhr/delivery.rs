mod abort;
mod failure;
mod pending;
mod progress;
mod response;
mod timeout;

use super::*;

pub(crate) use self::abort::{apply_xhr_abort, finish_xhr_abort};
pub(crate) use self::failure::{apply_xhr_failure, throw_synchronous_xhr_failure};
pub(super) use self::pending::{queue_xhr_failure_delivery, queue_xhr_response_delivery};
pub(in crate::network_host::xhr) use self::progress::clear_xhr_progress_throttle;
pub(super) use self::response::apply_xhr_response_pending_body;
pub(crate) use self::response::{
    apply_xhr_response, apply_xhr_response_body_source,
    apply_xhr_response_body_source_with_status_text, apply_xhr_streaming_response_body_source,
    apply_xhr_streaming_response_chunk, apply_xhr_streaming_response_head,
};
pub(crate) use self::timeout::apply_xhr_timeout;
pub(super) use self::timeout::{
    cancel_xhr_timeout, clear_xhr_timeout_start, mark_xhr_timeout_start,
    reschedule_xhr_timeout_after_timeout_change, schedule_xhr_timeout,
};

/// Clear the complete response surface before a new request or terminal event.
/// Streaming delivery may already have exposed partial response data.
pub(in crate::network_host::xhr) fn reset_xhr_response(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    set_xhr_state_number(scope, xhr, XHR_STATUS_SLOT, 0.0);
    set_xhr_state_string(scope, xhr, XHR_STATUS_TEXT_SLOT, "");
    set_xhr_state_string(scope, xhr, XHR_RESPONSE_TEXT_SLOT, "");
    set_xhr_state_string(scope, xhr, XHR_RESPONSE_URL_SLOT, "");
    set_xhr_state_string(scope, xhr, XHR_RESPONSE_HEADERS_SLOT, "[]");
    let empty_response: v8::Local<'_, v8::Value> = v8_string(scope, "")
        .map(|value| value.into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_xhr_state_value(scope, xhr, XHR_RESPONSE_SLOT, empty_response);
    set_xhr_state_value(scope, xhr, XHR_RESPONSE_XML_SLOT, v8::null(scope).into());
}

/// Resource cleanup is separate from event dispatch and request identity checks.
pub(in crate::network_host::xhr) fn clear_xhr_request_timers(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    cancel_xhr_timeout(scope, xhr);
    clear_xhr_progress_throttle(scope, xhr);
    clear_xhr_timeout_start(scope, xhr);
}

/// Prepare request-error state without running script. Each caller retains its
/// own event sequence and rechecks state where its algorithm requires it.
fn prepare_xhr_request_error(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    reset_xhr_response(scope, xhr);
    set_xhr_ready_state(scope, xhr, XhrReadyState::Done);
}
