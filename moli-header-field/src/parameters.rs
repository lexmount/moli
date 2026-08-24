use std::borrow::Cow;

/// Splits `value` on every `separator` that sits outside a quoted string.
///
/// Structured header field values carry their parameters after a separator —
/// `;` after a media type, `,` between list members — but a quoted string may
/// contain that separator without ending the parameter it belongs to. Plain
/// splitting lets such a separator escape the quoted string, so text that is
/// part of one parameter's value is read as a parameter of its own.
///
/// `separator` must be an ASCII character. A UTF-8 continuation byte can never
/// equal one, so every returned slice falls on a character boundary.
pub fn split_outside_quoted_strings(value: &str, separator: char) -> Vec<&str> {
    debug_assert!(
        separator.is_ascii(),
        "a non-ASCII separator cannot be matched bytewise"
    );
    let separator = separator as u8;
    let bytes = value.as_bytes();
    let mut segments = Vec::new();
    let mut segment_start = 0;
    let mut inside_quotes = false;
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'"' {
            inside_quotes = !inside_quotes;
        } else if byte == b'\\' && inside_quotes {
            // Step over the escaped byte so a quoted `\"` does not close the
            // string it appears in.
            index += 1;
        } else if byte == separator && !inside_quotes {
            segments.push(&value[segment_start..index]);
            segment_start = index + 1;
        }
        index += 1;
    }

    segments.push(&value[segment_start.min(value.len())..]);
    segments
}

/// Removes a quoted string's delimiters and its quoting backslashes.
///
/// A value that is not quoted is returned as-is. An unterminated quoted string
/// yields everything that was read rather than discarding the parameter, which
/// is how receivers generally treat one.
pub fn unquote_parameter_value(raw: &str) -> Cow<'_, str> {
    let Some(quoted) = raw.strip_prefix('"') else {
        return Cow::Borrowed(raw);
    };
    let mut unquoted = String::with_capacity(quoted.len());
    let mut characters = quoted.chars();
    while let Some(character) = characters.next() {
        match character {
            '"' => return Cow::Owned(unquoted),
            '\\' => {
                if let Some(escaped) = characters.next() {
                    unquoted.push(escaped);
                }
            }
            _ => unquoted.push(character),
        }
    }
    Cow::Owned(unquoted)
}
