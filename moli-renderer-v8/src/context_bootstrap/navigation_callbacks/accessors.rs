use super::*;
use crate::util::get_private_value;
use crate::webidl;

fn window_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let receiver = args.this();
    super::super::window_receiver::require_same_origin_window_receiver(scope, receiver, false)
        .then_some(receiver)
}

pub(in crate::context_bootstrap) fn window_history_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    match object_window_slot_value(scope, receiver, WINDOW_HISTORY_SLOT)
        .or_else(|| global_window_slot_value(scope, WINDOW_HISTORY_SLOT))
    {
        Some(v) => rv.set(v),
        None => rv.set(v8::undefined(scope).into()),
    }
}

pub(in crate::context_bootstrap) fn window_navigation_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    match object_window_slot_value(scope, receiver, WINDOW_NAVIGATION_SLOT)
        .or_else(|| global_window_slot_value(scope, WINDOW_NAVIGATION_SLOT))
    {
        Some(v) => rv.set(v),
        None => rv.set(v8::undefined(scope).into()),
    }
}

pub(in crate::context_bootstrap) fn window_location_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if crate::native_bridge::CrossOriginWindowAccessor::get_for_receiver(
        scope,
        args.this(),
        crate::native_bridge::CrossOriginWindowProperty::Location,
        &mut rv,
    ) {
        return;
    }
    let Some(receiver) = window_receiver(scope, &args) else {
        return;
    };
    let Some(value) = get_private_value(scope, receiver, WINDOW_LOCATION_SLOT)
        .filter(|v| !v.is_undefined())
        .or_else(|| global_location_slot_value(scope))
    else {
        webidl::throw_type_error(
            scope,
            "Window.location getter called on incompatible receiver.",
        );
        rv.set_null();
        return;
    };
    rv.set(value);
}

pub(in crate::context_bootstrap) fn document_location_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    if crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, receiver)
        .is_err()
    {
        webidl::throw_type_error(
            scope,
            "Document.location getter called on incompatible receiver.",
        );
        return;
    }
    match object_location_slot_value(scope, receiver).filter(|value| !value.is_undefined()) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(in crate::context_bootstrap) fn document_location_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    if crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, receiver)
        .is_err()
    {
        webidl::throw_type_error(
            scope,
            "Document.location setter called on incompatible receiver.",
        );
        return;
    }
    let Some(location) = object_location_slot_value(scope, receiver)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let value = args.get(0);
    let href = match value.is_null_or_undefined() {
        true => String::new(),
        false => match value.to_string(scope) {
            Some(value) => value.to_rust_string_lossy(scope),
            None => return,
        },
    };
    navigate_location_object(scope, location, LocationNavigationKind::Assign, Some(href));
    rv.set_undefined();
}

fn object_location_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    object_window_slot_value(scope, object, WINDOW_LOCATION_SLOT)
}

fn object_window_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, object, slot).filter(|value| !value.is_undefined())
}

fn global_location_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Value>> {
    global_window_slot_value(scope, WINDOW_LOCATION_SLOT)
}

fn global_window_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    object_window_slot_value(scope, global, slot)
}
