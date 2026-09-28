use super::*;
use crate::web_api_interfaces;

mod indexed;
mod serialization;

pub(super) use serialization::serialize;

const SEGMENTS_SLOT: &str = "__moliCssUnparsedSegments";
const VARIABLE_SLOT: &str = "__moliCssVariableReferenceName";
const FALLBACK_SLOT: &str = "__moliCssVariableReferenceFallback";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSUnparsedValue)]
struct UnparsedValueDeclaration<'s> {
    #[webapi(slot = SEGMENTS_SLOT)]
    segments: v8::Local<'s, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSVariableReferenceValue)]
struct VariableReferenceDeclaration<'s> {
    #[webapi(slot = VARIABLE_SLOT)]
    variable: String,
    #[webapi(slot = FALLBACK_SLOT)]
    fallback: v8::Local<'s, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSUnparsedValue, enumerable, receiver)]
struct UnparsedValuePrototypeDeclaration {
    #[webapi(accessor_property, getter = length_getter)]
    length: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoEntries)]
    entries: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoKeys)]
    keys: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoValues)]
    values: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoForEach)]
    for_each: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoValues, symbol = "iterator")]
    iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSVariableReferenceValue, enumerable, receiver)]
struct VariableReferencePrototypeDeclaration {
    #[webapi(accessor_property, getter = variable_getter, setter = variable_setter)]
    variable: (),
    #[webapi(accessor_property, getter = fallback_getter)]
    fallback: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSUnparsedValue")]
struct UnparsedValueArgs<'s> {
    #[webidl(required, with = segments_arg)]
    segments: Vec<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSVariableReferenceValue")]
struct VariableReferenceArgs<'s> {
    #[webidl(required, converter = "usv_string")]
    variable: String,
    #[webidl(with = fallback_arg)]
    fallback: v8::Local<'s, v8::Value>,
}

struct Segment<'s>(v8::Local<'s, v8::Value>);

impl<'s> webidl::WebIdlConverter<'s> for Segment<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && web_api_interfaces::CSSVariableReferenceValue::is_instance(scope, object)
        {
            return Ok(Self(value));
        }
        let text = webidl::convert::<webidl::UsvString>(scope, value, context)?.0;
        v8_string(scope, &text)
            .map(|value| Self(value.into()))
            .ok_or_else(|| webidl::WebIdlError::custom_message("Unable to create CSS string"))
    }
}

fn segments_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Vec<v8::Local<'s, v8::Value>>, webidl::WebIdlError> {
    let context = webidl::Context::argument("CSSUnparsedValue", (index + 1) as usize);
    if args.length() <= index {
        return Err(webidl::WebIdlError::missing_required(context));
    }
    webidl::convert::<webidl::Sequence<Segment<'s>>>(scope, args.get(index), context)
        .map(|values| values.0.into_iter().map(|value| value.0).collect())
}

fn fallback_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Value>, webidl::WebIdlError> {
    let value = args.get(index);
    if value.is_null_or_undefined() {
        return Ok(v8::null(scope).into());
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && web_api_interfaces::CSSUnparsedValue::is_instance(scope, object)
    {
        return Ok(value);
    }
    Err(webidl::WebIdlError::custom_message(
        "CSSVariableReferenceValue fallback must be a CSSUnparsedValue or null",
    ))
}

pub(super) fn install_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "CSSUnparsedValue" => {
            UnparsedValuePrototypeDeclaration::initialize_prototype_template(scope, prototype);
            indexed::install(template.instance_template(scope));
        }
        "CSSVariableReferenceValue" => {
            VariableReferencePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

pub(in crate::context_bootstrap) fn css_unparsed_value_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "CSSUnparsedValue requires new");
        return;
    }
    let Some(parsed) = webidl::parse_args::<UnparsedValueArgs>(scope, &args) else {
        return;
    };
    let segments = v8::Array::new_with_elements(scope, &parsed.segments);
    UnparsedValueDeclaration::new(segments)
        .initialize(scope, args.this())
        .expect("CSSUnparsedValue declaration should initialize");
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn css_variable_reference_value_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "CSSVariableReferenceValue requires new");
        return;
    }
    let Some(parsed) = webidl::parse_args::<VariableReferenceArgs>(scope, &args) else {
        return;
    };
    if !valid_variable_name(scope, &parsed.variable) {
        return;
    }
    VariableReferenceDeclaration::new(parsed.variable, parsed.fallback)
        .initialize(scope, args.this())
        .expect("CSSVariableReferenceValue declaration should initialize");
    rv.set(args.this().into());
}

fn valid_variable_name(scope: &mut v8::PinScope<'_, '_>, value: &str) -> bool {
    // Typed OM defines a name string, not an already-tokenized CSS identifier.
    if value.starts_with("--") {
        true
    } else {
        throw_type_error(scope, "A custom property name must start with --");
        false
    }
}

fn segments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_object(scope, object, SEGMENTS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(values) = segments(scope, args.this()) {
        rv.set_uint32(values.length());
    }
}

fn variable_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), VARIABLE_SLOT) {
        rv.set(value);
    }
}

fn fallback_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), FALLBACK_SLOT) {
        rv.set(value);
    }
}

fn variable_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = match webidl::convert::<webidl::UsvString>(
        scope,
        args.get(0),
        webidl::Context::member("CSSVariableReferenceValue", "variable"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if valid_variable_name(scope, &value)
        && let Some(value) = v8_string(scope, &value)
    {
        set_private_value(scope, args.this(), VARIABLE_SLOT, value.into());
    }
}
