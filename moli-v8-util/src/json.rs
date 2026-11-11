use v8::{Array, Function, Local, Object, PinScope, Value};

use crate::properties::get_static_property;
use crate::strings::v8_string;

/// Retains validated Inspector JSON fields without materializing a Value tree.
pub fn inspector_protocol_json(
    view: v8::inspector::StringView<'_>,
) -> serde_json::Result<moli_inspector_json::InspectorJson> {
    if let Some(bytes) = view.characters8()
        && bytes.is_ascii()
    {
        return moli_inspector_json::InspectorJson::from_slice(bytes);
    }
    if view.is_8bit() {
        return moli_inspector_json::InspectorJson::from_slice(
            inspector_protocol_message_text(view).as_bytes(),
        );
    }
    use std::fmt::Write as _;
    let units = view.characters16().expect("16-bit Inspector view");
    let mut text = String::with_capacity(units.len());
    for scalar in char::decode_utf16(units.iter().copied()) {
        match scalar {
            Ok(scalar) => text.push(scalar),
            Err(error) => write!(&mut text, "\\u{:04x}", error.unpaired_surrogate())
                .expect("writing a String is infallible"),
        }
    }
    moli_inspector_json::InspectorJson::from_slice(text.as_bytes())
}

/// Materializes an Inspector string view for diagnostics and uncommon text
/// decoding paths. Callers on the ASCII JSON fast path should avoid this so a
/// successful protocol message does not allocate an intermediate `String`.
pub fn inspector_protocol_message_text(view: v8::inspector::StringView<'_>) -> String {
    if let Some(bytes) = view.characters8() {
        return bytes.iter().copied().map(char::from).collect();
    }
    String::from_utf16_lossy(
        view.characters16()
            .expect("a non-8-bit Inspector StringView must expose UTF-16 units"),
    )
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

    use super::{inspector_protocol_json, inspector_protocol_message_text};

    #[test]
    fn inspector_protocol_raw_json_preserves_escaped_and_actual_utf16_surrogates() {
        let raw = br#"{"id":7,"result":{"value":"\ud800"}}"#;
        let message =
            super::inspector_protocol_json(v8::inspector::StringView::from(&raw[..])).unwrap();
        assert!(message.to_string().contains(r"\ud800"));
        assert!(!message.is_materialized());
        let mut units = "{\"id\":7,\"result\":{\"value\":\"中文"
            .encode_utf16()
            .collect::<Vec<_>>();
        units.push(0xd800);
        units.extend("\"}}".encode_utf16());
        let message =
            super::inspector_protocol_json(v8::inspector::StringView::from(&units[..])).unwrap();
        assert!(message.to_string().contains(r"\ud800"));
        assert!(message.to_string().contains("中文"));
        assert!(!message.is_materialized());
        assert_eq!(message.value()["result"]["value"], "中文�");
        assert!(message.to_string().contains(r"\ud800"));
    }

    #[test]
    fn inspector_protocol_latin1_and_raw_utf16_surrogates_use_one_replacement() {
        let latin1 = b"{\"value\":\"caf\xe9\\ud800\"}";
        assert_eq!(
            inspector_protocol_json(v8::inspector::StringView::from(&latin1[..]))
                .unwrap()
                .into_value(),
            json!({"value": "café�"}),
        );
        let mut units = "{\"value\":\"中文".encode_utf16().collect::<Vec<_>>();
        units.push(0xd800);
        units.extend("\"}".encode_utf16());
        assert_eq!(
            inspector_protocol_json(v8::inspector::StringView::from(&units[..]))
                .unwrap()
                .into_value(),
            json!({"value": "中文�"}),
        );
    }

    #[test]
    fn inspector_protocol_ascii_bytes_decode_without_an_intermediate_string() {
        let raw = br#"{"id":7,"result":{"value":"\u7f51\u6613"}}"#;
        let decoded = inspector_protocol_json(v8::inspector::StringView::from(&raw[..]))
            .expect("valid Inspector JSON");

        assert_eq!(
            decoded.value(),
            &json!({"id": 7, "result": {"value": "网易"}})
        );
    }

    #[test]
    fn inspector_protocol_latin1_and_utf16_views_keep_existing_text_semantics() {
        let latin1 = b"{\"value\":\"caf\xe9\"}";
        let decoded = inspector_protocol_json(v8::inspector::StringView::from(&latin1[..]))
            .expect("valid Latin-1 Inspector JSON");
        assert_eq!(decoded.value(), &json!({"value": "café"}));

        let utf16 = "{\"value\":\"中文\"}".encode_utf16().collect::<Vec<_>>();
        let decoded = inspector_protocol_json(v8::inspector::StringView::from(&utf16[..]))
            .expect("valid UTF-16 Inspector JSON");
        assert_eq!(decoded.value(), &json!({"value": "中文"}));
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
}
