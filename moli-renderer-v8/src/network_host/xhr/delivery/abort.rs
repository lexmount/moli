use super::super::events::xhr_dispatch_progress_event;
use super::super::*;

pub(crate) fn finish_xhr_abort(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    let ready_state = xhr_ready_state(scope, xhr);
    let send_flag = xhr_state_bool_property(scope, xhr, XHR_SEND_FLAG_SLOT).unwrap_or(false);
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);

    if (ready_state == XhrReadyState::Opened && send_flag)
        || matches!(
            ready_state,
            XhrReadyState::HeadersReceived | XhrReadyState::Loading
        )
    {
        set_xhr_state_bool(scope, xhr, XHR_ABORTED_SLOT, true);
        set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, false);
        super::prepare_xhr_request_error(scope, xhr);
        super::super::events::xhr_fire_readystatechange(scope, xhr, XhrReadyState::Done);
        super::super::send::dispatch_xhr_upload_abort_if_in_progress(scope, xhr);
        xhr_dispatch_progress_event(scope, xhr, "abort", 0.0, 0.0);
        xhr_dispatch_progress_event(scope, xhr, "loadend", 0.0, 0.0);
    }

    // An abort listener can reopen this object and start another request. Only
    // reset a request that is still DONE after all request-error callbacks.
    if xhr_ready_state(scope, xhr) == XhrReadyState::Done {
        set_xhr_ready_state(scope, xhr, XhrReadyState::Unsent);
        super::reset_xhr_response(scope, xhr);
    }
}

pub(crate) fn apply_xhr_abort(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    super::clear_xhr_request_timers(scope, xhr);
    set_xhr_state_bool(scope, xhr, XHR_ABORTED_SLOT, true);
    clear_xhr_active_request(scope, xhr);
    set_xhr_ready_state(scope, xhr, XhrReadyState::Unsent);
    super::reset_xhr_response(scope, xhr);
    super::super::send::dispatch_xhr_upload_abort_if_in_progress(scope, xhr);
    xhr_dispatch_progress_event(scope, xhr, "abort", 0.0, 0.0);
    xhr_dispatch_progress_event(scope, xhr, "loadend", 0.0, 0.0);
}
