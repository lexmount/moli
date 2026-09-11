use super::*;
use crate::web_api_interfaces;

const CSS_STYLE_VALUE_TEXT_SLOT: &str = "__moliCssStyleValueText";
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
    moli_webapi_declare::implements_interface(scope, receiver, "CSSStyleValue")
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
