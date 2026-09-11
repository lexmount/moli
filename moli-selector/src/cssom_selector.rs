use cssparser::{
    BasicParseErrorKind, ParseError, Parser, ParserInput, Token, parse_nth, serialize_identifier,
    serialize_string,
};
use std::ops::Range;

use moli_css_parse::serialize_component_values_single_line;

fn css_parse_error<'i>(error: cssparser::BasicParseError<'i>) -> ParseError<'i, ()> {
    error.into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssDirection {
    Ltr,
    Rtl,
}

impl CssDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ltr => "ltr",
            Self::Rtl => "rtl",
        }
    }
}

pub fn first_strong_text_direction(value: &str) -> Option<CssDirection> {
    value.chars().find_map(strong_char_direction)
}

fn strong_char_direction(ch: char) -> Option<CssDirection> {
    if is_rtl_strong_char(ch) {
        Some(CssDirection::Rtl)
    } else if ch.is_alphabetic() {
        Some(CssDirection::Ltr)
    } else {
        None
    }
}

fn is_rtl_strong_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0590..=0x08ff | 0xfb1d..=0xfdff | 0xfe70..=0xfefc | 0x10800..=0x10fff
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetComputedStylePseudoElement {
    OriginatingElement,
    EmptyStyle,
    PseudoElement(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CssAttributeSelectorOperator {
    Exists,
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssAttributeSelector {
    name: String,
    operator: CssAttributeSelectorOperator,
    expected: Option<String>,
}

pub(crate) fn selector_list_has_namespace_separator(selector_text: &str) -> bool {
    let mut input = ParserInput::new(selector_text);
    let mut input = Parser::new(&mut input);
    selector_tokens_have_namespace_separator(&mut input)
}

pub(crate) fn selector_list_namespace_prefixes(selector_text: &str) -> Vec<String> {
    let mut input = ParserInput::new(selector_text);
    let mut input = Parser::new(&mut input);
    let mut prefixes = Vec::new();
    collect_selector_namespace_prefixes(&mut input, &mut prefixes);
    prefixes
}

fn parse_css_attribute_selector(selector_text: &str) -> Option<CssAttributeSelector> {
    let mut input = ParserInput::new(selector_text);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_entirely(|parser| {
            parser
                .expect_square_bracket_block()
                .map_err(css_parse_error)?;
            parser.parse_nested_block(parse_css_attribute_selector_block)
        })
        .ok()
}

fn parse_css_attribute_selector_block<'i, 't>(
    input: &mut Parser<'i, 't>,
) -> Result<CssAttributeSelector, ParseError<'i, ()>> {
    let name = input.expect_ident_cloned().map_err(css_parse_error)?;
    if input.is_exhausted() {
        return Ok(CssAttributeSelector {
            name: name.to_string(),
            operator: CssAttributeSelectorOperator::Exists,
            expected: None,
        });
    }

    let operator = match input.next().map_err(css_parse_error)? {
        Token::Delim('=') => CssAttributeSelectorOperator::Equals,
        Token::IncludeMatch => CssAttributeSelectorOperator::Includes,
        Token::DashMatch => CssAttributeSelectorOperator::DashMatch,
        Token::PrefixMatch => CssAttributeSelectorOperator::Prefix,
        Token::SuffixMatch => CssAttributeSelectorOperator::Suffix,
        Token::SubstringMatch => CssAttributeSelectorOperator::Substring,
        _ => return Err(input.new_custom_error(())),
    };
    let expected = input
        .expect_ident_or_string()
        .map_err(css_parse_error)?
        .as_ref()
        .to_owned();
    if !input.is_exhausted() {
        return Err(input.new_custom_error(()));
    }

    Ok(CssAttributeSelector {
        name: name.to_string(),
        operator,
        expected: Some(expected),
    })
}

pub(crate) fn serialize_cssom_selector_text(
    selector_text: &str,
    has_default_namespace: bool,
    prefixes_matching_default_namespace: &[String],
) -> Option<String> {
    let selector_text = serialize_component_values_single_line(selector_text)?;
    Some(serialize_cssom_selector_text_from_single_line(
        &selector_text,
        has_default_namespace,
        prefixes_matching_default_namespace,
    ))
}

pub(crate) fn serialize_cssom_selector_text_preserving_invalid_forgiving_items(
    selector_text: &str,
    has_default_namespace: bool,
    prefixes_matching_default_namespace: &[String],
    is_strictly_valid: impl Fn(&str) -> bool,
) -> Option<String> {
    let (protected_selector, preserved_items) =
        protect_invalid_forgiving_selector_items(selector_text, is_strictly_valid)?;
    let mut serialized = serialize_cssom_selector_text(
        &protected_selector,
        has_default_namespace,
        prefixes_matching_default_namespace,
    )?;
    for (marker, item) in preserved_items {
        if serialized.matches(&marker).count() != 1 {
            return None;
        }
        serialized = serialized.replacen(&marker, &item, 1);
    }
    Some(serialized)
}

fn protect_invalid_forgiving_selector_items(
    selector_text: &str,
    is_strictly_valid: impl Fn(&str) -> bool,
) -> Option<(String, Vec<(String, String)>)> {
    let argument_ranges = outermost_forgiving_selector_argument_ranges(selector_text)?;
    if argument_ranges.is_empty() || is_strictly_valid(selector_text) {
        return Some((selector_text.to_owned(), Vec::new()));
    }

    let baseline =
        selector_with_forgiving_argument_replacements(selector_text, &argument_ranges, None);
    let can_classify_items = is_strictly_valid(&baseline);
    let mut invalid_item_ranges = Vec::new();
    for (argument_index, argument_range) in argument_ranges.iter().enumerate() {
        let argument = selector_text.get(argument_range.clone())?;
        for item_range in top_level_comma_separated_selector_item_ranges(argument)? {
            let item = argument.get(item_range.clone())?;
            let item_is_valid = can_classify_items
                && is_strictly_valid(&selector_with_forgiving_argument_replacements(
                    selector_text,
                    &argument_ranges,
                    Some((argument_index, item)),
                ));
            if !item_is_valid {
                invalid_item_ranges.push(
                    argument_range.start + item_range.start..argument_range.start + item_range.end,
                );
            }
        }
    }

    let collision_text = serialize_component_values_single_line(selector_text)?;
    let mut protected_selector = selector_text.to_owned();
    let mut preserved_items = Vec::with_capacity(invalid_item_ranges.len());
    for (index, range) in invalid_item_ranges.into_iter().enumerate().rev() {
        let item = selector_text.get(range.clone())?;
        let marker = unused_forgiving_selector_marker(selector_text, &collision_text, index);
        protected_selector.replace_range(range, &marker);
        preserved_items.push((marker, trim_css_whitespace(item).to_owned()));
    }
    Some((protected_selector, preserved_items))
}

fn selector_with_forgiving_argument_replacements(
    selector_text: &str,
    argument_ranges: &[Range<usize>],
    selected_item: Option<(usize, &str)>,
) -> String {
    let mut selector = selector_text.to_owned();
    for (index, range) in argument_ranges.iter().enumerate().rev() {
        let replacement = selected_item
            .filter(|(selected_index, _)| *selected_index == index)
            .map_or(":hover", |(_, item)| trim_css_whitespace(item));
        selector.replace_range(range.clone(), replacement);
    }
    selector
}

fn unused_forgiving_selector_marker(
    selector_text: &str,
    collision_text: &str,
    index: usize,
) -> String {
    let mut attempt = 0_u32;
    loop {
        let marker = format!(".__moli_cssom_invalid_forgiving_{index}_{attempt}");
        if !selector_text.contains(&marker) && !collision_text.contains(&marker) {
            return marker;
        }
        attempt = attempt
            .checked_add(1)
            .expect("forgiving selector marker search exhausted");
    }
}

fn outermost_forgiving_selector_argument_ranges(selector_text: &str) -> Option<Vec<Range<usize>>> {
    let mut input = ParserInput::new(selector_text);
    let mut parser = Parser::new(&mut input);
    let mut ranges = Vec::new();
    collect_outermost_forgiving_selector_argument_ranges(&mut parser, &mut ranges)?;
    Some(ranges)
}

fn collect_outermost_forgiving_selector_argument_ranges(
    input: &mut Parser<'_, '_>,
    ranges: &mut Vec<Range<usize>>,
) -> Option<()> {
    let mut previous_was_colon = false;
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        if token.is_parse_error() {
            return None;
        }
        match token {
            Token::Colon => previous_was_colon = true,
            Token::Comment(_) if previous_was_colon => {}
            Token::Function(name) => {
                let is_forgiving_selector = previous_was_colon
                    && (name.eq_ignore_ascii_case("is") || name.eq_ignore_ascii_case("where"));
                let argument_start = input.position().byte_index();
                let mut argument_end = None;
                let nested: Result<(), ParseError<'_, ()>> = input.parse_nested_block(|input| {
                    if is_forgiving_selector {
                        consume_component_values(input)?;
                        argument_end = Some(input.position().byte_index());
                        Ok(())
                    } else {
                        collect_outermost_forgiving_selector_argument_ranges(input, ranges)
                            .ok_or_else(|| input.new_custom_error::<(), ()>(()))
                    }
                });
                nested.ok()?;
                if let Some(argument_end) = argument_end {
                    ranges.push(argument_start..argument_end);
                }
                previous_was_colon = false;
            }
            Token::ParenthesisBlock | Token::CurlyBracketBlock => {
                let nested: Result<(), ParseError<'_, ()>> = input.parse_nested_block(|input| {
                    collect_outermost_forgiving_selector_argument_ranges(input, ranges)
                        .ok_or_else(|| input.new_custom_error::<(), ()>(()))
                });
                nested.ok()?;
                previous_was_colon = false;
            }
            Token::SquareBracketBlock => {
                consume_nested_component_value(input, &token).ok()?;
                previous_was_colon = false;
            }
            _ => previous_was_colon = false,
        }
    }
    Some(())
}

fn top_level_comma_separated_selector_item_ranges(argument: &str) -> Option<Vec<Range<usize>>> {
    let mut input = ParserInput::new(argument);
    let mut parser = Parser::new(&mut input);
    let mut ranges = Vec::new();
    let mut item_start = 0;
    while !parser.is_exhausted() {
        let token_start = parser.position().byte_index();
        let token = parser
            .next_including_whitespace_and_comments()
            .ok()?
            .clone();
        if token.is_parse_error() {
            return None;
        }
        if matches!(token, Token::Comma) {
            ranges.push(item_start..token_start);
            item_start = parser.position().byte_index();
        } else {
            consume_nested_component_value(&mut parser, &token).ok()?;
        }
    }
    ranges.push(item_start..argument.len());
    Some(ranges)
}

fn trim_css_whitespace(value: &str) -> &str {
    value.trim_matches(['\t', '\n', '\x0c', '\r', ' '])
}

fn serialize_cssom_selector_text_without_default_namespace(selector_text: &str) -> Option<String> {
    serialize_cssom_selector_text(selector_text, false, &[])
}

fn serialize_cssom_selector_text_from_single_line(
    selector_text: &str,
    has_default_namespace: bool,
    prefixes_matching_default_namespace: &[String],
) -> String {
    let mut output = String::with_capacity(selector_text.len());
    let mut index = 0;
    let mut at_compound_start = true;
    while index < selector_text.len() {
        let rest = &selector_text[index..];
        if rest.starts_with('[')
            && let Some((attribute, next_index)) =
                serialize_attribute_selector(selector_text, index)
        {
            output.push_str(&attribute);
            index = next_index;
            at_compound_start = false;
            continue;
        }
        if at_compound_start && !has_default_namespace && rest.starts_with("*|") {
            index += 2;
            continue;
        }
        if at_compound_start
            && let Some(prefix) =
                redundant_default_namespace_prefix(rest, prefixes_matching_default_namespace)
        {
            index += prefix.len() + 1;
            continue;
        }
        if at_compound_start
            && rest.starts_with('*')
            && rest
                .as_bytes()
                .get(1)
                .is_some_and(|next| matches!(next, b'.' | b'#' | b':' | b'['))
        {
            index += 1;
            continue;
        }
        if !output.ends_with(':')
            && rest.starts_with(':')
            && !rest.starts_with("::")
            && let Some((pseudo_element, next_index)) =
                legacy_pseudo_element_name(selector_text, index + 1)
        {
            output.push_str("::");
            output.push_str(pseudo_element);
            index = next_index;
            at_compound_start = false;
            continue;
        }
        if rest.starts_with(':')
            && let Some((nth_selector, next_index)) =
                serialize_nth_pseudo_class(selector_text, index)
        {
            output.push_str(&nth_selector);
            index = next_index;
            at_compound_start = false;
            continue;
        }
        if rest.starts_with(':')
            && let Some((pseudo_class, next_index)) =
                serialize_functional_selector_list_pseudo_class(selector_text, index)
        {
            output.push_str(&pseudo_class);
            index = next_index;
            at_compound_start = false;
            continue;
        }

        let ch = rest.chars().next().expect("non-empty selector remainder");
        output.push(ch);
        index += ch.len_utf8();
        at_compound_start = matches!(ch, ',' | '>' | '+' | '~' | ' ');
    }
    output
}

fn serialize_attribute_selector(selector_text: &str, open_index: usize) -> Option<(String, usize)> {
    let close_offset = selector_text[open_index..].find(']')?;
    let close_index = open_index + close_offset;
    let content = &selector_text[open_index + 1..close_index];
    if let Some(serialized) = serialize_attribute_selector_content(content) {
        return Some((serialized, close_index + 1));
    }
    let mut output = String::with_capacity(close_offset + 1);
    output.push('[');
    output.push_str(content.strip_prefix('|').unwrap_or(content));
    output.push(']');
    Some((output, close_index + 1))
}

fn serialize_attribute_selector_content(content: &str) -> Option<String> {
    let selector = parse_css_attribute_selector(&format!("[{content}]"))?;
    let mut output = String::new();
    output.push('[');
    serialize_identifier(&selector.name, &mut output).ok()?;
    if let Some(expected) = selector.expected.as_deref() {
        output.push_str(match selector.operator {
            CssAttributeSelectorOperator::Exists => return None,
            CssAttributeSelectorOperator::Equals => "=",
            CssAttributeSelectorOperator::Includes => "~=",
            CssAttributeSelectorOperator::DashMatch => "|=",
            CssAttributeSelectorOperator::Prefix => "^=",
            CssAttributeSelectorOperator::Suffix => "$=",
            CssAttributeSelectorOperator::Substring => "*=",
        });
        serialize_string(expected, &mut output).ok()?;
    }
    output.push(']');
    Some(output)
}

fn redundant_default_namespace_prefix<'a>(
    selector_text: &'a str,
    prefixes_matching_default_namespace: &[String],
) -> Option<&'a str> {
    let (prefix, rest) = selector_text.split_once('|')?;
    if prefix.is_empty()
        || prefix == "*"
        || !prefixes_matching_default_namespace
            .iter()
            .any(|existing| existing == prefix)
    {
        return None;
    }
    rest.chars()
        .next()
        .is_some_and(|ch| ch == '*' || is_identifier_start(ch))
        .then_some(prefix)
}

fn legacy_pseudo_element_name(selector_text: &str, name_index: usize) -> Option<(&str, usize)> {
    let name = ["after", "before", "first-letter", "first-line"]
        .into_iter()
        .find(|name| {
            selector_text[name_index..].starts_with(name)
                && selector_text[name_index + name.len()..]
                    .chars()
                    .next()
                    .is_none_or(|ch| !is_identifier_continue(ch))
        })?;
    Some((
        &selector_text[name_index..name_index + name.len()],
        name_index + name.len(),
    ))
}

fn serialize_nth_pseudo_class(selector_text: &str, colon_index: usize) -> Option<(String, usize)> {
    let rest = &selector_text[colon_index + 1..];
    let name = [
        "nth-child",
        "nth-last-child",
        "nth-of-type",
        "nth-last-of-type",
    ]
    .into_iter()
    .find(|name| rest.starts_with(*name) && rest.as_bytes().get(name.len()) == Some(&b'('))?;
    let open_index = colon_index + 1 + name.len();
    let close_index = matching_close_parenthesis(selector_text, open_index)?;
    let argument = &selector_text[open_index + 1..close_index];
    let (step, offset) = parse_nth_argument(argument)?;
    Some((
        format!(":{name}({})", serialize_an_plus_b(step, offset)),
        close_index + 1,
    ))
}

fn serialize_functional_selector_list_pseudo_class(
    selector_text: &str,
    colon_index: usize,
) -> Option<(String, usize)> {
    let rest = &selector_text[colon_index + 1..];
    let name = ["is", "where", "not"]
        .into_iter()
        .find(|name| rest.starts_with(*name) && rest.as_bytes().get(name.len()) == Some(&b'('))?;
    let open_index = colon_index + 1 + name.len();
    let close_index = matching_close_parenthesis(selector_text, open_index)?;
    let argument = &selector_text[open_index + 1..close_index];
    let argument = serialize_selector_list_function_argument(argument)?;
    Some((format!(":{name}({argument})"), close_index + 1))
}

fn serialize_selector_list_function_argument(argument: &str) -> Option<String> {
    let items = top_level_comma_separated_selector_items(argument)?;
    Some(
        items
            .into_iter()
            .map(|item| serialize_cssom_selector_text_without_default_namespace(item.trim()))
            .collect::<Option<Vec<_>>>()?
            .join(", "),
    )
}

fn top_level_comma_separated_selector_items(argument: &str) -> Option<Vec<&str>> {
    let mut items = Vec::new();
    let mut start = 0;
    let mut bracket_depth = 0_u32;
    let mut paren_depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in argument.char_indices() {
        if let Some(current_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == current_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '[' => bracket_depth = bracket_depth.saturating_add(1),
            ']' => bracket_depth = bracket_depth.checked_sub(1)?,
            '(' => paren_depth = paren_depth.saturating_add(1),
            ')' => paren_depth = paren_depth.checked_sub(1)?,
            ',' if bracket_depth == 0 && paren_depth == 0 => {
                let item = argument[start..index].trim();
                if item.is_empty() {
                    return None;
                }
                items.push(item);
                start = index + 1;
            }
            _ => {}
        }
    }
    if quote.is_some() || bracket_depth != 0 || paren_depth != 0 {
        return None;
    }
    let item = argument[start..].trim();
    if item.is_empty() {
        return None;
    }
    items.push(item);
    Some(items)
}

fn matching_close_parenthesis(selector_text: &str, open_index: usize) -> Option<usize> {
    let mut depth = 0_u32;
    for (index, ch) in selector_text[open_index..].char_indices() {
        match ch {
            '(' => depth = depth.saturating_add(1),
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open_index + index);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_nth_argument(argument: &str) -> Option<(i32, i32)> {
    let mut input = ParserInput::new(argument);
    let mut parser = Parser::new(&mut input);
    let parsed = parse_nth(&mut parser).ok()?;
    parser.is_exhausted().then_some(parsed)
}

fn serialize_an_plus_b(step: i32, offset: i32) -> String {
    match (step, offset) {
        (0, 0) => "0".to_owned(),
        (1, 0) => "n".to_owned(),
        (-1, 0) => "-n".to_owned(),
        (_, 0) => format!("{step}n"),
        (0, _) => offset.to_string(),
        (1, _) => format!("n{offset:+}"),
        (-1, _) => format!("-n{offset:+}"),
        (_, _) => format!("{step}n{offset:+}"),
    }
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch == '-' || ch.is_ascii_alphabetic() || !ch.is_ascii()
}

fn is_identifier_continue(ch: char) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit()
}

fn selector_tokens_have_namespace_separator(input: &mut Parser<'_, '_>) -> bool {
    let mut found = false;
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::Delim('|') => found = true,
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                let _ = input.parse_nested_block(|input| {
                    found |= selector_tokens_have_namespace_separator(input);
                    Ok::<_, cssparser::ParseError<'_, ()>>(())
                });
            }
            _ => {}
        }
    }
    found
}

fn collect_selector_namespace_prefixes(input: &mut Parser<'_, '_>, prefixes: &mut Vec<String>) {
    let mut previous_prefix_candidate: Option<String> = None;
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::Ident(name) => {
                previous_prefix_candidate = Some(name.to_string());
            }
            Token::Delim('*') => {
                previous_prefix_candidate = None;
            }
            Token::Delim('|') => {
                if let Some(prefix) = previous_prefix_candidate.take()
                    && !prefixes.iter().any(|existing| existing == &prefix)
                {
                    prefixes.push(prefix);
                }
            }
            Token::Function(_) | Token::ParenthesisBlock | Token::CurlyBracketBlock => {
                previous_prefix_candidate = None;
                let _ = input.parse_nested_block(|input| {
                    collect_selector_namespace_prefixes(input, prefixes);
                    Ok::<_, cssparser::ParseError<'_, ()>>(())
                });
            }
            Token::SquareBracketBlock => {
                previous_prefix_candidate = None;
                let _ = consume_nested_component_value(input, &token);
            }
            _ => {
                previous_prefix_candidate = None;
            }
        }
    }
}

pub(crate) fn dom_api_selector_text_with_trailing_attribute_recovery(
    selector_text: &str,
) -> String {
    if !selector_text.contains('[') {
        return selector_text.to_owned();
    }
    if let Some(open_index) = trailing_unclosed_attribute_selector_index(selector_text)
        && selector_text[open_index + 1..].contains('=')
    {
        let mut recovered = String::with_capacity(selector_text.len() + 1);
        recovered.push_str(selector_text);
        recovered.push(']');
        return recovered;
    }
    selector_text.to_owned()
}

fn trailing_unclosed_attribute_selector_index(selector_text: &str) -> Option<usize> {
    let mut quote = None;
    let mut bracket_stack = Vec::new();
    for (index, ch) in selector_text.char_indices() {
        match ch {
            '"' | '\'' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                }
            }
            '[' if quote.is_none() => bracket_stack.push(index),
            ']' if quote.is_none() => {
                bracket_stack.pop()?;
            }
            _ => {}
        }
    }
    if quote.is_none() && bracket_stack.len() == 1 {
        bracket_stack.pop()
    } else {
        None
    }
}

pub(crate) fn dom_api_selector_list_uses_defined_pseudo_class(selector_text: &str) -> bool {
    let mut input = ParserInput::new(selector_text);
    let mut parser = Parser::new(&mut input);
    selector_tokens_use_pseudo_class(&mut parser, "defined")
}

fn selector_tokens_use_pseudo_class(parser: &mut Parser<'_, '_>, pseudo_class_name: &str) -> bool {
    let mut previous_was_colon = false;
    while let Ok(token) = parser.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::Colon => {
                if previous_was_colon {
                    previous_was_colon = false;
                    continue;
                }
                previous_was_colon = true;
            }
            Token::Ident(name)
                if previous_was_colon && name.eq_ignore_ascii_case(pseudo_class_name) =>
            {
                return true;
            }
            Token::Function(_) | Token::ParenthesisBlock | Token::CurlyBracketBlock => {
                previous_was_colon = false;
                if parser
                    .parse_nested_block(|parser| {
                        selector_tokens_use_pseudo_class(parser, pseudo_class_name)
                            .then_some(())
                            .ok_or_else(|| parser.new_custom_error::<(), ()>(()))
                    })
                    .is_ok()
                {
                    return true;
                }
            }
            Token::SquareBracketBlock => {
                previous_was_colon = false;
                if consume_nested_component_value(parser, &token).is_err() {
                    return false;
                }
            }
            _ => {
                previous_was_colon = false;
            }
        }
    }
    false
}

pub(crate) fn dom_api_selector_list_has_only_known_pseudo_elements(selector_text: &str) -> bool {
    let mut input = ParserInput::new(selector_text);
    let mut parser = Parser::new(&mut input);
    let mut saw_any_selector = false;
    let mut current_has_tokens = false;
    let mut current_ends_with_known_pseudo_element = false;

    while !parser.is_exhausted() {
        let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
            return false;
        };
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => {}
            Token::Comma => {
                if !current_has_tokens || !current_ends_with_known_pseudo_element {
                    return false;
                }
                saw_any_selector = true;
                current_has_tokens = false;
                current_ends_with_known_pseudo_element = false;
            }
            Token::Colon => {
                current_has_tokens = true;
                current_ends_with_known_pseudo_element =
                    parse_known_pseudo_element_after_colon(&mut parser).is_some();
            }
            token => {
                if token.is_parse_error() {
                    return false;
                }
                current_has_tokens = true;
                current_ends_with_known_pseudo_element = false;
                if matches!(
                    token,
                    Token::Function(_)
                        | Token::ParenthesisBlock
                        | Token::SquareBracketBlock
                        | Token::CurlyBracketBlock
                ) && consume_nested_component_value(&mut parser, &token).is_err()
                {
                    return false;
                }
            }
        }
    }

    (saw_any_selector || current_has_tokens) && current_ends_with_known_pseudo_element
}

pub(crate) fn dom_api_selector_list_contains_known_pseudo_element(selector_text: &str) -> bool {
    let mut input = ParserInput::new(selector_text);
    let mut parser = Parser::new(&mut input);

    while !parser.is_exhausted() {
        let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
            return false;
        };
        match token {
            Token::Colon => {
                if parse_known_pseudo_element_after_colon(&mut parser).is_some() {
                    return true;
                }
            }
            token => {
                if token.is_parse_error() {
                    return false;
                }
                if matches!(
                    token,
                    Token::Function(_)
                        | Token::ParenthesisBlock
                        | Token::SquareBracketBlock
                        | Token::CurlyBracketBlock
                ) && consume_nested_component_value(&mut parser, &token).is_err()
                {
                    return false;
                }
            }
        }
    }

    false
}

pub(crate) fn webkit_compat_pseudo_element_validation_selector(
    selector_text: &str,
) -> Option<String> {
    let mut input = ParserInput::new(selector_text);
    let mut parser = Parser::new(&mut input);
    let mut replacements = Vec::new();

    while !parser.is_exhausted() {
        let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
            return None;
        };
        if !matches!(token, Token::Colon) {
            continue;
        }
        let Ok(Token::Colon) = parser.next_including_whitespace_and_comments().cloned() else {
            continue;
        };
        let name_start = parser.position().byte_index();
        let Ok(Token::Ident(name)) = parser.next_including_whitespace_and_comments().cloned()
        else {
            continue;
        };
        if is_unknown_webkit_pseudo_element_name(&name) {
            replacements.push((name_start, parser.position().byte_index()));
        }
    }

    if replacements.is_empty() {
        return None;
    }
    let mut selector = selector_text.to_owned();
    for (start, end) in replacements.into_iter().rev() {
        selector.replace_range(start..end, "part(webkit-compat)");
    }
    Some(selector)
}

pub(crate) fn selector_list_has_invalid_terminal_pseudo_element_chain(selector_text: &str) -> bool {
    let mut input = ParserInput::new(selector_text);
    let mut parser = Parser::new(&mut input);
    let mut selector_has_terminal_pseudo_element = false;

    while !parser.is_exhausted() {
        let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
            return false;
        };
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => {}
            Token::Comma => {
                selector_has_terminal_pseudo_element = false;
            }
            Token::Colon => {
                if let Some(kind) = parse_known_pseudo_element_after_colon(&mut parser) {
                    if selector_has_terminal_pseudo_element {
                        return true;
                    }
                    selector_has_terminal_pseudo_element = kind == KnownPseudoElementKind::Terminal;
                }
            }
            token => {
                if token.is_parse_error() {
                    return false;
                }
                if matches!(
                    token,
                    Token::Function(_)
                        | Token::ParenthesisBlock
                        | Token::SquareBracketBlock
                        | Token::CurlyBracketBlock
                ) && consume_nested_component_value(&mut parser, &token).is_err()
                {
                    return false;
                }
            }
        }
    }

    false
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KnownPseudoElementKind {
    Terminal,
    Chainable,
}

fn parse_known_pseudo_element_after_colon(
    parser: &mut Parser<'_, '_>,
) -> Option<KnownPseudoElementKind> {
    let state = parser.state();
    let has_double_colon = matches!(
        parser
            .next_including_whitespace_and_comments()
            .cloned()
            .ok()?,
        Token::Colon
    );
    if !has_double_colon {
        parser.reset(&state);
    }
    let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
        return None;
    };
    match token {
        Token::Ident(name) => (is_known_terminal_pseudo_element_name(&name)
            || (has_double_colon && is_unknown_webkit_pseudo_element_name(&name)))
        .then_some(KnownPseudoElementKind::Terminal),
        Token::Function(name) => {
            let kind = match name.to_ascii_lowercase().as_str() {
                "cue" | "highlight" => KnownPseudoElementKind::Terminal,
                "part" | "slotted" => KnownPseudoElementKind::Chainable,
                _ => return None,
            };
            let _ = consume_nested_component_value(parser, &Token::Function(name));
            Some(kind)
        }
        _ => None,
    }
}

fn is_unknown_webkit_pseudo_element_name(name: &str) -> bool {
    const WEBKIT_PREFIX: &str = "-webkit-";
    name.get(..WEBKIT_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(WEBKIT_PREFIX))
}

fn is_known_terminal_pseudo_element_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "after"
            | "backdrop"
            | "before"
            | "checkmark"
            | "cue"
            | "file-selector-button"
            | "first-letter"
            | "first-line"
            | "grammar-error"
            | "marker"
            | "picker-icon"
            | "placeholder"
            | "selection"
            | "spelling-error"
            | "target-text"
            | "view-transition"
    )
}

pub fn get_computed_style_pseudo_element(value: &str) -> GetComputedStylePseudoElement {
    let trimmed = value.trim_start();
    if !trimmed.starts_with(':') {
        return GetComputedStylePseudoElement::OriginatingElement;
    }
    let (double_colon, rest) = if let Some(rest) = trimmed.strip_prefix("::") {
        (true, rest)
    } else {
        (false, &trimmed[1..])
    };
    match parse_get_computed_style_pseudo_component(rest, double_colon) {
        Some(pseudo) => GetComputedStylePseudoElement::PseudoElement(pseudo),
        None => GetComputedStylePseudoElement::EmptyStyle,
    }
}

fn parse_get_computed_style_pseudo_component(value: &str, allow_modern: bool) -> Option<String> {
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    let token = parser
        .next_including_whitespace_and_comments()
        .ok()?
        .clone();
    let pseudo = match token {
        Token::Ident(name) => {
            let name = name.to_ascii_lowercase();
            if matches!(
                name.as_str(),
                "before" | "after" | "first-line" | "first-letter"
            ) || (allow_modern && is_known_terminal_pseudo_element_name(&name))
            {
                name
            } else {
                return None;
            }
        }
        Token::Function(name) if allow_modern => {
            let name = name.to_ascii_lowercase();
            let argument = parser
                .parse_nested_block(|nested| {
                    parse_functional_pseudo_element_argument(&name, nested)
                })
                .ok()?;
            format!("{name}({argument})")
        }
        _ => return None,
    };
    parser
        .next_including_whitespace_and_comments()
        .is_err()
        .then_some(pseudo)
}

fn parse_functional_pseudo_element_argument<'i, 't>(
    name: &str,
    parser: &mut Parser<'i, 't>,
) -> Result<String, cssparser::ParseError<'i, ()>> {
    let argument = match name {
        "highlight"
        | "view-transition-image-pair"
        | "view-transition-group"
        | "view-transition-old"
        | "view-transition-new" => {
            let ident = parser
                .expect_ident_cloned()
                .map_err(cssparser::ParseError::from)?;
            let ident = ident.to_string();
            if ident == "*" {
                return Err(parser.new_custom_error(()));
            }
            ident
        }
        "picker" => {
            let ident = parser
                .expect_ident_cloned()
                .map_err(cssparser::ParseError::from)?;
            if !ident.eq_ignore_ascii_case("select") {
                return Err(parser.new_custom_error(()));
            }
            "select".to_owned()
        }
        _ => return Err(parser.new_custom_error(())),
    };
    if parser.is_exhausted() {
        Ok(argument)
    } else {
        Err(parser.new_custom_error(()))
    }
}

pub(crate) fn consume_nested_component_value<'i>(
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
    use super::*;

    #[test]
    fn parses_simple_attribute_selectors_with_cssparser_tokens() {
        assert_eq!(
            parse_css_attribute_selector("[data-x]").unwrap(),
            CssAttributeSelector {
                name: "data-x".to_owned(),
                operator: CssAttributeSelectorOperator::Exists,
                expected: None,
            }
        );
        assert_eq!(
            parse_css_attribute_selector(r#"[data-x~="foo"]"#).unwrap(),
            CssAttributeSelector {
                name: "data-x".to_owned(),
                operator: CssAttributeSelectorOperator::Includes,
                expected: Some("foo".to_owned()),
            }
        );
        assert_eq!(
            parse_css_attribute_selector("[data-x^='foo']")
                .unwrap()
                .operator,
            CssAttributeSelectorOperator::Prefix
        );
        assert_eq!(
            parse_css_attribute_selector(r#"[data-x*=""]"#).unwrap(),
            CssAttributeSelector {
                name: "data-x".to_owned(),
                operator: CssAttributeSelectorOperator::Substring,
                expected: Some(String::new()),
            }
        );
    }

    #[test]
    fn rejects_attribute_selectors_outside_cssom_serializer_subset() {
        assert!(parse_css_attribute_selector("[data-x=\"foo\" i]").is_none());
        assert!(parse_css_attribute_selector("[|data-x]").is_none());
        assert!(parse_css_attribute_selector("[data-x=foo bar]").is_none());
        assert!(parse_css_attribute_selector("data-x=foo").is_none());
    }

    #[test]
    fn detects_namespace_separators_with_css_tokens() {
        assert!(selector_list_has_namespace_separator("myhtml|div"));
        assert!(selector_list_has_namespace_separator("*|div"));
        assert!(selector_list_has_namespace_separator(r#"[ns\:odd|foo]"#));
        assert!(!selector_list_has_namespace_separator(r#"[data-x|="foo"]"#));
        assert!(!selector_list_has_namespace_separator(r#"[data-x="a|b"]"#));
    }

    #[test]
    fn collects_namespace_prefixes_with_css_tokens() {
        assert_eq!(
            selector_list_namespace_prefixes("svg|a, :not(math|mi), *|span, |div"),
            vec!["svg".to_owned(), "math".to_owned()]
        );
        assert!(selector_list_namespace_prefixes(r#"[data-x|="foo"]"#).is_empty());
    }

    #[test]
    fn serializes_cssom_selector_text_without_default_namespace() {
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace("  #container ").unwrap(),
            "#container"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace("span  div  ").unwrap(),
            "span div"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace("*|div").unwrap(),
            "div"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace("*|*").unwrap(),
            "*"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace("[|lang]").unwrap(),
            "[lang]"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace("[*|lang]").unwrap(),
            "[*|lang]"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace("*:not(:active)").unwrap(),
            ":not(:active)"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace(":first-line").unwrap(),
            "::first-line"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace("[att=val]").unwrap(),
            r#"[att="val"]"#
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace(r"[ns\:foo]").unwrap(),
            r"[ns\:foo]"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace(r"[\30zonk]").unwrap(),
            r"[\30 zonk]"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace(r"[\@]").unwrap(),
            r"[\@]"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace(r#"[att~="val"]"#).unwrap(),
            r#"[att~="val"]"#
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace(":is(ul,ol,.list) > [hidden]")
                .unwrap(),
            ":is(ul, ol, .list) > [hidden]"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace(":where(:hover,:focus)")
                .unwrap(),
            ":where(:hover, :focus)"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace(":not([disabled],[selected])")
                .unwrap(),
            ":not([disabled], [selected])"
        );
    }

    #[test]
    fn preserves_only_invalid_forgiving_selector_items() {
        let serialize = |selector| {
            serialize_cssom_selector_text_preserving_invalid_forgiving_items(
                selector,
                false,
                &[],
                |candidate| crate::stylo::validate_supports_selector_list(candidate).is_ok(),
            )
            .unwrap()
        };

        assert_eq!(
            serialize("::part(foo):is([attr='value'])"),
            "::part(foo):is([attr='value'])"
        );
        assert_eq!(
            serialize("::part(foo):where([attr='value'])"),
            "::part(foo):where([attr='value'])"
        );
        assert_eq!(
            serialize(":is([att=val], ::before:HOVER  )"),
            r#":is([att="val"], ::before:HOVER)"#
        );
        assert_eq!(serialize(":is(,,, )"), ":is(, , , )");
        assert_eq!(serialize(":host(:is(,,,))"), ":host(:is(, , , ))");
        assert_eq!(
            serialize(":host(:is(.a, .b+.c, .d))"),
            ":host(:is(.a, .b+.c, .d))"
        );
        assert_eq!(
            serialize(":has(:is(:has(.a+.b)))"),
            ":has(:is(:has(.a+.b)))"
        );
        assert_eq!(
            serialize(":is([att='valid'], .other)"),
            r#":is([att="valid"], .other)"#
        );
    }

    #[test]
    fn parses_get_computed_style_pseudo_element_argument() {
        assert_eq!(
            get_computed_style_pseudo_element("before"),
            GetComputedStylePseudoElement::OriginatingElement
        );
        assert_eq!(
            get_computed_style_pseudo_element("file-selector-button"),
            GetComputedStylePseudoElement::OriginatingElement
        );
        assert_eq!(
            get_computed_style_pseudo_element(":before"),
            GetComputedStylePseudoElement::PseudoElement("before".to_owned())
        );
        assert_eq!(
            get_computed_style_pseudo_element(":checkmark"),
            GetComputedStylePseudoElement::EmptyStyle
        );
        assert_eq!(
            get_computed_style_pseudo_element("::checkmark"),
            GetComputedStylePseudoElement::PseudoElement("checkmark".to_owned())
        );
        assert_eq!(
            get_computed_style_pseudo_element("::highlight( n\\61me )"),
            GetComputedStylePseudoElement::PseudoElement("highlight(name)".to_owned())
        );
        assert_eq!(
            get_computed_style_pseudo_element("::picker(select)"),
            GetComputedStylePseudoElement::PseudoElement("picker(select)".to_owned())
        );
        assert_eq!(
            get_computed_style_pseudo_element("::picker(div)"),
            GetComputedStylePseudoElement::EmptyStyle
        );
        assert_eq!(
            get_computed_style_pseudo_element("::before(test)"),
            GetComputedStylePseudoElement::EmptyStyle
        );
    }

    #[test]
    fn serializes_cssom_nth_pseudo_class_arguments() {
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace(":nth-child(  3n - 0)")
                .unwrap(),
            ":nth-child(3n)"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace(":nth-child(even)").unwrap(),
            ":nth-child(2n)"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace(":nth-last-child(odd)")
                .unwrap(),
            ":nth-last-child(2n+1)"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace(":nth-of-type( -1n + 5 )")
                .unwrap(),
            ":nth-of-type(-n+5)"
        );
        assert_eq!(
            serialize_cssom_selector_text_without_default_namespace(":nth-last-of-type(+10)")
                .unwrap(),
            ":nth-last-of-type(10)"
        );
    }

    #[test]
    fn serializes_cssom_selector_text_with_default_namespace() {
        let prefixes = vec!["nsdefault".to_owned()];
        assert_eq!(
            serialize_cssom_selector_text("*|div", true, &prefixes).unwrap(),
            "*|div"
        );
        assert_eq!(
            serialize_cssom_selector_text("*|*.c", true, &prefixes).unwrap(),
            "*|*.c"
        );
        assert_eq!(
            serialize_cssom_selector_text("nsdefault|div", true, &prefixes).unwrap(),
            "div"
        );
        assert_eq!(
            serialize_cssom_selector_text("nsdefault|*.c", true, &prefixes).unwrap(),
            ".c"
        );
        assert_eq!(
            serialize_cssom_selector_text("svg|*.c", true, &prefixes).unwrap(),
            "svg|*.c"
        );
    }

    #[test]
    fn detects_known_terminal_pseudo_element_selectors_with_css_tokens() {
        assert!(dom_api_selector_list_has_only_known_pseudo_elements(
            "#target::before"
        ));
        assert!(dom_api_selector_list_has_only_known_pseudo_elements(
            "#target:first-line, #other::after"
        ));
        assert!(dom_api_selector_list_has_only_known_pseudo_elements(
            "#target::placeholder"
        ));
        assert!(dom_api_selector_list_has_only_known_pseudo_elements(
            "#target::selection, #other::marker"
        ));
        assert!(dom_api_selector_list_has_only_known_pseudo_elements(
            "#file::file-selector-button"
        ));
        assert!(!dom_api_selector_list_has_only_known_pseudo_elements(
            "#target::example"
        ));
        assert!(!dom_api_selector_list_has_only_known_pseudo_elements(
            "#target::before, .real"
        ));
        assert!(dom_api_selector_list_has_only_known_pseudo_elements(
            "::slotted(foo)"
        ));
        assert!(dom_api_selector_list_has_only_known_pseudo_elements(
            "::slotted(foo"
        ));
        assert!(dom_api_selector_list_has_only_known_pseudo_elements(
            "#target::part(label)"
        ));
        assert!(dom_api_selector_list_has_only_known_pseudo_elements(
            "#target::highlight(foo)"
        ));
        assert!(dom_api_selector_list_has_only_known_pseudo_elements(
            "span::-WeBkIt-something-invalid"
        ));
        assert!(!dom_api_selector_list_has_only_known_pseudo_elements(
            "input:-webkit-autofill"
        ));
    }

    #[test]
    fn detects_known_pseudo_elements_with_following_pseudo_classes() {
        assert!(dom_api_selector_list_contains_known_pseudo_element(
            "::part(label):hover"
        ));
        assert!(dom_api_selector_list_contains_known_pseudo_element(
            "::part(label):lang(en)"
        ));
        assert!(dom_api_selector_list_contains_known_pseudo_element(
            "#target::before:hover"
        ));
        assert!(!dom_api_selector_list_has_only_known_pseudo_elements(
            "::part(label):hover"
        ));
        assert!(!dom_api_selector_list_contains_known_pseudo_element(
            ".real:hover"
        ));
    }

    #[test]
    fn rewrites_webkit_compat_pseudo_elements_only_for_stylo_validation() {
        assert_eq!(
            webkit_compat_pseudo_element_validation_selector(
                "span::-WeBkIt-something-invalid:active"
            )
            .as_deref(),
            Some("span::part(webkit-compat):active")
        );
        assert_eq!(
            webkit_compat_pseudo_element_validation_selector(
                r#"[data-value='::-webkit-something-invalid']"#
            ),
            None
        );
    }

    #[test]
    fn detects_invalid_terminal_pseudo_element_chains() {
        assert!(selector_list_has_invalid_terminal_pseudo_element_chain(
            "::before::highlight(foo)"
        ));
        assert!(selector_list_has_invalid_terminal_pseudo_element_chain(
            "::highlight(foo)::after"
        ));
        assert!(selector_list_has_invalid_terminal_pseudo_element_chain(
            "::highlight(foo)::part(label)"
        ));
        assert!(selector_list_has_invalid_terminal_pseudo_element_chain(
            "span::-webkit-something-invalid::before"
        ));
        assert!(!selector_list_has_invalid_terminal_pseudo_element_chain(
            "span::-webkit-something-invalid:active"
        ));
        assert!(!selector_list_has_invalid_terminal_pseudo_element_chain(
            "input:-webkit-autofill::before"
        ));
        assert!(!selector_list_has_invalid_terminal_pseudo_element_chain(
            "::part(label)::highlight(foo)"
        ));
        assert!(!selector_list_has_invalid_terminal_pseudo_element_chain(
            "#target::highlight(foo), #other::after"
        ));
    }

    #[test]
    fn detects_defined_pseudo_class_with_css_tokens() {
        assert!(dom_api_selector_list_uses_defined_pseudo_class(":defined"));
        assert!(dom_api_selector_list_uses_defined_pseudo_class(
            "wpt-defined-case:Defined"
        ));
        assert!(dom_api_selector_list_uses_defined_pseudo_class(
            ":is(:DEFINED)"
        ));
        assert!(dom_api_selector_list_uses_defined_pseudo_class(
            r#":\64 efined"#
        ));
        assert!(!dom_api_selector_list_uses_defined_pseudo_class(".defined"));
        assert!(!dom_api_selector_list_uses_defined_pseudo_class(
            "[data-x=':defined']"
        ));
        assert!(!dom_api_selector_list_uses_defined_pseudo_class(
            "div:definedish"
        ));
        assert!(!dom_api_selector_list_uses_defined_pseudo_class(
            "div::defined"
        ));
    }

    #[test]
    fn recovers_trailing_unclosed_attribute_selector_text() {
        assert_eq!(
            dom_api_selector_text_with_trailing_attribute_recovery(r#"#x [align="center""#),
            r#"#x [align="center"]"#
        );
        assert_eq!(
            dom_api_selector_text_with_trailing_attribute_recovery("#x [align]"),
            "#x [align]"
        );
        assert_eq!(
            dom_api_selector_text_with_trailing_attribute_recovery("[class=space unquoted]"),
            "[class=space unquoted]"
        );
    }
}
