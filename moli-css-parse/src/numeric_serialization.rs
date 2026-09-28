//! Numeric serialization for Typed OM values and CSS token streams.
//!
//! A CSS numeric value uses CSSOM formatting; raw number/dimension tokens need
//! round-trippable double precision and must retain their lexical token type.

use cssparser::{ToCss, Token, serialize_identifier};
use std::fmt::{self, Write};

/// CSSOM's browser-compatible six-significant-digit numeric representation.
/// Rust's exponent formatter rounds ties to even, without first scaling the
/// double (which could overflow, underflow, or introduce another rounding).
/// Non-finite values use CSS math keywords; callers must put them in calc().
pub fn serialize_css_number(value: f64) -> String {
    if !value.is_finite() {
        return if value.is_nan() {
            "NaN"
        } else if value.is_sign_negative() {
            "-infinity"
        } else {
            "infinity"
        }
        .to_owned();
    }
    if value == 0.0 {
        return "0".to_owned();
    }
    let rounded = format!("{value:.5e}");
    let (mantissa, exponent) = rounded.split_once('e').expect("exponent notation");
    let exponent: i32 = exponent.parse().expect("numeric exponent");
    if !(-4..6).contains(&exponent) {
        return format!(
            "{}e{exponent:+03}",
            mantissa.trim_end_matches('0').trim_end_matches('.')
        );
    }
    let negative = mantissa.starts_with('-');
    let digits = mantissa.trim_start_matches('-').replace('.', "");
    let point = exponent + 1;
    let mut output = if negative {
        "-".to_owned()
    } else {
        String::new()
    };
    if point <= 0 {
        output.push_str("0.");
        output.extend(std::iter::repeat_n('0', (-point) as usize));
        output.push_str(&digits);
    } else {
        let point = point as usize;
        output.push_str(&digits[..point]);
        if point < digits.len() {
            output.push('.');
            output.push_str(&digits[point..]);
        }
    }
    if output.contains('.') {
        output.truncate(output.trim_end_matches('0').trim_end_matches('.').len());
    }
    output
}

// CSSParserToken in Chromium retains double precision but bounds the numeric
// range by f32::MAX. The source slice matters: cssparser's Token stores f32
// values and saturates its integer field at i32 boundaries.
pub fn css_numeric_token_value(token: &Token<'_>, source: &str) -> Option<f64> {
    let number = numeric_source(token, source)?.parse::<f64>().ok()?;
    Some(number.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)))
}

fn numeric_source<'a>(token: &Token<'_>, source: &'a str) -> Option<&'a str> {
    match token {
        Token::Number { .. } => Some(source),
        Token::Percentage { .. } => source.strip_suffix('%'),
        Token::Dimension { .. } => {
            // The tokenizer already validated the number. Find the boundary
            // without consuming the e in a unit such as em; escaped units are
            // handled by the original token rather than by slicing its name.
            let bytes = source.as_bytes();
            let mut end = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
            while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
            if bytes.get(end) == Some(&b'.') {
                end += 1;
                while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                }
            }
            if matches!(bytes.get(end), Some(b'e' | b'E')) {
                let mut exponent = end + 1;
                if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
                    exponent += 1;
                }
                if bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
                    end = exponent + 1;
                    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                        end += 1;
                    }
                }
            }
            Some(&source[..end])
        }
        _ => None,
    }
}

/// Serialize one tokenizer-produced token with its exact consumed source slice.
/// The caller owns token boundaries and insertion of any required separators.
pub fn serialize_css_token(token: &Token<'_>, source: &str, output: &mut String) -> fmt::Result {
    let Some(value) = css_numeric_token_value(token, source) else {
        return token.to_css(output);
    };
    match token {
        Token::Number { .. } => {
            if !source.contains(['.', 'e', 'E']) {
                // Match the browser's integer-token range, retaining values
                // beyond i32 without silently making an integer token a float.
                write!(output, "{}", value as i64)?;
            } else {
                let text = shortest_number(value);
                output.push_str(&text);
                if !text.contains(['.', 'e']) {
                    output.push_str(".0");
                }
            }
        }
        Token::Percentage { .. } => {
            output.push_str(&serialize_css_number(value));
            output.push('%');
        }
        Token::Dimension { unit, .. } => {
            output.push_str(&shortest_number(value));
            // A decoded unit such as e2 would turn 4e3e2 into a number token
            // (4000e2). Escape its leading e so re-tokenizing preserves both
            // the numeric value and the dimension type.
            let bytes = unit.as_bytes();
            let ambiguous = matches!(bytes.first(), Some(b'e' | b'E'))
                && (bytes.get(1).is_some_and(u8::is_ascii_digit)
                    || (matches!(bytes.get(1), Some(b'+' | b'-'))
                        && bytes.get(2).is_some_and(u8::is_ascii_digit)));
            if ambiguous {
                output.push_str(if bytes[0] == b'e' { "\\65 " } else { "\\45 " });
                cssparser::serialize_name(&unit[1..], output)?;
            } else {
                serialize_identifier(unit, output)?;
            }
        }
        _ => unreachable!("numeric token projection"),
    }
    Ok(())
}

fn shortest_number(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let magnitude = value.abs();
    let plain = value.to_string();
    if (1e-6..1e21).contains(&magnitude) {
        return plain;
    }
    let unsigned = plain.trim_start_matches('-');
    let point = unsigned.find('.').unwrap_or(unsigned.len());
    let digits = unsigned.replace('.', "");
    let significant = digits.trim_start_matches('0');
    let exponent = point as i32 - (digits.len() - significant.len()) as i32 - 1;
    let significant = significant.trim_end_matches('0');
    let mut output = if value.is_sign_negative() {
        "-".to_owned()
    } else {
        String::new()
    };
    output.push_str(&significant[..1]);
    if significant.len() > 1 {
        output.push('.');
        output.push_str(&significant[1..]);
    }
    write!(&mut output, "e{exponent:+}").expect("String writes cannot fail");
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use cssparser::{Parser, ParserInput};

    fn serialize(source: &str) -> String {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        let token = parser.next().unwrap().clone();
        let mut result = String::new();
        serialize_css_token(&token, source, &mut result).unwrap();
        result
    }

    #[test]
    fn css_number_serialization_rounds_once_and_preserves_the_full_double_range() {
        for (number, expected) in [
            (-0.0, "0"),
            (0.031400000000000004, "0.0314"),
            (1.234565, "1.23456"),
            (-1.234565, "-1.23456"),
            (999999.4, "999999"),
            (999999.5, "1e+06"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (f64::MAX, "1.79769e+308"),
            (f64::from_bits(1), "4.94066e-324"),
            (f64::INFINITY, "infinity"),
            (f64::NEG_INFINITY, "-infinity"),
            (f64::NAN, "NaN"),
        ] {
            assert_eq!(serialize_css_number(number), expected, "{number:?}");
        }
    }

    #[test]
    fn css_numeric_tokens_retain_precision_and_their_integer_or_number_type() {
        for (source, expected) in [
            ("0.123456789123456789px", "0.12345678912345678px"),
            ("0.123456789123456789", "0.12345678912345678"),
            ("0.123456789123456789%", "0.123457%"),
            ("+01.2300e+2px", "123px"),
            ("+01.2300e+2", "123.0"),
            ("-0", "0"),
            ("-0.0", "0.0"),
            ("-0px", "0px"),
            ("+0.0%", "0%"),
            ("2147483648", "2147483648"),
            ("9223372036854775808", "9223372036854775807"),
            ("1e100", "3.4028234663852886e+38"),
            ("1e1000px", "3.4028234663852886e+38px"),
            ("1e1000%", "3.40282e+38%"),
            ("1e-999px", "0px"),
            ("1e-7px", "1e-7px"),
            ("1e-6px", "0.000001px"),
            ("1e21px", "1e+21px"),
            ("1em", "1em"),
            ("1e2em", "100em"),
            ("1p\\78", "1px"),
        ] {
            assert_eq!(serialize(source), expected, "{source}");
        }
    }

    #[test]
    fn dimension_serialization_does_not_turn_a_unit_into_an_exponent() {
        for (source, unit, number) in [
            ("4e3e2", "e2", 4000.0),
            ("4e3\\65 2", "e2", 4000.0),
            ("1\\45 -2", "E-2", 1.0),
            ("1e21e2", "e2", 1e21),
            ("-2\\65 -foo", "e-foo", -2.0),
        ] {
            let text = serialize(source);
            let mut input = ParserInput::new(&text);
            let mut parser = Parser::new(&mut input);
            let token = parser.next().unwrap().clone();
            assert!(
                matches!(&token, Token::Dimension { unit: parsed, .. } if parsed.as_ref() == unit),
                "{source}: {text}"
            );
            assert_eq!(css_numeric_token_value(&token, &text), Some(number));
            parser.expect_exhausted().unwrap();
            assert_eq!(serialize(&text), text, "serialization must be idempotent");
        }
    }
}
