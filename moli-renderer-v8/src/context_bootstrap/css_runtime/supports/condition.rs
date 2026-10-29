pub(crate) fn css_supports_condition_text(condition: &str) -> bool {
    let trimmed = condition.trim();
    if trimmed.is_empty() {
        return false;
    }
    if super::stylo_supports_condition_text(trimmed).is_some_and(|supported| supported) {
        return true;
    }
    if let Some(result) = css_supports_font_feature_condition(trimmed) {
        return result;
    }
    if let Some(result) = css_supports_at_rule_condition(trimmed) {
        return result;
    }
    false
}

pub(super) fn css_supports_font_feature_condition(condition: &str) -> Option<bool> {
    let input = condition;
    let mut parser = cssparser::Parser::new(input);
    let function = parser.expect_function().ok()?.clone();
    let function = function.as_ref();
    if !matches!(function, "font-format" | "font-tech") {
        return None;
    }
    let result = parser.parse_nested_block(|nested| {
        let ident = nested.expect_ident()?.clone();
        nested.expect_exhausted()?;
        Ok::<_, cssparser::ParseError<()>>(match function {
            "font-format" => supports_font_format_keyword(ident.as_ref()),
            "font-tech" => supports_font_tech_keyword(ident.as_ref()),
            _ => false,
        })
    });
    if parser.expect_exhausted().is_err() {
        return Some(false);
    }
    Some(result.unwrap_or(false))
}

fn supports_font_format_keyword(keyword: &str) -> bool {
    matches!(
        keyword.to_ascii_lowercase().as_str(),
        "collection" | "embedded-opentype" | "opentype" | "svg" | "truetype" | "woff" | "woff2"
    )
}

fn supports_font_tech_keyword(keyword: &str) -> bool {
    matches!(
        keyword.to_ascii_lowercase().as_str(),
        "features-opentype"
            | "features-aat"
            | "features-graphite"
            | "color-colrv0"
            | "color-colrv1"
            | "color-svg"
            | "color-sbix"
            | "color-cbdt"
            | "variations"
            | "palettes"
    )
}

pub(super) fn css_supports_at_rule_condition(condition: &str) -> Option<bool> {
    let input = condition;
    let mut parser = cssparser::Parser::new(input);
    let function = parser.expect_function().ok()?.clone();
    if function.as_ref() != "at-rule" {
        return None;
    }
    let result = parser.parse_nested_block(|nested| {
        let token = nested.next()?;
        let cssparser::Token::AtKeyword(name) = token else {
            return Ok::<_, cssparser::ParseError<()>>(false);
        };
        let name = name.to_ascii_lowercase();
        if nested.expect_exhausted().is_err() {
            return Ok(false);
        }
        Ok(supports_at_rule_keyword(&name))
    });
    if parser.expect_exhausted().is_err() {
        return Some(false);
    }
    Some(result.unwrap_or(false))
}

fn supports_at_rule_keyword(keyword: &str) -> bool {
    matches!(
        keyword,
        "container"
            | "counter-style"
            | "font-face"
            | "font-feature-values"
            | "function"
            | "import"
            | "keyframes"
            | "-webkit-keyframes"
            | "media"
            | "namespace"
            | "page"
            | "property"
            | "starting-style"
            | "supports"
            | "swash"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_supports_selector_condition_uses_selector_parser() {
        assert!(css_supports_condition_text(
            "selector(::part(mypart):hover)"
        ));
        assert!(css_supports_condition_text(
            "selector(::part(mypart):lang(en))"
        ));
        assert!(css_supports_condition_text(
            "selector(::part(mypart):dir(ltr))"
        ));
        assert!(css_supports_condition_text(
            "selector(::part(mypart):is(:hover))"
        ));
        assert!(css_supports_condition_text(
            "selector(div:is(.primary, .secondary))"
        ));
        assert!(!css_supports_condition_text(
            "selector(::part(mypart):is(:first-child))"
        ));
        assert!(!css_supports_condition_text(
            "selector(::part(mypart):where(:first-child))"
        ));
        assert!(!css_supports_condition_text("selector()"));
        assert!(!css_supports_condition_text("selector(div, div)"));
    }

    #[test]
    fn css_supports_font_feature_conditions_use_known_single_idents() {
        assert!(css_supports_condition_text("font-format(opentype)"));
        assert!(css_supports_condition_text("font-format(TrueType)"));
        assert!(css_supports_condition_text("font-tech(features-opentype)"));
        assert!(css_supports_condition_text("font-tech(color-COLRv0)"));
        assert!(css_supports_condition_text(
            "(display: block) and font-tech(features-opentype)"
        ));
        assert!(!css_supports_condition_text("font-format(xyzzy)"));
        assert!(!css_supports_condition_text(
            "font-format(opentype, truetype)"
        ));
        assert!(!css_supports_condition_text("font-format('opentype')"));
        assert!(!css_supports_condition_text("font-tech(feature-opentype)"));
        assert!(!css_supports_condition_text(
            "font-tech(features-opentype color-COLRv1)"
        ));
        assert!(!css_supports_condition_text(
            "font-tech(features-opentype, color-COLRv0)"
        ));
        assert!(!css_supports_condition_text(
            "font-tech('features-opentype')"
        ));
    }

    #[test]
    fn css_supports_at_rule_condition_uses_known_single_at_keywords() {
        assert!(css_supports_condition_text("at-rule(@supports)"));
        assert!(css_supports_condition_text("at-rule( @media )"));
        assert!(css_supports_condition_text("at-rule(@counter-style)"));
        assert!(css_supports_condition_text("at-rule(@import)"));
        assert!(css_supports_condition_text("at-rule(@swash)"));
        assert!(css_supports_condition_text("at-rule(@starting-style)"));
        assert!(css_supports_condition_text(
            "at-rule(@media) and (display: block)"
        ));
        assert!(!css_supports_condition_text("not at-rule(@media)"));
        assert!(!css_supports_condition_text("at-rule(@doesnotexist)"));
        assert!(!css_supports_condition_text("at-rule(@charset)"));
        assert!(!css_supports_condition_text(
            "at-rule(@counter-style; system: fixed)"
        ));
        assert!(!css_supports_condition_text("at-rule(supports)"));
    }

    #[test]
    fn css_supports_condition_matches_import_condition_edges() {
        assert!(css_supports_condition_text("display: block !important"));
        assert!(!css_supports_condition_text("supports(display:block)"));
        assert!(!css_supports_condition_text("foo:bar"));
        assert!(css_supports_condition_text("(--be: to be)"));
        assert!(!css_supports_condition_text("not (--be: to be)"));
        assert!(!css_supports_condition_text("future-extension(4)"));
        assert!(!css_supports_condition_text(
            "(color: blue) and future-extension(4)"
        ));
        assert!(!css_supports_condition_text("()"));
        assert!(!css_supports_condition_text("(())"));
        assert!(!css_supports_condition_text("((test))"));
        assert!(!css_supports_condition_text("(test)"));
        assert!(css_supports_condition_text("(display:block) or (foo:bar)"));
    }
}
