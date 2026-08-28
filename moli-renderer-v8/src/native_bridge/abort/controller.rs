use super::{
    ABORT_CONTROLLER_SIGNAL_SLOT, AbortStore, abort_error_value, create_signal_with_prototype,
};
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
    let global = scope.get_current_context().global(scope);
    let Some(signal_ctor) = global
        .get(scope, v8str(scope, "AbortSignal").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set(args.this().into());
        return;
    };
    let Some(signal) =
        create_signal_with_prototype(scope, signal_ctor, unsafe { &mut *host_ptr }, false, None)
    else {
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
    if AbortStore::controller_id_from_object(scope, controller).is_none() {
        rv.set_undefined();
        return;
    }
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
    if AbortStore::signal_id_from_object(scope, signal).is_none() {
        rv.set_undefined();
        return;
    }
    if AbortStore::signal_aborted_from_object(scope, signal) {
        rv.set_undefined();
        return;
    }
    host.abort_signal(scope, signal, reason);
    rv.set_undefined();
}
