//! Signaling state shared with the host's connection event delivery.

use moli_v8_util::get_private_value;

pub const CURRENT_LOCAL: &str = "__moliRtcPeerConnectionCurrentLocalDescription";
pub const PENDING_LOCAL: &str = "__moliRtcPeerConnectionPendingLocalDescription";
pub const CURRENT_REMOTE: &str = "__moliRtcPeerConnectionCurrentRemoteDescription";
pub const PENDING_REMOTE: &str = "__moliRtcPeerConnectionPendingRemoteDescription";
pub const HANDLER: &str = "__moliRtcOnSignalingStateChange";

pub type DispatchStateChange =
    for<'s, 'p> fn(&mut v8::PinScope<'s, 'p>, v8::Local<'s, v8::Object>) -> bool;

#[derive(Clone, Copy)]
pub struct SignalingRuntime {
    pub dispatch_state_change: DispatchStateChange,
}

pub fn install_runtime(scope: &mut v8::PinScope<'_, '_>, runtime: SignalingRuntime) {
    scope.get_current_context().set_slot(runtime.into());
}

pub(super) fn dispatch_state_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> bool {
    let dispatch = scope
        .get_current_context()
        .get_slot::<SignalingRuntime>()
        .expect("install the signaling runtime before invoking RTC bindings")
        .dispatch_state_change;
    dispatch(scope, pc)
}

pub fn local_description<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    let pending = get_private_value(scope, pc, PENDING_LOCAL).expect("pending local description");
    if pending.is_null() {
        get_private_value(scope, pc, CURRENT_LOCAL).expect("current local description")
    } else {
        pending
    }
}
