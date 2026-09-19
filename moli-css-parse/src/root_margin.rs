use cssparser::{Parser, ParserInput, Token};

pub fn normalize_root_margin(value: &str) -> Option<String> {
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    let raw_parts = parser.parse_entirely(parse_root_margin_components).ok()?;
    let parts: Vec<&str> = match raw_parts.as_slice() {
        [] => return Some("0px 0px 0px 0px".to_owned()),
        [a] => vec![a.as_str(), a.as_str(), a.as_str(), a.as_str()],
        [a, b] => vec![a.as_str(), b.as_str(), a.as_str(), b.as_str()],
        [a, b, c] => vec![a.as_str(), b.as_str(), c.as_str(), b.as_str()],
        [a, b, c, d] => vec![a.as_str(), b.as_str(), c.as_str(), d.as_str()],
        _ => return None,
    };
    Some(parts.join(" "))
}

pub fn root_margin_components(value: &str, root_width: f64) -> [f64; 4] {
    let mut result = [0.0; 4];
    for (index, part) in value.split_ascii_whitespace().take(4).enumerate() {
        result[index] = if let Some(number) = part.strip_suffix("px") {
            number.parse::<f64>().unwrap_or(0.0)
        } else if let Some(number) = part.strip_suffix('%') {
            number
                .parse::<f64>()
                .map(|percentage| (percentage / 100.0) * root_width)
                .unwrap_or(0.0)
        } else {
            0.0
        };
    }
    result
}

fn parse_root_margin_components<'i, 't>(
    input: &mut Parser<'i, 't>,
) -> Result<Vec<String>, cssparser::ParseError<'i, ()>> {
    let mut parts = Vec::new();
    while !input.is_exhausted() {
        if parts.len() == 4 {
            return Err(input.new_custom_error(()));
        }
        parts.push(parse_root_margin_component(input)?);
    }
    Ok(parts)
}

fn parse_root_margin_component<'i, 't>(
    input: &mut Parser<'i, 't>,
) -> Result<String, cssparser::ParseError<'i, ()>> {
    match input.next()?.clone() {
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("px") => {
            format_css_number(value)
                .map(|number| format!("{number}px"))
                .ok_or_else(|| input.new_custom_error(()))
        }
        Token::Percentage { unit_value, .. } => format_css_number(unit_value * 100.0)
            .map(|number| format!("{number}%"))
            .ok_or_else(|| input.new_custom_error(())),
        _ => Err(input.new_custom_error(())),
    }
}

fn format_css_number(value: f32) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    if value == 0.0 {
        return Some("0".to_owned());
    }
    let mut text = value.to_string();
    if let Some(dot) = text.find('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.len() == dot + 1 {
            text.pop();
        }
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::{normalize_root_margin, root_margin_components};

    #[test]
    fn parser_uses_cssparser_tokens() {
        for value in ["", " \r\n\t\u{c}", "/**/", "/* unclosed", " /**/ \n /**/ "] {
            assert_eq!(
                normalize_root_margin(value).as_deref(),
                Some("0px 0px 0px 0px"),
                "{value:?}",
            );
        }
        assert_eq!(
            normalize_root_margin("10px /*comment*/ 5% 0px -2.5px").as_deref(),
            Some("10px 5% 0px -2.5px")
        );
        assert_eq!(
            normalize_root_margin("\n1PX\t2px").as_deref(),
            Some("1px 2px 1px 2px")
        );
    }

    #[test]
    fn parser_rejects_non_margin_tokens() {
        for value in [
            "0", "+0", "-0", "0.0", "0e0", "1e-50", "1px 0", "0% 0 1px", "\u{a0}", "\u{2003}",
            "\u{b}", "\u{85}", "\u{2028}", "\u{2029}", "()", "[]", "{}", ")", "]", "}",
        ] {
            assert_eq!(normalize_root_margin(value), None, "{value:?}");
        }
        assert_eq!(normalize_root_margin("calc(1px)").as_deref(), None);
        assert_eq!(normalize_root_margin("1px, 2px").as_deref(), None);
        assert_eq!(normalize_root_margin("1em").as_deref(), None);
        assert_eq!(
            normalize_root_margin("1px 2px 3px 4px 5px").as_deref(),
            None
        );
    }

    #[test]
    fn components_resolve_percentages_against_root_width() {
        assert_eq!(
            root_margin_components("10px 5% 0px -2.5px", 200.0),
            [10.0, 10.0, 0.0, -2.5]
        );
    }
}
