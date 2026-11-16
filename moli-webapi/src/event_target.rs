//! EventTarget capabilities supplied by the embedding dispatcher.
//! Binding values stay in V8; listener ownership and dispatch stay with the host.

use std::rc::Rc;

pub const SIMPLE_EVENT_TARGET_LISTENERS_SLOT: &str = "__moliEventTargetListeners";

pub struct EventTargetRuntime {
    pub initialize: for<'s, 'i> fn(&mut v8::PinScope<'s, 'i>, v8::Local<'s, v8::Object>),
    pub set_handler: for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        v8::Local<'s, v8::Object>,
        &str,
        &str,
        v8::Local<'s, v8::Value>,
    ),
}

pub fn install_runtime(scope: &mut v8::PinScope<'_, '_>, runtime: EventTargetRuntime) {
    scope.get_current_context().set_slot(Rc::new(runtime));
}

pub(crate) fn initialize_simple_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) {
    let initialize = scope
        .get_current_context()
        .get_slot::<EventTargetRuntime>()
        .expect("install EventTarget capabilities before exposing bindings")
        .initialize;
    initialize(scope, target);
}

pub(crate) fn simple_object_event_set_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &str,
    slot: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let set_handler = scope
        .get_current_context()
        .get_slot::<EventTargetRuntime>()
        .expect("install EventTarget capabilities before exposing bindings")
        .set_handler;
    set_handler(scope, target, event_type, slot, value);
}
