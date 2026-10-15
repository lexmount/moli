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
    let number = moli_css_parse::css_numeric_token_value(&token, text)?;
    match token {
        Token::Number { .. } => {
            if target_unit != "number" && number != 0.0 {
                return None;
            }
            Some(number)
        }
        Token::Percentage { .. } => match target_unit {
            "%" | "percent" => Some(number),
            "number" => Some(number * 0.01),
            _ => None,
        },
        Token::Dimension { unit, .. } if unit.eq_ignore_ascii_case(target_unit) => Some(number),
        _ => None,
    }
}
