use super::{ABORT_CONTROLLER_SIGNAL_SLOT, AbortStore, abort_error_value, create_signal};
use crate::util::{context_host_ptr_from_global_bridge, get_private_value, v8str};

pub(crate) fn abort_controller_constructor_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        scope.throw_exception(v8::Exception::type_error(
            scope,
            v8str(
                scope,
                "Failed to construct 'AbortController': Please use the 'new' operator.",
            ),
        ));
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let Some(signal) = create_signal(scope, unsafe { &mut *host_ptr }, false, None) else {
        rv.set(args.this().into());
        return;
    };
    unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .abort
        .init_controller(scope, args.this(), signal);
    rv.set(args.this().into());
}

pub(crate) fn abort_controller_signal_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match get_private_value(scope, args.this(), ABORT_CONTROLLER_SIGNAL_SLOT) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(crate) fn abort_controller_abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let controller = args.this();
    let Some(controller_id) = AbortStore::controller_id_from_object(scope, controller) else {
        rv.set_undefined();
        return;
    };
    let Some(signal) = get_private_value(scope, controller, ABORT_CONTROLLER_SIGNAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let reason = if args.length() > 0 && !args.get(0).is_undefined() {
        args.get(0)
    } else {
        abort_error_value(scope)
    };
    let host = unsafe { &mut *host_ptr };
    let Some(signal_id) = host
        .native_bridge_mut()
        .abort
        .controllers
        .get(&controller_id)
        .copied()
    else {
        rv.set_undefined();
        return;
    };
    if host
        .native_bridge_mut()
        .abort
        .signal_state(signal_id)
        .is_some_and(|state| state.aborted)
    {
        rv.set_undefined();
        return;
    }
    crate::native_bridge::abort::abort_signal(scope, signal, reason);
    rv.set_undefined();
}
