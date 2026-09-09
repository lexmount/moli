//! HTML canvas bitmap dimensions, independent of CSS sizing and pixel storage.

/// The canvas coordinate space exposed by the width/height IDL attributes and
/// used as its natural size. Parsing does not allocate or create a bitmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanvasDimensions {
    pub width: u32,
    pub height: u32,
}

impl Default for CanvasDimensions {
    fn default() -> Self {
        Self {
            width: 300,
            height: 150,
        }
    }
}

impl CanvasDimensions {
    pub fn from_attributes(width: Option<&str>, height: Option<&str>) -> Self {
        let default = Self::default();
        Self {
            width: width
                .and_then(parse_dimension_attribute)
                .filter(|value| *value <= i32::MAX as u32)
                .unwrap_or(default.width),
            height: height
                .and_then(parse_dimension_attribute)
                .filter(|value| *value <= i32::MAX as u32)
                .unwrap_or(default.height),
        }
    }
}

/// Parse a canvas dimension content attribute as an HTML non-negative integer.
/// The aspect-ratio presentation hint uses this value without bitmap defaults
/// or the bitmap's signed 32-bit limit. Percentage/decimal suffixes are ignored.
pub fn parse_dimension_attribute(value: &str) -> Option<u32> {
    let value = value.trim_start_matches(['\t', '\n', '\u{000c}', '\r', ' ']);
    let (digits, negative) = match value.as_bytes().first() {
        Some(b'+') => (&value.as_bytes()[1..], false),
        Some(b'-') => (&value.as_bytes()[1..], true),
        _ => (value.as_bytes(), false),
    };
    if !digits.first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let mut number = 0_u32;
    for digit in digits.iter().take_while(|digit| digit.is_ascii_digit()) {
        number = number
            .checked_mul(10)?
            .checked_add(u32::from(digit - b'0'))?;
    }
    (!negative || number == 0).then_some(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_attributes_retain_integers_outside_the_bitmap_range() {
        assert_eq!(parse_dimension_attribute("2147483648"), Some(2147483648));
        assert_eq!(parse_dimension_attribute("4294967295"), Some(u32::MAX));
        assert_eq!(parse_dimension_attribute("4294967296"), None);
        assert_eq!(parse_dimension_attribute("+0.5"), Some(0));
        assert_eq!(parse_dimension_attribute("90%"), Some(90));
        assert_eq!(parse_dimension_attribute("-1"), None);
    }

    #[test]
    fn dimensions_are_independent_bitmap_integers_not_css_lengths() {
        assert_eq!(
            CanvasDimensions::from_attributes(None, None),
            CanvasDimensions::default()
        );
        for (attribute, width) in [
            ("600", 600),
            ("60.5", 60),
            ("60%", 60),
            (" +60px", 60),
            ("-60", 300),
            ("0", 0),
            ("-0.5", 0),
            ("00012", 12),
            ("", 300),
            (".5", 300),
            ("\u{000b}60", 300),
            ("2147483647", 2147483647),
            ("2147483648", 300),
            ("999999999999999999999999999999", 300),
        ] {
            assert_eq!(
                CanvasDimensions::from_attributes(Some(attribute), None),
                CanvasDimensions { width, height: 150 },
                "{attribute:?}",
            );
        }
        assert_eq!(
            CanvasDimensions::from_attributes(Some("600"), Some("-10")),
            CanvasDimensions {
                width: 600,
                height: 150
            },
        );
        assert_eq!(
            CanvasDimensions::from_attributes(None, Some("0")),
            CanvasDimensions {
                width: 300,
                height: 0
            },
        );
    }
}
