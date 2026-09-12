use super::super::events::xhr_dispatch_progress_event;
use super::super::*;

pub(crate) fn finish_xhr_abort(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    let ready_state =
        xhr_state_number_property(scope, xhr, XHR_READY_STATE_SLOT).unwrap_or(0.0) as u32;
    let send_flag = xhr_state_bool_property(scope, xhr, XHR_SEND_FLAG_SLOT).unwrap_or(false);
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);

    if (ready_state == 1 && send_flag) || matches!(ready_state, 2 | 3) {
        set_xhr_state_bool(scope, xhr, XHR_ABORTED_SLOT, true);
        set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, false);
        set_xhr_state_number(scope, xhr, XHR_READY_STATE_SLOT, 4.0);
        super::reset_xhr_response_for_request_error(scope, xhr);
        super::super::events::xhr_fire_readystatechange(scope, xhr, 4);
        super::super::upload::dispatch_xhr_upload_error_if_in_progress(scope, xhr, "abort");
        xhr_dispatch_progress_event(scope, xhr, "abort", 0.0, 0.0);
        xhr_dispatch_progress_event(scope, xhr, "loadend", 0.0, 0.0);
    }

    // An abort listener can reopen this object and start another request. Only
    // reset a request that is still DONE after all request-error callbacks.
    if xhr_state_number_property(scope, xhr, XHR_READY_STATE_SLOT) == Some(4.0) {
        set_xhr_state_number(scope, xhr, XHR_READY_STATE_SLOT, 0.0);
        super::reset_xhr_response_for_request_error(scope, xhr);
    }
}

pub(crate) fn apply_xhr_abort(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    super::cancel_xhr_timeout(scope, xhr);
    super::clear_xhr_progress_throttle(scope, xhr);
    set_xhr_state_bool(scope, xhr, XHR_ABORTED_SLOT, true);
    set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, false);
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);
    set_xhr_state_number(scope, xhr, XHR_READY_STATE_SLOT, 0.0);
    super::reset_xhr_response_for_request_error(scope, xhr);
    super::super::upload::dispatch_xhr_upload_error_if_in_progress(scope, xhr, "abort");
    xhr_dispatch_progress_event(scope, xhr, "abort", 0.0, 0.0);
    xhr_dispatch_progress_event(scope, xhr, "loadend", 0.0, 0.0);
}
