/// Whether a literal FontFace family needs CSS string serialization.
///
/// Family names are literal names, so backslashes and quotes in the input
/// are not CSS syntax to parse or unescape.
pub fn font_family_name_needs_quotes(name: &[u16]) -> bool {
    let ident = name.strip_prefix(&[b'-' as u16]).unwrap_or(name);
    let name_start = |unit: u16| {
        unit >= 0x80
            || unit == b'_' as u16
            || (b'a' as u16..=b'z' as u16).contains(&unit)
            || (b'A' as u16..=b'Z' as u16).contains(&unit)
    };
    if !ident.first().is_some_and(|unit| name_start(*unit))
        || !ident.iter().all(|unit| {
            name_start(*unit) || *unit == b'-' as u16 || (b'0' as u16..=b'9' as u16).contains(unit)
        })
    {
        return true;
    }
    let text = String::from_utf16_lossy(name);
    [
        "initial",
        "inherit",
        "unset",
        "revert",
        "revert-layer",
        "default",
    ]
    .iter()
    .any(|keyword| text.eq_ignore_ascii_case(keyword))
        || matches!(
            text.as_str(),
            "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy" | "system-ui" | "math"
        )
}

/// Serialize a literal FontFace family without interpreting it as CSS input.
pub fn serialize_font_family_name_utf16(name: &[u16]) -> Vec<u16> {
    if !font_family_name_needs_quotes(name) {
        return name.to_vec();
    }
    let mut output = Vec::with_capacity(name.len() + 2);
    output.push(b'"' as u16);
    for character in char::decode_utf16(name.iter().copied()) {
        // String serialization operates on code points. As in browser CSSOM,
        // an unpaired surrogate has no code point and is escaped as zero.
        // The native family identity remains the original UTF-16 string.
        let character = character.unwrap_or('\0');
        match character {
            '\0'..='\u{1f}' | '\u{7f}' => {
                output.extend(format!("\\{:x} ", character as u32).encode_utf16());
            }
            '"' | '\\' => {
                output.push(b'\\' as u16);
                output.push(character as u16);
            }
            _ => {
                output.extend_from_slice(character.encode_utf16(&mut [0; 2]));
            }
        }
    }
    output.push(b'"' as u16);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_family_serialization_does_not_parse_css_escapes_or_lists() {
        for (name, expected) in [
            ("Demo", "Demo"),
            ("Demo Face", "\"Demo Face\""),
            (" a  b ", "\" a  b \""),
            ("sans-serif", "\"sans-serif\""),
            ("INHERIT", "\"INHERIT\""),
            ("A, B", "\"A, B\""),
            ("A\\ B", "\"A\\\\ B\""),
            ("\"A\"", "\"\\\"A\\\"\""),
            ("--name", "\"--name\""),
            ("a\0\nb", "\"a\\0 \\a b\""),
            ("", "\"\""),
            ("漢字", "漢字"),
        ] {
            assert_eq!(
                serialize_font_family_name_utf16(&name.encode_utf16().collect::<Vec<_>>()),
                expected.encode_utf16().collect::<Vec<_>>(),
                "{name:?}"
            );
        }
    }

    #[test]
    fn literal_family_identity_keeps_unpaired_surrogates() {
        assert_eq!(serialize_font_family_name_utf16(&[0xd800]), [0xd800]);
        assert_eq!(serialize_font_family_name_utf16(&[0xdc00]), [0xdc00]);
        assert_eq!(
            serialize_font_family_name_utf16(&[0xd83d, 0xde00]),
            [0xd83d, 0xde00]
        );
        assert_eq!(
            serialize_font_family_name_utf16(&[0xd800, b' ' as u16]),
            "\"\\0  \"".encode_utf16().collect::<Vec<_>>()
        );
    }
}
