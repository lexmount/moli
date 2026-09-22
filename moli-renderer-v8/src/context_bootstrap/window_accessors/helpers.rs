use super::*;
use crate::util::get_private_value;
use crate::webidl;

pub(super) fn window_hidden_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    if let Some(value) =
        get_private_value(scope, receiver, slot).filter(|value| !value.is_undefined())
    {
        return Some(value);
    }
    if window_slot_uses_legacy_hidden_fallback(slot)
        && let Some(value) =
            object_own_hidden_value(scope, receiver, slot).filter(|value| !value.is_undefined())
    {
        return Some(value);
    }
    None
}

fn window_slot_uses_legacy_hidden_fallback(slot: &str) -> bool {
    matches!(
        slot,
        WINDOW_SELF_SLOT
            | WINDOW_PARENT_SLOT
            | WINDOW_TOP_SLOT
            | WINDOW_FRAMES_SLOT
            | WINDOW_CUSTOM_ELEMENTS_SLOT
            | WINDOW_PERFORMANCE_SLOT
    )
}

pub(in crate::context_bootstrap) fn window_child_context_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<crate::document_runtime::DomHandle> {
    let global = scope.get_current_context().global(scope);
    let receiver_is_current_global = receiver.strict_equals(global.into());
    let marked_handle = if receiver_is_current_global {
        get_private_value(scope, global, WINDOW_CHILD_CONTEXT_HANDLE_SLOT)
    } else {
        get_private_value(scope, receiver, WINDOW_CHILD_CONTEXT_HANDLE_SLOT)
    }
    .and_then(|value| super::super::navigation_window::dom_handle_from_marker_value(scope, value));
    if marked_handle.is_some() {
        return marked_handle;
    }
    // A callback can run promise reactions from another realm before its
    // legacy dispatch marker is restored. The holder's native realm remains
    // authoritative for WindowProperties, including a top-level holder.
    if let Some(context) = receiver.get_creation_context(scope)
        && let Some(host_ptr) = context_host_ptr_from_context_slot(context)
        && let Some(identity) =
            unsafe { &*host_ptr }.window_execution_context_identity_for_access_check(context)
    {
        return unsafe { &*host_ptr }
            .window_execution_context_identity_is_current(identity)
            .then(|| identity.dispatch_scope().child_window())
            .flatten();
    }
    if let Some(handle) = crate::native_bridge::active_child_window_handle(scope) {
        if receiver_is_current_global {
            return Some(handle);
        }
        let current_context = scope.get_current_context();
        let receiver_context = receiver.get_creation_context(scope);
        if receiver_context == Some(current_context)
            && crate::native_bridge::lightweight_popup_id_from_window(scope, receiver).is_none()
            && crate::native_bridge::cross_origin_lightweight_popup_id(scope, receiver).is_none()
        {
            // V8 can invoke a Window accessor with the current realm's
            // WindowProperties/proxy holder as `this` rather than the global
            // proxy itself. Its creation context is still the exact current
            // realm. The explicit owner-dispatch marker disambiguates that
            // holder from a Window borrowed from another realm.
            return Some(handle);
        }
        let host_ptr = window_host_ptr(scope, receiver);
        let live_window_matches = host_ptr.is_some_and(|host_ptr| {
            unsafe { &*host_ptr }
                .existing_child_browsing_context_window_wrapper(scope, handle)
                .is_some_and(|window| receiver.strict_equals(window.into()))
        });
        if live_window_matches {
            return Some(handle);
        }
    }

    None
}

pub(crate) fn window_host_ptr(
    scope: &mut v8::PinScope<'_, '_>,
    receiver: v8::Local<'_, v8::Object>,
) -> Option<*mut JsContextHost> {
    context_host_ptr_from_window_object(scope, receiver)
        .or_else(|| context_host_ptr_from_global_bridge(scope))
}

pub(super) fn window_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let receiver = args.this();
    if super::super::window_receiver::is_window_receiver(scope, receiver) {
        return Some(receiver);
    }
    webidl::throw_type_error(scope, "Window getter called on incompatible receiver.");
    None
}

pub(crate) fn current_window_style_viewport(
    scope: &mut v8::PinScope<'_, '_>,
    host: &crate::native_bridge::JsContextHost,
) -> crate::style_engine::StyleViewport {
    let global = scope.get_current_context().global(scope);
    window_child_context_handle(scope, global)
        .and_then(|frame_handle| {
            crate::native_bridge::element::iframe_handle_viewport(host, frame_handle)
        })
        .unwrap_or_else(|| host.style_viewport())
}
