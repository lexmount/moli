//! CSSOM value projection and compatibility helpers.
//!
//! These helpers are not the owner for ordinary CSS property grammar. Property
//! validity belongs to Stylo/PDB or a narrow Stylo-backed adapter. The helpers
//! here are limited to CSSOM token serialization, legacy compatibility
//! projection, and custom-property / var() syntax checks needed before Stylo can
//! resolve substituted values.

use cssparser::{
    ParseError, Parser, ParserInput, ToCss, Token, TokenSerializationType, serialize_string,
};

/// This is a conservative token trigger, not grammar validation.
pub fn css_value_may_contain_env_function(value: &str) -> bool {
    value
        .as_bytes()
        .windows(4)
        .any(|window| matches!(window, [b'e' | b'E', b'n' | b'N', b'v' | b'V', b'(']))
}

/// Ordinary property grammar still belongs to Stylo/PDB; this only prevents
/// malformed env() token trees from entering fallback storage.
pub fn css_declaration_value_has_valid_env_functions(value: &str) -> bool {
    !css_value_may_contain_env_function(value)
        || normalize_cssom_component_value_serialization(value).is_some()
}

/// This is a conservative token trigger, not grammar validation.
pub fn css_value_may_contain_var_function(value: &str) -> bool {
    value
        .as_bytes()
        .windows(4)
        .any(|window| matches!(window, [b'v' | b'V', b'a' | b'A', b'r' | b'R', b'(']))
}

/// Whether `value` is a valid standalone `var()` reference whose final
/// parenthesis was omitted at the end of a CSS input fragment.
///
/// CSSOM preserves that omission in the specified value even though Stylo's
/// declaration storage currently appends the missing parenthesis internally.
pub fn css_value_is_eof_open_var_function(value: &str) -> bool {
    recover_left_open_css_var_function(value).is_some()
}

/// This validates var() token structure before substitution, but does not decide
/// ordinary property grammar.
pub(crate) fn css_declaration_value_has_valid_var_functions(value: &str) -> bool {
    !css_value_may_contain_var_function(value) || css_var_functions_are_valid(value)
}

/// Custom property syntax is token-stream based; this trims only CSSOM edge
/// whitespace/comments and validates var() token shape.
pub fn normalize_css_variable_specified_value(value: &str) -> Option<String> {
    let trimmed = trim_css_whitespace_and_comments(value);
    if trimmed.is_empty() {
        return Some(" ".to_owned());
    }
    let normalized = recover_left_open_css_var_function(trimmed).unwrap_or(trimmed);
    if !css_declaration_value_has_valid_var_functions(normalized) {
        return None;
    }
    Some(normalized.to_owned())
}

/// Custom properties preserve token text; this helper intentionally does not
/// interpret ordinary property grammar.
pub fn normalize_custom_property_specified_value(value: &str) -> Option<String> {
    let trimmed = trim_css_whitespace_and_comments(value);
    if trimmed.is_empty() {
        return Some(String::new());
    }
    let normalized = recover_left_open_css_var_function(trimmed).unwrap_or(trimmed);
    Some(normalized.to_owned())
}

/// This preserves component value structure and validates nested env() syntax.
/// It is a serializer, not a property grammar parser.
pub fn normalize_cssom_component_value_serialization(value: &str) -> Option<String> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    let mut css_text = String::new();
    serialize_cssom_component_values(&mut input, &mut css_text, TokenSerializationType::Nothing)?;
    Some(css_text.trim().to_owned())
}

/// Token-serialize component values onto one line without deciding validity.
pub fn serialize_component_values_single_line(value: &str) -> Option<String> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    let mut css_text = String::new();
    serialize_component_values_preserving_tokens(
        &mut input,
        &mut css_text,
        TokenSerializationType::Nothing,
    )?;
    Some(css_text.trim().to_owned())
}

fn serialize_cssom_component_values<'i, 't>(
    input: &mut Parser<'i, 't>,
    css_text: &mut String,
    mut previous_token: TokenSerializationType,
) -> Option<TokenSerializationType> {
    let mut pending_whitespace = false;
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        if matches!(token, Token::WhiteSpace(_) | Token::Comment(_)) {
            pending_whitespace = true;
            continue;
        }

        let token_type = token.serialization_type();
        if pending_whitespace {
            if !css_text.ends_with([' ', '{', '(', '[']) {
                css_text.push(' ');
            }
        } else if previous_token.needs_separator_when_before(token_type) {
            css_text.push_str("/**/");
        }
        pending_whitespace = false;
        previous_token = token_type;
        serialize_cssom_component_token(&token, css_text)?;
        let closing_token = match token {
            Token::Function(_) | Token::ParenthesisBlock => Some(Token::CloseParenthesis),
            Token::SquareBracketBlock => Some(Token::CloseSquareBracket),
            Token::CurlyBracketBlock => Some(Token::CloseCurlyBracket),
            _ => None,
        };
        if let Some(closing_token) = closing_token {
            if let Token::Function(name) = &token
                && name.eq_ignore_ascii_case("env")
            {
                let nested: Result<(), ParseError<'_, ()>> = input.parse_nested_block(|input| {
                    serialize_cssom_env_function_body(input, css_text)
                        .ok_or_else(|| input.new_custom_error::<(), ()>(()))
                });
                nested.ok()?;
            } else {
                let nested: Result<TokenSerializationType, ParseError<'_, ()>> = input
                    .parse_nested_block(|input| {
                        serialize_cssom_component_values(input, css_text, previous_token)
                            .ok_or_else(|| input.new_custom_error::<(), ()>(()))
                    });
                nested.ok()?;
            }
            closing_token.to_css(css_text).ok()?;
            previous_token = closing_token.serialization_type();
        }
    }
    Some(previous_token)
}

fn serialize_component_values_preserving_tokens<'i, 't>(
    input: &mut Parser<'i, 't>,
    css_text: &mut String,
    mut previous_token: TokenSerializationType,
) -> Option<TokenSerializationType> {
    let mut pending_whitespace = false;
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        if matches!(token, Token::WhiteSpace(_) | Token::Comment(_)) {
            pending_whitespace = true;
            continue;
        }
        let token_type = token.serialization_type();
        if pending_whitespace {
            if !css_text.ends_with([' ', '{', '(', '[']) {
                css_text.push(' ');
            }
        } else if previous_token.needs_separator_when_before(token_type) {
            css_text.push_str("/**/");
        }
        pending_whitespace = false;
        previous_token = token_type;
        token.to_css(css_text).ok()?;
        let closing_token = match token {
            Token::Function(_) | Token::ParenthesisBlock => Some(Token::CloseParenthesis),
            Token::SquareBracketBlock => Some(Token::CloseSquareBracket),
            Token::CurlyBracketBlock => Some(Token::CloseCurlyBracket),
            _ => None,
        };
        if let Some(closing_token) = closing_token {
            let nested: Result<TokenSerializationType, ParseError<'_, ()>> = input
                .parse_nested_block(|input| {
                    serialize_component_values_preserving_tokens(input, css_text, previous_token)
                        .ok_or_else(|| input.new_custom_error::<(), ()>(()))
                });
            nested.ok()?;
            closing_token.to_css(css_text).ok()?;
            previous_token = closing_token.serialization_type();
        }
    }
    Some(previous_token)
}

fn serialize_cssom_env_function_body<'i, 't>(
    input: &mut Parser<'i, 't>,
    css_text: &mut String,
) -> Option<()> {
    let name = next_non_whitespace_component_token(input)?;
    if !matches!(name, Token::Ident(_)) {
        return None;
    }
    serialize_cssom_component_token(&name, css_text)?;

    let mut pending_whitespace = false;
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => {
                pending_whitespace = true;
            }
            Token::Comma => {
                return serialize_cssom_env_fallback(input, css_text);
            }
            token if css_env_index_token_is_valid(&token) => {
                if !pending_whitespace {
                    return None;
                }
                if !css_text.ends_with([' ', '(']) {
                    css_text.push(' ');
                }
                serialize_cssom_component_token(&token, css_text)?;
                pending_whitespace = false;
            }
            _ => return None,
        }
    }

    Some(())
}

fn next_non_whitespace_component_token<'i, 't>(input: &mut Parser<'i, 't>) -> Option<Token<'i>> {
    loop {
        let token = input
            .next_including_whitespace_and_comments()
            .cloned()
            .ok()?;
        if !matches!(token, Token::WhiteSpace(_) | Token::Comment(_)) {
            return Some(token);
        }
    }
}

fn css_env_index_token_is_valid(token: &Token<'_>) -> bool {
    matches!(
        token,
        Token::Number {
            int_value: Some(integer),
            ..
        } if *integer >= 0
    )
}

fn serialize_cssom_env_fallback<'i, 't>(
    input: &mut Parser<'i, 't>,
    css_text: &mut String,
) -> Option<()> {
    let fallback_start = input.position();
    let mut fallback = String::new();
    serialize_cssom_component_values(input, &mut fallback, TokenSerializationType::Nothing)?;
    let fallback = fallback.trim();

    css_text.push(',');
    if fallback.is_empty() {
        if css_slice_contains_whitespace_or_comment(input.slice_from(fallback_start)) {
            css_text.push(' ');
        }
    } else {
        css_text.push(' ');
        css_text.push_str(fallback);
    }
    Some(())
}

fn css_var_functions_are_valid(value: &str) -> bool {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    validate_css_var_functions_in_component_values(&mut input).is_some()
        || left_open_css_var_function_is_valid(value)
}

fn left_open_css_var_function_is_valid(value: &str) -> bool {
    let Some(value) = recover_left_open_css_var_function(value) else {
        return false;
    };
    let value = trim_css_whitespace_and_comments(value);
    let Some(body) = strip_ascii_case_prefix(value, "var(") else {
        return false;
    };
    let mut input = ParserInput::new(body);
    let mut input = Parser::new(&mut input);
    validate_css_var_function_body(&mut input).is_some()
}

fn recover_left_open_css_var_function(value: &str) -> Option<&str> {
    let value = trim_css_whitespace_and_comments(value);
    let recovered = value.strip_suffix(';').map(str::trim_end).unwrap_or(value);
    let body = strip_ascii_case_prefix(value, "var(")?;
    if body.contains(')') || recovered.is_empty() {
        return None;
    }
    let body = strip_ascii_case_prefix(recovered, "var(")?;
    if body.contains(')') {
        return None;
    }
    let mut input = ParserInput::new(body);
    let mut input = Parser::new(&mut input);
    validate_css_var_function_body(&mut input)
        .is_some()
        .then_some(recovered)
}

fn strip_ascii_case_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        .then(|| &value[prefix.len()..])
}

fn validate_css_var_functions_in_component_values(input: &mut Parser<'_, '_>) -> Option<()> {
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        let nested_block = match token {
            Token::Function(name) => Some((Some(name), Token::CloseParenthesis)),
            Token::ParenthesisBlock => Some((None, Token::CloseParenthesis)),
            Token::SquareBracketBlock => Some((None, Token::CloseSquareBracket)),
            Token::CurlyBracketBlock => Some((None, Token::CloseCurlyBracket)),
            _ => None,
        };
        let Some((function_name, _)) = nested_block else {
            continue;
        };
        if function_name
            .as_ref()
            .is_some_and(|name| name.eq_ignore_ascii_case("var"))
        {
            let nested: Result<(), ParseError<'_, ()>> = input.parse_nested_block(|input| {
                validate_css_var_function_body(input)
                    .ok_or_else(|| input.new_custom_error::<(), ()>(()))
            });
            nested.ok()?;
        } else {
            let nested: Result<(), ParseError<'_, ()>> = input.parse_nested_block(|input| {
                validate_css_var_functions_in_component_values(input)
                    .ok_or_else(|| input.new_custom_error::<(), ()>(()))
            });
            nested.ok()?;
        }
    }
    Some(())
}

fn validate_css_var_function_body(input: &mut Parser<'_, '_>) -> Option<()> {
    let name = next_non_whitespace_component_token(input)?;
    let Token::Ident(name) = name else {
        return None;
    };
    if !name.starts_with("--") {
        return None;
    }

    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => {}
            Token::Comma => return validate_css_var_functions_in_component_values(input),
            _ => return None,
        }
    }
    Some(())
}

fn css_slice_contains_whitespace_or_comment(value: &str) -> bool {
    value.contains("/*") || value.chars().any(char::is_whitespace)
}

fn trim_css_whitespace_and_comments(value: &str) -> &str {
    let mut value = value;
    loop {
        let trimmed = value.trim_start();
        let Some(rest) = trimmed.strip_prefix("/*") else {
            value = trimmed;
            break;
        };
        let Some(comment_end) = rest.find("*/") else {
            return "";
        };
        value = &rest[comment_end + 2..];
    }

    loop {
        let trimmed = value.trim_end();
        if !trimmed.ends_with("*/") {
            return trimmed;
        }
        let Some(comment_start) = trimmed.rfind("/*") else {
            return trimmed;
        };
        value = &trimmed[..comment_start];
    }
}

fn serialize_cssom_component_token(token: &Token<'_>, css_text: &mut String) -> Option<()> {
    match token {
        Token::UnquotedUrl(value) => {
            css_text.push_str("url(");
            serialize_string(value, css_text).ok()?;
            css_text.push(')');
        }
        Token::Number { value, .. } if is_css_negative_zero(*value) => css_text.push('0'),
        Token::Percentage { unit_value, .. } if is_css_negative_zero(*unit_value) => {
            css_text.push_str("0%");
        }
        Token::Dimension { value, unit, .. } if is_css_negative_zero(*value) => {
            css_text.push('0');
            cssparser::serialize_identifier(unit, css_text).ok()?;
        }
        _ => token.to_css(css_text).ok()?,
    }
    Some(())
}

fn is_css_negative_zero(value: f32) -> bool {
    value == 0.0 && value.is_sign_negative()
}

#[cfg(test)]
mod tests {
    use super::{
        css_declaration_value_has_valid_env_functions,
        css_declaration_value_has_valid_var_functions, normalize_css_variable_specified_value,
        normalize_cssom_component_value_serialization, normalize_custom_property_specified_value,
        serialize_component_values_single_line,
    };

    #[test]
    fn cssom_component_value_serializer_normalizes_urls_and_numbers() {
        assert_eq!(
            normalize_cssom_component_value_serialization("url(http://localhost/)").as_deref(),
            Some(r#"url("http://localhost/")"#)
        );
        assert_eq!(
            normalize_cssom_component_value_serialization("5% .5% -.5% .1em -0px").as_deref(),
            Some("5% 0.5% -0.5% 0.1em 0px")
        );
    }

    #[test]
    fn cssom_component_value_serializer_validates_env_function_syntax() {
        assert_eq!(
            normalize_cssom_component_value_serialization("env(safe-area-inset-top)").as_deref(),
            Some("env(safe-area-inset-top)")
        );
        assert_eq!(
            normalize_cssom_component_value_serialization("env(safe-area-inset-top,)").as_deref(),
            Some("env(safe-area-inset-top,)")
        );
        assert_eq!(
            normalize_cssom_component_value_serialization("env(safe-area-inset-top, )").as_deref(),
            Some("env(safe-area-inset-top, )")
        );
        assert_eq!(
            normalize_cssom_component_value_serialization("env( test 0 1 , green)").as_deref(),
            Some("env(test 0 1, green)")
        );
        assert_eq!(
            normalize_cssom_component_value_serialization("calc(env(test, env(another, blue)))")
                .as_deref(),
            Some("calc(env(test, env(another, blue)))")
        );

        for invalid in [
            "env()",
            "env(safe-area-inset-top ())",
            "env(safe-area-inset-top () )",
            "env(safe-area-inset-top() )",
            "env(safe-area-inset-top (),)",
            "env(safe-area-inset-top(),)",
            "env(test1 test2, green)",
            "env(test1 10 20 test2, green)",
            "env(test 0.1, green)",
            "env(test -1, green)",
            "env(test+0, green)",
        ] {
            assert_eq!(normalize_cssom_component_value_serialization(invalid), None);
            assert!(!css_declaration_value_has_valid_env_functions(invalid));
        }
    }

    #[test]
    fn component_values_single_line_serialization_removes_newlines() {
        assert_eq!(
            serialize_component_values_single_line(
                "src: local(\"foo\");\nfont-family: foo;\nfont-weight: bold;"
            ),
            Some(String::from(
                "src: local(\"foo\"); font-family: foo; font-weight: bold;"
            ))
        );
        assert_eq!(
            serialize_component_values_single_line("body {\n color: red;\n }"),
            Some(String::from("body {color: red;}"))
        );
    }

    #[test]
    fn css_var_function_validator_matches_custom_property_syntax() {
        for valid in [
            "var(--x)",
            "var(--x,)",
            "var(--x, )",
            "var(--x, 20px)",
            "calc(var(--x))",
            "var(--prop1, var(--prop2))",
            "var(--prop1, var(--prop2, var(--prop3, auto)))",
            "var(--prop1) var(--prop2)",
            "var(--prop",
        ] {
            assert!(
                css_declaration_value_has_valid_var_functions(valid),
                "{valid} should be accepted"
            );
        }

        for invalid in [
            "var()",
            "var(prop)",
            "var(-prop)",
            "var(20px)",
            "var(var(--prop))",
            "var(--x ())",
            "var(--x () )",
            "var(--x() )",
            "var(--x (),)",
            "var(--x(),)",
            "var(--prop 20px)",
            "var(--prop, var(prop))",
            "var(--prop, var(-prop))",
        ] {
            assert!(
                !css_declaration_value_has_valid_var_functions(invalid),
                "{invalid} should be rejected"
            );
        }
    }

    #[test]
    fn css_var_specified_value_normalizer_trims_only_edge_comments() {
        assert_eq!(
            normalize_css_variable_specified_value("").as_deref(),
            Some(" ")
        );
        assert_eq!(
            normalize_css_variable_specified_value("  /* dropped */  ").as_deref(),
            Some(" ")
        );
        assert_eq!(
            normalize_css_variable_specified_value(
                " /* dropped */ var(--prop)  /* kept */ var(--prop) /* dropped */ "
            )
            .as_deref(),
            Some("var(--prop)  /* kept */ var(--prop)")
        );
        assert_eq!(
            normalize_css_variable_specified_value("var(--x;").as_deref(),
            Some("var(--x")
        );
        assert_eq!(normalize_css_variable_specified_value("var(--x ())"), None);
        assert_eq!(
            normalize_css_variable_specified_value("red").as_deref(),
            Some("red")
        );
    }

    #[test]
    fn eof_open_var_function_detection_is_narrow() {
        assert!(super::css_value_is_eof_open_var_function("var(--x"));
        assert!(super::css_value_is_eof_open_var_function("VAR(--x;"));
        assert!(!super::css_value_is_eof_open_var_function("var(--x)"));
        assert!(!super::css_value_is_eof_open_var_function("1px var(--x"));
        assert!(!super::css_value_is_eof_open_var_function("var(-x"));
    }

    #[test]
    fn custom_property_specified_value_normalizer_preserves_ident_var_names() {
        assert_eq!(
            normalize_custom_property_specified_value("").as_deref(),
            Some("")
        );
        assert_eq!(
            normalize_custom_property_specified_value("  /* dropped */  ").as_deref(),
            Some("")
        );
        assert_eq!(
            normalize_custom_property_specified_value(
                r#" var(ident("--myprop" calc(3 * sign(1em - 1px))), FAIL) "#
            )
            .as_deref(),
            Some(r#"var(ident("--myprop" calc(3 * sign(1em - 1px))), FAIL)"#)
        );
        assert_eq!(
            normalize_css_variable_specified_value(r#"var(ident("--myprop"), FAIL)"#),
            None
        );
    }
}
