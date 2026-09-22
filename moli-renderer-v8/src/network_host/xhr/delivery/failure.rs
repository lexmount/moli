use super::super::events::{
    xhr_dispatch_progress_event, xhr_fire_readystatechange, xhr_is_aborted,
};
use super::super::send::xhr_open_generation_changed;
use super::super::upload::dispatch_xhr_upload_error_if_in_progress;
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
    apply_xhr_request_error(scope, xhr, "error");
}

pub(crate) fn apply_xhr_document_abort(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    apply_xhr_request_error(scope, xhr, "abort");
}

fn apply_xhr_request_error(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    event: &str,
) {
    super::clear_xhr_request_timers(scope, xhr);
    clear_xhr_active_request(scope, xhr);
    if xhr_is_aborted(scope, xhr) {
        return;
    }
    let open_generation =
        xhr_state_number_property(scope, xhr, XHR_OPEN_GENERATION_SLOT).unwrap_or(0.0);
    super::prepare_xhr_request_error(scope, xhr);
    xhr_fire_readystatechange(scope, xhr, XhrReadyState::Done);
    if scope.is_execution_terminating()
        || xhr_is_aborted(scope, xhr)
        || xhr_open_generation_changed(scope, xhr, open_generation)
    {
        return;
    }
    dispatch_xhr_upload_error_if_in_progress(scope, xhr, event);
    if scope.is_execution_terminating()
        || xhr_is_aborted(scope, xhr)
        || xhr_open_generation_changed(scope, xhr, open_generation)
    {
        return;
    }
    xhr_dispatch_progress_event(scope, xhr, event, 0.0, 0.0);
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
    super::clear_xhr_request_timers(scope, xhr);
    clear_xhr_active_request(scope, xhr);
    super::prepare_xhr_request_error(scope, xhr);
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
