use crate::util::{context_host_ptr_from_window_object, throw_type_error};

fn receiver_is_window(
    scope: &mut v8::PinScope<'_, '_>,
    receiver: v8::Local<'_, v8::Object>,
) -> bool {
    if context_host_ptr_from_window_object(scope, receiver).is_none() {
        return false;
    }
    let global = scope.get_current_context().global(scope);
    if receiver.strict_equals(global.into()) {
        return true;
    }
    receiver
        .get_internal_field(scope, 1)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .and_then(|value| value.number_value(scope))
        .is_some_and(|marker| marker.is_finite() && marker.fract() == 0.0 && marker == 0.0)
}

pub(in crate::context_bootstrap) fn window_obsolete_noop_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_is_window(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set_undefined();
}
