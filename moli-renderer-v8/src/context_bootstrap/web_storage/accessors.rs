use super::helpers::storage_access_allows_web_storage_for_window;
use super::*;
fn window_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let window = args.this();
    super::super::window_receiver::require_same_origin_window_receiver(scope, window, false)
        .then_some(window)
}

pub(in crate::context_bootstrap) fn window_local_storage_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(window) = window_receiver(scope, &args) else {
        return;
    };
    if !storage_access_allows_web_storage_for_window(scope, window) {
        throw_web_storage_security_error(scope, "localStorage");
        return;
    }
    match ensure_storage_runtime_state_for_window(scope, window, WINDOW_LOCAL_STORAGE_SLOT, "local")
    {
        Some(storage) => rv.set(storage.into()),
        None => rv.set(v8::undefined(scope).into()),
    }
}

pub(in crate::context_bootstrap) fn window_session_storage_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(window) = window_receiver(scope, &args) else {
        return;
    };
    if !storage_access_allows_web_storage_for_window(scope, window) {
        throw_web_storage_security_error(scope, "sessionStorage");
        return;
    }
    match ensure_storage_runtime_state_for_window(
        scope,
        window,
        WINDOW_SESSION_STORAGE_SLOT,
        "session",
    ) {
        Some(storage) => rv.set(storage.into()),
        None => rv.set(v8::undefined(scope).into()),
    }
}

fn throw_web_storage_security_error(scope: &mut v8::PinScope<'_, '_>, _property: &str) {
    crate::native_bridge::throw_dom_exception(
        scope,
        "SecurityError",
        18,
        "Access to WebStorage is denied for this document's storage origin.",
    );
}
