use cssparser::{Parser, Token};

pub use style::moli_font_face::{CssFontFace, normalize_font_face_src, parse_font_faces};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CssFontSource {
    Local(String),
    Url(String),
}

/// Parse through Stylo first; the token walk only extracts validated sources.
pub fn parse_font_face_sources(source: &str) -> Option<Vec<CssFontSource>> {
    let normalized = normalize_font_face_src(source)?;
    let mut parser = Parser::new(&normalized);
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
                    Err(cssparser::ParseError::custom(()))
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
                        && !values.is_empty()
                        && values.iter().all(|v| {
                            matches!(
                                legacy_variations_font_format(v).unwrap_or(v),
                                "woff"
                                    | "woff2"
                                    | "truetype"
                                    | "ttf"
                                    | "opentype"
                                    | "otf"
                                    | "collection"
                            )
                        });
                    Ok::<_, cssparser::ParseError<()>>(())
                })?;
            }
            Ok(supported.then_some(source))
        })
        .ok()
        .map(|sources| sources.into_iter().flatten().collect())
}

/// Maps a legacy variable font format hint to its container format.
pub fn legacy_variations_font_format(format: &str) -> Option<&str> {
    format
        .strip_suffix("-variations")
        .filter(|format| matches!(*format, "woff" | "woff2" | "truetype" | "opentype"))
}

/// A decoded family name or a generic selected by the CSS parser. Quoting a
/// generic keyword makes it a named family rather than a generic fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CssFontFamily {
    Named(String),
    Generic(style::values::computed::font::GenericFontFamily),
}

/// A CSS font-family lookup key matching Blink's case-folding hash: Unicode
/// simple folding of UTF-16 code units, without normalization or locale tailoring.
/// This is separate from platform local() full-name lookup.
pub fn font_family_name_key(name: &str) -> String {
    if name.is_ascii() {
        return name.to_ascii_lowercase();
    }
    name.chars()
        .map(|character| {
            // Blink hashes each UTF-16 code unit; surrogate pairs do not fold.
            if character.len_utf16() == 2 {
                return character;
            }
            unicode_case_mapping::case_folded(character)
                .and_then(|folded| char::from_u32(folded.get()))
                .unwrap_or(character)
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct CssFontShorthand {
    pub family_css: String,
    pub families: Vec<CssFontFamily>,
    pub size: String,
    pub style: String,
    pub weight: String,
    pub stretch: String,
}

/// Shares the CSS font grammar with style declarations, including quoted
/// multi-word families and fallback lists. No guessed last-whitespace token.
pub fn parse_font_shorthand(value: &str) -> Option<CssFontShorthand> {
    use style::{
        parser::Parse,
        properties::{PropertyId, parse_property_declaration_list},
        stylesheets::CssRuleType,
        values::computed::font::SingleFontFamily,
    };
    use style_traits::CssStringWriter;
    if crate::escape_top_level_semicolons(value) != value
        || crate::split_important_priority(value).1
    {
        return None;
    }
    crate::stylo_stylesheet::with_descriptor_declaration_context(CssRuleType::Style, |context| {
        let text = format!("font: {value}");
        let block = parse_property_declaration_list(context, &mut Parser::new(&text), &[]);
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
        let families = Parser::new(&family_css)
            .parse_comma_separated(|p| {
                SingleFontFamily::parse(context, p).map(|family| match family {
                    SingleFontFamily::FamilyName(name) => {
                        CssFontFamily::Named(name.name.to_string())
                    }
                    SingleFontFamily::Generic(generic) => CssFontFamily::Generic(generic),
                })
            })
            .ok()?;
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
    let input = query;
    let mut input = Parser::new(input);
    while let Ok(token) = input.next() {
        match token {
            Token::Ident(value)
                if is_css_wide_keyword(value.as_ref()) || value.eq_ignore_ascii_case("default") =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// FontFaceSet queries use the font shorthand grammar, without cascade or
/// custom-property substitution. Keep parsing shared with CSS declarations.
pub fn font_load_query_is_valid(query: &str) -> bool {
    // Servo's Stylo configuration does not parse system fonts. Their entire
    // shorthand syntax is one of these six identifiers; use CSS tokens so
    // escapes, comments and case folding work without accepting trailing input.
    let input = query;
    let mut parser = Parser::new(input);
    if let Ok(name) = parser.expect_ident_cloned()
        && matches!(
            name.to_ascii_lowercase().as_str(),
            "caption" | "icon" | "menu" | "message-box" | "small-caption" | "status-bar"
        )
        && parser.is_exhausted()
    {
        return true;
    }
    if font_load_query_contains_css_wide_keyword(query) {
        return false;
    }
    let mut block = crate::CssDeclarationBlock::default();
    let projection = block.set_property_with_projection("font", query, false);
    projection.set_result != crate::CssSetResult::ParseError
        && !projection.has_unresolved_value
        && !block.is_empty()
}

pub fn font_load_query_family(query: &str) -> Option<String> {
    font_load_query_families(query)
        .or_else(|| parse_font_family_list(query))?
        .pop()
}

/// Extract literal family names from a font shorthand.
/// Stylo expands the shorthand; CSS tokens decode quoted names and escapes.
/// Unquoted generic families do not identify a user-provided FontFace.
pub fn font_load_query_families(query: &str) -> Option<Vec<String>> {
    let mut block = crate::CssDeclarationBlock::default();
    let projection = block.set_property_with_projection("font", query, false);
    let family_text = (projection.set_result != crate::CssSetResult::ParseError
        && !projection.has_unresolved_value)
        .then(|| block.property_value("font-family"))
        .flatten()?;
    parse_font_family_list(&family_text)
}

fn parse_font_family_list(input: &str) -> Option<Vec<String>> {
    let mut parser = Parser::new(input);
    let families = parser
        .parse_comma_separated::<_, _, ()>(|input| {
            if let Ok(name) = input.try_parse(|input| input.expect_string_cloned()) {
                input.expect_exhausted()?;
                return Ok(Some(name.to_string()));
            }
            let start = input.position();
            let mut parts = vec![input.expect_ident_cloned()?.to_string()];
            while !input.is_exhausted() {
                parts.push(input.expect_ident_cloned()?.to_string());
            }
            let named = crate::parse_font_face_descriptor_entry_with_stylo(
                "font-family",
                input.slice_from(start),
            )
            .is_some();
            Ok(named.then(|| parts.join(" ")))
        })
        .ok()?;
    Some(families.into_iter().flatten().collect())
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
    fn font_name_keys_follow_chromium_case_folding_without_expansion_or_normalization() {
        use super::font_family_name_key;
        for (a, b) in [
            ("Äpfel", "äpfel"),
            ("Σπίτι", "σπίτι"),
            ("ΟΣ", "ος"),
            ("K", "k"),
            ("ẞ", "ß"),
            ("µ", "Μ"),
        ] {
            assert_eq!(
                font_family_name_key(a),
                font_family_name_key(b),
                "{a} / {b}"
            );
        }
        for (a, b) in [
            ("Straße", "STRASSE"),
            ("Å", "A\u{030a}"),
            ("İ", "i"),
            ("İ", "i\u{0307}"),
            ("𐐀", "𐐨"),
        ] {
            assert_ne!(
                font_family_name_key(a),
                font_family_name_key(b),
                "{a} / {b}"
            );
        }
    }

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
    fn font_sources_accept_legacy_variable_formats_and_keep_unknown_fallbacks() {
        use super::*;
        for format in [
            "woff-variations",
            "woff2-variations",
            "truetype-variations",
            "opentype-variations",
        ] {
            assert_eq!(
                parse_font_face_sources(&format!("url(variable) format('{format}')")),
                Some(vec![CssFontSource::Url("variable".into())]),
            );
        }
        assert_eq!(
            parse_font_face_sources(
                "url(unknown) format('svg-variations'), url(empty) format(), url(fallback) format('woff2')"
            ),
            Some(vec![CssFontSource::Url("fallback".into())]),
        );
    }

    #[test]
    fn font_shorthand_distinguishes_quoted_and_escaped_generic_keywords() {
        use super::{CssFontFamily, parse_font_shorthand};
        use style::values::computed::font::GenericFontFamily;
        for keyword in ["serif", "SeRiF", r"s\65 rif"] {
            let font = parse_font_shorthand(&format!("16px {keyword}")).unwrap();
            assert_eq!(
                font.families,
                [CssFontFamily::Generic(GenericFontFamily::Serif)]
            );
        }
        for name in [r#""serif""#, r#""s\65 rif""#, "Named serif"] {
            let font = parse_font_shorthand(&format!("16px {name}")).unwrap();
            let expected = if name == "Named serif" {
                "Named serif"
            } else {
                "serif"
            };
            assert_eq!(font.families, [CssFontFamily::Named(expected.into())]);
        }
    }

    #[test]
    fn font_shorthand_uses_stylo_validation_and_keeps_all_families() {
        let font =
            super::parse_font_shorthand("italic bold 10.5px/1.2 \"A B\", Another Face, serif")
                .unwrap();
        assert_eq!(
            font.families,
            [
                super::CssFontFamily::Named("A B".into()),
                super::CssFontFamily::Named("Another Face".into()),
                super::CssFontFamily::Generic(
                    style::values::computed::font::GenericFontFamily::Serif
                ),
            ]
        );
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
        font_load_query_contains_css_wide_keyword, font_load_query_family,
        font_load_query_is_valid, normalize_font_face_src, parse_font_faces,
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
    fn font_load_query_validates_the_complete_shorthand_without_substitution() {
        for query in [
            "",
            "inherit",
            "default",
            "12px inherit",
            "12px default",
            r#""inherit""#,
            "12px",
            "serif",
            "12px serif; color: red",
            "-1px serif",
            "var(--x) serif",
            "var(--x, 10px) serif",
            "env(size) serif",
            "12px serif !important",
            "12px serif,",
            "caption garbage",
            r#""caption""#,
        ] {
            assert!(!font_load_query_is_valid(query), "{query}");
        }
        for query in [
            "12px serif",
            r#"12px "inherit""#,
            r#"12px "default""#,
            r#"italic 700 16px/1.2 "A B", serif"#,
            "calc(1em + 2px) serif",
            "caption",
            "ICON",
            "menu",
            "message-box",
            "small-caption",
            "status-bar",
            r"c\61 ption",
            "caption/**/",
            "normal normal normal normal 12px serif",
        ] {
            assert!(font_load_query_is_valid(query), "{query}");
        }
    }

    #[test]
    fn font_load_query_distinguishes_reserved_identifiers_from_quoted_families() {
        for keyword in [
            "inherit",
            "initial",
            "unset",
            "default",
            "revert",
            "revert-layer",
        ] {
            for query in [keyword.to_owned(), format!("medium {keyword}")] {
                assert!(font_load_query_contains_css_wide_keyword(&query), "{query}");
            }
            for query in [format!("12px \"{keyword}\""), format!("12px '{keyword}'")] {
                assert!(
                    !font_load_query_contains_css_wide_keyword(&query),
                    "{query}"
                );
            }
        }
        assert!(font_load_query_contains_css_wide_keyword(
            r"12px \64 efault"
        ));
        assert!(!font_load_query_contains_css_wide_keyword(
            r#"12px "\69 nherit""#
        ));
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
            Some("A B")
        );
        assert_eq!(
            font_load_query_family(r#""Standalone Family""#).as_deref(),
            Some("Standalone Family")
        );
        assert_eq!(
            super::font_load_query_families(
                r#"italic bold 16px/2 "A, B", Literal\ Family, "Literal\"Quote", serif"#
            ),
            Some(vec![
                "A, B".to_owned(),
                "Literal Family".to_owned(),
                "Literal\"Quote".to_owned(),
            ])
        );
        assert_eq!(
            super::font_load_query_families(r#"16px "sans-serif", sans-serif"#),
            Some(vec!["sans-serif".to_owned()])
        );
        assert_eq!(
            super::font_load_query_families(r#"16px "a\0 b""#),
            Some(vec!["a\u{fffd}b".to_owned()])
        );
        assert_eq!(super::font_load_query_families("caption"), None);
    }
}
