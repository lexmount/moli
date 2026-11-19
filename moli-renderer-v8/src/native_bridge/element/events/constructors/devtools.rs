use super::*;
use crate::runtime::RendererPointerEventProperties;
use moli_webapi::events::dispatch_init::{
    DragEventInitDeclaration, PointerEventInitDeclaration, WheelEventInitDeclaration,
};

pub(crate) fn construct_mouse_event_with_modifiers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    coordinates: InputEventCoordinates,
    button: i32,
    buttons: i32,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_mouse_event_with_detail_and_related_target(
        scope,
        event_type,
        coordinates,
        0,
        button,
        buttons,
        modifiers,
        None,
        Some(scope.get_current_context().global(scope)),
    )
}

pub(crate) fn construct_mouse_event_with_detail_and_modifiers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    coordinates: InputEventCoordinates,
    detail: i32,
    button: i32,
    buttons: i32,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_mouse_event_with_detail_and_related_target(
        scope,
        event_type,
        coordinates,
        detail,
        button,
        buttons,
        modifiers,
        None,
        Some(scope.get_current_context().global(scope)),
    )
}

pub(crate) fn construct_mouse_event_with_related_target_and_modifiers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    coordinates: InputEventCoordinates,
    button: i32,
    buttons: i32,
    related_target: Option<v8::Local<'s, v8::Value>>,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_mouse_event_with_detail_and_related_target(
        scope,
        event_type,
        coordinates,
        0,
        button,
        buttons,
        modifiers,
        related_target,
        Some(scope.get_current_context().global(scope)),
    )
}

fn construct_mouse_event_with_detail_and_related_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    coordinates: InputEventCoordinates,
    detail: i32,
    button: i32,
    buttons: i32,
    modifiers: u8,
    related_target: Option<v8::Local<'s, v8::Value>>,
    view: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = mouse_event_init(
        scope,
        event_type,
        coordinates,
        detail,
        button,
        buttons,
        modifiers,
        related_target,
        view,
    )?;
    construct_event(scope, "MouseEvent", event_type, init)
}

pub(crate) fn construct_pointer_event_with_modifiers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    coordinates: InputEventCoordinates,
    button: i32,
    buttons: i32,
    pointer: &RendererPointerEventProperties,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_pointer_event_with_related_target_and_modifiers(
        scope,
        event_type,
        coordinates,
        button,
        buttons,
        pointer,
        None,
        modifiers,
    )
}

pub(crate) fn construct_pointer_event_with_related_target_and_modifiers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    coordinates: InputEventCoordinates,
    button: i32,
    buttons: i32,
    pointer: &RendererPointerEventProperties,
    related_target: Option<v8::Local<'s, v8::Value>>,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    let is_enter_or_leave = matches!(event_type, "pointerenter" | "pointerleave");
    let pointer_type = v8_string(scope, &pointer.pointer_type)?;
    let modifier_keys = modifier_key_state(modifiers);
    let init = PointerEventInitDeclaration::new(
        !is_enter_or_leave,
        !is_enter_or_leave
            && !matches!(
                event_type,
                "pointercancel" | "gotpointercapture" | "lostpointercapture"
            ),
        true,
        coordinates.screen_x,
        coordinates.screen_y,
        coordinates.client_x,
        coordinates.client_y,
        button,
        buttons,
        modifier_keys.alt,
        modifier_keys.ctrl,
        modifier_keys.meta,
        modifier_keys.shift,
        pointer_type,
        pointer.pressure,
        pointer.tangential_pressure,
        pointer.tilt_x,
        pointer.tilt_y,
        pointer.twist,
        f64::from(pointer.pointer_id),
        1.0,
        1.0,
        pointer.is_primary,
        related_target,
    )
    .bind(scope)
    .ok()?;
    construct_event(scope, "PointerEvent", event_type, init)
}

pub(crate) fn construct_pointer_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    coordinates: InputEventCoordinates,
    button: i32,
    buttons: i32,
    pointer: &RendererPointerEventProperties,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_pointer_event_with_modifiers(
        scope,
        event_type,
        coordinates,
        button,
        buttons,
        pointer,
        0,
    )
}

pub(crate) fn construct_pointer_event_with_related_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    coordinates: InputEventCoordinates,
    button: i32,
    buttons: i32,
    pointer: &RendererPointerEventProperties,
    related_target: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_pointer_event_with_related_target_and_modifiers(
        scope,
        event_type,
        coordinates,
        button,
        buttons,
        pointer,
        related_target,
        0,
    )
}

pub(crate) fn construct_drag_event_with_related_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    coordinates: InputEventCoordinates,
    data_transfer: v8::Local<'s, v8::Value>,
    related_target: Option<v8::Local<'s, v8::Value>>,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    let modifier_keys = modifier_key_state(modifiers);
    let view = scope.get_current_context().global(scope);
    let init = DragEventInitDeclaration::new(
        true,
        !matches!(event_type, "dragleave" | "dragend"),
        true,
        coordinates.screen_x,
        coordinates.screen_y,
        coordinates.client_x,
        coordinates.client_y,
        modifier_keys.alt,
        modifier_keys.ctrl,
        modifier_keys.meta,
        modifier_keys.shift,
        view,
        data_transfer,
        related_target,
    )
    .bind(scope)
    .ok()?;
    construct_event(scope, "DragEvent", event_type, init)
}

pub(crate) fn construct_wheel_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    coordinates: InputEventCoordinates,
    delta_x: f64,
    delta_y: f64,
    button: i32,
    buttons: i32,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    let modifier_keys = modifier_key_state(modifiers);
    let init = WheelEventInitDeclaration::new(
        true,
        true,
        true,
        coordinates.screen_x,
        coordinates.screen_y,
        coordinates.client_x,
        coordinates.client_y,
        delta_x,
        delta_y,
        button,
        buttons,
        modifier_keys.alt,
        modifier_keys.ctrl,
        modifier_keys.meta,
        modifier_keys.shift,
        scope.get_current_context().global(scope),
    )
    .bind(scope)
    .ok()?;
    init.set_prototype(scope, v8::null(scope).into())?
        .then_some(())?;
    construct_event(scope, "WheelEvent", event_type, init)
}

pub(crate) fn construct_drop_input_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    data_transfer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_input_event_with_transfer(
        scope,
        event_type,
        TextEditInputType::InsertFromDrop,
        None,
        Some(data_transfer),
    )
}

pub(crate) fn construct_mouse_event_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    event_type: &str,
    coordinates: InputEventCoordinates,
    button: i32,
    buttons: i32,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, target)?;
    let context = crate::native_bridge::node_relevant_context(scope, target)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    construct_mouse_event_with_modifiers(scope, event_type, coordinates, button, buttons, modifiers)
}

pub(crate) fn construct_mouse_event_with_related_target_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    event_type: &str,
    coordinates: InputEventCoordinates,
    button: i32,
    buttons: i32,
    related_target: Option<v8::Local<'s, v8::Value>>,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, target)?;
    let context = crate::native_bridge::node_relevant_context(scope, target)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    construct_mouse_event_with_related_target_and_modifiers(
        scope,
        event_type,
        coordinates,
        button,
        buttons,
        related_target,
        modifiers,
    )
}

pub(crate) fn construct_mouse_event_with_detail_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    event_type: &str,
    coordinates: InputEventCoordinates,
    detail: i32,
    button: i32,
    buttons: i32,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, target)?;
    let context = crate::native_bridge::node_relevant_context(scope, target)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    construct_mouse_event_with_detail_and_modifiers(
        scope,
        event_type,
        coordinates,
        detail,
        button,
        buttons,
        modifiers,
    )
}

pub(crate) fn construct_wheel_event_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    event_type: &str,
    coordinates: InputEventCoordinates,
    delta_x: f64,
    delta_y: f64,
    buttons: i32,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, target)?;
    let context = crate::native_bridge::node_relevant_context(scope, target)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    // Wheel input does not change a mouse button.
    construct_wheel_event(
        scope,
        event_type,
        coordinates,
        delta_x,
        delta_y,
        0,
        buttons,
        modifiers,
    )
}
