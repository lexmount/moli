use super::*;
use style::typed_om::{NumericValue, TypedValue};

const INLINE_MAP_SLOT: &str = "__moliInlineStylePropertyMap";
const RULE_MAP_SLOT: &str = "__moliRuleStylePropertyMap";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::StylePropertyMap)]
struct StylePropertyMapDeclaration<'s> {
    #[webapi(slot = STYLE_PROPERTY_MAP_STYLE_SLOT)]
    style: v8::Local<'s, v8::Object>,
}

macro_rules! inline_map_binding {
    ($declaration:ident, $interface:ident) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = web_api_interfaces::$interface, enumerable, receiver)]
        struct $declaration {
            #[webapi(accessor_property = "attributeStyleMap", getter = inline_map_getter)]
            attribute_style_map: (),
        }
    };
}
inline_map_binding!(HtmlInlineMapDeclaration, HTMLElement);
inline_map_binding!(SvgInlineMapDeclaration, SVGElement);
inline_map_binding!(MathInlineMapDeclaration, MathMLElement);

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSStyleRule, enumerable, receiver)]
struct RuleMapDeclaration {
    #[webapi(accessor_property = "styleMap", getter = rule_map_getter)]
    style_map: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::StylePropertyMap, enumerable, receiver)]
struct MapPrototypeDeclaration {
    #[webapi(method, callback = set_callback, length = 1)]
    set: (),
    #[webapi(method, callback = append_callback, length = 1)]
    append: (),
    #[webapi(method, callback = delete_callback, length = 1)]
    delete: (),
    #[webapi(method, callback = clear_callback, length = 0)]
    clear: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "HTMLElement" => HtmlInlineMapDeclaration::initialize_prototype_template(scope, prototype),
        "SVGElement" => SvgInlineMapDeclaration::initialize_prototype_template(scope, prototype),
        "MathMLElement" => {
            MathInlineMapDeclaration::initialize_prototype_template(scope, prototype)
        }
        "CSSStyleRule" => RuleMapDeclaration::initialize_prototype_template(scope, prototype),
        "StylePropertyMap" => {
            MapPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

fn inline_map_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(map) = get_private_object(scope, args.this(), INLINE_MAP_SLOT) {
        rv.set(map.into());
        return;
    }
    let context = args
        .this()
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(style) = native_bridge::element::style_for_element(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(cache_map(scope, args.this(), style, INLINE_MAP_SLOT).into());
}

fn rule_map_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(map) = get_private_object(scope, args.this(), RULE_MAP_SLOT) {
        rv.set(map.into());
        return;
    }
    let context = args
        .this()
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let style = crate::context_bootstrap::css_stylesheet_runtime::css_style_rule_style_object(
        scope,
        args.this(),
    );
    rv.set(cache_map(scope, args.this(), style, RULE_MAP_SLOT).into());
}

fn cache_map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    style: v8::Local<'s, v8::Object>,
    slot: &str,
) -> v8::Local<'s, v8::Object> {
    let map = StylePropertyMapDeclaration::new(style)
        .bind(scope)
        .expect("native StylePropertyMap should bind");
    if slot == INLINE_MAP_SLOT {
        set_private_value(scope, map, STYLE_PROPERTY_MAP_ELEMENT_SLOT, owner.into());
    }
    set_private_value(scope, owner, slot, map.into());
    map
}

enum StyleValueOrString<'s> {
    Value(v8::Local<'s, v8::Object>),
    String(String),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "StylePropertyMap")]
struct SetArgs<'s> {
    #[webidl(required, converter = "usv_string")]
    property: String,
    #[webidl(with = values_arg)]
    values: Vec<StyleValueOrString<'s>>,
}

fn values_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Vec<StyleValueOrString<'s>>, webidl::WebIdlError> {
    (index..args.length())
        .map(|index| {
            let value = args.get(index);
            if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
                && web_api_interfaces::CSSStyleValue::is_instance(scope, object)
            {
                return Ok(StyleValueOrString::Value(object));
            }
            // WebIDL's string arm applies to objects which do not implement the
            // interface, including author Proxies. Preserve conversion exceptions.
            webidl::convert::<webidl::UsvString>(
                scope,
                value,
                webidl::Context::argument("StylePropertyMap", (index + 1) as usize),
            )
            .map(|value| StyleValueOrString::String(value.0))
        })
        .collect()
}

fn set_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    mutate(scope, &args, false);
}
fn append_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    mutate(scope, &args, true);
}
fn delete_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((style, property)) = map_style_and_property(scope, &args) else {
        return;
    };
    detached_css_style::set_css_declaration_property(scope, style, &property, "");
}
fn clear_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(style) = map_style_object(scope, args.this()) {
        detached_css_style::clear_css_declaration(scope, style);
    }
}

fn mutate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    append: bool,
) {
    let Some(arguments) = webidl::parse_args::<SetArgs>(scope, args) else {
        return;
    };
    let Some(property) = canonical_map_property_name(&arguments.property) else {
        throw_type_error(scope, "Invalid CSS property");
        return;
    };
    let Some(style) = map_style_object(scope, args.this()) else {
        return;
    };
    if append && arguments.values.is_empty() {
        return;
    }
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let base_url = parse::base_url(scope);
    let result = coerce_values(
        scope,
        &property,
        arguments.values,
        base_url.as_ref(),
        append,
    )
    .and_then(|coerced| {
        if !append {
            return Ok(coerced);
        }
        let Some(previous) = style_property_text(scope, style, &property) else {
            return Ok(coerced);
        };
        let previous =
            moli_css_parse::parse_typed_style_value(&property, &previous, base_url.as_ref())
                .ok_or(CoercionError::Invalid)?;
        if is_unparsed(&previous) {
            return Err(CoercionError::Invalid);
        }
        let joined = format!("{}, {}", previous.css_text, coerced.text);
        moli_css_parse::parse_typed_style_value(&property, &joined, base_url.as_ref())
            .map(|_| CoercedValues {
                text: joined,
                unit: None,
            })
            .ok_or(CoercionError::Invalid)
    });
    match result {
        Ok(coerced) => detached_css_style::set_typed_css_declaration_property(
            scope,
            style,
            &property,
            &coerced.text,
            coerced.unit,
        ),
        Err(CoercionError::Invalid) => throw_type_error(scope, "Invalid CSS value for property"),
        Err(CoercionError::UnparsedStorage) => webidl::throw_dom_exception(
            scope,
            "NotSupportedError",
            "Storing this unparsed declaration is not supported yet",
        ),
    }
}

enum CoercionError {
    Invalid,
    UnparsedStorage,
}

struct CoercedValues {
    text: String,
    unit: Option<moli_css_parse::CssDeclaredUnitValue>,
}

fn is_unparsed(parsed: &moli_css_parse::ParsedTypedStyleValue) -> bool {
    parsed.values.as_ref().is_some_and(|list| {
        list.values
            .iter()
            .any(|v| matches!(v, TypedValue::Unparsed(_)))
    })
}

fn coerce_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    property: &str,
    inputs: Vec<StyleValueOrString<'s>>,
    base_url: Option<&url::Url>,
    append: bool,
) -> Result<CoercedValues, CoercionError> {
    if inputs.is_empty() {
        return Err(CoercionError::Invalid);
    }
    let multiple = inputs.len() > 1;
    let shorthand = native_bridge::element::computed_style_property_is_shorthand(property);
    let mut texts = Vec::with_capacity(inputs.len());
    let mut retained_unit = None;
    for input in inputs {
        let (text, object) = match input {
            StyleValueOrString::String(text) => (text, None),
            StyleValueOrString::Value(object) => {
                let text = if let Some(unit) = values::css_unit_value_unit(scope, object) {
                    let number = values::css_unit_value_number(scope, object)
                        .ok_or(CoercionError::Invalid)?;
                    let suffix = match unit.as_str() {
                        "number" => "",
                        "percent" => "%",
                        other => other,
                    };
                    // CSSStyleValue's stringifier rounds for display. A native
                    // declaration write must receive the actual double.
                    format!("{number}{suffix}")
                } else {
                    values::serialize(scope, object).ok_or(CoercionError::Invalid)?
                };
                (text, Some(object))
            }
        };
        let mut parsed = moli_css_parse::parse_typed_style_value(property, &text, base_url);
        if let Some(object) = object {
            if web_api_interfaces::CSSImageValue::is_instance(scope, object)
                && !parsed
                    .as_ref()
                    .and_then(|v| v.values.as_ref())
                    .is_some_and(|v| {
                        v.values.len() == 1 && matches!(&v.values[0], TypedValue::Image(_))
                    })
            {
                return Err(CoercionError::Invalid);
            }
            if values::associated_property(scope, object).is_some_and(|name| name != property) {
                return Err(CoercionError::Invalid);
            }
            if web_api_interfaces::CSSUnparsedValue::is_instance(scope, object) {
                if multiple || append {
                    return Err(CoercionError::Invalid);
                }
                if moli_css_parse::reify_unparsed_style_value(&text, None).is_none() {
                    return Err(CoercionError::Invalid);
                }
                // The pinned Stylo API cannot construct arbitrary WithVariables
                // declarations. Do not reparse a typed unparsed value as a unit,
                // drop an empty declaration, or silently accept an invalid write.
                if !parsed.as_ref().is_some_and(is_unparsed)
                    || parsed
                        .as_ref()
                        .is_some_and(|parsed| is_css_wide_keyword(&parsed.css_text))
                {
                    return Err(CoercionError::UnparsedStorage);
                }
            } else if property.starts_with("--")
                || (shorthand && web_api_interfaces::CSSMathValue::is_instance(scope, object))
            {
                return Err(CoercionError::Invalid);
            } else if let Some(unit) = values::css_unit_value_unit(scope, object) {
                if shorthand {
                    return Err(CoercionError::Invalid);
                }
                let suffix = match unit.as_str() {
                    "number" => "",
                    "percent" => "%",
                    other => other,
                };
                // A typed number 0 must not turn into a length through CSS's
                // unitless-zero compatibility grammar. Ask the parser for the
                // numeric type with a non-zero representative of the same unit.
                let probe = moli_css_parse::parse_typed_style_value(
                    property,
                    &format!("1{suffix}"),
                    base_url,
                )
                .and_then(|v| v.values)
                .ok_or(CoercionError::Invalid)?;
                if probe.values.len() != 1
                    || !matches!(&probe.values[0], TypedValue::Numeric(NumericValue::Unit(value))
                        if values::native_unit_name(value.unit_str()) == unit
                            || unit == "percent" && value.unit_str() == "number")
                {
                    return Err(CoercionError::Invalid);
                }
                // calc() defers numeric range restrictions (negative lengths,
                // fractional integers) to computed-value time.
                if parsed.is_none() {
                    parsed = moli_css_parse::parse_typed_style_value(
                        property,
                        &format!("calc({text})"),
                        base_url,
                    );
                } else if !multiple {
                    retained_unit = Some(moli_css_parse::CssDeclaredUnitValue {
                        value: values::css_unit_value_number(scope, object)
                            .ok_or(CoercionError::Invalid)?,
                        unit,
                    });
                }
            } else if web_api_interfaces::CSSKeywordValue::is_instance(scope, object)
                && !is_css_wide_keyword(&text)
                && !parsed
                    .as_ref()
                    .and_then(|v| v.values.as_ref())
                    .is_some_and(|v| {
                        v.values.len() == 1 && matches!(&v.values[0], TypedValue::Keyword(_))
                    })
            {
                return Err(CoercionError::Invalid);
            }
        }
        let parsed = parsed.ok_or(CoercionError::Invalid)?;
        if (multiple || append) && is_unparsed(&parsed) {
            return Err(CoercionError::Invalid);
        }
        texts.push(if retained_unit.is_some() {
            text
        } else {
            parsed.css_text
        });
    }
    let text = texts.join(", ");
    let parsed = moli_css_parse::parse_typed_style_value(property, &text, base_url)
        .ok_or(CoercionError::Invalid)?;
    if multiple || append {
        if shorthand || property.starts_with("--") {
            return Err(CoercionError::Invalid);
        }
        // Subdivision is owned by Stylo's typed projection; commas inside an
        // opaque single value do not make it an appendable list.
        let doubled = format!("{text}, {text}");
        let repeated = moli_css_parse::parse_typed_style_value(property, &doubled, base_url)
            .and_then(|v| v.values)
            .is_some_and(|v| v.values.len() >= 2);
        if !repeated {
            return Err(CoercionError::Invalid);
        }
    }
    Ok(CoercedValues {
        text: if retained_unit.is_some() {
            text
        } else {
            parsed.css_text
        },
        unit: retained_unit,
    })
}

fn is_css_wide_keyword(text: &str) -> bool {
    matches!(
        text.trim().to_ascii_lowercase().as_str(),
        "initial" | "inherit" | "unset" | "revert" | "revert-layer"
    )
}
