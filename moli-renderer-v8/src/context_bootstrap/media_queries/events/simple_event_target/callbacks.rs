use super::*;

pub(crate) fn simple_event_target_add_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let target = args.this();
    let slot_name = simple_event_target_slot_name(scope, target);
    let Some(slot_name) = slot_name.as_deref() else {
        return;
    };
    simple_object_event_target_add_listener(scope, &args, slot_name);
}

pub(crate) fn simple_event_target_remove_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let target = args.this();
    let slot_name = simple_event_target_slot_name(scope, target);
    let Some(slot_name) = slot_name.as_deref() else {
        return;
    };
    simple_object_event_target_remove_listener(scope, &args, slot_name);
}

pub(crate) fn simple_event_target_dispatch_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let target = args.this();
    let slot_name = simple_event_target_slot_name(scope, target);
    let Some(slot_name) = slot_name.as_deref() else {
        rv.set(v8::Boolean::new(scope, true).into());
        return;
    };
    simple_object_event_target_dispatch(scope, &args, slot_name, &mut rv);
}
