//! WaveShaperNode control state; signal processing still requires a DSP backend.

use super::*;
use webidl::{WebIdlDictionary, WebIdlEnum};

const CURVE: &str = "__moliWaveShaperCurve";
const CURVE_SET: &str = "__moliWaveShaperCurveSet";
const OVERSAMPLE: &str = "__moliWaveShaperOversample";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WaveShaperNode, enumerable, receiver)]
struct PrototypeDeclaration {
    #[webapi(accessor_property, getter = curve_getter, setter = curve_setter)]
    curve: (),
    #[webapi(accessor_property, getter = oversample_getter, setter = oversample_setter)]
    oversample: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    PrototypeDeclaration::initialize_prototype_template(scope, template.prototype_template(scope));
}

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
#[webidl(name = "OverSampleType")]
enum OverSampleType {
    #[default]
    #[webidl(token = "none")]
    None,
    #[webidl(token = "2x")]
    Twice,
    #[webidl(token = "4x")]
    FourTimes,
}

impl OverSampleType {
    fn token(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Twice => "2x",
            Self::FourTimes => "4x",
        }
    }
}

#[derive(Default)]
struct Options {
    node: node::AudioNodeOptions,
    curve: Option<Vec<f32>>,
    oversample: OverSampleType,
}

fn parse_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Options, webidl::WebIdlError> {
    let mut options = Options::default();
    let Some(object) =
        webidl::dictionary_value(value, webidl::Context::argument("WaveShaperNode", 2))?
    else {
        return Ok(options);
    };
    options.node = node::AudioNodeOptions::parse_dictionary(scope, object)?;
    options.curve = webidl::optional_member::<webidl::Sequence<webidl::Float>>(
        scope,
        object,
        "curve",
        webidl::Context::member("WaveShaperOptions", "curve"),
    )?
    .map(|sequence| sequence.0.into_iter().map(|value| value.0).collect());
    options.oversample = webidl::optional_member_or::<webidl::EnumValue<OverSampleType>>(
        scope,
        object,
        "oversample",
        webidl::Context::member("WaveShaperOptions", "oversample"),
        webidl::EnumValue(OverSampleType::None),
    )?
    .0;
    Ok(options)
}

fn set_curve<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    bytes: Option<Vec<u8>>,
) -> bool {
    if let Some(bytes) = bytes {
        if bytes.len() < 2 * size_of::<f32>() {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "A waveshaping curve needs at least two entries.",
            );
            return false;
        }
        if get_private_value(scope, node, CURVE_SET).is_some_and(|value| value.boolean_value(scope))
        {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "A non-null waveshaping curve has already been set.",
            );
            return false;
        }
        let curve: Vec<_> = bytes
            .chunks_exact(size_of::<f32>())
            .map(|value| f32::from_ne_bytes(value.try_into().expect("Float32Array element")))
            .collect();
        let middle = curve.len() / 2;
        let zero_output = if curve.len() % 2 == 1 {
            f64::from(curve[middle])
        } else {
            (f64::from(curve[middle - 1]) + f64::from(curve[middle])) / 2.0
        };
        let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
        let curve = v8::ArrayBuffer::with_backing_store(scope, &store);
        set_private_value(scope, node, CURVE, curve.into());
        set_private_value(scope, node, CURVE_SET, v8::Boolean::new(scope, true).into());
        graph::set_unsupported_generator(scope, node, zero_output != 0.0);
    } else {
        set_private_value(scope, node, CURVE, v8::null(scope).into());
        // Clearing the curve does not reset the spec's [[curve set]] flag.
        graph::set_unsupported_generator(scope, node, false);
    }
    true
}

fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
    options: Options,
) -> bool {
    graph::initialize_node(scope, node, context);
    if !node::apply_options(scope, node, options.node) {
        return false;
    }
    graph::mark_unsupported_processor(scope, node);
    let bytes = options
        .curve
        .map(|curve| curve.into_iter().flat_map(f32::to_ne_bytes).collect());
    if !set_curve(scope, node, bytes) {
        return false;
    }
    set_private_value(
        scope,
        node,
        OVERSAMPLE,
        v8str(scope, options.oversample.token()).into(),
    );
    true
}

pub(in crate::context_bootstrap) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "WaveShaperNode requires new.");
        return;
    }
    let context = v8::Local::<v8::Object>::try_from(args.get(0)).ok();
    let Some(context) = context
        .filter(|context| web_api_interfaces::BaseAudioContext::is_instance(scope, *context))
    else {
        throw_type_error(scope, "WaveShaperNode requires a BaseAudioContext.");
        return;
    };
    let options = match parse_options(scope, args.get(1)) {
        Ok(options) => options,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    web_api_interfaces::initialize(scope, args.this(), "WaveShaperNode")
        .expect("WaveShaperNode brand should initialize");
    if initialize(scope, args.this(), context, options) {
        rv.set(args.this().into());
    }
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let realm = args
        .this()
        .get_creation_context(scope)
        .expect("Audio context should have a creation realm");
    let scope = &mut v8::ContextScope::new(scope, realm);
    let prototype = super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "WaveShaperNode",
    )
    .expect("WaveShaperNode prototype should exist");
    let node = v8::Object::new(scope);
    let _ = node.set_prototype(scope, prototype.into());
    web_api_interfaces::initialize(scope, node, "WaveShaperNode")
        .expect("WaveShaperNode brand should initialize");
    if initialize(scope, node, args.this(), Options::default()) {
        rv.set(node.into());
    }
}

fn curve_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(curve) = get_private_value(scope, args.this(), CURVE)
        .and_then(|value| v8::Local::<v8::ArrayBuffer>::try_from(value).ok())
    else {
        rv.set_null();
        return;
    };
    let bytes: Vec<_> = curve
        .get_backing_store()
        .iter()
        .map(std::cell::Cell::get)
        .collect();
    let length = bytes.len() / size_of::<f32>();
    let realm = args
        .this()
        .get_creation_context(scope)
        .expect("WaveShaperNode should have a creation realm");
    let scope = &mut v8::ContextScope::new(scope, realm);
    let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    rv.set(
        v8::Float32Array::new(scope, buffer, 0, length)
            .expect("Curve view should allocate")
            .into(),
    );
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WaveShaperNode.curve")]
struct CurveArgs<'s> {
    #[webidl(required, with = curve_value)]
    value: Option<v8::Local<'s, v8::Float32Array>>,
}

fn curve_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Option<v8::Local<'s, v8::Float32Array>>, webidl::WebIdlError> {
    let value = args.get(index);
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    buffer::float32_array_value(
        scope,
        value,
        webidl::Context::argument("WaveShaperNode.curve", 1),
    )
    .map(Some)
}

fn curve_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<CurveArgs>(scope, &args) else {
        return;
    };
    let bytes = parsed.value.map(|view| {
        let mut bytes = vec![0; view.byte_length()];
        let written = view.copy_contents(&mut bytes);
        bytes.truncate(written);
        bytes
    });
    set_curve(scope, args.this(), bytes);
}

fn oversample_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(value) = get_private_value(scope, args.this(), OVERSAMPLE) {
        rv.set(value);
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WaveShaperNode.oversample")]
struct OversampleArgs {
    #[webidl(required)]
    value: String,
}

fn oversample_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<OversampleArgs>(scope, &args) else {
        return;
    };
    if let Some(value) = OverSampleType::parse_token(&parsed.value) {
        set_private_value(
            scope,
            args.this(),
            OVERSAMPLE,
            v8str(scope, value.token()).into(),
        );
    }
}
