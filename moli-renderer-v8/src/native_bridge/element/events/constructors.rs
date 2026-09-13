pub(crate) use moli_webapi::events::subclasses::construct_native_clipboard_event as construct_clipboard_event;
pub(super) use moli_webapi::events::subclasses::construct_native_input_event as construct_editing_input_event;
#[cfg(any(test, feature = "devtools"))]
use moli_webapi::events::subclasses::construct_native_keyboard_event;
pub(crate) use moli_webapi::events::subclasses::{
    construct_native_command_event as construct_command_event,
    construct_native_form_data_event as construct_form_data_event,
    construct_native_interest_event as construct_interest_event,
    construct_native_toggle_event as construct_toggle_event,
};
pub(crate) use moli_webapi::events::subclasses::construct_native_clipboard_event as construct_clipboard_event;
pub(super) use moli_webapi::events::subclasses::construct_native_input_event as construct_editing_input_event;
use moli_webapi::events::subclasses::{construct_native_focus_event, construct_native_submit_event};
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

use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};
pub(crate) use moli_webapi::events::subclasses::InputEventCoordinates;
#[cfg(any(test, feature = "devtools"))]
use moli_webapi::events::subclasses::construct_native_mouse_event;
use moli_webapi::events::subclasses::{MouseEventInit, construct_native_pointer_click};
use moli_webapi::events::{construct_native_event, init::EventInit, state::mark_event_trusted};

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
    InsertFromPaste,
    DeleteByCut,
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
            Self::InsertFromPaste => "insertFromPaste",
            Self::DeleteByCut => "deleteByCut",
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
            Self::InsertFromPaste => (!text.is_empty()).then_some(text),
            Self::DeleteByCut | Self::None => None,
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
) -> Option<MouseEventInit<'s>> {
    let is_boundary = matches!(event_type, "mouseenter" | "mouseleave");
    let button = if matches!(
        event_type,
        "mousemove" | "mouseover" | "mouseout" | "mouseenter" | "mouseleave"
    ) {
        0
    } else {
        button
    };
    MouseEventInit::from_native(
        scope,
        EventInit {
            bubbles: !is_boundary,
            cancelable: !is_boundary,
            composed: !is_boundary,
        },
        coordinates,
        detail,
        button,
        buttons,
        modifiers,
        related_target,
        view,
    )
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
    construct_native_keyboard_event(
        scope, event_type, key, code, alt, ctrl, meta, shift, repeat, location, char_code,
        key_code, which,
    )
}

pub(crate) fn construct_focus_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    related_target: Option<v8::Local<'s, v8::Value>>,
    bubbles: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let event = construct_native_focus_event(scope, event_type, related_target, bubbles)?;
    mark_event_trusted(scope, event);
    Some(event)
}

pub(crate) fn construct_input_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    input_type: TextEditInputType,
    data: Option<&str>,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_editing_input_event(scope, event_type, input_type.as_str(), data, None)
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

pub(crate) fn construct_simple_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let event = construct_native_event(
        scope,
        event_type,
        EventInit {
            bubbles,
            cancelable,
            composed,
        },
    )?;
    mark_event_trusted(scope, event);
    Some(event)
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
    construct_native_submit_event(scope, submitter, bubbles, cancelable)
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
    match source {
        #[cfg(any(test, feature = "devtools"))]
        ClickEventSource::Pointer => construct_native_mouse_event(scope, "click", init),
        #[cfg(feature = "devtools")]
        ClickEventSource::Touch { pointer_id } => {
            construct_native_pointer_click(scope, init, pointer_id, "touch")
        }
        ClickEventSource::UserAgent | ClickEventSource::Script => {
            // Blink reserves -1 for clicks with no pointing device.
            construct_native_pointer_click(scope, init, -1, "")
        }
    }
}
