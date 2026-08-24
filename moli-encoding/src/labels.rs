use encoding_rs::Encoding;

pub fn encoding_for_label(label: &str) -> Option<&'static Encoding> {
    Encoding::for_label(label.trim().as_bytes())
}

/// The `charset` parameter of a `Content-Type` value.
///
/// Parameters are separated by the `;` characters that sit outside a quoted
/// string, and a quoted value has its quoting backslashes removed. Splitting
/// on every `;` would let a `charset` written inside another parameter's
/// quoted string escape that string and be read as a parameter of its own.
pub fn charset_from_content_type(value: &str) -> Option<String> {
    // The first segment is the media type itself, not a parameter.
    for parameter in content_type_parameters(value).into_iter().skip(1) {
        let Some((name, parameter_value)) = parameter.split_once('=') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("charset") {
            continue;
        }
        let charset = unquote_parameter_value(parameter_value.trim());
        if !charset.is_empty() {
            return Some(charset);
        }
    }
    None
}

/// Splits a `Content-Type` value on the `;` separators outside quoted strings.
fn content_type_parameters(value: &str) -> Vec<&str> {
    let bytes = value.as_bytes();
    let mut segments = Vec::new();
    let mut segment_start = 0;
    let mut inside_quotes = false;
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'"' => inside_quotes = !inside_quotes,
            // Step over the escaped byte so a quoted `\"` does not close the
            // string. Multi-byte UTF-8 is unaffected: a continuation byte can
            // never equal one of the ASCII bytes matched here.
            b'\\' if inside_quotes => index += 1,
            b';' if !inside_quotes => {
                segments.push(&value[segment_start..index]);
                segment_start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }

    segments.push(&value[segment_start.min(value.len())..]);
    segments
}

/// Unwraps a parameter value, removing quoting backslashes from a quoted one.
fn unquote_parameter_value(raw: &str) -> String {
    let Some(quoted) = raw.strip_prefix('"') else {
        return raw
            .trim_matches(|ch| ch == '"' || ch == '\'')
            .trim()
            .to_owned();
    };
    let mut unquoted = String::with_capacity(quoted.len());
    let mut characters = quoted.chars();
    while let Some(character) = characters.next() {
        match character {
            '"' => return unquoted,
            '\\' => {
                if let Some(escaped) = characters.next() {
                    unquoted.push(escaped);
                }
            }
            _ => unquoted.push(character),
        }
    }
    // An unterminated quoted string keeps what was read, which is what the
    // previous `trim_matches` behavior did for `charset="gbk`.
    unquoted
}

pub fn charset_from_headers(headers: &[(String, String)]) -> Option<String> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .and_then(|(_, value)| charset_from_content_type(value))
}
