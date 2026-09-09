use super::*;

pub(crate) fn simple_event_target_add_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(target) = resolve_simple_event_target(scope, args.this()) else {
        return;
    };
    simple_object_event_target_add_listener_for_target(
        scope,
        &args,
        target.identity.listener_owner,
        target.registry,
    );
}

pub(crate) fn simple_event_target_remove_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(target) = resolve_simple_event_target(scope, args.this()) else {
        return;
    };
    simple_object_event_target_remove_listener_in_registry(scope, &args, target.registry);
}

pub(crate) fn simple_event_target_dispatch_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(target) = resolve_simple_event_target(scope, args.this()) else {
        rv.set(v8::Boolean::new(scope, true).into());
        return;
    };
    simple_object_event_target_dispatch(scope, &args, target, &mut rv);
}
