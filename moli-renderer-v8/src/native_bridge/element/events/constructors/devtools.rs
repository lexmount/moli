use super::*;
use crate::runtime::RendererPointerEventProperties;
use moli_webapi::events::subclasses::{
    construct_native_drag_event, construct_native_pointer_event, construct_native_wheel_event,
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
    construct_native_mouse_event(scope, event_type, init)
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
    let mouse = MouseEventInit::from_native(
        scope,
        EventInit {
            bubbles: !is_enter_or_leave,
            cancelable: !is_enter_or_leave
                && !matches!(
                    event_type,
                    "pointercancel" | "gotpointercapture" | "lostpointercapture"
                ),
            composed: true,
        },
        coordinates,
        0,
        button,
        buttons,
        modifiers,
        related_target,
        None,
    )?;
    construct_native_pointer_event(scope, event_type, mouse, pointer)
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
    let view = scope.get_current_context().global(scope);
    let mouse = MouseEventInit::from_native(
        scope,
        EventInit {
            bubbles: true,
            cancelable: !matches!(event_type, "dragleave" | "dragend"),
            composed: true,
        },
        coordinates,
        0,
        0,
        0,
        modifiers,
        related_target,
        Some(view),
    )?;
    construct_native_drag_event(scope, event_type, mouse, data_transfer)
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
    let view = scope.get_current_context().global(scope);
    let mouse = MouseEventInit::from_native(
        scope,
        EventInit {
            bubbles: true,
            cancelable: true,
            composed: true,
        },
        coordinates,
        0,
        button,
        buttons,
        modifiers,
        None,
        Some(view),
    )?;
    construct_native_wheel_event(scope, event_type, mouse, delta_x, delta_y)
}

pub(crate) fn construct_drop_input_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    data_transfer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_editing_input_event(
        scope,
        event_type,
        TextEditInputType::InsertFromDrop.as_str(),
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
