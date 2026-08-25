use encoding_rs::Encoding;
use moli_header_field::{split_outside_quoted_strings, unquote_parameter_value};

pub fn encoding_for_label(label: &str) -> Option<&'static Encoding> {
    Encoding::for_label(label.trim().as_bytes())
}

/// The `charset` parameter of a `Content-Type` value.
///
/// Parameters are separated by the `;` characters outside a quoted string, and
/// a quoted value has its quoting backslashes removed, so a `charset` written
/// inside another parameter's quoted string is not read as a parameter here.
pub fn charset_from_content_type(value: &str) -> Option<String> {
    // The first segment is the media type itself, not a parameter.
    for parameter in split_outside_quoted_strings(value, ';').into_iter().skip(1) {
        let Some((name, parameter_value)) = parameter.split_once('=') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("charset") {
            continue;
        }
        let parameter_value = parameter_value.trim();
        let charset = if parameter_value.starts_with('"') {
            // Delimiters are already gone, so any `"` left is data produced by
            // an escaped quote and must not be trimmed away.
            unquote_parameter_value(parameter_value).into_owned()
        } else {
            // Apostrophe delimiters are not a quoted string, but receivers
            // have long tolerated them here, so keep stripping them.
            parameter_value
                .trim_matches(|ch| ch == '"' || ch == '\'')
                .trim()
                .to_owned()
        };
        if !charset.is_empty() {
            return Some(charset);
        }
    }
    None
}

pub fn charset_from_headers(headers: &[(String, String)]) -> Option<String> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .and_then(|(_, value)| charset_from_content_type(value))
}
