use encoding_rs::Encoding;

use crate::encoding_for_label;

/// Decode CSS bytes using CSS Syntax's fallback selection and Encoding's BOM
/// precedence. Return the actual encoding for descendant @import requests.
pub fn decode_css_stylesheet(
    bytes: &[u8],
    protocol_encoding_label: Option<&str>,
    environment_encoding: Option<&str>,
) -> (String, &'static str) {
    let fallback = protocol_encoding_label
        .and_then(encoding_for_label)
        .or_else(|| charset_encoding(bytes))
        .or_else(|| environment_encoding.and_then(encoding_for_label))
        .unwrap_or(encoding_rs::UTF_8);
    let (text, actual_encoding, _) = fallback.decode(bytes);
    (text.into_owned(), actual_encoding.name())
}

fn charset_encoding(bytes: &[u8]) -> Option<&'static Encoding> {
    let prefix = bytes[..bytes.len().min(1024)].strip_prefix(b"@charset \"")?;
    let end = prefix.iter().position(|byte| *byte == b'"')?;
    if !prefix[end..].starts_with(b"\";") || !prefix[..end].is_ascii() {
        return None;
    }
    let encoding = Encoding::for_label(&prefix[..end])?;
    Some(
        if encoding == encoding_rs::UTF_16BE || encoding == encoding_rs::UTF_16LE {
            encoding_rs::UTF_8
        } else {
            encoding
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stylesheet_charset_sniffing_stops_at_1024_bytes() {
        for (length, body, expected_encoding) in [
            (1024, "€".as_bytes(), "UTF-8"),
            (1025, b"\x80".as_slice(), "windows-1252"),
        ] {
            let mut bytes = format!("@charset \"{}utf-8\";", " ".repeat(length - 17)).into_bytes();
            assert_eq!(bytes.len(), length);
            bytes.extend_from_slice(body);
            let (text, encoding) = decode_css_stylesheet(&bytes, None, Some("windows-1252"));
            assert_eq!(encoding, expected_encoding);
            assert!(text.ends_with('€'));
        }
    }
}
