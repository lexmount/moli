use super::*;
use crate::devtools_runtime::DevToolsKeyEventType;
use chromiumoxide_cdp::cdp::browser_protocol::input::{
    DispatchKeyEventParams as KeyEventParams, DispatchKeyEventType as KeyEventType,
    InsertTextParams,
};

fn optional_i64_to_u8(value: Option<i64>) -> Option<u8> {
    match value {
        Some(value) => value.try_into().ok(),
        None => Some(0),
    }
}

pub(super) struct ParsedDispatchKeyEvent {
    pub(super) event_type: DevToolsKeyEventType,
    pub(super) key: String,
    pub(super) code: String,
    pub(super) text: String,
    pub(super) modifiers: u8,
    pub(super) auto_repeat: bool,
    pub(super) should_insert_text: bool,
}

fn normalize_cdp_dom_key_string(key: String) -> String {
    // Chromium converts the CDP string through KeyStringToDomKey and then
    // exposes DomKeyToKeyString on KeyboardEvent. The Unicode control-key
    // spellings therefore surface as their canonical DOM key names. Rod uses
    // the carriage-return spelling for Enter.
    match key.as_str() {
        "\u{0008}" => "Backspace".to_owned(),
        "\u{0009}" => "Tab".to_owned(),
        "\u{000d}" => "Enter".to_owned(),
        "\u{001b}" => "Escape".to_owned(),
        "\u{007f}" => "Delete".to_owned(),
        _ => key,
    }
}

pub(super) fn parse_dispatch_key_event(
    cmd: &Cmd<'_>,
) -> Result<ParsedDispatchKeyEvent, &'static str> {
    let params: KeyEventParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err("InvalidParams"),
    };
    let Some(modifiers) = optional_i64_to_u8(params.modifiers) else {
        return Err("InvalidParams");
    };

    // keyDown combines a DOM keydown with a character phase; rawKeyDown
    // dispatches only keydown, even when the client supplies text.
    let (event_type, should_insert_text) = match params.r#type {
        KeyEventType::KeyDown => (
            DevToolsKeyEventType::KeyDown,
            params.text.as_deref().is_some_and(|text| !text.is_empty()),
        ),
        KeyEventType::RawKeyDown => (DevToolsKeyEventType::KeyDown, false),
        KeyEventType::KeyUp => (DevToolsKeyEventType::KeyUp, false),
        KeyEventType::Char => (DevToolsKeyEventType::KeyPress, true),
    };

    Ok(ParsedDispatchKeyEvent {
        event_type,
        key: normalize_cdp_dom_key_string(params.key.unwrap_or_default()),
        code: params.code.unwrap_or_default(),
        text: params.text.unwrap_or_default(),
        modifiers,
        auto_repeat: params.auto_repeat.unwrap_or(false),
        should_insert_text,
    })
}

pub(super) fn parse_insert_text(cmd: &Cmd<'_>) -> Result<String, &'static str> {
    let params: InsertTextParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err("InvalidParams"),
    };
    Ok(params.text)
}

pub(super) fn devtools_key_event_dom_event_name(event_type: DevToolsKeyEventType) -> &'static str {
    match event_type {
        DevToolsKeyEventType::KeyDown => "keydown",
        DevToolsKeyEventType::KeyUp => "keyup",
        DevToolsKeyEventType::KeyPress => "keypress",
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_cdp_dom_key_string;

    #[test]
    fn chromium_control_character_key_spellings_are_all_canonicalized() {
        for (input, expected) in [
            ("\u{0008}", "Backspace"),
            ("\u{0009}", "Tab"),
            ("\u{000d}", "Enter"),
            ("\u{001b}", "Escape"),
            ("\u{007f}", "Delete"),
            ("a", "a"),
        ] {
            assert_eq!(normalize_cdp_dom_key_string(input.to_owned()), expected);
        }
    }
}
