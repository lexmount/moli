use super::helpers::{
    same_origin_window_receiver, window_child_context_handle, window_current_dispatch_scope,
    window_hidden_value, window_host_ptr, window_receiver,
};
use super::*;

pub(crate) fn window_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    if let Some(value) = super::values::window_length_value(scope, receiver) {
        rv.set(value);
    }
}

pub(in crate::context_bootstrap) fn window_frame_element_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = same_origin_window_receiver(scope, &args) else {
        return;
    };
    let Some(host_ptr) = window_host_ptr(scope, receiver) else {
        rv.set_null();
        return;
    };
    let Some(handle) = window_current_dispatch_scope(scope, receiver, unsafe { &*host_ptr })
        .and_then(|owner| owner.child_window())
    else {
        rv.set_null();
        return;
    };
    let Some(owner_context) =
        crate::native_bridge::node_owner_document_relevant_context(scope, host_ptr, handle)
    else {
        rv.set_null();
        return;
    };
    // The container belongs to the parent Document. Authorizing this Window
    // does not grant access to that Document, including for sandboxed children
    // and one-sided document.domain changes. Use the getter's current realm.
    let accessing_context = scope.get_current_context();
    if !crate::native_bridge::window_contexts_allow_access(scope, accessing_context, owner_context)
    {
        rv.set_null();
        return;
    }
    if let Some(value) = window_hidden_value(scope, receiver, WINDOW_FRAME_ELEMENT_SLOT) {
        rv.set(value);
        return;
    }
    let scope = &mut v8::ContextScope::new(scope, owner_context);
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
    let Some(receiver) = same_origin_window_receiver(scope, &args) else {
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
    let Some(receiver) = same_origin_window_receiver(scope, &args) else {
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
