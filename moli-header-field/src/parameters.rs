use std::borrow::Cow;

/// Splits `value` on every `separator` that is not inside a quoted parameter
/// value.
///
/// Structured header field values carry their parameters after a separator —
/// `;` after a media type, `,` between list members — but a quoted value may
/// contain that separator without ending the parameter it belongs to. Plain
/// splitting lets such a separator escape the quoted value, so text that is
/// part of one parameter is read as a parameter of its own.
///
/// Quoting only begins where a parameter value begins, which is the first
/// non-whitespace character after `=`. A `"` anywhere else is ordinary data
/// and does not open a quoted string, so a stray quote cannot swallow the rest
/// of the field and hide the parameters behind it.
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
    let mut at_value_start = false;
    let mut inside_quotes = false;
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];

        if inside_quotes {
            if byte == b'\\' {
                // Step over the escaped byte so a quoted `\"` does not close
                // the value.
                index += 2;
                continue;
            }
            if byte == b'"' {
                inside_quotes = false;
            }
            index += 1;
            continue;
        }

        if byte == separator {
            segments.push(&value[segment_start..index]);
            segment_start = index + 1;
            at_value_start = false;
        } else if byte == b'=' {
            at_value_start = true;
        } else if byte == b'"' && at_value_start {
            inside_quotes = true;
            at_value_start = false;
        } else if !matches!(byte, b' ' | b'\t') || !at_value_start {
            // Whitespace between `=` and the value keeps the value still to
            // come; anything else means the value was not quoted.
            at_value_start = false;
        }
        index += 1;
    }

    segments.push(&value[segment_start.min(value.len())..]);
    segments
}

/// Removes a quoted value's delimiters and its quoting backslashes.
///
/// A value that is not quoted is returned as-is. An unterminated quoted value
/// yields everything that was read rather than discarding the parameter, and a
/// trailing `\` with nothing to escape is kept, so a malformed value is not
/// quietly turned into a well-formed one.
pub fn unquote_parameter_value(raw: &str) -> Cow<'_, str> {
    let Some(quoted) = raw.strip_prefix('"') else {
        return Cow::Borrowed(raw);
    };
    let mut unquoted = String::with_capacity(quoted.len());
    let mut characters = quoted.chars();
    while let Some(character) = characters.next() {
        match character {
            '"' => return Cow::Owned(unquoted),
            '\\' => match characters.next() {
                Some(escaped) => unquoted.push(escaped),
                None => unquoted.push('\\'),
            },
            _ => unquoted.push(character),
        }
    }
    Cow::Owned(unquoted)
}
