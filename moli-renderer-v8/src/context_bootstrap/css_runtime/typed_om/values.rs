use super::*;
use style::typed_om::{NumericValue, TypedValue};
use moli_webapi::css_values::{
    CssKeywordValueObjectDeclaration, CssUnitValueObjectDeclaration,
    css_keyword_value,
};
pub(super) use moli_webapi::css_values::{
    css_keyword_value_constructor_callback, css_unit_value_constructor_callback,
    style_value_from_text, unit_value, opaque_style_value, associated_property,
    css_unit_value_number, css_unit_value_unit, normalize_unit_name,
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
                    unit_value(scope, number, name.to_owned())
                }
                // Math expression reification and native transform/image objects
                // remain to be connected. Preserve an immutable, property-associated
                // value rather than guessing a keyword or a single numeric unit.
                TypedValue::Numeric(NumericValue::Math(_))
                | TypedValue::Transform(_)
                | TypedValue::Image(_) => opaque_style_value(scope, property, text.trim()),
            }
        })
        .collect()
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = crate::web_api_interfaces::CSSStyleValue, enumerable)]
struct CssStyleValueSerializationDeclaration {
    #[webapi(method = "toString", callback = css_style_value_to_string_callback, length = 0)]
    to_string: (),
}

pub(super) fn install_typed_value_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "CSSStyleValue" {
        let prototype = template.prototype_template(scope);
        CssStyleValueSerializationDeclaration::initialize_prototype_template(scope, prototype);
    } else {
        moli_webapi::css_values::install_typed_value_template_bindings(
            scope,
            template,
            interface_name,
        );
    }
}

fn css_style_value_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(text) = serialize(scope, args.this()).and_then(|text| v8_string(scope, &text)) {
        rv.set(text.into());
    }
}

/// Serializes genuine CSSStyleValue internal slots, never author-overridable
/// toString/value/unit properties. Callers must first validate the native brand.
pub(super) fn serialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if crate::web_api_interfaces::CSSUnparsedValue::is_instance(scope, value) {
        return super::unparsed::serialize(scope, value);
    }
    if web_api_interfaces::CSSMathValue::is_instance(scope, value) {
        return super::math::serialize(scope, value);
    }
    if let Some(unit) = css_unit_value_unit(scope, value) {
        let value = css_unit_value_number(scope, value)?;
        let number = moli_css_parse::serialize_css_number(value);
        return Some(if !value.is_finite() {
            if unit == "number" {
                format!("calc({number})")
            } else {
                format!(
                    "calc({number} * 1{})",
                    if unit == "percent" { "%" } else { &unit }
                )
            }
        } else {
            match unit.as_str() {
                "number" => number,
                "percent" => format!("{number}%"),
                _ => format!("{number}{unit}"),
            }
        });
    }
    if let Some(keyword) = css_keyword_value(scope, value) {
        let mut text = String::new();
        cssparser::serialize_identifier(&keyword, &mut text).ok()?;
        return Some(text);
    }
    moli_webapi::css_values::style_value_text(scope, value)
}
