//! Native Fourier coefficients for custom oscillators. Wave-table generation,
//! normalization and anti-aliased PCM rendering still need an audio backend.

use super::*;
use webidl::WebIdlDictionary;

const CONTEXT: &str = "__moliPeriodicWaveContext";
const REAL: &str = "__moliPeriodicWaveReal";
const IMAG: &str = "__moliPeriodicWaveImag";
const NORMALIZE: &str = "__moliPeriodicWaveNormalize";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PeriodicWave)]
struct ObjectDeclaration<'s> {
    #[webapi(slot = CONTEXT)]
    context: v8::Local<'s, v8::Object>,
    #[webapi(slot = REAL)]
    real: v8::Local<'s, v8::ArrayBuffer>,
    #[webapi(slot = IMAG)]
    imag: v8::Local<'s, v8::ArrayBuffer>,
    #[webapi(slot = NORMALIZE)]
    normalize: bool,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PeriodicWaveConstraints")]
struct Constraints {
    #[webidl(default = false)]
    disable_normalization: bool,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PeriodicWaveOptions")]
struct Coefficients {
    #[webidl(converter = "raw")]
    imag: Option<webidl::Sequence<webidl::Float>>,
    #[webidl(converter = "raw")]
    real: Option<webidl::Sequence<webidl::Float>>,
}

fn parse_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<(Constraints, Coefficients), webidl::WebIdlError> {
    let Some(object) =
        webidl::dictionary_value(value, webidl::Context::argument("PeriodicWave", 2))?
    else {
        return Ok((Constraints::default(), Coefficients::default()));
    };
    // Inherited members precede the derived dictionary's lexical imag/real order.
    let constraints = Constraints::parse_dictionary(scope, object)?;
    let coefficients = Coefficients::parse_dictionary(scope, object)?;
    Ok((constraints, coefficients))
}

fn prepare_coefficients<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    coefficients: Coefficients,
) -> Option<(Vec<f32>, Vec<f32>)> {
    let real = coefficients
        .real
        .map(|values| values.0.into_iter().map(|v| v.0).collect::<Vec<_>>());
    let imag = coefficients
        .imag
        .map(|values| values.0.into_iter().map(|v| v.0).collect::<Vec<_>>());
    let length = real.as_ref().or(imag.as_ref()).map_or(2, Vec::len);
    if length < 2 || imag.as_ref().is_some_and(|values| values.len() != length) {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "Periodic wave coefficients need equal lengths of at least two.",
        );
        return None;
    }
    let defaults = real.is_none() && imag.is_none();
    let mut real = real.unwrap_or_else(|| vec![0.0; length]);
    let mut imag = imag.unwrap_or_else(|| vec![0.0; length]);
    if defaults {
        imag[1] = 1.0;
    }
    // Conversion includes the DC entries (even non-finite values), before the
    // algorithm discards them. Never mutate the author's source sequences.
    real[0] = 0.0;
    imag[0] = 0.0;
    Some((real, imag))
}

fn coefficient_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: Vec<f32>,
) -> v8::Local<'s, v8::ArrayBuffer> {
    let bytes = values
        .into_iter()
        .flat_map(f32::to_ne_bytes)
        .collect::<Vec<_>>();
    let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    v8::ArrayBuffer::with_backing_store(scope, &store)
}

fn declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    coefficients: (Vec<f32>, Vec<f32>),
    constraints: Constraints,
) -> ObjectDeclaration<'s> {
    let real = coefficient_buffer(scope, coefficients.0);
    let imag = coefficient_buffer(scope, coefficients.1);
    ObjectDeclaration::new(context, real, imag, !constraints.disable_normalization)
}

pub(in crate::context_bootstrap) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if !args.is_construct_call() || args.length() < 1 {
        throw_type_error(scope, "PeriodicWave requires new and a context.");
        return;
    }
    let Some(context) = v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .filter(|context| web_api_interfaces::BaseAudioContext::is_instance(scope, *context))
    else {
        throw_type_error(scope, "PeriodicWave requires a BaseAudioContext.");
        return;
    };
    let (constraints, coefficients) = match parse_options(scope, args.get(1)) {
        Ok(options) => options,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let Some(coefficients) = prepare_coefficients(scope, coefficients) else {
        return;
    };
    declaration(scope, context, coefficients, constraints)
        .initialize(scope, args.this())
        .expect("PeriodicWave declaration should initialize");
    rv.set(args.this().into());
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "BaseAudioContext.createPeriodicWave")]
struct CreateArgs {
    #[webidl(required, converter = "raw")]
    real: webidl::Sequence<webidl::Float>,
    #[webidl(required, converter = "raw")]
    imag: webidl::Sequence<webidl::Float>,
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<CreateArgs>(scope, &args) else {
        return;
    };
    let constraints = match webidl::dictionary_value(
        args.get(2),
        webidl::Context::argument("BaseAudioContext.createPeriodicWave", 3),
    )
    .and_then(|object| {
        object.map_or_else(
            || Ok(Constraints::default()),
            |object| Constraints::parse_dictionary(scope, object),
        )
    }) {
        Ok(constraints) => constraints,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    // All positional conversions, including constraints, precede length rules.
    let Some(coefficients) = prepare_coefficients(
        scope,
        Coefficients {
            real: Some(parsed.real),
            imag: Some(parsed.imag),
        },
    ) else {
        return;
    };
    let context = args.this();
    let realm = context
        .get_creation_context(scope)
        .expect("Audio context should have a realm");
    let scope = &mut v8::ContextScope::new(scope, realm);
    let wave = declaration(scope, context, coefficients, constraints)
        .bind(scope)
        .expect("PeriodicWave declaration should bind");
    rv.set(wave.into());
}
