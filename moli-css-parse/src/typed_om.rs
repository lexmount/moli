//! Typed OM projection of Stylo-owned property grammar and value types.

use cssparser::{Parser, ParserInput};
use style::{
    context::QuirksMode,
    custom_properties::SpecifiedValue,
    properties::{
        CSSWideKeyword, Importance, PropertyDeclaration, PropertyDeclarationBlock, PropertyId,
        ShorthandId, SourcePropertyDeclaration, parse_one_declaration_into,
    },
    stylesheets::{CssRuleType, CssRuleTypes, Origin, UrlExtraData},
    typed_om::{ToTyped, TypedValue, TypedValueList, UnparsedValue},
};
use style_traits::ParsingMode;

mod images;
mod tokens;

pub use images::{TypedImageValue, computed_typed_style_value_list};

/// Stylo's general projection plus image ASTs which its pinned ToTyped
/// implementation does not yet cover (gradients, image-set and mixed lists).
pub enum TypedStyleValueList {
    Native(TypedValueList),
    Images(Vec<TypedImageValue>),
}

impl TypedStyleValueList {
    pub fn native(&self) -> Option<&TypedValueList> {
        match self {
            Self::Native(values) => Some(values),
            Self::Images(_) => None,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Native(values) => values.values.len(),
            Self::Images(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_single_image(&self) -> bool {
        match self {
            Self::Native(values) => matches!(values.values.as_slice(), [TypedValue::Image(_)]),
            Self::Images(values) => matches!(values.as_slice(), [TypedImageValue::Image(_)]),
        }
    }
}

pub struct ParsedTypedStyleValue {
    /// CSSOM serialization of the validated declaration (including shorthands).
    pub css_text: String,
    /// None denotes a property-associated value without a specialized projection.
    pub values: Option<TypedStyleValueList>,
}

/// Parse exactly one value, never a declaration list or an !important suffix.
/// Interface exposure and Chromium compatibility property aliases belong to the
/// caller; the pinned Stylo parser owns the accepted property grammar.
pub fn parse_typed_style_value(
    property: &str,
    css_text: &str,
    base_url: Option<&url::Url>,
) -> Option<ParsedTypedStyleValue> {
    let id = PropertyId::parse_enabled_for_all_content(property).ok()?;
    if let PropertyId::NonCustom(id) = &id
        && !id.allowed_in_rule(CssRuleTypes::from(CssRuleType::Style))
    {
        return None;
    }
    // Empty or comment-only text cannot produce a CSSStyleValue for parse().
    let mut input = ParserInput::new(css_text);
    Parser::new(&mut input).next().ok()?;
    let base_url = base_url
        .cloned()
        .unwrap_or_else(|| url::Url::parse("about:blank").expect("static URL"));
    let url_data = UrlExtraData::from(base_url);
    let mut declarations = SourcePropertyDeclaration::default();
    parse_one_declaration_into(
        &mut declarations,
        id.clone(),
        css_text,
        Origin::Author,
        &url_data,
        None,
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        CssRuleType::Style,
    )
    .ok()?;
    let mut block = PropertyDeclarationBlock::new();
    block.extend(declarations.drain(), Importance::Normal);
    let mut serialized = String::new();
    block.property_value_to_css(&id, &mut serialized).ok()?;
    let unparsed = property.starts_with("--")
        || block
            .declaration_importance_iter()
            .any(|(value, _)| matches!(value, PropertyDeclaration::WithVariables(_)));
    let values = if unparsed {
        Some(TypedValueList {
            values: [TypedValue::Unparsed(reify_unparsed_style_value(
                css_text,
                Some(&url_data),
            )?)]
            .into_iter()
            .collect(),
        })
    } else if matches!(id.as_shorthand(), Ok(ShorthandId::All))
        && let Some(keyword) = css_wide_keyword(&block)
    {
        // Typed OM explicitly reifies `all` as an identifier. Other
        // shorthands keep their property-specific projection. Read the
        // parsed keyword instead of guessing from serialized longhands.
        keyword.to_typed_value_list()
    } else {
        block.property_value_to_typed_value_list(&id).ok()?
    };
    let values = values.map(TypedStyleValueList::Native).or_else(|| {
        let (declaration, _) = block.get(id.as_shorthand().err()?)?;
        images::from_declaration(declaration).map(TypedStyleValueList::Images)
    });
    Some(ParsedTypedStyleValue {
        css_text: serialized,
        values,
    })
}

fn css_wide_keyword(block: &PropertyDeclarationBlock) -> Option<CSSWideKeyword> {
    let mut declarations = block.declaration_importance_iter();
    let keyword = declarations.next()?.0.get_css_wide_keyword()?;
    declarations
        .all(|(declaration, _)| declaration.get_css_wide_keyword() == Some(keyword))
        .then_some(keyword)
}

/// Preserve var() references in raw tokens, including inside other functions.
/// Stylo validates and recovers the token stream first. The token projection is
/// iterative and leaves env()/attr() as raw syntax instead of treating them as
/// CSSVariableReferenceValue objects (whose names must begin with --).
pub fn reify_unparsed_style_value(
    css_text: &str,
    url_data: Option<&UrlExtraData>,
) -> Option<UnparsedValue> {
    let fallback_url;
    let url_data = match url_data {
        Some(value) => value,
        None => {
            fallback_url = UrlExtraData::from(url::Url::parse("about:blank").ok()?);
            &fallback_url
        }
    };
    let mut input = ParserInput::new(css_text);
    let value = Parser::new(&mut input)
        .parse_entirely(|input| SpecifiedValue::parse(input, None, url_data))
        .ok()?;
    tokens::reify(value.css_text())
}

#[cfg(test)]
mod tests {
    use super::*;
    use style::typed_om::{NumericValue, UnparsedSegment};

    fn native_values(parsed: ParsedTypedStyleValue) -> TypedValueList {
        match parsed.values.expect("typed values") {
            TypedStyleValueList::Native(values) => values,
            TypedStyleValueList::Images(_) => panic!("unexpected image projection"),
        }
    }

    #[test]
    fn all_reifies_keywords_without_retyping_ordinary_shorthands() {
        for property in ["all", "width", "color", "animation-name"] {
            for keyword in ["initial", "inherit", "unset", "revert", "revert-layer"] {
                let source = format!("/**/{} /**/", keyword.to_ascii_uppercase());
                let parsed = parse_typed_style_value(property, &source, None)
                    .unwrap_or_else(|| panic!("{property}: {source}"));
                let values = native_values(parsed);
                assert_eq!(values.values.len(), 1, "{property}: {keyword}");
                assert!(
                    matches!(&values.values[0], TypedValue::Keyword(value) if value.0 == keyword),
                    "{property}: {keyword}"
                );
            }
        }
        let parsed = parse_typed_style_value("all", r"\69 nherit", None).unwrap();
        assert!(matches!(
            &native_values(parsed).values[0], TypedValue::Keyword(value) if value.0 == "inherit"
        ));
        for (property, source) in [
            ("margin", "initial 1px"),
            ("margin", "initial, inherit"),
            ("all", "auto"),
            ("unknown-property", "initial"),
        ] {
            assert!(parse_typed_style_value(property, source, None).is_none());
        }
        for (property, source) in [
            ("margin", "initial"),
            ("background", "inherit"),
            ("border", "revert-layer"),
            ("margin", "auto"),
            ("background", "none"),
            ("font", "medium serif"),
        ] {
            assert!(
                parse_typed_style_value(property, source, None)
                    .unwrap()
                    .values
                    .is_none()
            );
        }
        for (property, source) in [
            ("all", "var(--x, initial)"),
            ("margin", "var(--x, initial)"),
            ("--x", "initial"),
        ] {
            let parsed = parse_typed_style_value(property, source, None).unwrap();
            assert!(matches!(
                &native_values(parsed).values[0],
                TypedValue::Unparsed(_)
            ));
        }
    }

    #[test]
    fn typed_property_parsing_uses_grammar_and_property_types() {
        for (name, value) in [
            ("width", "10deg"),
            ("margin", "10deg"),
            ("display", "block; color: red"),
            ("width", "1px !important"),
            ("--x", "one; two"),
            ("--x", "!important"),
            ("--x", ""),
            ("--x", "/**/"),
        ] {
            assert!(
                parse_typed_style_value(name, value, None).is_none(),
                "{name}: {value}"
            );
        }
        let parsed = parse_typed_style_value("width", "0", None).unwrap();
        let TypedValue::Numeric(NumericValue::Unit(value)) = &native_values(parsed).values[0]
        else {
            panic!("length zero must remain a dimension");
        };
        assert_eq!(value.unit_str(), "px");
        assert_eq!(value.value, 0.0);
        assert_eq!(
            parse_typed_style_value("transition-duration", "1s, 2s", None)
                .unwrap()
                .values
                .unwrap()
                .len(),
            2
        );
        assert!(
            parse_typed_style_value("margin", "1px", None)
                .unwrap()
                .values
                .is_none()
        );
        assert!(
            parse_typed_style_value("color", "red", None)
                .unwrap()
                .values
                .is_none()
        );
    }

    #[test]
    fn unparsed_projection_retains_empty_nested_and_non_var_fallbacks() {
        let parsed =
            parse_typed_style_value("margin", "calc(1px + var(--x, env(foo, var(--y,))))", None)
                .unwrap();
        let TypedValue::Unparsed(parts) = &native_values(parsed).values[0] else {
            panic!("unparsed shorthand");
        };
        assert!(matches!(&parts[0], UnparsedSegment::String(text) if text == "calc(1px + "));
        let UnparsedSegment::VariableReference(reference) = &parts[1] else {
            panic!("outer reference");
        };
        assert_eq!(reference.variable, "--x");
        assert!(reference.has_fallback);
        assert!(
            matches!(&reference.fallback[0], UnparsedSegment::String(text) if text == " env(foo, ")
        );
        let UnparsedSegment::VariableReference(inner) = &reference.fallback[1] else {
            panic!("inner reference");
        };
        assert!(inner.has_fallback);
        assert!(inner.fallback.is_empty());
        assert!(matches!(&reference.fallback[2], UnparsedSegment::String(text) if text == ")"));
        assert!(matches!(&parts[2], UnparsedSegment::String(text) if text == ")"));
        let eof = reify_unparsed_style_value("var(--eof", None).unwrap();
        assert!(
            matches!(&eof[0], UnparsedSegment::VariableReference(value) if value.variable == "--eof" && !value.has_fallback)
        );
    }
}
