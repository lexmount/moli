//! Native scheduled-source bindings for the partial audio backend.
//!
//! Store source properties and scheduling state without sending newly exposed
//! sources through the legacy oscillator fingerprint. PCM playback and timed
//! `ended` dispatch still need a rendering backend.

use super::super::media_queries::{
    install_simple_event_target_ordered_handlers, simple_event_target_slot_name,
    simple_object_event_set_ordered_handler,
};
use super::*;

const BUFFER: &str = "__moliBufferSourceBuffer";
const BUFFER_SET: &str = "__moliBufferSourceBufferSet";
const ACQUIRED_BUFFER: &str = "__moliBufferSourceAcquiredBuffer";
const PLAYBACK_RATE: &str = "__moliBufferSourcePlaybackRate";
const DETUNE: &str = "__moliBufferSourceDetune";
const LOOP: &str = "__moliBufferSourceLoop";
const LOOP_START: &str = "__moliBufferSourceLoopStart";
const LOOP_END: &str = "__moliBufferSourceLoopEnd";
const START_OFFSET: &str = "__moliBufferSourceStartOffset";
const START_DURATION: &str = "__moliBufferSourceStartDuration";
const OFFSET: &str = "__moliConstantSourceOffset";
const ONENDED: &str = "__moliAudioSourceOnended";

#[derive(Default, WebApiObject)]
#[webapi(interface = web_api_interfaces::AudioBufferSourceNode)]
struct BufferSourceObjectDeclaration {}

#[derive(Default, WebApiObject)]
#[webapi(interface = web_api_interfaces::ConstantSourceNode)]
struct ConstantSourceObjectDeclaration {}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AudioScheduledSourceNode, enumerable, receiver)]
struct ScheduledSourcePrototypeDeclaration {
    #[webapi(method, length = 0, callback = start)]
    start: (),
    #[webapi(method, length = 0, callback = stop)]
    stop: (),
    #[webapi(accessor_property, getter = onended_getter, setter = onended_setter)]
    onended: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AudioBufferSourceNode, enumerable, receiver)]
struct BufferSourcePrototypeDeclaration {
    #[webapi(accessor_property, getter = slot_getter, setter = buffer_setter, data = v8str(scope, BUFFER))]
    buffer: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, PLAYBACK_RATE))]
    playback_rate: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, DETUNE))]
    detune: (),
    #[webapi(accessor_property = "loop", getter = slot_getter, setter = loop_setter, data = v8str(scope, LOOP))]
    looping: (),
    #[webapi(accessor_property, getter = slot_getter, setter = loop_time_setter, data = v8str(scope, LOOP_START))]
    loop_start: (),
    #[webapi(accessor_property, getter = slot_getter, setter = loop_time_setter, data = v8str(scope, LOOP_END))]
    loop_end: (),
    #[webapi(method, length = 0, callback = buffer_start)]
    start: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ConstantSourceNode, enumerable, receiver)]
struct ConstantSourcePrototypeDeclaration {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, OFFSET))]
    offset: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "AudioScheduledSourceNode" => {
            ScheduledSourcePrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "AudioBufferSourceNode" => {
            BufferSourcePrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "ConstantSourceNode" => {
            ConstantSourcePrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

pub(super) fn initialize_scheduling<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) {
    graph::initialize_source(scope, node);
    set_private_value(scope, node, ONENDED, v8::null(scope).into());
    install_simple_event_target_ordered_handlers(scope, node);
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AudioBufferSourceOptions")]
struct BufferSourceOptions<'s> {
    #[webidl(with = buffer_member)]
    buffer: Option<v8::Local<'s, v8::Object>>,
    #[webidl(with = float_member)]
    detune: f64,
    #[webidl(name = "loop", default = false)]
    looping: bool,
    #[webidl(converter = "double", default = 0.0)]
    loop_end: f64,
    #[webidl(converter = "double", default = 0.0)]
    loop_start: f64,
    #[webidl(with = float_member)]
    playback_rate: f64,
}

impl Default for BufferSourceOptions<'_> {
    fn default() -> Self {
        Self {
            buffer: None,
            detune: 0.0,
            looping: false,
            loop_end: 0.0,
            loop_start: 0.0,
            playback_rate: 1.0,
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ConstantSourceOptions")]
struct ConstantSourceOptions {
    #[webidl(with = float_member)]
    offset: f64,
}

impl Default for ConstantSourceOptions {
    fn default() -> Self {
        Self { offset: 1.0 }
    }
}

fn float_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<f64, webidl::WebIdlError> {
    let prefix = if name == "offset" {
        "ConstantSourceOptions"
    } else {
        "AudioBufferSourceOptions"
    };
    let context = webidl::Context::member(prefix, name);
    let Some(value) = webidl::property_result(scope, object, name, context)?
        .filter(|value| !value.is_undefined())
    else {
        return Ok(if name == "detune" { 0.0 } else { 1.0 });
    };
    let value =
        <webidl::Double as webidl::WebIdlConverter>::convert(scope, value, context, &())?.0 as f32;
    if !value.is_finite() {
        return Err(webidl::WebIdlError::cannot_convert(context, "float"));
    }
    Ok(f64::from(value))
}

fn buffer_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Result<Option<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    buffer::nullable_value(scope, value, context)
}

fn buffer_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Option<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    let context = webidl::Context::member("AudioBufferSourceOptions", name);
    let value = webidl::property_result(scope, object, name, context)?
        .unwrap_or_else(|| v8::undefined(scope).into());
    buffer_value(scope, value, context)
}

fn constructor_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !args.is_construct_call() {
        throw_type_error(scope, "Audio source constructors require the new operator.");
        return None;
    }
    let context = v8::Local::<v8::Object>::try_from(args.get(0)).ok();
    if !context
        .is_some_and(|context| web_api_interfaces::BaseAudioContext::is_instance(scope, context))
    {
        throw_type_error(
            scope,
            "Audio source constructors require a BaseAudioContext.",
        );
        return None;
    }
    context
}

pub(in crate::context_bootstrap) fn audio_buffer_source_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(context) = constructor_context(scope, &args) else {
        return;
    };
    let options = match webidl::parse_dictionary::<BufferSourceOptions>(
        scope,
        args.get(1),
        webidl::Context::argument("AudioBufferSourceNode", 2),
    ) {
        Ok(options) => options.unwrap_or_default(),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let node = args.this();
    BufferSourceObjectDeclaration::default()
        .initialize(scope, node)
        .expect("BufferSource declaration should initialize");
    initialize_buffer_source(scope, node, context, options);
    rv.set(node.into());
}

pub(in crate::context_bootstrap) fn constant_source_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(context) = constructor_context(scope, &args) else {
        return;
    };
    let options = match webidl::parse_dictionary::<ConstantSourceOptions>(
        scope,
        args.get(1),
        webidl::Context::argument("ConstantSourceNode", 2),
    ) {
        Ok(options) => options.unwrap_or_default(),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let node = args.this();
    ConstantSourceObjectDeclaration::default()
        .initialize(scope, node)
        .expect("ConstantSource declaration should initialize");
    initialize_constant_source(scope, node, context, options.offset);
    rv.set(node.into());
}

fn source_param<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    default: f64,
    value: f64,
    rate: &'static str,
    fixed: bool,
) -> v8::Local<'s, v8::Object> {
    let param = audio_param(scope, default, -f64::from(f32::MAX), f64::from(f32::MAX));
    define_non_enumerable_number_property(scope, param, "value", value);
    audio_param::initialize_rate(scope, param, rate, fixed);
    param
}

fn initialize_buffer_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
    options: BufferSourceOptions<'s>,
) {
    graph::initialize_node(scope, node, context);
    initialize_scheduling(scope, node);
    let buffer = options
        .buffer
        .map_or_else(|| v8::null(scope).into(), Into::into);
    let buffer_set = v8::Boolean::new(scope, options.buffer.is_some());
    set_private_value(scope, node, BUFFER, buffer);
    set_private_value(scope, node, BUFFER_SET, buffer_set.into());
    set_private_value(
        scope,
        node,
        LOOP,
        v8::Boolean::new(scope, options.looping).into(),
    );
    set_web_audio_number_slot(scope, node, LOOP_START, options.loop_start);
    set_web_audio_number_slot(scope, node, LOOP_END, options.loop_end);
    let playback_rate = source_param(scope, 1.0, options.playback_rate, "k-rate", true);
    let detune = source_param(scope, 0.0, options.detune, "k-rate", true);
    set_private_value(scope, node, PLAYBACK_RATE, playback_rate.into());
    set_private_value(scope, node, DETUNE, detune.into());
}

fn initialize_constant_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
    offset: f64,
) {
    graph::initialize_node(scope, node, context);
    initialize_scheduling(scope, node);
    let param = source_param(scope, 1.0, offset, "a-rate", false);
    set_private_value(scope, node, OFFSET, param.into());
}

pub(super) fn create_buffer_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let node = BufferSourceObjectDeclaration::default()
        .bind(scope)
        .expect("BufferSource declaration should bind");
    initialize_buffer_source(scope, node, args.this(), BufferSourceOptions::default());
    rv.set(node.into());
}

pub(super) fn create_constant_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let node = ConstantSourceObjectDeclaration::default()
        .bind(scope)
        .expect("ConstantSource declaration should bind");
    initialize_constant_source(scope, node, args.this(), 1.0);
    rv.set(node.into());
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

fn buffer_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let value = match buffer_value(
        scope,
        args.get(0),
        webidl::Context::argument("AudioBufferSourceNode.buffer", 1),
    ) {
        Ok(value) => value,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let node = args.this();
    if value.is_some()
        && get_private_value(scope, node, BUFFER_SET)
            .is_some_and(|value| value.boolean_value(scope))
    {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The audio source buffer has already been set.",
        );
        return;
    }
    if let Some(buffer) = value {
        set_private_value(
            scope,
            node,
            BUFFER_SET,
            v8::Boolean::new(scope, true).into(),
        );
        if graph::source_started(scope, node) {
            let content = buffer::acquire(scope, buffer);
            set_private_value(scope, node, ACQUIRED_BUFFER, content.into());
        }
    }
    set_private_value(
        scope,
        node,
        BUFFER,
        value.map_or_else(|| v8::null(scope).into(), Into::into),
    );
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioBufferSourceNode.loop")]
struct LoopArgs {
    #[webidl(required)]
    value: bool,
}

fn loop_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<LoopArgs>(scope, &args) else {
        return;
    };
    set_private_value(
        scope,
        args.this(),
        LOOP,
        v8::Boolean::new(scope, parsed.value).into(),
    );
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioBufferSourceNode loop time")]
struct LoopTimeArgs {
    #[webidl(required, converter = "double")]
    value: f64,
}

fn loop_time_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<LoopTimeArgs>(scope, &args) else {
        return;
    };
    let slot = args.data().to_rust_string_lossy(scope);
    let value = v8::Number::new(scope, parsed.value);
    set_private_value(scope, args.this(), &slot, value.into());
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioScheduledSourceNode")]
struct WhenArgs {
    #[webidl(converter = "double", default = 0.0)]
    when: f64,
}

fn start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<WhenArgs>(scope, &args) else {
        return;
    };
    if graph::start_source(scope, args.this(), parsed.when, &[])
        && web_api_interfaces::AudioBufferSourceNode::is_instance(scope, args.this())
    {
        record_buffer_start(scope, args.this(), 0.0, f64::INFINITY);
    }
}

fn stop<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<WhenArgs>(scope, &args) else {
        return;
    };
    graph::stop_source(scope, args.this(), parsed.when);
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioBufferSourceNode.start")]
struct BufferStartArgs {
    #[webidl(converter = "double", default = 0.0)]
    when: f64,
    #[webidl(converter = "double")]
    offset: Option<f64>,
    #[webidl(converter = "double")]
    duration: Option<f64>,
}

fn buffer_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<BufferStartArgs>(scope, &args) else {
        return;
    };
    let node = args.this();
    let offset = parsed.offset.unwrap_or(0.0);
    if !graph::start_source(
        scope,
        node,
        parsed.when,
        &[offset, parsed.duration.unwrap_or(0.0)],
    ) {
        return;
    }
    record_buffer_start(
        scope,
        node,
        offset,
        parsed.duration.unwrap_or(f64::INFINITY),
    );
}

fn record_buffer_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    offset: f64,
    duration: f64,
) {
    set_web_audio_number_slot(scope, node, START_OFFSET, offset);
    set_web_audio_number_slot(scope, node, START_DURATION, duration);
    if let Some(buffer) = web_audio_object_slot(scope, node, BUFFER) {
        let content = buffer::acquire(scope, buffer);
        set_private_value(scope, node, ACQUIRED_BUFFER, content.into());
    }
}

fn onended_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    rv.set(
        get_private_value(scope, args.this(), ONENDED).unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn onended_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let value = args.get(0);
    let active = value.is_object();
    set_private_value(
        scope,
        args.this(),
        ONENDED,
        if active {
            value
        } else {
            v8::null(scope).into()
        },
    );
    if let Some(listeners) = simple_event_target_slot_name(scope, args.this()) {
        simple_object_event_set_ordered_handler(
            scope,
            args.this(),
            &listeners,
            "ended",
            ONENDED,
            active,
        );
    }
}

pub(super) fn needs_rendering_backend<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    if web_api_interfaces::ConstantSourceNode::is_instance(scope, node) {
        return true;
    }
    web_api_interfaces::AudioBufferSourceNode::is_instance(scope, node)
        && web_audio_number_slot(scope, node, START_DURATION) != Some(0.0)
        && (web_audio_object_slot(scope, node, BUFFER).is_some()
            || web_audio_object_slot(scope, node, ACQUIRED_BUFFER).is_some())
}
