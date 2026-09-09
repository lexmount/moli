//! Moli-facing CSS transform parsing hooks.

use std::borrow::Cow;

use cssparser::{BasicParseErrorKind, Parser, ParserInput, SourcePosition, Token};
use style_traits::ParsingMode;

use crate::{
    context::QuirksMode,
    custom_properties::AttrTaint,
    parser::{Parse, ParserContext},
    stylesheets::{CssRuleType, Namespaces, Origin, UrlExtraData},
    values::specified::transform::Transform,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CssTransformFunction {
    pub name: String,
    pub arguments: Vec<String>,
}

pub fn parse_transform_function_list(raw: &str) -> Option<Vec<CssTransformFunction>> {
    if raw.trim().is_empty() {
        return None;
    }
    parse_stylo_transform(raw)
        .or_else(|| parse_compat_transform_function_list(raw))
        .filter(|functions| !functions.is_empty())
}

fn parse_stylo_transform(raw: &str) -> Option<Vec<CssTransformFunction>> {
    with_stylo_transform_context(|context| {
        let mut input = ParserInput::new(raw.trim());
        let mut parser = Parser::new(&mut input);
        let _ = Transform::parse(context, &mut parser).ok()?;
        parser.expect_exhausted().ok()?;
        parse_compat_transform_function_list(raw)
    })?
}

fn with_stylo_transform_context<R>(f: impl FnOnce(&ParserContext) -> R) -> Option<R> {
    let url_data = UrlExtraData::from(url::Url::parse("about:blank").ok()?);
    let context = ParserContext::new(
        Origin::Author,
        &url_data,
        Some(CssRuleType::Style),
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        Cow::Owned(Namespaces::default()),
        None,
        None,
        AttrTaint::default(),
    );
    Some(f(&context))
}

fn parse_compat_transform_function_list(raw: &str) -> Option<Vec<CssTransformFunction>> {
    let mut input = ParserInput::new(raw);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_entirely(|input| {
            let mut functions = Vec::new();
            while !input.is_exhausted() {
                let token = input.next().map_err(cssparser::ParseError::from)?.clone();
                let Token::Function(name) = token else {
                    return Err(input.new_error(BasicParseErrorKind::UnexpectedToken(token)));
                };
                let name = name.to_ascii_lowercase();
                if name.is_empty() || !name.chars().all(|ch| ch.is_ascii_alphanumeric()) {
                    return Err(input.new_custom_error(()));
                }
                let arguments = input.parse_nested_block(parse_transform_arguments)?;
                functions.push(CssTransformFunction { name, arguments });
            }
            if functions.is_empty() {
                return Err(input.new_custom_error(()));
            }
            Ok(functions)
        })
        .ok()
}

fn parse_transform_arguments<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<String>, cssparser::ParseError<'i, ()>> {
    let mut arguments = Vec::new();
    let mut argument_start: Option<SourcePosition> = None;
    let mut argument_end: Option<SourcePosition> = None;
    let mut pending_whitespace = false;
    let mut saw_comma = false;
    let mut saw_whitespace_separator = false;

    loop {
        let token_start = input.position();
        let token = match input.next_including_whitespace_and_comments() {
            Ok(token) => token.clone(),
            Err(error) if matches!(error.kind, BasicParseErrorKind::EndOfInput) => break,
            Err(error) => return Err(error.into()),
        };
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => {
                if argument_start.is_some() {
                    pending_whitespace = true;
                }
            },
            Token::Comma => {
                if saw_whitespace_separator {
                    return Err(input.new_custom_error(()));
                }
                push_argument(
                    input,
                    &mut arguments,
                    argument_start.take(),
                    argument_end.take(),
                )?;
                saw_comma = true;
                pending_whitespace = false;
            },
            token if token.is_parse_error() => {
                return Err(input.new_error(BasicParseErrorKind::UnexpectedToken(token)));
            },
            token => {
                if pending_whitespace && !saw_comma {
                    push_argument(
                        input,
                        &mut arguments,
                        argument_start.take(),
                        argument_end.take(),
                    )?;
                    saw_whitespace_separator = true;
                }
                if argument_start.is_none() {
                    argument_start = Some(token_start);
                }
                consume_nested_component_value(input, &token)?;
                argument_end = Some(input.position());
                pending_whitespace = false;
            },
        }
    }

    push_argument(input, &mut arguments, argument_start, argument_end)?;
    Ok(arguments)
}

fn push_argument<'i>(
    input: &Parser<'i, '_>,
    arguments: &mut Vec<String>,
    start: Option<SourcePosition>,
    end: Option<SourcePosition>,
) -> Result<(), cssparser::ParseError<'i, ()>> {
    let (Some(start), Some(end)) = (start, end) else {
        return Err(input.new_custom_error(()));
    };
    let argument = input.slice(start..end).trim();
    if argument.is_empty() {
        return Err(input.new_custom_error(()));
    }
    arguments.push(argument.to_owned());
    Ok(())
}

fn consume_nested_component_value<'i>(
    input: &mut Parser<'i, '_>,
    token: &Token<'i>,
) -> Result<(), cssparser::ParseError<'i, ()>> {
    if matches!(
        token,
        Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock
    ) {
        input.parse_nested_block(consume_component_values)?;
    }
    Ok(())
}

fn consume_component_values<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(), cssparser::ParseError<'i, ()>> {
    while !input.is_exhausted() {
        let token = input
            .next_including_whitespace_and_comments()
            .map_err(cssparser::ParseError::from)?
            .clone();
        if token.is_parse_error() {
            return Err(input.new_error(BasicParseErrorKind::UnexpectedToken(token)));
        }
        consume_nested_component_value(input, &token)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_transform_function_list;

    #[test]
    fn transform_functions_use_stylo_standard_path_and_preserve_arguments() {
        let functions =
            parse_transform_function_list("translate(10px, 20px) rotate(calc(0.25turn))").unwrap();

        assert_eq!(functions[0].name, "translate");
        assert_eq!(functions[0].arguments, ["10px", "20px"]);
        assert_eq!(functions[1].name, "rotate");
        assert_eq!(functions[1].arguments, ["calc(0.25turn)"]);
    }

    #[test]
    fn transform_functions_keep_moli_compat_splitter_for_whitespace_arguments() {
        let functions =
            parse_transform_function_list("translate(calc(10px + 2px) 20px) matrix(1 0 0 1 5 6)")
                .unwrap();

        assert_eq!(functions[0].name, "translate");
        assert_eq!(functions[0].arguments, ["calc(10px + 2px)", "20px"]);
        assert_eq!(functions[1].name, "matrix");
        assert_eq!(functions[1].arguments, ["1", "0", "0", "1", "5", "6"]);
    }

    #[test]
    fn transform_functions_reject_empty_or_mixed_argument_separators() {
        assert!(parse_transform_function_list("").is_none());
        assert!(parse_transform_function_list("/**/").is_none());
        assert!(parse_transform_function_list("translate()").is_none());
        assert!(parse_transform_function_list("translate(10px,)").is_none());
        assert!(parse_transform_function_list("translate(10px 20px, 30px)").is_none());
    }
}
