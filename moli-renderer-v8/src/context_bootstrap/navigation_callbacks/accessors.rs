use super::*;
use crate::util::get_private_value;
use crate::webidl;

fn window_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    member: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    let receiver = args.this();
    if super::super::is_window_receiver(scope, receiver) {
        return Some(receiver);
    }
    webidl::throw_type_error(
        scope,
        &format!("Window.{member} getter called on incompatible receiver."),
    );
    None
}

pub(in crate::context_bootstrap) fn window_history_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = window_receiver(scope, &args, "history") else {
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
    let Some(receiver) = window_receiver(scope, &args, "navigation") else {
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
    let Some(receiver) = window_receiver(scope, &args, "location") else {
        return;
    };
    let Some(value) = object_window_slot_value(scope, receiver, WINDOW_LOCATION_SLOT)
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
    if !crate::web_api_interfaces::Document::is_instance(scope, receiver) {
        webidl::throw_type_error(
            scope,
            "Document.location getter called on incompatible receiver.",
        );
        return;
    }
    match super::super::location_runtime::document_location_value(scope, receiver) {
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
    if !crate::web_api_interfaces::Document::is_instance(scope, receiver) {
        webidl::throw_type_error(
            scope,
            "Document.location setter called on incompatible receiver.",
        );
        return;
    }
    let location = super::super::location_runtime::document_location_value(scope, receiver)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let Some(location) = location else {
        webidl::throw_type_error(scope, "Cannot assign href to a null Location.");
        return;
    };
    // PutForwards gets the Location first and leaves conversion, exception
    // identity and navigation to its existing href setter.
    if location.set(scope, v8str(scope, "href").into(), args.get(0)) == Some(false) {
        webidl::throw_type_error(scope, "Cannot assign href to Location.");
        return;
    }
    rv.set_undefined();
}

fn object_window_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    let context = object.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
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
