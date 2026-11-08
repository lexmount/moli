use super::super::events::{
    xhr_dispatch_progress_event, xhr_fire_readystatechange, xhr_is_aborted,
};
use super::super::*;

pub(crate) fn apply_xhr_failure(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    super::clear_xhr_request_timers(scope, xhr);
    clear_xhr_active_request(scope, xhr);
    if xhr_is_aborted(scope, xhr) {
        return;
    }
    super::prepare_xhr_request_error(scope, xhr);
    xhr_fire_readystatechange(scope, xhr, XhrReadyState::Done);
    if scope.is_execution_terminating() {
        return;
    }
    if xhr_is_aborted(scope, xhr) {
        return;
    }
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
