//! AudioParam-backed node interfaces, independent of the signal-processing backend.

use super::*;
use webidl::WebIdlDictionary;

const PARAM: &str = "__moliAudioNodePrimaryParam";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::GainNode, enumerable, receiver)]
struct GainPrototypeDeclaration {
    #[webapi(accessor_property, getter = param_getter)]
    gain: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DelayNode, enumerable, receiver)]
struct DelayPrototypeDeclaration {
    #[webapi(accessor_property, getter = param_getter)]
    delay_time: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::StereoPannerNode, enumerable, receiver)]
struct StereoPannerPrototypeDeclaration {
    #[webapi(accessor_property, getter = param_getter)]
    pan: (),
}

#[derive(Clone, Copy)]
enum Kind {
    Gain,
    Delay,
    StereoPanner,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Gain => "GainNode",
            Self::Delay => "DelayNode",
            Self::StereoPanner => "StereoPannerNode",
        }
    }

    fn options_name(self) -> &'static str {
        match self {
            Self::Gain => "GainOptions",
            Self::Delay => "DelayOptions",
            Self::StereoPanner => "StereoPannerOptions",
        }
    }

    fn default_value(self) -> f64 {
        if matches!(self, Self::Gain) { 1.0 } else { 0.0 }
    }
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "GainNode" => GainPrototypeDeclaration::initialize_prototype_template(scope, prototype),
        "DelayNode" => DelayPrototypeDeclaration::initialize_prototype_template(scope, prototype),
        "StereoPannerNode" => {
            StereoPannerPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

fn param_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(param) = get_private_value(scope, args.this(), PARAM) {
        rv.set(param);
    }
}

struct Options {
    node: node::AudioNodeOptions,
    value: f64,
    max_delay: f64,
}

fn parse_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    kind: Kind,
) -> Result<Options, webidl::WebIdlError> {
    let mut options = Options {
        node: node::AudioNodeOptions::default(),
        value: kind.default_value(),
        max_delay: 1.0,
    };
    let Some(object) = webidl::dictionary_value(value, webidl::Context::argument(kind.name(), 2))?
    else {
        return Ok(options);
    };
    // Web IDL converts all inherited members first, then the derived members
    // in lexical order. AudioNode state constraints run after all conversions.
    options.node = node::AudioNodeOptions::parse_dictionary(scope, object)?;
    let prefix = kind.options_name();
    if matches!(kind, Kind::Delay) {
        options.value = webidl::optional_member_or::<webidl::Double>(
            scope,
            object,
            "delayTime",
            webidl::Context::member(prefix, "delayTime"),
            webidl::Double(0.0),
        )?
        .0;
        options.max_delay = webidl::optional_member_or::<webidl::Double>(
            scope,
            object,
            "maxDelayTime",
            webidl::Context::member(prefix, "maxDelayTime"),
            webidl::Double(1.0),
        )?
        .0;
    } else {
        let key = if matches!(kind, Kind::Gain) {
            "gain"
        } else {
            "pan"
        };
        let context = webidl::Context::member(prefix, key);
        let value = webidl::optional_member_or::<webidl::Double>(
            scope,
            object,
            key,
            context,
            webidl::Double(options.value),
        )?
        .0 as f32;
        if !value.is_finite() {
            return Err(webidl::WebIdlError::cannot_convert(context, "float"));
        }
        options.value = f64::from(value);
    }
    Ok(options)
}

fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
    kind: Kind,
    options: Options,
) -> bool {
    if matches!(kind, Kind::Delay) && !(options.max_delay > 0.0 && options.max_delay < 180.0) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Maximum delay time must be greater than zero and less than 180 seconds.",
        );
        return false;
    }
    graph::initialize_node(scope, node, context);
    if !node::apply_options(scope, node, options.node) {
        return false;
    }
    let (min, max) = match kind {
        Kind::Gain => (-f64::from(f32::MAX), f64::from(f32::MAX)),
        Kind::Delay => (0.0, f64::from(options.max_delay as f32)),
        Kind::StereoPanner => (-1.0, 1.0),
    };
    let param = audio_param(scope, kind.default_value(), min, max);
    let value = f64::from(options.value as f32).clamp(min, max);
    define_non_enumerable_number_property(scope, param, "value", value);
    set_private_value(scope, node, PARAM, param.into());
    graph::mark_unsupported_processor(scope, node);
    true
}

fn construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
    kind: Kind,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "Audio node constructors require the new operator.");
        return;
    }
    let context = v8::Local::<v8::Object>::try_from(args.get(0)).ok();
    let Some(context) = context
        .filter(|context| web_api_interfaces::BaseAudioContext::is_instance(scope, *context))
    else {
        throw_type_error(scope, "Audio node constructors require a BaseAudioContext.");
        return;
    };
    let options = match parse_options(scope, args.get(1), kind) {
        Ok(options) => options,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let node = args.this();
    web_api_interfaces::initialize(scope, node, kind.name())
        .expect("Audio node brand should initialize");
    if initialize(scope, node, context, kind, options) {
        rv.set(node.into());
    }
}

pub(in crate::context_bootstrap) fn gain_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    construct(scope, args, rv, Kind::Gain);
}

pub(in crate::context_bootstrap) fn delay_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    construct(scope, args, rv, Kind::Delay);
}

pub(in crate::context_bootstrap) fn stereo_panner_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    construct(scope, args, rv, Kind::StereoPanner);
}

fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
    kind: Kind,
    max_delay: f64,
) {
    let prototype =
        super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(scope, kind.name())
            .expect("Audio node prototype should exist");
    let node = v8::Object::new(scope);
    let _ = node.set_prototype(scope, prototype.into());
    web_api_interfaces::initialize(scope, node, kind.name())
        .expect("Audio node brand should initialize");
    let options = Options {
        node: node::AudioNodeOptions::default(),
        value: kind.default_value(),
        max_delay,
    };
    if initialize(scope, node, args.this(), kind, options) {
        rv.set(node.into());
    }
}

pub(super) fn create_gain<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    create(scope, args, rv, Kind::Gain, 1.0);
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "BaseAudioContext.createDelay")]
struct DelayArgs {
    #[webidl(converter = "double", default = 1.0)]
    max_delay_time: f64,
}

pub(super) fn create_delay<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<DelayArgs>(scope, &args) else {
        return;
    };
    create(scope, args, rv, Kind::Delay, parsed.max_delay_time);
}

pub(super) fn create_stereo_panner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    create(scope, args, rv, Kind::StereoPanner, 1.0);
}
