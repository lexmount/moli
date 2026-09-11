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

/// Applies the response reset shared by the XMLHttpRequest request-error
/// steps before the terminal event sequence is dispatched.
///
/// Streaming delivery makes partial response state observable during LOADING,
/// so abort, failure, and timeout must all clear the same complete response
/// surface rather than only resetting status and URL.
pub(crate) fn reset_xhr_response_for_request_error(
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
