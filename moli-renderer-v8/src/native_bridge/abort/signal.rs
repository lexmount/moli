use super::AbortStore;

pub(crate) fn abort_signal_aborted_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(AbortStore::signal_aborted_from_object(scope, args.this()));
}

pub(crate) fn abort_signal_reason_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match AbortStore::signal_reason_from_object(scope, args.this()) {
        Some(reason) => rv.set(reason),
        None => rv.set_undefined(),
    }
}

pub(crate) fn abort_signal_throw_if_aborted_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let signal = args.this();
    if !AbortStore::signal_aborted_from_object(scope, signal) {
        rv.set_undefined();
        return;
    }
    let reason = AbortStore::signal_reason_from_object(scope, signal)
        .unwrap_or_else(|| v8::undefined(scope).into());
    scope.throw_exception(reason);
}
