use cssparser::{Parser, Token};

pub(crate) enum ValueDependency<'a> {
    Variable(&'a str),
    Dimension(&'a str),
    Percentage,
    Identifier(&'a str),
    Function(&'a str),
}

/// Inspect a CSS value, including the active branch of var(). A missing
/// variable is reported as None so the scanner can visit its fallback.
pub(crate) fn value_has_dependency(
    value: &str,
    inspect: &mut impl FnMut(ValueDependency<'_>) -> Option<bool>,
) -> bool {
    tokens_have_dependency(&mut Parser::new(value), inspect, 0)
}

pub(crate) fn is_container_unit(unit: &str) -> bool {
    ["cqw", "cqh", "cqi", "cqb", "cqmin", "cqmax"]
        .iter()
        .any(|known| unit.eq_ignore_ascii_case(known))
}

fn tokens_have_dependency(
    input: &mut Parser<'_>,
    inspect: &mut impl FnMut(ValueDependency<'_>) -> Option<bool>,
    depth: usize,
) -> bool {
    if depth >= 64 {
        return true;
    }
    let mut depends = false;
    while let Ok(token) = input.next().cloned() {
        depends |= match token {
            Token::Dimension { unit, .. } => inspect(ValueDependency::Dimension(&unit)),
            Token::Percentage { .. } => inspect(ValueDependency::Percentage),
            Token::Ident(name) => inspect(ValueDependency::Identifier(&name)),
            Token::Function(name) if name.eq_ignore_ascii_case("var") => Some(
                input
                    .parse_nested_block(|arguments| {
                        let name = arguments.expect_ident_cloned()?;
                        if let Some(result) = inspect(ValueDependency::Variable(&name)) {
                            while arguments.next().is_ok() {}
                            return Ok::<_, cssparser::ParseError<()>>(result);
                        }
                        if arguments.try_parse(Parser::expect_comma).is_ok() {
                            return Ok(tokens_have_dependency(arguments, inspect, depth + 1));
                        }
                        Ok(false)
                    })
                    .unwrap_or(true),
            ),
            Token::Function(name) => {
                let own_dependency = inspect(ValueDependency::Function(&name)).unwrap_or(false);
                let nested = input
                    .parse_nested_block(|nested| {
                        Ok::<_, cssparser::ParseError<()>>(tokens_have_dependency(
                            nested,
                            inspect,
                            depth + 1,
                        ))
                    })
                    .unwrap_or(true);
                Some(own_dependency || nested)
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => Some(
                input
                    .parse_nested_block(|nested| {
                        Ok::<_, cssparser::ParseError<()>>(tokens_have_dependency(
                            nested,
                            inspect,
                            depth + 1,
                        ))
                    })
                    .unwrap_or(true),
            ),
            _ => None,
        }
        .unwrap_or(false);
    }
    depends
}
