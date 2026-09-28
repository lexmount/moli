use cssparser::{Parser, ParserInput, Token};

// The property grammar and numeric type have already been established by
// Stylo. Recover a single numeric token's original double precision instead
// of exposing Stylo/cssparser's f32 rounding as a WebIDL double. Expressions
// and property-specific conversions beyond number/percentage use Stylo's value.
pub(super) fn from_literal(source: &str, target_unit: &str) -> Option<f64> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parser.skip_whitespace();
    let start = parser.position();
    let token = parser.next().ok()?.clone();
    let text = parser.slice_from(start);
    parser.expect_exhausted().ok()?;
    let number = match token {
        Token::Number { .. } => {
            let number = text.parse::<f64>().ok()?;
            if target_unit != "number" && number != 0.0 {
                return None;
            }
            number
        }
        Token::Percentage { .. } => {
            let number = text.strip_suffix('%')?.parse::<f64>().ok()?;
            match target_unit {
                "%" | "percent" => number,
                "number" => number * 0.01,
                _ => return None,
            }
        }
        Token::Dimension { unit, .. } if unit.eq_ignore_ascii_case(target_unit) => {
            // The token is a validated CSS dimension. Locate its numeric part,
            // allowing an exponent only when e/E is followed by signed digits
            // (so 1em and 1e2em have distinct boundaries). Units may be escaped.
            let bytes = text.as_bytes();
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
            text[..end].parse::<f64>().ok()?
        }
        _ => return None,
    };
    number.is_finite().then_some(number)
}
