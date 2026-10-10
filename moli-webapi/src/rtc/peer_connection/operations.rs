//! Integration with the host's native peer connection operations chain.

#[derive(Clone, Copy)]
pub enum Kind {
    CreateOffer,
    SetLocalDescription,
    ReplaceTrack,
}

pub type ConnectionOperation = for<'s, 'p> fn(&mut v8::PinScope<'s, 'p>, v8::Local<'s, v8::Object>);
pub type EnqueueOperation = for<'s, 'p> fn(
    &mut v8::PinScope<'s, 'p>,
    v8::Local<'s, v8::Object>,
    Kind,
    v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Promise>>;
pub type OperationPayload = for<'s, 'p> fn(
    &mut v8::PinScope<'s, 'p>,
    v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value>;
pub type SettleOperation =
    for<'s, 'p> fn(&mut v8::PinScope<'s, 'p>, v8::Local<'s, v8::Object>, v8::Local<'s, v8::Value>);

#[derive(Clone, Copy)]
pub struct OperationsRuntime {
    pub initialize: ConnectionOperation,
    pub enqueue: EnqueueOperation,
    pub payload: OperationPayload,
    pub resolve: SettleOperation,
    pub reject: SettleOperation,
    pub close: ConnectionOperation,
}

pub fn install_runtime(scope: &mut v8::PinScope<'_, '_>, runtime: OperationsRuntime) {
    scope.get_current_context().set_slot(runtime.into());
}

fn runtime(scope: &mut v8::PinScope<'_, '_>) -> OperationsRuntime {
    *scope
        .get_current_context()
        .get_slot::<OperationsRuntime>()
        .expect("install the operations runtime before invoking RTC bindings")
}

pub(super) fn initialize<'s>(scope: &mut v8::PinScope<'s, '_>, pc: v8::Local<'s, v8::Object>) {
    (runtime(scope).initialize)(scope, pc);
}

pub(super) fn enqueue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
    kind: Kind,
    payload: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Promise>> {
    (runtime(scope).enqueue)(scope, pc, kind, payload)
}

pub(super) fn payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    (runtime(scope).payload)(scope, request)
}

pub(super) fn resolve<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    (runtime(scope).resolve)(scope, request, value);
}

pub(super) fn reject<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    (runtime(scope).reject)(scope, request, reason);
}

pub(super) fn close<'s>(scope: &mut v8::PinScope<'s, '_>, pc: v8::Local<'s, v8::Object>) {
    (runtime(scope).close)(scope, pc);
}
