use super::super::events::{
    xhr_dispatch_progress_event, xhr_fire_readystatechange, xhr_is_aborted,
};
use super::super::*;

pub(crate) fn apply_xhr_streaming_failure(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    internal_id: u64,
) {
    if super::progress::xhr_stream_is_current(scope, xhr, internal_id) {
        apply_xhr_failure(scope, xhr);
    }
}

pub(crate) fn apply_xhr_failure(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    super::cancel_xhr_timeout(scope, xhr);
    super::clear_xhr_progress_throttle(scope, xhr);
    super::clear_xhr_timeout_start(scope, xhr);
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);
    set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, false);
    if xhr_is_aborted(scope, xhr) {
        return;
    }
    super::reset_xhr_response_for_request_error(scope, xhr);
    set_xhr_state_number(scope, xhr, XHR_READY_STATE_SLOT, 4.0);
    xhr_fire_readystatechange(scope, xhr, 4);
    if scope.is_execution_terminating() {
        return;
    }
    if xhr_is_aborted(scope, xhr) {
        return;
    }
    super::super::upload::dispatch_xhr_upload_error_if_in_progress(scope, xhr, "error");
    xhr_dispatch_progress_event(scope, xhr, "error", 0.0, 0.0);
    if scope.is_execution_terminating() {
        return;
    }
    xhr_dispatch_progress_event(scope, xhr, "loadend", 0.0, 0.0);
}

/// Apply the synchronous XHR request-error state without dispatching async
/// progress events. The caller must throw the corresponding DOMException after
/// recording the browser-visible failure.
pub(crate) fn apply_synchronous_xhr_failure(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    super::cancel_xhr_timeout(scope, xhr);
    super::clear_xhr_progress_throttle(scope, xhr);
    super::clear_xhr_timeout_start(scope, xhr);
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);
    set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, false);
    super::reset_xhr_response_for_request_error(scope, xhr);
    set_xhr_state_number(scope, xhr, XHR_READY_STATE_SLOT, 4.0);
}

pub(crate) fn throw_synchronous_xhr_failure(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    request_url: &str,
    exception_name: &str,
) {
    let exception_message =
        format!("Failed to execute 'send' on 'XMLHttpRequest': Failed to load '{request_url}'.");
    apply_synchronous_xhr_failure(scope, xhr);
    crate::context_bootstrap::throw_dom_exception_value(scope, &exception_message, exception_name);
}
