use super::*;
use crate::util::{get_private_value, set_private_value};
use crate::webidl;
use moli_webapi_declare::WebApiObject;

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct WindowLocationAccessorDeclaration {
    #[webapi(
        accessor_property,
        getter = window_location_getter,
        setter = window_location_setter,
        enumerable,
        dont_delete
    )]
    location: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct WindowHistoryAccessorDeclaration {
    #[webapi(
        accessor_property,
        getter = window_history_getter,
        enumerable
    )]
    history: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct WindowNavigationAccessorDeclaration {
    #[webapi(
        accessor_property,
        getter = window_navigation_getter,
        setter = window_navigation_setter,
        enumerable
    )]
    navigation: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct DocumentLocationAccessorDeclaration {
    #[webapi(
        accessor_property,
        getter = document_location_getter,
        setter = document_location_setter,
        enumerable,
        dont_delete
    )]
    location: (),
}

pub(crate) fn sync_document_location_runtime_state_from_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    window: v8::Local<'s, v8::Object>,
) {
    let Some(location) = window_location_slot_value(scope, window) else {
        return;
    };
    set_private_value(scope, document, WINDOW_LOCATION_SLOT, location);
    install_public_location_accessor(scope, document);
}

pub(crate) fn sync_global_location_runtime_state(scope: &mut v8::PinScope<'_, '_>, href: &str) {
    let Some(location) = global_location_slot_value(scope)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    sync_location_object(scope, location, href);
}

pub(crate) fn navigate_top_level_window_location_from_cross_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    kind: LocationNavigationKind,
    raw_target: String,
) -> bool {
    let Some(location) = window_location_slot_value(scope, window)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    navigate_location_object(scope, location, kind, Some(raw_target));
    true
}

pub(crate) fn sync_window_location_history_navigation_runtime_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) {
    if window_runtime_slot_value(scope, window, WINDOW_LOCATION_SLOT).is_some() {
        install_public_window_location_accessor(scope, window);
    }
    if window_runtime_slot_value(scope, window, WINDOW_HISTORY_SLOT).is_some() {
        install_public_window_history_accessor(scope, window);
    }
    if window_runtime_slot_value(scope, window, WINDOW_NAVIGATION_SLOT).is_some() {
        install_public_window_navigation_accessor(scope, window);
    }
}

fn install_public_window_location_accessor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    // The [Global] Window template already owns the complete unforgeable
    // getter/setter pair. V8's WindowProxy can report that template property
    // as non-own here even though redefining the non-configurable property on
    // the underlying global would fail.
    if object.strict_equals(scope.get_current_context().global(scope).into()) {
        return;
    }
    let key = v8str(scope, "location");
    if object.has_own_property(scope, key.into()).unwrap_or(false) {
        return;
    }
    let _ = WindowLocationAccessorDeclaration::default().initialize(scope, object);
}

fn install_public_window_history_accessor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    let key = v8str(scope, "history");
    if object.has_own_property(scope, key.into()).unwrap_or(false) {
        return;
    }
    let _ = WindowHistoryAccessorDeclaration::default().initialize(scope, object);
}

fn install_public_window_navigation_accessor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    let key = v8str(scope, "navigation");
    if object.has_own_property(scope, key.into()).unwrap_or(false) {
        return;
    }
    let _ = WindowNavigationAccessorDeclaration::default().initialize(scope, object);
}

fn window_location_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    let Some(value) = window_location_slot_value(scope, receiver) else {
        webidl::throw_type_error(
            scope,
            "Window.location getter called on incompatible receiver.",
        );
        return;
    };
    rv.set(value);
}

pub(in crate::context_bootstrap) fn window_location_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    let Some(location) = window_location_slot_value(scope, receiver)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        webidl::throw_type_error(
            scope,
            "Window.location setter called on incompatible receiver.",
        );
        return;
    };
    let Some(href) = super::helpers::v8_value_to_string(scope, args.get(0)) else {
        return;
    };
    navigate_location_object(scope, location, LocationNavigationKind::Assign, Some(href));
}

fn window_location_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    window_runtime_slot_value(scope, window, WINDOW_LOCATION_SLOT)
}

fn window_runtime_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, window, slot).filter(|value| !value.is_undefined())
}

fn global_location_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    window_runtime_slot_value(scope, global, WINDOW_LOCATION_SLOT)
}

fn window_history_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    let Some(value) = window_runtime_slot_value(scope, receiver, WINDOW_HISTORY_SLOT) else {
        webidl::throw_type_error(
            scope,
            "Window.history getter called on incompatible receiver.",
        );
        return;
    };
    rv.set(value);
}

fn window_navigation_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    let Some(value) = window_runtime_slot_value(scope, receiver, WINDOW_NAVIGATION_SLOT) else {
        webidl::throw_type_error(
            scope,
            "Window.navigation getter called on incompatible receiver.",
        );
        return;
    };
    rv.set(value);
}

pub(in crate::context_bootstrap) fn window_navigation_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    let value = args.get(0);
    let _ = receiver.define_own_property(
        scope,
        v8str(scope, "navigation").into(),
        value,
        v8::PropertyAttribute::NONE,
    );
}

fn install_public_location_accessor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    let key = v8str(scope, "location");
    if object.has_own_property(scope, key.into()).unwrap_or(false) {
        return;
    }
    let _ = DocumentLocationAccessorDeclaration::default().initialize(scope, object);
}
