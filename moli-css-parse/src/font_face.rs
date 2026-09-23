use cssparser::{Parser, ParserInput, Token};

use crate::unquote_css_string;

pub use style::moli_font_face::{CssFontFace, normalize_font_face_src, parse_font_faces};

pub fn font_load_query_contains_css_wide_keyword(query: &str) -> bool {
    let mut input = ParserInput::new(query);
    let mut input = Parser::new(&mut input);
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
    let mut input = ParserInput::new(query);
    let mut parser = Parser::new(&mut input);
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
            Some("serif")
        );
        assert_eq!(
            font_load_query_family(r#""Standalone Family""#).as_deref(),
            Some("Standalone Family")
        );
    }
}
