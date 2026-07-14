pub fn normalize_custom_validation_message(message: &str) -> String {
    message.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn normalize_form_submission_newlines(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                normalized.push('\r');
                normalized.push('\n');
                if chars.peek() == Some(&'\n') {
                    let _ = chars.next();
                }
            }
            '\n' => {
                normalized.push('\r');
                normalized.push('\n');
            }
            _ => normalized.push(ch),
        }
    }
    normalized
}

pub fn parse_non_negative_integer_prefix(value: &str) -> i32 {
    let digits = integer_prefix_digits(value);
    if digits.is_empty() {
        0
    } else {
        digits.parse::<i32>().unwrap_or(0)
    }
}

pub fn parse_positive_integer_prefix(value: &str) -> Option<u32> {
    let digits = integer_prefix_digits(value);
    digits
        .parse::<i32>()
        .ok()
        .filter(|value| *value > 0)
        .map(|value| value as u32)
}

pub fn parse_non_negative_length_attribute(value: &str) -> Option<usize> {
    parse_html_integer_prefix(value)
        .filter(|value| *value >= 0)
        .map(|value| value as usize)
}

pub fn text_control_value_length(value: &str) -> usize {
    value.encode_utf16().count()
}

pub fn text_control_suffers_too_long(value: &str, max_length: Option<&str>) -> bool {
    max_length
        .and_then(parse_non_negative_length_attribute)
        .is_some_and(|max| text_control_value_length(value) > max)
}

pub fn text_control_suffers_too_short(value: &str, min_length: Option<&str>) -> bool {
    let value_len = text_control_value_length(value);
    value_len > 0
        && min_length
            .and_then(parse_non_negative_length_attribute)
            .is_some_and(|min| value_len < min)
}

fn integer_prefix_digits(value: &str) -> &str {
    let value = value.trim_start();
    let end = value
        .bytes()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(value.len());
    &value[..end]
}

fn parse_html_integer_prefix(value: &str) -> Option<i32> {
    let value = value.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
    let mut chars = value.chars();
    let (sign, rest) = match chars.next() {
        Some('+') => (1_i64, chars.as_str()),
        Some('-') => (-1_i64, chars.as_str()),
        Some(_) => (1_i64, value),
        None => return None,
    };
    let digits = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    let value = sign * digits.parse::<i64>().ok()?;
    i32::try_from(value).ok()
}
