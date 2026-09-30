//! Immutable IIR coefficients and synchronous frequency response. Time-domain
//! filtering still requires the audio processing backend.

use super::*;
use webidl::WebIdlDictionary;

const FEEDFORWARD: &str = "__moliIirFeedforward";
const FEEDBACK: &str = "__moliIirFeedback";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IIRFilterNode, enumerable, receiver)]
struct PrototypeDeclaration {
    #[webapi(method, length = 3, callback = get_frequency_response)]
    get_frequency_response: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    PrototypeDeclaration::initialize_prototype_template(scope, template.prototype_template(scope));
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "IIRFilterOptions")]
struct Coefficients {
    // WebIDL visits members lexically, after the inherited AudioNodeOptions.
    #[webidl(required, converter = "raw")]
    feedback: webidl::Sequence<webidl::Double>,
    #[webidl(required, converter = "raw")]
    feedforward: webidl::Sequence<webidl::Double>,
}

fn parse_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<(node::AudioNodeOptions, Coefficients), webidl::WebIdlError> {
    let object = webidl::dictionary_value(value, webidl::Context::argument("IIRFilterNode", 2))?
        .ok_or_else(|| {
            webidl::WebIdlError::missing_required(webidl::Context::member(
                "IIRFilterOptions",
                "feedback",
            ))
        })?;
    let options = node::AudioNodeOptions::parse_dictionary(scope, object)?;
    let coefficients = Coefficients::parse_dictionary(scope, object)?;
    Ok((options, coefficients))
}

fn valid_coefficients<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    feedforward: &[f64],
    feedback: &[f64],
) -> bool {
    if !(1..=20).contains(&feedback.len()) || !(1..=20).contains(&feedforward.len()) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "IIR coefficient sequences need 1 to 20 entries.",
        );
        return false;
    }
    if feedback[0] == 0.0 || feedforward.iter().all(|value| *value == 0.0) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The first feedback and at least one feedforward coefficient must be nonzero.",
        );
        return false;
    }
    // Stability is not a creation constraint. In particular, poles on or
    // outside the unit circle remain valid coefficients for this interface.
    true
}

fn store_coefficients<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    slot: &'static str,
    coefficients: &[f64],
) {
    let bytes = coefficients
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect::<Vec<_>>();
    let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    set_private_value(scope, node, slot, buffer.into());
}

fn coefficients<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Vec<f64> {
    let buffer = get_private_value(scope, node, slot)
        .and_then(|value| v8::Local::<v8::ArrayBuffer>::try_from(value).ok())
        .expect("IIRFilterNode should have native coefficients");
    buffer
        .get_backing_store()
        .chunks_exact(size_of::<f64>())
        .map(|bytes| {
            let bytes = std::array::from_fn(|index| bytes[index].get());
            f64::from_ne_bytes(bytes)
        })
        .collect()
}

fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
    options: node::AudioNodeOptions,
    feedforward: &[f64],
    feedback: &[f64],
) -> bool {
    graph::initialize_node(scope, node, context);
    if !node::apply_options(scope, node, options) {
        return false;
    }
    store_coefficients(scope, node, FEEDFORWARD, feedforward);
    store_coefficients(scope, node, FEEDBACK, feedback);
    graph::mark_unsupported_processor(scope, node);
    true
}

pub(in crate::context_bootstrap) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "IIRFilterNode requires new.");
        return;
    }
    // Required arity precedes any conversion, including dictionary getters.
    if args.length() < 2 {
        throw_type_error(scope, "IIRFilterNode requires a context and options.");
        return;
    }
    let Some(context) = v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .filter(|context| web_api_interfaces::BaseAudioContext::is_instance(scope, *context))
    else {
        throw_type_error(scope, "IIRFilterNode requires a BaseAudioContext.");
        return;
    };
    let (options, coefficients) = match parse_options(scope, args.get(1)) {
        Ok(parsed) => parsed,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let feedforward: Vec<_> = coefficients
        .feedforward
        .0
        .into_iter()
        .map(|value| value.0)
        .collect();
    let feedback: Vec<_> = coefficients
        .feedback
        .0
        .into_iter()
        .map(|value| value.0)
        .collect();
    if !valid_coefficients(scope, &feedforward, &feedback) {
        return;
    }
    web_api_interfaces::initialize(scope, args.this(), "IIRFilterNode")
        .expect("IIRFilterNode brand should initialize");
    if initialize(
        scope,
        args.this(),
        context,
        options,
        &feedforward,
        &feedback,
    ) {
        rv.set(args.this().into());
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "BaseAudioContext.createIIRFilter")]
struct CreateArgs {
    #[webidl(required, converter = "raw")]
    feedforward: webidl::Sequence<webidl::Double>,
    #[webidl(required, converter = "raw")]
    feedback: webidl::Sequence<webidl::Double>,
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<CreateArgs>(scope, &args) else {
        return;
    };
    let feedforward: Vec<_> = parsed
        .feedforward
        .0
        .into_iter()
        .map(|value| value.0)
        .collect();
    let feedback: Vec<_> = parsed.feedback.0.into_iter().map(|value| value.0).collect();
    // Throw validation errors in the binding realm, then create the result in
    // the context's relevant realm without consulting public constructors.
    if !valid_coefficients(scope, &feedforward, &feedback) {
        return;
    }
    let realm = args
        .this()
        .get_creation_context(scope)
        .expect("Audio context should have a creation realm");
    let scope = &mut v8::ContextScope::new(scope, realm);
    let prototype = super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "IIRFilterNode",
    )
    .expect("IIRFilterNode prototype should exist");
    let node = v8::Object::new(scope);
    let _ = node.set_prototype(scope, prototype.into());
    web_api_interfaces::initialize(scope, node, "IIRFilterNode")
        .expect("IIRFilterNode brand should initialize");
    if initialize(
        scope,
        node,
        args.this(),
        node::AudioNodeOptions::default(),
        &feedforward,
        &feedback,
    ) {
        rv.set(node.into());
    }
}

fn float32_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Float32Array>, webidl::WebIdlError> {
    buffer::float32_array_value(
        scope,
        args.get(index),
        webidl::Context::argument("IIRFilterNode.getFrequencyResponse", index as usize + 1),
    )
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IIRFilterNode.getFrequencyResponse")]
struct ResponseArgs<'s> {
    #[webidl(required, with = float32_array)]
    frequencies: v8::Local<'s, v8::Float32Array>,
    #[webidl(required, with = float32_array)]
    magnitudes: v8::Local<'s, v8::Float32Array>,
    #[webidl(required, with = float32_array)]
    phases: v8::Local<'s, v8::Float32Array>,
}

// Scaling each polynomial avoids overflowing sums of valid finite doubles;
// coefficients stay immutable and keep double precision, including tiny a0.
fn polynomial(coefficients: &[f64], z: (f64, f64)) -> ((f64, f64), f64) {
    let scale = coefficients
        .iter()
        .fold(0.0_f64, |scale, value| scale.max(value.abs()));
    let value = coefficients
        .iter()
        .rev()
        .fold((0.0, 0.0), |(real, imag), coefficient| {
            (
                real * z.0 - imag * z.1 + coefficient / scale,
                real * z.1 + imag * z.0,
            )
        });
    (value, scale)
}

fn response(feedforward: &[f64], feedback: &[f64], frequency: f64, sample_rate: f64) -> (f32, f32) {
    if !(0.0..=sample_rate / 2.0).contains(&frequency) {
        return (f32::NAN, f32::NAN);
    }
    let omega = -std::f64::consts::TAU * frequency / sample_rate;
    let z = (omega.cos(), omega.sin());
    let ((nr, ni), ns) = polynomial(feedforward, z);
    let ((dr, di), ds) = polynomial(feedback, z);
    let numerator = nr.hypot(ni);
    let denominator = dr.hypot(di);
    if denominator == 0.0 {
        return ((numerator / denominator) as f32, f32::NAN);
    }
    let mut magnitude = (numerator / denominator) * (ns / ds);
    if numerator > 0.0 && (!magnitude.is_finite() || magnitude == 0.0) {
        magnitude = (numerator.ln() - denominator.ln() + ns.ln() - ds.ln()).exp();
    }
    let dr = dr / denominator;
    let di = di / denominator;
    let phase = (ni * dr - nr * di).atan2(nr * dr + ni * di);
    (magnitude as f32, phase as f32)
}

fn get_frequency_response<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<ResponseArgs>(scope, &args) else {
        return;
    };
    let length = parsed.frequencies.length();
    if parsed.magnitudes.length() != length || parsed.phases.length() != length {
        throw_dom_exception(
            scope,
            "InvalidAccessError",
            15,
            "Frequency response arrays must have equal lengths.",
        );
        return;
    }
    if length == 0 {
        return;
    }
    let context = graph::require_node_context(scope, args.this())
        .expect("IIRFilterNode should have a context");
    let rate = audio_context_sample_rate(scope, context);
    let feedforward = coefficients(scope, args.this(), FEEDFORWARD);
    let feedback = coefficients(scope, args.this(), FEEDBACK);
    let frequency_store = parsed
        .frequencies
        .get_backing_store()
        .expect("Frequencies should have storage");
    let offset = parsed.frequencies.byte_offset();
    // Snapshot frequency inputs before outputs: legal views may overlap them.
    let frequencies: Vec<_> = frequency_store[offset..offset + parsed.frequencies.byte_length()]
        .chunks_exact(size_of::<f32>())
        .map(|bytes| f32::from_ne_bytes(std::array::from_fn(|index| bytes[index].get())))
        .collect();
    let magnitudes = parsed
        .magnitudes
        .get_backing_store()
        .expect("Magnitudes should have storage");
    let phases = parsed
        .phases
        .get_backing_store()
        .expect("Phases should have storage");
    for (index, frequency) in frequencies.into_iter().enumerate() {
        let (magnitude, phase) = response(&feedforward, &feedback, f64::from(frequency), rate);
        for (store, offset, value) in [
            (&magnitudes, parsed.magnitudes.byte_offset(), magnitude),
            (&phases, parsed.phases.byte_offset(), phase),
        ] {
            let start = offset + index * size_of::<f32>();
            for (target, byte) in store[start..start + size_of::<f32>()]
                .iter()
                .zip(value.to_ne_bytes())
            {
                target.set(byte);
            }
        }
    }
}
