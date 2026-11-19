#[cfg(feature = "devtools")]
mod devtools;
#[cfg(feature = "devtools")]
pub(crate) use devtools::{
    construct_drag_event_with_related_target, construct_drop_input_event,
    construct_mouse_event_for_target, construct_mouse_event_with_detail_for_target,
    construct_mouse_event_with_related_target_for_target, construct_pointer_event,
    construct_pointer_event_with_modifiers, construct_pointer_event_with_related_target,
    construct_pointer_event_with_related_target_and_modifiers, construct_wheel_event_for_target,
};

use crate::util::{v8_string, v8str};
use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

use super::construct_event;
use moli_webapi_declare::WebApiObject;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::native_bridge::element) enum ClickEventSource {
    #[cfg(any(test, feature = "devtools"))]
    Pointer,
    #[cfg(feature = "devtools")]
    Touch {
        pointer_id: i32,
    },
    UserAgent,
    Script,
}

impl ClickEventSource {
    pub(in crate::native_bridge::element) fn is_trusted(self) -> bool {
        self != Self::Script
    }
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct MouseEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    screen_x: f64,
    screen_y: f64,
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
    view: Option<v8::Local<'scope, v8::Object>>,
}

#[cfg(any(test, feature = "devtools"))]
#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct KeyboardEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    key: v8::Local<'scope, v8::Value>,
    code: v8::Local<'scope, v8::Value>,
    location: u32,
    char_code: f64,
    key_code: f64,
    which: f64,
    alt_key: bool,
    ctrl_key: bool,
    meta_key: bool,
    shift_key: bool,
    repeat: bool,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct InputEventInitDeclaration<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    input_type: v8::Local<'scope, v8::String>,
    data: v8::Local<'scope, v8::Value>,
    data_transfer: v8::Local<'scope, v8::Value>,
}

#[derive(Clone, Copy)]
pub(crate) enum TextEditInputType {
    None,
    InsertText,
    #[cfg(feature = "devtools")]
    InsertLineBreak,
    #[cfg(feature = "devtools")]
    InsertParagraph,
    #[cfg(feature = "devtools")]
    InsertFromDrop,
    #[cfg(feature = "devtools")]
    DeleteContentBackward,
    #[cfg(feature = "devtools")]
    DeleteContentForward,
}

impl TextEditInputType {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::InsertText => "insertText",
            #[cfg(feature = "devtools")]
            Self::InsertLineBreak => "insertLineBreak",
            #[cfg(feature = "devtools")]
            Self::InsertParagraph => "insertParagraph",
            #[cfg(feature = "devtools")]
            Self::InsertFromDrop => "insertFromDrop",
            #[cfg(feature = "devtools")]
            Self::DeleteContentBackward => "deleteContentBackward",
            #[cfg(feature = "devtools")]
            Self::DeleteContentForward => "deleteContentForward",
        }
    }

    pub(crate) fn data(self, text: &str) -> Option<&str> {
        match self {
            Self::InsertText => Some(text),
            #[cfg(feature = "devtools")]
            Self::InsertFromDrop => Some(text),
            Self::None => None,
            #[cfg(feature = "devtools")]
            Self::InsertLineBreak
            | Self::InsertParagraph
            | Self::DeleteContentBackward
            | Self::DeleteContentForward => None,
        }
    }
}

#[cfg(any(test, feature = "devtools"))]
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

#[cfg(any(test, feature = "devtools"))]
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

#[cfg(any(test, feature = "devtools"))]
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

#[cfg(any(test, feature = "devtools"))]
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

#[cfg(any(test, feature = "devtools"))]
fn single_ascii_digit(value: &str) -> Option<u8> {
    if value.len() != 1 {
        return None;
    }
    let digit = value.as_bytes()[0];
    digit.is_ascii_digit().then_some(digit - b'0')
}

#[cfg(any(test, feature = "devtools"))]
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
    #[webapi(prototype, value = v8::null(scope))]
    prototype: (),
    bubbles: bool,
    cancelable: bool,
    composed: bool,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct SubmitEventInitDeclaration<'scope> {
    #[webapi(prototype, value = v8::null(scope))]
    prototype: (),
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

/// Root/screen and target-frame client coordinates for a dispatched input event.
#[derive(Clone, Copy, Default)]
pub(crate) struct InputEventCoordinates {
    pub screen_x: f64,
    pub screen_y: f64,
    pub client_x: f64,
    pub client_y: f64,
}

impl InputEventCoordinates {
    pub(crate) fn from_root(x: f64, y: f64) -> Self {
        // Moli's top-level window is at the screen origin. Frame transforms
        // change client coordinates without changing this screen position.
        Self {
            screen_x: x,
            screen_y: y,
            client_x: x,
            client_y: y,
        }
    }

    #[cfg(feature = "devtools")]
    pub(crate) fn with_client(self, x: f64, y: f64) -> Self {
        Self {
            client_x: x,
            client_y: y,
            ..self
        }
    }
}

fn mouse_event_init<'s>(
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
    let modifier_keys = modifier_key_state(modifiers);
    let is_boundary = matches!(event_type, "mouseenter" | "mouseleave");
    let button = if matches!(
        event_type,
        "mousemove" | "mouseover" | "mouseout" | "mouseenter" | "mouseleave"
    ) {
        0
    } else {
        button
    };
    let init = MouseEventInitDeclaration::new(
        !is_boundary,
        !is_boundary,
        !is_boundary,
        coordinates.screen_x,
        coordinates.screen_y,
        coordinates.client_x,
        coordinates.client_y,
        detail,
        button,
        buttons,
        modifier_keys.alt,
        modifier_keys.ctrl,
        modifier_keys.meta,
        modifier_keys.shift,
        related_target,
        view,
    )
    .bind(scope)
    .ok()?;
    init.set_prototype(scope, v8::null(scope).into())?
        .then_some(())?;
    Some(init)
}

#[cfg(any(feature = "devtools", test))]
#[derive(Clone, Copy)]
pub(crate) struct TouchEventPoint<'s> {
    pub identifier: i32,
    pub x: f64,
    pub y: f64,
    pub target: v8::Local<'s, v8::Object>,
    pub is_target_touch: bool,
}

#[cfg(any(feature = "devtools", test))]
pub(crate) fn construct_touch_event_with_points<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    active_points: &[TouchEventPoint<'s>],
    changed_points: &[TouchEventPoint<'s>],
) -> Option<v8::Local<'s, v8::Object>> {
    let touches = construct_touches(scope, active_points)?;
    let target_touches = touches
        .iter()
        .copied()
        .zip(active_points)
        .filter_map(|(touch, point)| point.is_target_touch.then_some(touch))
        .collect();
    let changed_touches = construct_touches(scope, changed_points)?;
    crate::context_bootstrap::construct_native_touch_event(
        scope,
        event_type,
        touches,
        target_touches,
        changed_touches,
    )
}

#[cfg(any(feature = "devtools", test))]
fn construct_touches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    points: &[TouchEventPoint<'s>],
) -> Option<Vec<v8::Local<'s, v8::Object>>> {
    points
        .iter()
        .map(|point| {
            crate::context_bootstrap::construct_native_touch(
                scope,
                point.identifier,
                point.target,
                point.x,
                point.y,
            )
        })
        .collect()
}

#[cfg(any(test, feature = "devtools"))]
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
    location: u32,
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
        location,
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

pub(crate) fn construct_focus_event<'s>(
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

pub(crate) fn construct_input_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    input_type: TextEditInputType,
    data: Option<&str>,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_input_event_with_transfer(scope, event_type, input_type, data, None)
}

#[cfg(feature = "devtools")]
pub(crate) fn construct_text_input_event_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    text: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, target)?;
    let context = crate::native_bridge::node_relevant_context(scope, target)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    crate::context_bootstrap::construct_original_text_input_event(scope, text)
}

fn construct_input_event_with_transfer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    input_type: TextEditInputType,
    data: Option<&str>,
    data_transfer: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let input_type = v8_string(scope, input_type.as_str())?;
    let data = match data {
        Some(text) => v8_string(scope, text)?.into(),
        None => v8::null(scope).into(),
    };
    let data_transfer = data_transfer
        .map(|value| value.into())
        .unwrap_or_else(|| v8::null(scope).into());
    let init = InputEventInitDeclaration::new(
        true,
        event_type == "beforeinput",
        true,
        input_type,
        data,
        data_transfer,
    )
    .bind(scope)
    .ok()?;
    construct_event(scope, "InputEvent", event_type, init)
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
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    submitter: Option<v8::Local<'s, v8::Value>>,
    bubbles: bool,
    cancelable: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, target)?;
    let context = crate::native_bridge::node_relevant_context(scope, target)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let init = SubmitEventInitDeclaration::new(
        bubbles,
        cancelable,
        submitter.unwrap_or_else(|| v8::null(scope).into()),
    )
    .bind(scope)
    .ok()?;
    construct_event(scope, "SubmitEvent", "submit", init)
}

pub(crate) fn construct_simple_event_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    target: DomHandle,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, target)?;
    let context = crate::native_bridge::node_relevant_context(scope, target)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    construct_simple_event(scope, event_type, bubbles, cancelable, composed)
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct FormDataEventInitDeclaration<'s> {
    #[webapi(prototype, value = v8::null(scope))]
    prototype: (),
    form_data: v8::Local<'s, v8::Object>,
    bubbles: bool,
}

pub(crate) fn construct_form_data_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    form_data: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = FormDataEventInitDeclaration::new(form_data, true)
        .bind(scope)
        .ok()?;
    construct_event(scope, "FormDataEvent", "formdata", init)
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
    coordinates: InputEventCoordinates,
    detail: i32,
    button: i32,
    buttons: i32,
    modifiers: u8,
    view: Option<v8::Local<'s, v8::Object>>,
    source: ClickEventSource,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = mouse_event_init(
        scope,
        "click",
        coordinates,
        detail,
        button,
        buttons,
        modifiers,
        None,
        view,
    )?;
    let interface = match source {
        #[cfg(any(test, feature = "devtools"))]
        ClickEventSource::Pointer => "MouseEvent",
        #[cfg(feature = "devtools")]
        ClickEventSource::Touch { pointer_id } => {
            let key = v8str(scope, "pointerId");
            let id = v8::Integer::new(scope, pointer_id);
            init.create_data_property(scope, key.into(), id.into())?
                .then_some(())?;
            let key = v8str(scope, "pointerType");
            let kind = v8str(scope, "touch");
            init.create_data_property(scope, key.into(), kind.into())?
                .then_some(())?;
            "PointerEvent"
        }
        ClickEventSource::UserAgent | ClickEventSource::Script => {
            // Blink reserves -1 for clicks with no pointing device. PointerEvent's
            // other defaults describe a synthetic click, unlike physical input.
            let key = v8str(scope, "pointerId");
            let pointer_id = v8::Integer::new(scope, -1);
            init.create_data_property(scope, key.into(), pointer_id.into())?
                .then_some(())?;
            "PointerEvent"
        }
    };
    construct_event(scope, interface, "click", init)
}
