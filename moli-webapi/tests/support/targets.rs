use moli_v8_util::{new_null_prototype_object, set_private_value};
use moli_webapi::event_target::{self, EventTargetRuntime, SIMPLE_EVENT_TARGET_LISTENERS_SLOT};

// Only the binding-to-host handoff is exercised here. Renderer tests retain
// coverage of listener registration, dispatch, wrapper worlds and lifetime.
pub fn install(scope: &mut v8::PinScope<'_, '_>) {
    event_target::install_runtime(
        scope,
        EventTargetRuntime {
            initialize,
            set_handler,
        },
    );
}

fn initialize<'s>(scope: &mut v8::PinScope<'s, '_>, target: v8::Local<'s, v8::Object>) {
    let listeners = new_null_prototype_object(scope);
    set_private_value(
        scope,
        target,
        SIMPLE_EVENT_TARGET_LISTENERS_SLOT,
        listeners.into(),
    );
}

fn set_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    _event_type: &str,
    slot: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let value = if value.is_object() {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, target, slot, value);
}
