//! Hooks for host-owned RTP objects associated with a peer connection.

pub const NEGOTIATION_HANDLER: &str = "__moliRtcOnNegotiationNeeded";

pub type ConnectionOperation = for<'s, 'p> fn(
    &mut v8::PinScope<'s, 'p>,
    v8::Local<'s, v8::Object>,
);
pub type InitializeConnection =
    for<'s, 'p> fn(&mut v8::PinScope<'s, 'p>, v8::Local<'s, v8::Object>) -> Option<()>;
pub type CreateOffer = for<'s, 'p> fn(
    &mut v8::PinScope<'s, 'p>,
    v8::Local<'s, v8::Object>,
    bool,
) -> Option<String>;

#[derive(Clone, Copy)]
pub struct RtpRuntime {
    pub initialize_pc: InitializeConnection,
    pub close: ConnectionOperation,
    pub create_offer: CreateOffer,
}

pub fn install_runtime(scope: &mut v8::PinScope<'_, '_>, runtime: RtpRuntime) {
    scope.get_current_context().set_slot(runtime.into());
}

fn runtime(scope: &mut v8::PinScope<'_, '_>) -> RtpRuntime {
    *scope.get_current_context().get_slot::<RtpRuntime>()
        .expect("install the RTP runtime before invoking RTC bindings")
}

pub(super) fn initialize_pc<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> Option<()> {
    (runtime(scope).initialize_pc)(scope, pc)
}

pub(super) fn close<'s>(scope: &mut v8::PinScope<'s, '_>, pc: v8::Local<'s, v8::Object>) {
    (runtime(scope).close)(scope, pc);
}

pub(super) fn create_offer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
    data: bool,
) -> Option<String> {
    (runtime(scope).create_offer)(scope, pc, data)
}

pub(super) fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, object)
        .expect("RTCPeerConnection receiver")
}
