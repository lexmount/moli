use v8::{Array, Function, Local, Object, PinScope, Value};

use crate::properties::get_static_property;
use crate::strings::v8_string;

/// JSON text from Inspector, preserving JavaScript's UTF-16 string contents.
/// ASCII buffers stay borrowed; a raw unpaired UTF-16 unit is escaped rather
/// than substituted with the distinct U+FFFD character.
pub fn inspector_protocol_message_json<'a>(
    view: &'a v8::inspector::StringView<'_>,
) -> std::borrow::Cow<'a, str> {
    if let Some(bytes) = view.characters8() {
        if bytes.is_ascii() {
            return std::borrow::Cow::Borrowed(
                std::str::from_utf8(bytes).expect("ASCII Inspector JSON"),
            );
        }
        return std::borrow::Cow::Owned(bytes.iter().copied().map(char::from).collect());
    }
    let mut text = String::with_capacity(view.len());
    for unit in char::decode_utf16(
        view.characters16()
            .expect("UTF-16 Inspector JSON")
            .iter()
            .copied(),
    ) {
        match unit {
            Ok(character) => text.push(character),
            Err(error) => {
                use std::fmt::Write;
                write!(&mut text, "\\u{:04x}", error.unpaired_surrogate())
                    .expect("write Inspector JSON escape");
            }
        }
    }
    std::borrow::Cow::Owned(text)
}

/// Materialize protocol text for diagnostics without substituting UTF-16 units.
pub fn inspector_protocol_message_text(view: v8::inspector::StringView<'_>) -> String {
    inspector_protocol_message_json(&view).into_owned()
}

pub fn v8_json_parse<'s>(scope: &mut PinScope<'s, '_>, json: &str) -> Option<Local<'s, Value>> {
    let global = scope.get_current_context().global(scope);
    let json_object = get_static_property(scope, global, "JSON")
        .and_then(|value| Local::<Object>::try_from(value).ok())?;
    let parse = get_static_property(scope, json_object, "parse")
        .and_then(|value| Local::<Function>::try_from(value).ok())?;
    let json = v8_string(scope, json)?;
    parse.call(scope, json_object.into(), &[json.into()])
}

pub fn array_contains_strict(
    scope: &mut PinScope<'_, '_>,
    array: Local<'_, Array>,
    expected: Local<'_, Value>,
) -> bool {
    for index in 0..array.length() {
        if array
            .get_index(scope, index)
            .is_some_and(|candidate| candidate.strict_equals(expected))
        {
            return true;
        }
    }
    false
}

pub fn array_push_value(
    scope: &mut PinScope<'_, '_>,
    array: Local<'_, Array>,
    value: Local<'_, Value>,
) {
    let _ = array.set_index(scope, array.length(), value);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{inspector_protocol_message_json, inspector_protocol_message_text};

    #[test]
    fn inspector_protocol_ascii_bytes_decode_without_an_intermediate_string() {
        let raw = br#"{"id":7,"result":{"value":"\u7f51\u6613"}}"#;
        let view = v8::inspector::StringView::from(&raw[..]);
        let text = inspector_protocol_message_json(&view);
        assert!(matches!(text, std::borrow::Cow::Borrowed(_)));
        let decoded: serde_json::Value = serde_json::from_str(&text).expect("valid Inspector JSON");

        assert_eq!(decoded, json!({"id": 7, "result": {"value": "网易"}}));
    }

    #[test]
    fn inspector_protocol_latin1_and_utf16_views_keep_existing_text_semantics() {
        let latin1 = b"{\"value\":\"caf\xe9\"}";
        let decoded = serde_json::from_str::<serde_json::Value>(&inspector_protocol_message_json(
            &v8::inspector::StringView::from(&latin1[..]),
        ))
        .expect("valid Latin-1 Inspector JSON");
        assert_eq!(decoded, json!({"value": "café"}));

        let utf16 = "{\"value\":\"中文\"}".encode_utf16().collect::<Vec<_>>();
        let decoded = serde_json::from_str::<serde_json::Value>(&inspector_protocol_message_json(
            &v8::inspector::StringView::from(&utf16[..]),
        ))
        .expect("valid UTF-16 Inspector JSON");
        assert_eq!(decoded, json!({"value": "中文"}));
    }

    #[test]
    fn inspector_protocol_diagnostics_preserve_latin1_and_utf16_message_text() {
        let latin1 = b"{broken caf\xe9";
        assert_eq!(
            inspector_protocol_message_text(v8::inspector::StringView::from(&latin1[..])),
            "{broken café"
        );

        let utf16 = "{broken 中文".encode_utf16().collect::<Vec<_>>();
        assert_eq!(
            inspector_protocol_message_text(v8::inspector::StringView::from(&utf16[..])),
            "{broken 中文"
        );
    }
    #[test]
    fn inspector_utf16_preserves_lone_units_replacement_character_and_valid_pairs() {
        let mut units: Vec<u16> = r#"{"value":""#.encode_utf16().collect();
        units.extend([0xd800, 0x61, 0xdc00, 0xfffd, 0xd83d, 0xde00]);
        units.extend(r#""}"#.encode_utf16());
        let view = v8::inspector::StringView::from(&units[..]);
        assert_eq!(
            inspector_protocol_message_json(&view),
            r#"{"value":"\ud800a\udc00�😀"}"#
        );
    }

    #[test]
    fn inspector_ascii_keeps_literal_backslashes_distinct_from_utf16_escapes() {
        for bytes in [
            br#"{"value":"\ud800"}"#.as_slice(),
            br#"{"value":"\\ud800"}"#.as_slice(),
        ] {
            let view = v8::inspector::StringView::from(bytes);
            assert_eq!(inspector_protocol_message_json(&view).as_bytes(), bytes);
        }
    }
}
