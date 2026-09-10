use cssparser::{Parser, ParserInput, Token};

use crate::unquote_css_string;

pub use style::moli_font_face::{CssFontFace, normalize_font_face_src, parse_font_faces};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CssFontSource {
    Local(String),
    Url(String),
}

/// Parse through Stylo first; the token walk only extracts validated sources.
pub fn parse_font_face_sources(source: &str) -> Option<Vec<CssFontSource>> {
    let normalized = normalize_font_face_src(source)?;
    let mut input = ParserInput::new(&normalized);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_comma_separated(|parser| {
            let name = parser.expect_function()?.clone();
            let source = parser.parse_nested_block(|nested| {
                if name.eq_ignore_ascii_case("local") {
                    let mut parts = Vec::new();
                    while !nested.is_exhausted() {
                        parts.push(nested.expect_ident_or_string()?.to_string());
                    }
                    Ok(CssFontSource::Local(parts.join(" ")))
                } else if name.eq_ignore_ascii_case("url") {
                    Ok(CssFontSource::Url(nested.expect_string()?.to_string()))
                } else {
                    Err(nested.new_custom_error(()))
                }
            })?;
            let mut supported = true;
            while !parser.is_exhausted() {
                let kind = parser.expect_function()?.clone();
                parser.parse_nested_block(|nested| {
                    let mut values = Vec::new();
                    while !nested.is_exhausted() {
                        if nested.try_parse(|p| p.expect_comma()).is_ok() {
                            continue;
                        }
                        values.push(nested.expect_ident_or_string()?.to_ascii_lowercase());
                    }
                    supported &= kind.eq_ignore_ascii_case("format")
                        && values.iter().all(|v| {
                            matches!(
                                v.as_str(),
                                "woff"
                                    | "woff2"
                                    | "truetype"
                                    | "ttf"
                                    | "opentype"
                                    | "otf"
                                    | "collection"
                            )
                        });
                    Ok::<_, cssparser::ParseError<'_, ()>>(())
                })?;
            }
            Ok(supported.then_some(source))
        })
        .ok()
        .map(|sources| sources.into_iter().flatten().collect())
}

#[derive(Clone, Debug)]
pub struct CssFontShorthand {
    pub family_css: String,
    pub families: Vec<String>,
    pub size: String,
    pub style: String,
    pub weight: String,
    pub stretch: String,
}

/// Shares the CSS font grammar with style declarations, including quoted
/// multi-word families and fallback lists. No guessed last-whitespace token.
pub fn parse_font_shorthand(value: &str) -> Option<CssFontShorthand> {
    use style::{
        properties::{PropertyId, parse_property_declaration_list},
        stylesheets::CssRuleType,
    };
    use style_traits::CssStringWriter;
    if crate::escape_top_level_semicolons(value) != value
        || crate::split_important_priority(value).1
    {
        return None;
    }
    crate::stylo_stylesheet::with_descriptor_declaration_context(CssRuleType::Style, |context| {
        let text = format!("font: {value}");
        let mut input = ParserInput::new(&text);
        let block = parse_property_declaration_list(context, &mut Parser::new(&mut input), &[]);
        let property = |name: &str| -> Option<String> {
            let mut out = CssStringWriter::new();
            block
                .property_value_to_css(&PropertyId::parse(name, context).ok()?, &mut out)
                .ok()?;
            (!out.is_empty()).then(|| out.to_string())
        };
        let family_css = property("font-family")?;
        let size = property("font-size")?;
        if is_css_wide_keyword(&size) || size.contains("var(") {
            return None;
        }
        let mut input = ParserInput::new(&family_css);
        let families = Parser::new(&mut input)
            .parse_comma_separated(|p| {
                let mut parts = Vec::new();
                while !p.is_exhausted() {
                    parts.push(p.expect_ident_or_string()?.to_string());
                }
                Ok::<_, cssparser::ParseError<'_, ()>>(parts.join(" "))
            })
            .ok()?;
        drop(input);
        Some(CssFontShorthand {
            family_css,
            families,
            size,
            style: property("font-style")?,
            weight: property("font-weight")?,
            stretch: property("font-stretch")?,
        })
    })
}

pub fn font_load_query_contains_css_wide_keyword(query: &str) -> bool {
    let mut input = ParserInput::new(query);
    let mut input = Parser::new(&mut input);
    while let Ok(token) = input.next() {
        match token {
            Token::Ident(value) | Token::QuotedString(value)
                if is_css_wide_keyword(value.as_ref()) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

pub fn font_load_query_family(query: &str) -> Option<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut quote = None;
    let mut last_ws = None;
    for (index, ch) in trimmed.char_indices() {
        match ch {
            '\'' | '"' if quote == Some(ch) => quote = None,
            '\'' | '"' if quote.is_none() => quote = Some(ch),
            _ if quote.is_none() && ch.is_whitespace() => last_ws = Some(index),
            _ => {}
        }
    }
    let family = last_ws
        .map(|index| trimmed[index..].trim())
        .unwrap_or(trimmed);
    Some(unquote_css_string(family))
}

fn is_css_wide_keyword(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "revert-rule"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn font_sources_preserve_local_names_and_supported_url_fallback_order() {
        use super::*;
        assert_eq!(
            parse_font_face_sources(
                "local(Missing Font), url(bad) format('unsupported'), url(good.woff2) format('woff2')"
            ),
            Some(vec![
                CssFontSource::Local("Missing Font".into()),
                CssFontSource::Url("good.woff2".into())
            ])
        );
        assert!(parse_font_face_sources("not a source").is_none());
    }

    #[test]
    fn font_shorthand_uses_stylo_validation_and_keeps_all_families() {
        let font =
            super::parse_font_shorthand("italic bold 10.5px/1.2 \"A B\", Another Face, serif")
                .unwrap();
        assert_eq!(font.families, ["A B", "Another Face", "serif"]);
        assert_eq!(font.size, "10.5px");
        assert_eq!(font.style, "italic");
        for invalid in [
            "",
            "12px",
            "not a font",
            "inherit",
            "12px serif; color:red",
            "12px serif !important",
        ] {
            assert!(super::parse_font_shorthand(invalid).is_none(), "{invalid}");
        }
    }
    use super::{
        font_load_query_contains_css_wide_keyword, font_load_query_family, normalize_font_face_src,
        parse_font_faces,
    };

    #[test]
    fn font_face_parser_uses_cssparser_rule_boundaries() {
        let entries = parse_font_faces(
            r#"
            .ignored { content: "@font-face { font-family: Bad; src: url(bad.woff2); }"; }
            @font-face {
                font-family: "A; B";
                src: url("data:font/woff2;base64;a;b");
            }
            @FONT-FACE {
                font-family: CaseFace;
                src: local("Case Face");
            }
            "#,
        );
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].family, "A; B");
        assert_eq!(entries[0].source, r#"url("data:font/woff2;base64;a;b")"#);
        assert_eq!(entries[1].family, "CaseFace");
        assert_eq!(entries[1].source, r#"local("Case Face")"#);
    }

    #[test]
    fn font_face_parser_filters_invalid_and_incomplete_faces() {
        let entries = parse_font_faces(
            r#"
            @font-face { font-family: serif; src: url(generic.woff2); }
            @font-face { font-family: MissingSource; }
            @font-face { src: url(missing-family.woff2); }
            @font-face { font-family: Valid; src: url(valid.woff2); }
            "#,
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].family, "Valid");
        assert_eq!(entries[0].source, r#"url("valid.woff2")"#);
    }

    #[test]
    fn font_face_src_normalizer_quotes_unquoted_urls() {
        assert_eq!(
            normalize_font_face_src("local(STIXGeneral), url(/stixfonts/STIXGeneral.otf)")
                .as_deref(),
            Some(r#"local(STIXGeneral), url("/stixfonts/STIXGeneral.otf")"#)
        );
        assert_eq!(
            normalize_font_face_src("url(http://foo/bar/font.ttf)").as_deref(),
            Some(r#"url("http://foo/bar/font.ttf")"#)
        );
    }

    #[test]
    fn font_load_query_uses_css_tokens_for_family_and_keywords() {
        assert!(font_load_query_contains_css_wide_keyword(
            r#"italic 16px inherit"#
        ));
        assert!(!font_load_query_contains_css_wide_keyword(
            r#"16px "inheritance""#
        ));
        assert_eq!(
            font_load_query_family(r#"italic small-caps bold 16px/2 "A B", serif"#).as_deref(),
            Some("serif")
        );
        assert_eq!(
            font_load_query_family(r#""Standalone Family""#).as_deref(),
            Some("Standalone Family")
        );
    }
}
