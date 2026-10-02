use super::helpers::{
    window_child_context_handle, window_hidden_value, window_host_ptr, window_receiver,
};
use super::*;
use crate::native_bridge::WindowEnvironmentSettings;

pub(in crate::context_bootstrap) fn window_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    let Some(host_ptr) = window_host_ptr(scope, receiver) else {
        rv.set(v8::Number::new(scope, 0.0).into());
        return;
    };
    let runtime = unsafe { &mut *host_ptr };
    let count = if let Some(handle) = window_child_context_handle(scope, receiver) {
        runtime.child_browsing_context_child_frame_count(handle)
    } else {
        runtime.child_browsing_context_count()
    };
    rv.set(v8::Number::new(scope, count as f64).into());
}

pub(in crate::context_bootstrap) fn window_frame_element_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    if let Some(value) = window_hidden_value(scope, receiver, WINDOW_FRAME_ELEMENT_SLOT) {
        rv.set(value);
        return;
    }
    let Some(handle) = window_child_context_handle(scope, receiver) else {
        rv.set_null();
        return;
    };
    let Some(host_ptr) = window_host_ptr(scope, receiver) else {
        rv.set_null();
        return;
    };
    match unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, handle)
    {
        Some(frame_element) => rv.set(frame_element.into()),
        None => rv.set_null(),
    }
}

pub(in crate::context_bootstrap) fn window_credentialless_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    let Some(handle) = window_child_context_handle(scope, receiver) else {
        rv.set_bool(false);
        return;
    };
    let Some(host_ptr) = window_host_ptr(scope, receiver) else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(unsafe { &*host_ptr }.child_browsing_context_document_credentialless(handle));
}

pub(in crate::context_bootstrap) fn window_cross_origin_isolated_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    let Some(host_ptr) = window_host_ptr(scope, receiver) else {
        rv.set_bool(false);
        return;
    };
    let host = unsafe { &*host_ptr };
    let isolated = window_child_context_handle(scope, receiver)
        .and_then(|handle| host.child_browsing_context_policy_container_snapshot(handle))
        .map(|policy| policy.cross_origin_isolated)
        .unwrap_or_else(|| host.cross_origin_isolated());
    rv.set_bool(isolated);
}

pub(in crate::context_bootstrap) fn window_document_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    let Some(host_ptr) = window_host_ptr(scope, receiver) else {
        rv.set_null();
        return;
    };
    let Some(context) = receiver.get_creation_context(scope) else {
        rv.set_null();
        return;
    };
    // A borrowed getter runs in its function's realm. Resolve both settings
    // and wrappers in the receiver's realm, including a retained old Window.
    let scope = &mut v8::ContextScope::new(scope, context);
    let handle = if let Some(settings) = WindowEnvironmentSettings::for_current_realm(scope) {
        let window = context.global(scope);
        if let Some(document) = window_hidden_value(
            scope,
            window,
            WindowEnvironmentSettings::DOCUMENT_WRAPPER_SLOT,
        ) {
            rv.set(document);
            return;
        }
        settings.document_handle()
    } else if let Some(handle) = window_child_context_handle(scope, receiver) {
        // During bootstrap the Document wrapper can precede settings binding.
        // Only this exact live Window may follow the child's current route.
        let host = unsafe { &*host_ptr };
        if host
            .window_execution_context_identity_for_access_check(context)
            .is_none_or(|identity| !host.window_execution_context_identity_is_current(identity))
        {
            rv.set_null();
            return;
        }
        match unsafe { &mut *host_ptr }.child_browsing_context_document_wrapper(scope, handle) {
            Some(document) => rv.set(document.into()),
            None => rv.set_null(),
        }
        return;
    } else {
        unsafe { &*host_ptr }.document_handle()
    };
    match unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, handle)
    {
        Some(document) => rv.set(document.into()),
        None => rv.set_null(),
    }
}
