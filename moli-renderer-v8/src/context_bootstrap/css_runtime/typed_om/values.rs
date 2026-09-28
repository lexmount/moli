use super::*;
use style::typed_om::{NumericValue, TypedValue};
use moli_webapi::css_values::{
    CssKeywordValueObjectDeclaration, CssUnitValueObjectDeclaration, opaque_style_value,
};
pub(super) use moli_webapi::css_values::{
    css_keyword_value_constructor_callback, css_unit_value_constructor_callback,
    install_typed_value_template_bindings, style_value_from_text,
};

mod number;

pub(super) fn from_parsed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    property: &str,
    source: &str,
    parsed: moli_css_parse::ParsedTypedStyleValue,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(typed) = parsed.values.filter(|list| !list.values.is_empty()) else {
        return vec![opaque_style_value(scope, property, &parsed.css_text)];
    };
    // Stylo determines whether this property is list-valued. Only subdivide
    // its serialization when that projection has multiple items; a shorthand
    // or unsupported property can contain commas without becoming a list.
    let texts = if typed.values.len() == 1 {
        vec![parsed.css_text.clone()]
    } else {
        top_level_comma_separated_component_values(&parsed.css_text).unwrap_or_default()
    };
    if texts.len() != typed.values.len() {
        return vec![opaque_style_value(scope, property, &parsed.css_text)];
    }
    let sources = if typed.values.len() == 1 {
        vec![source.to_owned()]
    } else {
        top_level_comma_separated_component_values(source).unwrap_or_default()
    };
    typed
        .values
        .into_iter()
        .zip(texts)
        .enumerate()
        .map(|(index, (value, text))| {
            match value {
                TypedValue::Unparsed(parts) => super::unparsed::from_native(scope, parts),
                TypedValue::Keyword(keyword) => CssKeywordValueObjectDeclaration::new(keyword.0)
                    .bind(scope)
                    .expect("CSSKeywordValue declaration should bind"),
                TypedValue::Numeric(NumericValue::Unit(unit)) => {
                    let name = if unit.unit_str() == "%" {
                        "percent"
                    } else {
                        unit.unit_str()
                    };
                    let number = sources
                        .get(index)
                        .and_then(|text| number::from_literal(text, unit.unit_str()))
                        .unwrap_or_else(|| f64::from(unit.value));
                    CssUnitValueObjectDeclaration::new(number, name.to_owned())
                        .bind(scope)
                        .expect("CSSUnitValue declaration should bind")
                }
                // CSSMath*, transforms and images still need their own native DOM
                // interfaces. Preserve an immutable, property-associated CSS value
                // in the meantime, rather than guessing a keyword or numeric type.
                TypedValue::Numeric(NumericValue::Math(_))
                | TypedValue::Transform(_)
                | TypedValue::Image(_) => opaque_style_value(scope, property, text.trim()),
            }
        })
        .collect()
}

