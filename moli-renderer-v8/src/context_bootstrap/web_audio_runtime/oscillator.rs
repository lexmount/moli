//! Oscillator control state shared by constructors and context factories.

use super::*;
use webidl::{WebIdlConverter, WebIdlDictionary, WebIdlEnum};

const TYPE: &str = "__moliOscillatorType";
const FREQUENCY: &str = "__moliOscillatorFrequency";
const DETUNE: &str = "__moliOscillatorDetune";
const WAVE: &str = "__moliOscillatorPeriodicWave";

#[derive(Default, Clone, Copy, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "OscillatorType", rename_all = "lowercase")]
enum Kind {
    #[default]
    Sine,
    Square,
    Sawtooth,
    Triangle,
    Custom,
}

impl Kind {
    fn token(self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::Square => "square",
            Self::Sawtooth => "sawtooth",
            Self::Triangle => "triangle",
            Self::Custom => "custom",
        }
    }
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::OscillatorNode, enumerable, receiver)]
struct PrototypeDeclaration {
    #[webapi(accessor_property = "type", getter = slot_getter, setter = type_setter, data = v8str(scope, TYPE))]
    kind: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, FREQUENCY))]
    frequency: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, DETUNE))]
    detune: (),
    #[webapi(method, length = 1, callback = set_periodic_wave)]
    set_periodic_wave: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    PrototypeDeclaration::initialize_prototype_template(scope, template.prototype_template(scope));
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "OscillatorOptions")]
struct Options<'s> {
    #[webidl(with = float_member)]
    detune: f64,
    #[webidl(with = float_member)]
    frequency: f64,
    #[webidl(with = wave_member)]
    periodic_wave: Option<v8::Local<'s, v8::Object>>,
    #[webidl(name = "type", with = type_member)]
    kind: Kind,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            detune: 0.0,
            frequency: 440.0,
            periodic_wave: None,
            kind: Kind::Sine,
        }
    }
}

fn float_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<f64, webidl::WebIdlError> {
    let context = webidl::Context::member("OscillatorOptions", name);
    let Some(value) = webidl::property_result(scope, object, name, context)?
        .filter(|value| !value.is_undefined())
    else {
        return Ok(if name == "frequency" { 440.0 } else { 0.0 });
    };
    Ok(f64::from(
        webidl::Float::convert(scope, value, context, &())?.0,
    ))
}

fn wave_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Option<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    let context = webidl::Context::member("OscillatorOptions", name);
    let Some(value) = webidl::property_result(scope, object, name, context)?
        .filter(|value| !value.is_undefined())
    else {
        return Ok(None);
    };
    wave_value(scope, value, context).map(Some)
}

fn wave_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    v8::Local::<v8::Object>::try_from(value)
        .ok()
        .filter(|wave| web_api_interfaces::PeriodicWave::is_instance(scope, *wave))
        .ok_or_else(|| webidl::WebIdlError::cannot_convert(context, "PeriodicWave"))
}

fn type_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Kind, webidl::WebIdlError> {
    let context = webidl::Context::member("OscillatorOptions", name);
    let Some(value) = webidl::property_result(scope, object, name, context)?
        .filter(|value| !value.is_undefined())
    else {
        return Ok(Kind::Sine);
    };
    Ok(webidl::EnumValue::<Kind>::convert(scope, value, context, &())?.0)
}

fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    oscillator: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
    options: node::AudioNodeOptions,
    controls: Options<'s>,
) -> bool {
    graph::initialize_node(scope, oscillator, context);
    if !node::apply_options(scope, oscillator, options) {
        return false;
    }
    source::initialize_scheduling(scope, oscillator);
    let nyquist = audio_context_sample_rate(scope, context) / 2.0;
    let frequency = audio_param(scope, 440.0, -nyquist, nyquist);
    let detune = detune_param(scope);
    // Keep the AudioParam's default metadata independent of its initial value.
    define_non_enumerable_number_property(
        scope,
        frequency,
        "value",
        controls.frequency.clamp(-nyquist, nyquist),
    );
    let detune_limit = (1200.0_f32 * f32::MAX.log2()) as f64;
    define_non_enumerable_number_property(
        scope,
        detune,
        "value",
        controls.detune.clamp(-detune_limit, detune_limit),
    );
    set_private_value(scope, oscillator, FREQUENCY, frequency.into());
    set_private_value(scope, oscillator, DETUNE, detune.into());
    set_type(scope, oscillator, controls.kind);
    if let Some(wave) = controls.periodic_wave {
        set_wave(scope, oscillator, wave);
    }
    true
}

pub(in crate::context_bootstrap) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if !args.is_construct_call() || args.length() < 1 {
        throw_type_error(scope, "OscillatorNode requires new and a context.");
        return;
    }
    let Some(context) = v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .filter(|context| web_api_interfaces::BaseAudioContext::is_instance(scope, *context))
    else {
        throw_type_error(scope, "OscillatorNode requires a BaseAudioContext.");
        return;
    };
    let parsed =
        webidl::dictionary_value(args.get(1), webidl::Context::argument("OscillatorNode", 2))
            .and_then(|object| match object {
                Some(object) => Ok((
                    node::AudioNodeOptions::parse_dictionary(scope, object)?,
                    Options::parse_dictionary(scope, object)?,
                )),
                None => Ok((node::AudioNodeOptions::default(), Options::default())),
            });
    let (options, controls) = match parsed {
        Ok(options) => options,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if controls.kind == Kind::Custom && controls.periodic_wave.is_none() {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "A custom oscillator requires a PeriodicWave.",
        );
        return;
    }
    web_api_interfaces::initialize(scope, args.this(), "OscillatorNode")
        .expect("OscillatorNode brand should initialize");
    if initialize(scope, args.this(), context, options, controls) {
        rv.set(args.this().into());
    }
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let context = args.this();
    let realm = context
        .get_creation_context(scope)
        .expect("Audio context should have a realm");
    let scope = &mut v8::ContextScope::new(scope, realm);
    let prototype = super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "OscillatorNode",
    )
    .expect("OscillatorNode prototype should exist");
    let oscillator = v8::Object::new(scope);
    let _ = oscillator.set_prototype(scope, prototype.into());
    web_api_interfaces::initialize(scope, oscillator, "OscillatorNode")
        .expect("OscillatorNode brand should initialize");
    if initialize(
        scope,
        oscillator,
        context,
        node::AudioNodeOptions::default(),
        Options::default(),
    ) {
        rv.set(oscillator.into());
    }
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    if let Some(value) = get_private_value(scope, args.this(), &slot) {
        rv.set(value);
    }
}

fn set_type<'s>(scope: &mut v8::PinScope<'s, '_>, node: v8::Local<'s, v8::Object>, kind: Kind) {
    let token = v8str(scope, kind.token());
    set_private_value(scope, node, TYPE, token.into());
    set_private_value(scope, node, WAVE, v8::null(scope).into());
}

fn set_wave<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    wave: v8::Local<'s, v8::Object>,
) {
    let token = v8str(scope, Kind::Custom.token());
    set_private_value(scope, node, TYPE, token.into());
    set_private_value(scope, node, WAVE, wave.into());
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "OscillatorNode.type")]
struct TypeArgs {
    #[webidl(required)]
    value: String,
}

fn type_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<TypeArgs>(scope, &args) else {
        return;
    };
    // Enum attribute setters ignore unknown strings after ToString.
    let Some(kind) = Kind::parse_token(&parsed.value) else {
        return;
    };
    if kind == Kind::Custom {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "Use setPeriodicWave to select a custom waveform.",
        );
        return;
    }
    set_type(scope, args.this(), kind);
}

fn set_periodic_wave<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    if args.length() < 1 {
        throw_type_error(scope, "setPeriodicWave requires a PeriodicWave.");
        return;
    }
    match wave_value(
        scope,
        args.get(0),
        webidl::Context::argument("OscillatorNode.setPeriodicWave", 1),
    ) {
        Ok(wave) => set_wave(scope, args.this(), wave),
        Err(error) => webidl::throw_error(scope, &error),
    }
}

pub(super) fn needs_rendering_backend<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::OscillatorNode::is_instance(scope, node)
        && get_private_value(scope, node, TYPE)
            .is_some_and(|value| value.to_rust_string_lossy(scope) == "custom")
}
