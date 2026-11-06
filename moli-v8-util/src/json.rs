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
            return decode_inspector_json(bytes);
        }
        let text = inspector_protocol_message_text(view);
        return decode_inspector_json(text.as_bytes());
    }

    let text = inspector_protocol_message_text(view);
    decode_inspector_json(text.as_bytes())
}

fn decode_inspector_json(bytes: &[u8]) -> serde_json::Result<JsonValue> {
    match serde_json::from_slice(bytes) {
        Ok(value) => Ok(value),
        Err(error) => {
            // Keep ordinary Inspector messages on the allocation-free byte
            // path. Only retry after repairing an unpaired surrogate escape;
            // serde_json still validates the complete JSON syntax.
            let Some(repaired) = replace_unpaired_surrogate_escapes(bytes) else {
                return Err(error);
            };
            serde_json::from_slice(&repaired)
        }
    }
}

fn replace_unpaired_surrogate_escapes(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut repaired = None;
    let mut in_string = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                in_string = !in_string;
                index += 1;
            }
            b'\\' if in_string => {
                let Some(unit) = escaped_utf16_unit(&bytes[index..]) else {
                    // Skip an escaped quote or backslash as a single token.
                    // Invalid escapes remain unchanged for serde_json.
                    index += 2;
                    continue;
                };
                if (0xd800..=0xdbff).contains(&unit)
                    && escaped_utf16_unit(&bytes[index + 6..])
                        .is_some_and(|next| (0xdc00..=0xdfff).contains(&next))
                {
                    index += 12;
                    continue;
                }
                if (0xd800..=0xdfff).contains(&unit) {
                    let output = repaired.get_or_insert_with(|| bytes.to_vec());
                    output[index..index + 6].copy_from_slice(br"\ufffd");
                }
                index += 6;
            }
            _ => index += 1,
        }
    }
    repaired
}

fn escaped_utf16_unit(bytes: &[u8]) -> Option<u16> {
    bytes
        .strip_prefix(br"\u")?
        .get(..4)?
        .iter()
        .try_fold(0_u16, |unit, digit| {
            char::from(*digit)
                .to_digit(16)
                .map(|digit| (unit << 4) | digit as u16)
        })
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
