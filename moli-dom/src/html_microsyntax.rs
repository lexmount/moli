// SPDX-License-Identifier: MIT OR Apache-2.0

/// Parses the HTML rules for non-negative integers.
///
/// Only HTML whitespace is skipped. A leading plus and trailing garbage are
/// accepted, minus zero is accepted, and values outside `u32` are rejected.
pub fn parse_non_negative_integer(value: &str) -> Option<u32> {
    let value = value.trim_start_matches([' ', '\t', '\n', '\r', '\u{000c}']);
    let (value, negative) = if let Some(value) = value.strip_prefix('+') {
        (value, false)
    } else if let Some(value) = value.strip_prefix('-') {
        (value, true)
    } else {
        (value, false)
    };
    let digits = value.bytes().take_while(u8::is_ascii_digit).count();
    let parsed = (digits != 0)
        .then(|| &value[..digits])
        .and_then(|value| value.parse().ok())?;
    (!negative || parsed == 0).then_some(parsed)
}

#[cfg(test)]
mod tests {
    use super::parse_non_negative_integer;

    #[test]
    fn non_negative_integer_matches_html_prefix_rules() {
        assert_eq!(parse_non_negative_integer("  +12px"), Some(12));
        assert_eq!(parse_non_negative_integer("0"), Some(0));
        assert_eq!(parse_non_negative_integer("-0garbage"), Some(0));
        assert_eq!(parse_non_negative_integer("-1"), None);
        assert_eq!(parse_non_negative_integer("not-a-number"), None);
        assert_eq!(parse_non_negative_integer("4294967296"), None);
        assert_eq!(parse_non_negative_integer("\u{000b}12"), None);
    }
}
