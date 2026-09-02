use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
};

use crate::{
    canonical_style_property_name, split_important_priority, unescape_top_level_semicolons,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssDeclaration {
    pub name: String,
    pub value: String,
    pub important: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DeclarationParseOptions {
    /// When true, the property name is canonicalised via
    /// [`canonical_style_property_name`] before being stored on the result.
    pub canonicalize_property_name: bool,
    /// When true, top-level `\;` escape sequences in non-custom values are
    /// unescaped. Custom property values keep source escapes because CSSOM
    /// exposes the token stream text.
    pub unescape_value_semicolons: bool,
    /// When true, declarations with an empty value are returned to the caller.
    /// This is used for CSSOM's specified-value shorthand serialization.
    pub preserve_empty_values: bool,
}

pub fn parse_declaration_list(
    style_text: &str,
    options: DeclarationParseOptions,
) -> Vec<CssDeclaration> {
    let mut input = ParserInput::new(style_text);
    let mut input = Parser::new(&mut input);
    let mut parser = DeclarationListParser { options };
    RuleBodyParser::new(&mut input, &mut parser)
        .filter_map(Result::ok)
        .collect()
}

/// Recover the declaration-list serialization for a stylesheet that ended in
/// one valid, unclosed `var()` reference.
///
/// Stylo accepts the EOF-recovered rule but currently appends the missing `)`
/// to its stored value. This helper only removes that character when the raw
/// final declaration and Stylo's serialized final declaration otherwise match
/// exactly. The caller is responsible for associating the result with the
/// rightmost native style rule.
pub fn recover_stylesheet_eof_open_var_declaration_text(
    stylesheet_text: &str,
    serialized_declaration_text: &str,
) -> Option<String> {
    let options = DeclarationParseOptions {
        canonicalize_property_name: false,
        unescape_value_semicolons: true,
        preserve_empty_values: false,
    };
    let serialized_declaration = parse_declaration_list(serialized_declaration_text, options)
        .into_iter()
        .last()?;

    for block_start in eof_open_curly_block_starts(stylesheet_text) {
        let Some(source_declaration) =
            parse_declaration_list(&stylesheet_text[block_start + 1..], options)
                .into_iter()
                .last()
        else {
            continue;
        };
        if !crate::value::css_value_is_eof_open_var_function(&source_declaration.value)
            || crate::canonical_style_property_name(&source_declaration.name)
                != crate::canonical_style_property_name(&serialized_declaration.name)
            || source_declaration.important != serialized_declaration.important
            || serialized_declaration.value != format!("{})", source_declaration.value)
        {
            continue;
        }

        let value_start = serialized_declaration_text.rfind(&serialized_declaration.value)?;
        let closing_parenthesis = value_start + serialized_declaration.value.len() - 1;
        if serialized_declaration_text.as_bytes()[closing_parenthesis] != b')'
            || !serialized_declaration_suffix_matches(
                &serialized_declaration_text[closing_parenthesis + 1..],
                serialized_declaration.important,
            )
        {
            continue;
        }

        let mut recovered = String::with_capacity(serialized_declaration_text.len() - 1);
        recovered.push_str(&serialized_declaration_text[..closing_parenthesis]);
        recovered.push_str(&serialized_declaration_text[closing_parenthesis + 1..]);
        return Some(recovered);
    }
    None
}

fn serialized_declaration_suffix_matches(suffix: &str, important: bool) -> bool {
    let suffix = suffix.trim();
    if important {
        suffix.eq_ignore_ascii_case("!important;")
    } else {
        suffix == ";"
    }
}

fn eof_open_curly_block_starts(css_text: &str) -> Vec<usize> {
    let mut open_blocks = Vec::new();
    let mut input = ParserInput::new(css_text);
    let mut input = Parser::new(&mut input);
    collect_eof_open_curly_block_starts(&mut input, &mut open_blocks);
    open_blocks
}

fn collect_eof_open_curly_block_starts<'i, 't>(
    input: &mut Parser<'i, 't>,
    open_blocks: &mut Vec<usize>,
) {
    loop {
        let token_start = input.position();
        let Ok(token) = input.next_including_whitespace_and_comments().cloned() else {
            break;
        };
        let is_curly = matches!(token, cssparser::Token::CurlyBracketBlock);
        if !matches!(
            token,
            cssparser::Token::Function(_)
                | cssparser::Token::ParenthesisBlock
                | cssparser::Token::SquareBracketBlock
                | cssparser::Token::CurlyBracketBlock
        ) {
            continue;
        }

        let mut nested_end = None;
        let nested: Result<(), ParseError<'i, ()>> = input.parse_nested_block(|input| {
            collect_eof_open_curly_block_starts(input, open_blocks);
            nested_end = Some(input.position());
            Ok(())
        });
        if nested.is_ok() && is_curly && nested_end == Some(input.position()) {
            open_blocks.push(token_start.byte_index());
        }
    }
}

struct DeclarationListParser {
    options: DeclarationParseOptions,
}

impl<'i> DeclarationParser<'i> for DeclarationListParser {
    type Declaration = CssDeclaration;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &cssparser::ParserState,
    ) -> Result<Self::Declaration, ParseError<'i, Self::Error>> {
        let value_start = input.position();
        if !input.is_exhausted() {
            let first_token_ok = input
                .try_parse(|input| input.expect_no_error_token())
                .is_ok();
            if !first_token_ok {
                let raw_value = input.slice_from(value_start).trim();
                if !crate::value::css_declaration_value_has_valid_var_functions(raw_value) {
                    return Err(input.new_custom_error(()));
                }
            }
        }
        let raw_name = name.as_ref().trim();
        let name = if self.options.canonicalize_property_name {
            canonical_style_property_name(raw_name)
        } else {
            raw_name.to_owned()
        };
        let (value, important) = split_important_priority(input.slice_from(value_start));
        let value = if self.options.unescape_value_semicolons && !name.starts_with("--") {
            unescape_top_level_semicolons(&value)
        } else {
            value
        };
        if name.is_empty() || (value.is_empty() && !self.options.preserve_empty_values) {
            return Err(input.new_custom_error(()));
        }
        Ok(CssDeclaration {
            name,
            value,
            important,
        })
    }
}

impl<'i> AtRuleParser<'i> for DeclarationListParser {
    type Prelude = ();
    type AtRule = CssDeclaration;
    type Error = ();
}

impl<'i> QualifiedRuleParser<'i> for DeclarationListParser {
    type Prelude = ();
    type QualifiedRule = CssDeclaration;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, CssDeclaration, ()> for DeclarationListParser {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{DeclarationParseOptions, parse_declaration_list};

    fn renderer_opts() -> DeclarationParseOptions {
        DeclarationParseOptions {
            canonicalize_property_name: false,
            unescape_value_semicolons: true,
            preserve_empty_values: false,
        }
    }

    fn cdp_opts() -> DeclarationParseOptions {
        DeclarationParseOptions {
            canonicalize_property_name: true,
            unescape_value_semicolons: false,
            preserve_empty_values: false,
        }
    }

    #[test]
    fn declaration_parser_preserves_nested_semicolons() {
        let entries = parse_declaration_list(
            r#"color: red; content: "a;b"; background-image: url("data:image/svg+xml;a=b");"#,
            renderer_opts(),
        );
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[1].name, "content");
        assert_eq!(entries[1].value, r#""a;b""#);
        assert_eq!(entries[2].value, r#"url("data:image/svg+xml;a=b")"#);
    }

    #[test]
    fn declaration_parser_preserves_escaped_custom_property_names() {
        let entries = parse_declaration_list(
            r#"--a\;b:value; --\\: value; --value: a\;b;"#,
            renderer_opts(),
        );
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "--a;b");
        assert_eq!(entries[0].value, "value");
        assert_eq!(entries[1].name, r#"--\"#);
        assert_eq!(entries[1].value, "value");
        assert_eq!(entries[2].name, "--value");
        assert_eq!(entries[2].value, r#"a\;b"#);
    }

    #[test]
    fn declaration_parser_handles_priority_and_invalid_blocks() {
        let entries = parse_declaration_list(
            "display: block !important; broken { color: red; } width: 10px;",
            renderer_opts(),
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "display");
        assert_eq!(entries[0].value, "block");
        assert!(entries[0].important);
    }

    #[test]
    fn declaration_parser_recovers_left_open_var_function_at_eof() {
        let entries = parse_declaration_list("width: var(--prop", renderer_opts());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "width");
        assert_eq!(entries[0].value, "var(--prop");
    }

    #[test]
    fn stylesheet_eof_open_var_recovery_removes_only_stylos_inserted_parenthesis() {
        let source = r#"
            .ignored::before { content: "}"; }
            /* ignored { */
            @media all { .target { color: red; width: var(--prop
        "#;
        assert_eq!(
            super::recover_stylesheet_eof_open_var_declaration_text(
                source,
                "color: red; width: var(--prop);",
            )
            .as_deref(),
            Some("color: red; width: var(--prop;")
        );
        assert_eq!(
            super::recover_stylesheet_eof_open_var_declaration_text(
                ".target { width: var(--prop)",
                "width: var(--prop);",
            ),
            None
        );
        assert_eq!(
            super::recover_stylesheet_eof_open_var_declaration_text(
                ".target { width: var(--prop",
                "width: var(--other);",
            ),
            None
        );
    }

    #[test]
    fn declaration_parser_canonicalizes_when_requested() {
        let entries = parse_declaration_list(
            "CSSFloat: left; -Webkit-Box-Sizing: border-box;",
            cdp_opts(),
        );
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "cssfloat");
        assert_eq!(entries[1].name, "box-sizing");
    }

    #[test]
    fn stylo_declaration_block_surface_uses_pdb_semantics() {
        let block =
            crate::parse_declaration_block("padding: 1px 2px; color: nope; color: red !important;");

        assert_eq!(block.property_value("padding").as_deref(), Some("1px 2px"));
        assert_eq!(block.property_value("color").as_deref(), Some("red"));
        assert!(block.property_priority("color"));
        assert_eq!(block.css_text(), "padding: 1px 2px; color: red !important;");
    }
}
