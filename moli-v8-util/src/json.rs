use serde_json::Value as JsonValue;
use v8::{Array, Function, Local, Object, PinScope, Value};

use crate::properties::get_static_property;
use crate::strings::v8_string;

/// Decodes one V8 Inspector protocol message without first materializing an
/// additional UTF-8 `String` when V8 already exposes ASCII JSON bytes.
///
/// Inspector's 8-bit string view contains Latin-1 code units rather than an
/// unconditional UTF-8 byte string. JSON emitted by V8 is normally ASCII at
/// this boundary (non-ASCII string contents are escaped), so that common path
/// can be parsed in place. The non-ASCII 8-bit and 16-bit paths retain the
/// prior text conversion semantics.
///
/// JavaScript strings may contain unpaired UTF-16 surrogates. Replace these
/// with U+FFFD at the Rust UTF-8 boundary, consistently with raw UTF-16 views,
/// while retaining valid pairs and literal backslash sequences.
pub fn decode_inspector_protocol_message(
    view: v8::inspector::StringView<'_>,
) -> serde_json::Result<JsonValue> {
    if let Some(bytes) = view.characters8() {
        if bytes.is_ascii() {
            return moli_inspector_json::decode_lossy_json(bytes);
        }
        let text = inspector_protocol_message_text(view);
        return moli_inspector_json::decode_lossy_json(text.as_bytes());
    }

    let text = inspector_protocol_message_text(view);
    moli_inspector_json::decode_lossy_json(text.as_bytes())
}

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
    use std::fmt::Write;

    use super::{decode_inspector_protocol_message, inspector_protocol_message_text};

    fn assert_inspector_views(raw: &str, expected: serde_json::Value) {
        assert!(raw.is_ascii());
        let decoded =
            decode_inspector_protocol_message(v8::inspector::StringView::from(raw.as_bytes()))
                .expect("Inspector JSON must accept UTF-16 string contents");
        assert_eq!(decoded, expected, "8-bit input: {raw}");
        let units = raw.encode_utf16().collect::<Vec<_>>();
        let decoded =
            decode_inspector_protocol_message(v8::inspector::StringView::from(&units[..]))
                .expect("UTF-16 Inspector JSON must accept UTF-16 string contents");
        assert_eq!(decoded, expected, "16-bit input: {raw}");
    }

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
    fn inspector_protocol_lone_surrogates_preserve_pairs_literals_and_structure() {
        for (escaped, expected) in [
            (r"\ud800", "�"),
            (r"\udfff", "�"),
            (r"\uD800\uD800\uDC00", "�𐀀"),
            (r"\uDC00\uD800\uDC00", "�𐀀"),
            (r"\uD83D\uDE00\ud800", "😀�"),
            (r"\ud800a\udfff", "�a�"),
            (r"\ud800\u0061", "�a"),
            (r"\u0000\ud800", "\0�"),
        ] {
            assert_inspector_views(
                &format!(r#"{{"id":7,"result":{{"value":"{escaped}"}}}}"#),
                json!({"id": 7, "result": {"value": expected}}),
            );
        }
        assert_inspector_views(
            r#"{"\ud800":["\udfff",{"literal":"\\ud800","paired":"\uD83D\uDE00"}]}"#,
            json!({"�": ["�", {"literal": r"\ud800", "paired": "😀"}]}),
        );
        assert_inspector_views(
            r#"{"value":"a\"\\\ud800\\udfff"}"#,
            json!({"value": "a\"\\�\\udfff"}),
        );
    }

    #[test]
    fn inspector_protocol_replaces_every_isolated_surrogate_code_unit() {
        for unit in 0xd800_u16..=0xdfff {
            assert_inspector_views(
                &format!(r#"{{"value":"\u{unit:04x}"}}"#),
                json!({"value": "�"}),
            );
        }
    }

    #[test]
    fn inspector_protocol_mixed_escapes_match_standard_utf16_lossy_decoding() {
        let alphabet = [
            0xd800, 0xdbff, 0xdc00, 0xdfff, 0x5c, 0x22, 0x41, 0xa, 0, 0xffff, 0xfeff,
        ];
        let mut seed = 0xc0de_2026_u64;
        for _ in 0..256 {
            let mut units = Vec::new();
            let mut raw = String::from(r#"{"value":""#);
            for _ in 0..64 {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let unit = if seed & 1 == 0 {
                    alphabet[(seed >> 32) as usize % alphabet.len()]
                } else {
                    (seed >> 32) as u16
                };
                units.push(unit);
                write!(&mut raw, "\\u{unit:04x}").unwrap();
            }
            raw.push_str(r#"","literal":"\\ud800"}"#);
            assert_inspector_views(
                &raw,
                json!({"value": String::from_utf16_lossy(&units), "literal": r"\ud800"}),
            );
        }
    }

    #[test]
    fn inspector_protocol_surrogate_recovery_keeps_malformed_json_invalid() {
        for raw in [
            r#"{"value":"\ud800\q"}"#,
            r#"{"value":"\ud800","other":}"#,
            r#"{"value":"\uD80X"}"#,
            r#"{"value":"\uD800\uDE0X"}"#,
            r#"{"value":"\ud800"} trailing"#,
            r#"{"value":\ud800}"#,
            "{\"value\":\"\\ud800\n\"}",
        ] {
            assert!(
                decode_inspector_protocol_message(v8::inspector::StringView::from(raw.as_bytes()))
                    .is_err(),
                "{raw}"
            );
        }
    }

    #[test]
    fn inspector_protocol_latin1_and_raw_utf16_surrogates_use_one_replacement() {
        let latin1 = b"{\"value\":\"caf\xe9\\ud800\"}";
        assert_eq!(
            decode_inspector_protocol_message(v8::inspector::StringView::from(&latin1[..]))
                .unwrap(),
            json!({"value": "café�"}),
        );
        let mut units = "{\"value\":\"中文".encode_utf16().collect::<Vec<_>>();
        units.push(0xd800);
        units.extend("\"}".encode_utf16());
        assert_eq!(
            decode_inspector_protocol_message(v8::inspector::StringView::from(&units[..])).unwrap(),
            json!({"value": "中文�"}),
        );
    }

    #[test]
    fn inspector_protocol_ascii_bytes_decode_without_an_intermediate_string() {
        let raw = br#"{"id":7,"result":{"value":"\u7f51\u6613"}}"#;
        let decoded = decode_inspector_protocol_message(v8::inspector::StringView::from(&raw[..]))
            .expect("valid Inspector JSON");

        assert_eq!(decoded, json!({"id": 7, "result": {"value": "网易"}}));
    }

    #[test]
    fn inspector_protocol_latin1_and_utf16_views_keep_existing_text_semantics() {
        let latin1 = b"{\"value\":\"caf\xe9\"}";
        let decoded =
            decode_inspector_protocol_message(v8::inspector::StringView::from(&latin1[..]))
                .expect("valid Latin-1 Inspector JSON");
        assert_eq!(decoded, json!({"value": "café"}));

        let utf16 = "{\"value\":\"中文\"}".encode_utf16().collect::<Vec<_>>();
        let decoded =
            decode_inspector_protocol_message(v8::inspector::StringView::from(&utf16[..]))
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
}
