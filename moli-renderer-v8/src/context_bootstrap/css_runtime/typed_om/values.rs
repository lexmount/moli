use super::*;
use crate::web_api_interfaces;
use style::typed_om::{NumericValue, TypedValue};

mod number;

const CSS_STYLE_VALUE_TEXT_SLOT: &str = "__moliCssStyleValueText";
const CSS_STYLE_VALUE_PROPERTY_SLOT: &str = "__moliCssStyleValueProperty";
const CSS_KEYWORD_VALUE_VALUE_SLOT: &str = "__moliCssKeywordValueValue";
const CSS_UNIT_VALUE_VALUE_SLOT: &str = "__moliCssUnitValueValue";
const CSS_UNIT_VALUE_UNIT_SLOT: &str = "__moliCssUnitValueUnit";

const VALID_UNIT_NAMES: &[&str] = &[
    "number", "percent", "em", "ex", "ch", "ic", "rem", "lh", "rlh", "vw", "vh", "vi", "vb",
    "vmin", "vmax", "cm", "mm", "q", "in", "pt", "pc", "px", "deg", "grad", "rad", "turn", "s",
    "ms", "hz", "khz", "dpi", "dpcm", "dppx", "fr",
];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSStyleValue)]
struct CssStyleValueObjectDeclaration {
    #[webapi(slot = CSS_STYLE_VALUE_TEXT_SLOT)]
    text: String,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSKeywordValue)]
struct CssKeywordValueObjectDeclaration {
    #[webapi(slot = CSS_KEYWORD_VALUE_VALUE_SLOT)]
    value: String,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSUnitValue)]
struct CssUnitValueObjectDeclaration {
    #[webapi(slot = CSS_UNIT_VALUE_VALUE_SLOT)]
    value: f64,
    #[webapi(slot = CSS_UNIT_VALUE_UNIT_SLOT)]
    unit: String,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSKeywordValue, enumerable)]
struct CssKeywordValuePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = css_keyword_value_value_getter_callback,
        setter = css_keyword_value_value_setter_callback
    )]
    value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSStyleValue, enumerable)]
struct CssStyleValuePrototypeDeclaration {
    #[webapi(method = "toString", callback = css_style_value_to_string_callback, length = 0)]
    to_string: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSUnitValue, enumerable)]
struct CssUnitValuePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = css_unit_value_value_getter_callback,
        setter = css_unit_value_value_setter_callback
    )]
    value: (),
    #[webapi(accessor_property, getter = css_unit_value_unit_getter_callback)]
    unit: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSUnitValue")]
struct CssUnitValueConstructorArgs {
    #[webidl(required)]
    value: f64,
    #[webidl(required)]
    unit: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSKeywordValue")]
struct CssKeywordValueConstructorArgs {
    #[webidl(required)]
    keyword: String,
}

pub(super) fn install_typed_value_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "CSSStyleValue" => {
            CssStyleValuePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSKeywordValue" => {
            CssKeywordValuePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSUnitValue" => {
            CssUnitValuePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

pub(in crate::context_bootstrap) fn css_keyword_value_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'CSSKeywordValue': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<CssKeywordValueConstructorArgs>(scope, &args) else {
        return;
    };
    if parsed.keyword.is_empty() {
        throw_type_error(scope, "CSSKeywordValue does not support empty strings");
        return;
    }
    CssKeywordValueObjectDeclaration::new(parsed.keyword)
        .initialize(scope, args.this())
        .expect("CSSKeywordValue declaration should initialize");
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn css_unit_value_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'CSSUnitValue': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<CssUnitValueConstructorArgs>(scope, &args) else {
        return;
    };
    let Some(unit) = normalize_unit_name(&parsed.unit) else {
        throw_type_error(scope, &format!("Invalid unit: {}", parsed.unit));
        return;
    };
    CssUnitValueObjectDeclaration::new(parsed.value, unit)
        .initialize(scope, args.this())
        .expect("CSSUnitValue declaration should initialize");
    rv.set(args.this().into());
}

pub(super) fn style_value_from_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    text: &str,
    allow_keyword: bool,
) -> v8::Local<'s, v8::Object> {
    if let Some((value, unit)) = parse_single_unit_value(text) {
        return CssUnitValueObjectDeclaration::new(value, unit)
            .bind(scope)
            .expect("CSSUnitValue declaration should bind");
    }
    if allow_keyword && let Some(keyword) = parse_single_keyword_value(text) {
        return CssKeywordValueObjectDeclaration::new(keyword)
            .bind(scope)
            .expect("CSSKeywordValue declaration should bind");
    }
    CssStyleValueObjectDeclaration::new(text.to_owned())
        .bind(scope)
        .expect("CSSStyleValue declaration should bind")
}

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

fn opaque_style_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    property: &str,
    text: &str,
) -> v8::Local<'s, v8::Object> {
    let object = CssStyleValueObjectDeclaration::new(text.to_owned())
        .bind(scope)
        .expect("CSSStyleValue declaration should bind");
    let property = v8_string(scope, property).expect("CSS property name");
    set_private_value(
        scope,
        object,
        CSS_STYLE_VALUE_PROPERTY_SLOT,
        property.into(),
    );
    object
}

fn parse_single_keyword_value(text: &str) -> Option<String> {
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_entirely(|input| match input.next()?.clone() {
            Token::Ident(keyword) => Ok(keyword.to_string()),
            _ => Err(input.new_custom_error::<(), ()>(())),
        })
        .ok()
}

fn parse_single_unit_value(text: &str) -> Option<(f64, String)> {
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_entirely(|input| {
            let token = input.next()?.clone();
            match token {
                Token::Number { value, .. } => Ok((f64::from(value), "number".to_owned())),
                Token::Percentage { unit_value, .. } => {
                    Ok((f64::from(unit_value) * 100.0, "percent".to_owned()))
                }
                Token::Dimension { value, unit, .. } => {
                    let unit = unit.to_ascii_lowercase();
                    if normalize_unit_name(&unit).is_none() {
                        return Err(input.new_custom_error::<(), ()>(()));
                    }
                    Ok((f64::from(value), unit))
                }
                _ => Err(input.new_custom_error::<(), ()>(())),
            }
        })
        .ok()
}

fn normalize_unit_name(unit: &str) -> Option<String> {
    let unit = unit.to_ascii_lowercase();
    VALID_UNIT_NAMES.contains(&unit.as_str()).then_some(unit)
}

fn css_style_value_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !css_style_value_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if web_api_interfaces::CSSUnparsedValue::is_instance(scope, args.this()) {
        if let Some(text) =
            super::unparsed::serialize(scope, args.this()).and_then(|text| v8_string(scope, &text))
        {
            rv.set(text.into());
        }
        return;
    }
    if let Some(unit) = css_unit_value_unit(scope, args.this()) {
        let value = css_unit_value_number(scope, args.this()).unwrap_or(0.0);
        let number = v8::Number::new(scope, value)
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_else(|| value.to_string());
        let text = match unit.as_str() {
            "number" => number,
            "percent" => format!("{number}%"),
            _ => format!("{number}{unit}"),
        };
        if let Some(text) = v8_string(scope, &text) {
            rv.set(text.into());
        }
        return;
    }
    if let Some(keyword) = css_keyword_value(scope, args.this()) {
        if let Some(keyword) = v8_string(scope, &keyword) {
            rv.set(keyword.into());
        }
        return;
    }
    let text = get_private_value(scope, args.this(), CSS_STYLE_VALUE_TEXT_SLOT)
        .and_then(|value| value.to_string(scope))
        .unwrap_or_else(|| v8::String::empty(scope));
    rv.set(text.into());
}

fn css_keyword_value_value_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(keyword) = css_keyword_value(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    if let Some(keyword) = v8_string(scope, &keyword) {
        rv.set(keyword.into());
    }
}

fn css_keyword_value_value_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if css_keyword_value(scope, args.this()).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let keyword = match webidl::convert::<webidl::DomString>(
        scope,
        args.get(0),
        webidl::Context::member("CSSKeywordValue", "value"),
    ) {
        Ok(keyword) => keyword.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if keyword.is_empty() {
        throw_type_error(scope, "CSSKeywordValue does not support empty strings");
        return;
    }
    if let Some(keyword) = v8_string(scope, &keyword) {
        set_private_value(
            scope,
            args.this(),
            CSS_KEYWORD_VALUE_VALUE_SLOT,
            keyword.into(),
        );
    }
    rv.set_undefined();
}

fn css_unit_value_value_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = css_unit_value_number(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(v8::Number::new(scope, value).into());
}

fn css_unit_value_value_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if css_unit_value_unit(scope, args.this()).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let value = match webidl::convert::<webidl::Double>(
        scope,
        args.get(0),
        webidl::Context::member("CSSUnitValue", "value"),
    ) {
        Ok(value) => f64::from(value),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    set_private_value(
        scope,
        args.this(),
        CSS_UNIT_VALUE_VALUE_SLOT,
        v8::Number::new(scope, value).into(),
    );
    rv.set_undefined();
}

fn css_unit_value_unit_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(unit) = css_unit_value_unit(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    if let Some(unit) = v8_string(scope, &unit) {
        rv.set(unit.into());
    }
}

fn css_style_value_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::CSSStyleValue::is_instance(scope, receiver)
}

fn css_unit_value_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<f64> {
    get_private_value(scope, receiver, CSS_UNIT_VALUE_VALUE_SLOT)
        .and_then(|value| value.number_value(scope))
}

fn css_keyword_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<String> {
    get_private_value(scope, receiver, CSS_KEYWORD_VALUE_VALUE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn css_unit_value_unit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<String> {
    get_private_value(scope, receiver, CSS_UNIT_VALUE_UNIT_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}
