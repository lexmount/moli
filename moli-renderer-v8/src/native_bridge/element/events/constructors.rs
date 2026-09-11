use crate::runtime::RendererPointerEventProperties;
use crate::util::{serialize_v8_iter_array, v8_string};

use super::{construct_event, event_constructor};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct MouseEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    client_x: f64,
    client_y: f64,
    detail: i32,
    button: i32,
    buttons: i32,
    alt_key: bool,
    ctrl_key: bool,
    meta_key: bool,
    shift_key: bool,
    related_target: Option<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct PointerEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    client_x: f64,
    client_y: f64,
    button: i32,
    buttons: i32,
    alt_key: bool,
    ctrl_key: bool,
    meta_key: bool,
    shift_key: bool,
    pointer_type: v8::Local<'scope, v8::String>,
    pressure: f64,
    tangential_pressure: f64,
    tilt_x: f64,
    tilt_y: f64,
    twist: f64,
    pointer_id: f64,
    width: f64,
    height: f64,
    is_primary: bool,
    related_target: Option<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct DragEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    client_x: f64,
    client_y: f64,
    alt_key: bool,
    ctrl_key: bool,
    meta_key: bool,
    shift_key: bool,
    data_transfer: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct WheelEventInitDeclaration {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    client_x: f64,
    client_y: f64,
    delta_x: f64,
    delta_y: f64,
    button: i32,
    buttons: i32,
    alt_key: bool,
    ctrl_key: bool,
    meta_key: bool,
    shift_key: bool,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct TouchEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    touches: v8::Local<'scope, v8::Array>,
    target_touches: v8::Local<'scope, v8::Array>,
    changed_touches: v8::Local<'scope, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct TouchInitDeclaration<'scope> {
    identifier: i32,
    target: v8::Local<'scope, v8::Object>,
    screen_x: f64,
    screen_y: f64,
    client_x: f64,
    client_y: f64,
    page_x: f64,
    page_y: f64,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct KeyboardEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    key: v8::Local<'scope, v8::Value>,
    code: v8::Local<'scope, v8::Value>,
    char_code: f64,
    key_code: f64,
    which: f64,
    alt_key: bool,
    ctrl_key: bool,
    meta_key: bool,
    shift_key: bool,
    repeat: bool,
}

fn keyboard_event_legacy_codes(event_type: &str, key: &str, code: &str) -> (u32, u32, u32) {
    let char_code = keyboard_event_char_code(event_type, key);
    let mut key_code = keyboard_event_key_code(key, code);
    if event_type == "keypress" && char_code != 0 {
        key_code = char_code;
    }
    let which = if event_type == "keypress" && char_code != 0 {
        char_code
    } else {
        key_code
    };
    (char_code, key_code, which)
}

fn keyboard_event_char_code(event_type: &str, key: &str) -> u32 {
    if event_type != "keypress" {
        return 0;
    }
    if key == "Enter" {
        return 13;
    }
    let mut chars = key.chars();
    let Some(ch) = chars.next() else {
        return 0;
    };
    if chars.next().is_some() || ch.is_control() {
        return 0;
    }
    ch as u32
}

fn keyboard_event_key_code(key: &str, code: &str) -> u32 {
    match key {
        "Backspace" => 8,
        "Tab" => 9,
        "Clear" => 12,
        "Enter" => 13,
        "Shift" => 16,
        "Control" => 17,
        "Alt" => 18,
        "Pause" => 19,
        "CapsLock" => 20,
        "Escape" | "Esc" => 27,
        " " | "Space" | "Spacebar" => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" | "Left" => 37,
        "ArrowUp" | "Up" => 38,
        "ArrowRight" | "Right" => 39,
        "ArrowDown" | "Down" => 40,
        "PrintScreen" => 44,
        "Insert" => 45,
        "Delete" | "Del" => 46,
        "Meta" | "OS" => 91,
        "ContextMenu" => 93,
        "NumLock" => 144,
        "ScrollLock" => 145,
        _ => keyboard_event_key_code_for_code(code)
            .or_else(|| keyboard_event_key_code_for_printable_key(key))
            .unwrap_or(0),
    }
}

fn keyboard_event_key_code_for_code(code: &str) -> Option<u32> {
    if let Some(suffix) = code.strip_prefix("Key")
        && suffix.len() == 1
    {
        let ch = suffix.as_bytes()[0];
        if ch.is_ascii_uppercase() {
            return Some(ch as u32);
        }
    }

    if let Some(digit) = code.strip_prefix("Digit").and_then(single_ascii_digit) {
        return Some(b'0' as u32 + digit as u32);
    }
    if let Some(digit) = code.strip_prefix("Numpad").and_then(single_ascii_digit) {
        return Some(96 + digit as u32);
    }
    if let Some(function) = code
        .strip_prefix('F')
        .and_then(|value| value.parse::<u32>().ok())
        && (1..=24).contains(&function)
    {
        return Some(111 + function);
    }

    Some(match code {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" | "NumpadEnter" => 13,
        "ShiftLeft" | "ShiftRight" => 16,
        "ControlLeft" | "ControlRight" => 17,
        "AltLeft" | "AltRight" => 18,
        "Pause" => 19,
        "CapsLock" => 20,
        "Escape" => 27,
        "Space" => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "PrintScreen" => 44,
        "Insert" => 45,
        "Delete" => 46,
        "MetaLeft" | "MetaRight" | "OSLeft" | "OSRight" => 91,
        "ContextMenu" => 93,
        "NumpadMultiply" => 106,
        "NumpadAdd" => 107,
        "NumpadSeparator" | "NumpadComma" => 108,
        "NumpadSubtract" => 109,
        "NumpadDecimal" => 110,
        "NumpadDivide" => 111,
        "NumLock" => 144,
        "ScrollLock" => 145,
        "Semicolon" => 186,
        "Equal" => 187,
        "Comma" => 188,
        "Minus" => 189,
        "Period" => 190,
        "Slash" => 191,
        "Backquote" => 192,
        "BracketLeft" => 219,
        "Backslash" => 220,
        "BracketRight" => 221,
        "Quote" => 222,
        "IntlBackslash" => 226,
        _ => return None,
    })
}

fn single_ascii_digit(value: &str) -> Option<u8> {
    if value.len() != 1 {
        return None;
    }
    let digit = value.as_bytes()[0];
    digit.is_ascii_digit().then_some(digit - b'0')
}

fn keyboard_event_key_code_for_printable_key(key: &str) -> Option<u32> {
    let mut chars = key.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(match ch {
        'a'..='z' => ch.to_ascii_uppercase() as u32,
        'A'..='Z' | '0'..='9' => ch as u32,
        ' ' => 32,
        ';' | ':' => 186,
        '=' | '+' => 187,
        ',' | '<' => 188,
        '-' | '_' => 189,
        '.' | '>' => 190,
        '/' | '?' => 191,
        '`' | '~' => 192,
        '[' | '{' => 219,
        '\\' | '|' => 220,
        ']' | '}' => 221,
        '\'' | '"' => 222,
        _ => return None,
    })
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct FocusEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    related_target: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct SimpleEventInitDeclaration {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct SubmitEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    submitter: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct CommandEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    source: v8::Local<'scope, v8::Value>,
    command: v8::Local<'scope, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct ToggleEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    source: v8::Local<'scope, v8::Value>,
    old_state: v8::Local<'scope, v8::String>,
    new_state: v8::Local<'scope, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct InterestEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    source: v8::Local<'scope, v8::Value>,
}

#[derive(Clone, Copy)]
struct ModifierKeyState {
    alt: bool,
    ctrl: bool,
    meta: bool,
    shift: bool,
}

fn modifier_key_state(modifiers: u8) -> ModifierKeyState {
    ModifierKeyState {
        alt: modifiers & 1 == 1,
        ctrl: modifiers & 2 == 2,
        meta: modifiers & 4 == 4,
        shift: modifiers & 8 == 8,
    }
}

pub(crate) fn construct_mouse_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    x: f64,
    y: f64,
    button: i32,
    buttons: i32,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_mouse_event_with_modifiers(scope, event_type, x, y, button, buttons, 0)
}

pub(crate) fn construct_mouse_event_with_modifiers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    x: f64,
    y: f64,
    button: i32,
    buttons: i32,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_mouse_event_with_detail_and_related_target(
        scope, event_type, x, y, 0, button, buttons, modifiers, None,
    )
}

pub(crate) fn construct_mouse_event_with_detail_and_modifiers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    x: f64,
    y: f64,
    detail: i32,
    button: i32,
    buttons: i32,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_mouse_event_with_detail_and_related_target(
        scope, event_type, x, y, detail, button, buttons, modifiers, None,
    )
}

pub(crate) fn construct_mouse_event_with_related_target_and_modifiers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    x: f64,
    y: f64,
    button: i32,
    buttons: i32,
    related_target: Option<v8::Local<'s, v8::Value>>,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_mouse_event_with_detail_and_related_target(
        scope,
        event_type,
        x,
        y,
        0,
        button,
        buttons,
        modifiers,
        related_target,
    )
}

fn construct_mouse_event_with_detail_and_related_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    x: f64,
    y: f64,
    detail: i32,
    button: i32,
    buttons: i32,
    modifiers: u8,
    related_target: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let modifier_keys = modifier_key_state(modifiers);
    let init = MouseEventInitDeclaration::new(
        true,
        true,
        true,
        x,
        y,
        detail,
        button,
        buttons,
        modifier_keys.alt,
        modifier_keys.ctrl,
        modifier_keys.meta,
        modifier_keys.shift,
        related_target,
    )
    .bind(scope)
    .ok()?;
    construct_event(scope, "MouseEvent", event_type, init)
}

pub(crate) fn construct_pointer_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    x: f64,
    y: f64,
    button: i32,
    buttons: i32,
    pointer: &RendererPointerEventProperties,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_pointer_event_with_modifiers(scope, event_type, x, y, button, buttons, pointer, 0)
}

pub(crate) fn construct_pointer_event_with_modifiers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    x: f64,
    y: f64,
    button: i32,
    buttons: i32,
    pointer: &RendererPointerEventProperties,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_pointer_event_with_related_target_and_modifiers(
        scope, event_type, x, y, button, buttons, pointer, None, modifiers,
    )
}

pub(crate) fn construct_pointer_event_with_related_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    x: f64,
    y: f64,
    button: i32,
    buttons: i32,
    pointer: &RendererPointerEventProperties,
    related_target: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_pointer_event_with_related_target_and_modifiers(
        scope,
        event_type,
        x,
        y,
        button,
        buttons,
        pointer,
        related_target,
        0,
    )
}

pub(crate) fn construct_pointer_event_with_related_target_and_modifiers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    x: f64,
    y: f64,
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
        !is_enter_or_leave,
        true,
        x,
        y,
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
        true,
        related_target,
    )
    .bind(scope)
    .ok()?;
    construct_event(scope, "PointerEvent", event_type, init)
}

pub(crate) fn construct_drag_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    x: f64,
    y: f64,
    data_transfer: v8::Local<'s, v8::Value>,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    let modifier_keys = modifier_key_state(modifiers);
    let init = DragEventInitDeclaration::new(
        true,
        true,
        true,
        x,
        y,
        modifier_keys.alt,
        modifier_keys.ctrl,
        modifier_keys.meta,
        modifier_keys.shift,
        data_transfer,
    )
    .bind(scope)
    .ok()?;
    construct_event(scope, "DragEvent", event_type, init)
}

pub(crate) fn construct_wheel_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    x: f64,
    y: f64,
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
        x,
        y,
        delta_x,
        delta_y,
        button,
        buttons,
        modifier_keys.alt,
        modifier_keys.ctrl,
        modifier_keys.meta,
        modifier_keys.shift,
    )
    .bind(scope)
    .ok()?;
    construct_event(scope, "WheelEvent", event_type, init)
}

pub(crate) fn construct_touch_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    x: f64,
    y: f64,
    target: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let point = TouchEventPoint {
        identifier: 0,
        x,
        y,
        target,
        is_target_touch: true,
    };
    let active_points = match event_type {
        "touchend" | "touchcancel" => Vec::new(),
        _ => vec![point],
    };
    construct_touch_event_with_points(scope, event_type, &active_points, &[point])
}

#[derive(Clone, Copy)]
pub(crate) struct TouchEventPoint<'s> {
    pub identifier: i32,
    pub x: f64,
    pub y: f64,
    pub target: v8::Local<'s, v8::Object>,
    pub is_target_touch: bool,
}

pub(crate) fn construct_touch_event_with_points<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    active_points: &[TouchEventPoint<'s>],
    changed_points: &[TouchEventPoint<'s>],
) -> Option<v8::Local<'s, v8::Object>> {
    let touch_ctor = event_constructor(scope, "Touch")?;
    let touches = construct_touch_array(scope, touch_ctor, active_points)?;
    let target_touch_points = active_points
        .iter()
        .copied()
        .filter(|point| point.is_target_touch)
        .collect::<Vec<_>>();
    let target_touches = construct_touch_array(scope, touch_ctor, &target_touch_points)?;
    let changed_touches = construct_touch_array(scope, touch_ctor, changed_points)?;

    let init =
        TouchEventInitDeclaration::new(true, true, true, touches, target_touches, changed_touches)
            .bind(scope)
            .ok()?;
    construct_event(scope, "TouchEvent", event_type, init)
}

fn construct_touch_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    touch_ctor: v8::Local<'s, v8::Function>,
    points: &[TouchEventPoint<'s>],
) -> Option<v8::Local<'s, v8::Array>> {
    let touches = points
        .iter()
        .map(|point| construct_touch(scope, touch_ctor, *point))
        .collect::<Option<Vec<_>>>()?;
    serialize_v8_iter_array(scope, touches)
}

fn construct_touch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    touch_ctor: v8::Local<'s, v8::Function>,
    point: TouchEventPoint<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let touch_init = TouchInitDeclaration::new(
        point.identifier,
        point.target,
        0.0,
        0.0,
        point.x,
        point.y,
        point.x,
        point.y,
    )
    .bind(scope)
    .ok()?;
    touch_ctor.new_instance(scope, &[touch_init.into()])
}

pub(crate) fn construct_keyboard_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    key: &str,
    code: &str,
    alt: bool,
    ctrl: bool,
    meta: bool,
    shift: bool,
    repeat: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let (char_code, key_code, which) = keyboard_event_legacy_codes(event_type, key, code);
    let key = v8_string(scope, key)
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let code = v8_string(scope, code)
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let init = KeyboardEventInitDeclaration::new(
        true,
        true,
        true,
        key,
        code,
        char_code as f64,
        key_code as f64,
        which as f64,
        alt,
        ctrl,
        meta,
        shift,
        repeat,
    )
    .bind(scope)
    .ok()?;
    construct_event(scope, "KeyboardEvent", event_type, init)
}

pub(in crate::native_bridge::element) fn construct_focus_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    related_target: Option<v8::Local<'s, v8::Value>>,
    bubbles: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = FocusEventInitDeclaration::new(
        bubbles,
        false,
        true,
        related_target.unwrap_or_else(|| v8::null(scope).into()),
    )
    .bind(scope)
    .ok()?;
    construct_event(scope, "FocusEvent", event_type, init)
}

pub(crate) fn construct_simple_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = SimpleEventInitDeclaration::new(bubbles, cancelable, composed)
        .bind(scope)
        .ok()?;
    construct_event(scope, "Event", event_type, init)
}

pub(crate) fn construct_submit_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    submitter: Option<v8::Local<'s, v8::Value>>,
    bubbles: bool,
    cancelable: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = SubmitEventInitDeclaration::new(
        bubbles,
        cancelable,
        submitter.unwrap_or_else(|| v8::null(scope).into()),
    )
    .bind(scope)
    .ok()?;
    construct_event(scope, "SubmitEvent", "submit", init)
}

pub(crate) fn construct_command_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    command: &str,
    source: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let command = v8_string(scope, command)?;
    let init = CommandEventInitDeclaration::new(false, true, false, source, command)
        .bind(scope)
        .ok()?;
    construct_event(scope, "CommandEvent", "command", init)
}

pub(crate) fn construct_toggle_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    old_state: &str,
    new_state: &str,
    cancelable: bool,
    source: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let old_state = v8_string(scope, old_state)?;
    let new_state = v8_string(scope, new_state)?;
    let init =
        ToggleEventInitDeclaration::new(false, cancelable, false, source, old_state, new_state)
            .bind(scope)
            .ok()?;
    construct_event(scope, "ToggleEvent", event_type, init)
}

pub(crate) fn construct_interest_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    source: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = InterestEventInitDeclaration::new(false, false, false, source)
        .bind(scope)
        .ok()?;
    construct_event(scope, "InterestEvent", event_type, init)
}

pub(in crate::native_bridge::element) fn construct_click_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    x: f64,
    y: f64,
    button: i32,
    buttons: i32,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_mouse_event(scope, "click", x, y, button, buttons)
}

pub(in crate::native_bridge::element) fn construct_click_event_with_detail_and_modifiers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    x: f64,
    y: f64,
    detail: i32,
    button: i32,
    buttons: i32,
    modifiers: u8,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_mouse_event_with_detail_and_modifiers(
        scope, "click", x, y, detail, button, buttons, modifiers,
    )
}
