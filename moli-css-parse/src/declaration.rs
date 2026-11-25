use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, Delimiter, ParseError, Parser, QualifiedRuleParser,
    RuleBodyItemParser, RuleBodyParser, parse_important,
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
    scan_declarations(style_text, options, None).0
}

/// Scan for CSSOM adapters and report whether the normalized rows preserve the
/// original property names, values, and priorities for native parsing.
pub fn parse_declaration_list_with_source_check(
    style_text: &str,
    options: DeclarationParseOptions,
) -> (Vec<CssDeclaration>, bool) {
    scan_declarations(style_text, options, Some(true))
}

fn scan_declarations(
    style_text: &str,
    options: DeclarationParseOptions,
    source_preserved: Option<bool>,
) -> (Vec<CssDeclaration>, bool) {
    let mut input = Parser::new(style_text);
    let mut parser = DeclarationListParser {
        options,
        source_preserved,
    };
    let declarations = RuleBodyParser::new(&mut input, &mut parser)
        .filter_map(Result::ok)
        .collect();
    (declarations, parser.source_preserved.unwrap_or(false))
}

struct DeclarationListParser {
    options: DeclarationParseOptions,
    source_preserved: Option<bool>,
}

impl<'i> DeclarationParser<'i> for DeclarationListParser {
    type Declaration = CssDeclaration;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _declaration_start: &cssparser::ParserState,
    ) -> Result<Self::Declaration, ParseError<Self::Error>> {
        let value_start = input.position();
        if !input.is_exhausted() {
            let first_token_ok = input
                .try_parse(|input| input.expect_no_error_token())
                .is_ok();
            if !first_token_ok {
                let raw_value = input.slice_from(value_start).trim();
                if !crate::value::css_declaration_value_has_valid_var_functions(raw_value) {
                    return Err(cssparser::ParseError::custom(()));
                }
            }
        }
        let raw_name = name.as_ref();
        let name = if self.options.canonicalize_property_name {
            canonical_style_property_name(raw_name)
        } else {
            raw_name.trim().to_owned()
        };
        let raw_value = input.slice_from(value_start);
        let (value, important) = if let Some(source_preserved) = &mut self.source_preserved {
            *source_preserved &= raw_name == name;
            if let Some((value, important)) = split_declaration_priority(raw_value) {
                let trimmed = value.trim();
                *source_preserved &= trimmed == value.trim_matches(is_css_whitespace);
                (trimmed.to_owned(), important)
            } else {
                *source_preserved = false;
                split_important_priority(raw_value)
            }
        } else {
            split_important_priority(raw_value)
        };
        let value = if self.options.unescape_value_semicolons && !name.starts_with("--") {
            let unescaped = unescape_top_level_semicolons(&value);
            if let Some(source_preserved) = &mut self.source_preserved {
                *source_preserved &= unescaped == value;
            }
            unescaped
        } else {
            value
        };
        if name.is_empty() || (value.is_empty() && !self.options.preserve_empty_values) {
            return Err(cssparser::ParseError::custom(()));
        }
        Ok(CssDeclaration {
            name,
            value,
            important,
        })
    }
}

fn is_css_whitespace(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

pub(crate) fn split_declaration_priority(value: &str) -> Option<(&str, bool)> {
    if !value.contains('!') {
        return Some((value, false));
    }
    let mut input = Parser::new(value);
    let value = input
        .parse_until_before(Delimiter::Bang, |input| {
            let start = input.position();
            input.expect_no_error_token()?;
            Ok::<_, ParseError<()>>(input.slice_from(start))
        })
        .ok()?;
    if input.is_exhausted() {
        return Some((value, false));
    }
    parse_important(&mut input).ok()?;
    input.expect_exhausted().ok()?;
    Some((value, true))
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
