//! Typed access to the request phase and transitions that do not run script.
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum XhrReadyState {
    Unsent = 0,
    Opened = 1,
    HeadersReceived = 2,
    Loading = 3,
    Done = 4,
}

pub(super) fn xhr_ready_state(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> XhrReadyState {
    match xhr_state_number_property(scope, xhr, XHR_READY_STATE_SLOT).unwrap_or(0.0) as u32 {
        1 => XhrReadyState::Opened,
        2 => XhrReadyState::HeadersReceived,
        3 => XhrReadyState::Loading,
        4 => XhrReadyState::Done,
        _ => XhrReadyState::Unsent,
    }
}

pub(super) fn set_xhr_ready_state(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    state: XhrReadyState,
) {
    set_xhr_state_number(scope, xhr, XHR_READY_STATE_SLOT, f64::from(state as u8));
}

/// Sending starts before a host task exists and before loadstart can run script.
pub(crate) fn begin_xhr_send(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, true);
    set_xhr_state_bool(scope, xhr, XHR_ABORTED_SLOT, false);
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);
}

pub(super) fn clear_xhr_active_request(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);
    set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, false);
}
