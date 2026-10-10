//! Shared AudioNode bindings and native channel metadata for the partial backend.

use super::*;

const INPUT_COUNT: &str = "__moliAudioNodeInputCount";
const OUTPUT_COUNT: &str = "__moliAudioNodeOutputCount";
const CHANNEL_COUNT: &str = "__moliAudioNodeChannelCount";
const CHANNEL_MODE: &str = "__moliAudioNodeChannelMode";
const INTERPRETATION: &str = "__moliAudioNodeChannelInterpretation";
const LISTENERS: &str = "__moliAudioNodeListeners";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AudioNode, enumerable, receiver)]
struct AudioNodePrototypeDeclaration {
    #[webapi(method, length = 1, callback = audio_node_connect_callback)]
    connect: (),
    #[webapi(method, length = 0, callback = audio_node_disconnect_callback)]
    disconnect: (),
    #[webapi(accessor_property, getter = context_getter)]
    context: (),
    #[webapi(accessor_property, getter = metadata_getter, data = v8str(scope, INPUT_COUNT))]
    number_of_inputs: (),
    #[webapi(accessor_property, getter = metadata_getter, data = v8str(scope, OUTPUT_COUNT))]
    number_of_outputs: (),
    #[webapi(accessor_property, getter = channel_count_getter, setter = channel_count_setter)]
    channel_count: (),
    #[webapi(accessor_property, getter = metadata_getter, setter = channel_mode_setter, data = v8str(scope, CHANNEL_MODE))]
    channel_count_mode: (),
    #[webapi(accessor_property, getter = metadata_getter, setter = interpretation_setter, data = v8str(scope, INTERPRETATION))]
    channel_interpretation: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AudioDestinationNode, enumerable, receiver)]
struct DestinationPrototypeDeclaration {
    #[webapi(accessor_property, getter = max_channel_count_getter)]
    max_channel_count: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "AudioNode" => {
            AudioNodePrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "AudioDestinationNode" => {
            DestinationPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

pub(super) fn initialize<'s>(scope: &mut v8::PinScope<'s, '_>, node: v8::Local<'s, v8::Object>) {
    let source = web_api_interfaces::OscillatorNode::is_instance(scope, node);
    let destination = web_api_interfaces::AudioDestinationNode::is_instance(scope, node);
    let compressor = web_api_interfaces::DynamicsCompressorNode::is_instance(scope, node);
    set_web_audio_number_slot(scope, node, INPUT_COUNT, if source { 0.0 } else { 1.0 });
    // The existing terminal destination backend has no output port. Capturing
    // its output, as proposed by Web Audio 1.1, needs a separate backend change.
    set_web_audio_number_slot(
        scope,
        node,
        OUTPUT_COUNT,
        if destination { 0.0 } else { 1.0 },
    );
    set_web_audio_number_slot(scope, node, CHANNEL_COUNT, 2.0);
    let mode = if destination {
        "explicit"
    } else if compressor {
        "clamped-max"
    } else {
        "max"
    };
    set_private_value(scope, node, CHANNEL_MODE, v8str(scope, mode).into());
    set_private_value(scope, node, INTERPRETATION, v8str(scope, "speakers").into());
    super::super::media_queries::mark_simple_event_target_slot(scope, node, LISTENERS);
}

fn context_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(context) = graph::node_context(scope, args.this()) {
        rv.set(context.into());
    }
}

fn metadata_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    if let Some(value) = get_private_value(scope, args.this(), &slot) {
        rv.set(value);
    }
}

fn offline_destination_channel_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<f64> {
    if !web_api_interfaces::AudioDestinationNode::is_instance(scope, node) {
        return None;
    }
    let context = graph::node_context(scope, node)?;
    web_audio_number_slot(scope, context, OFFLINE_AUDIO_CHANNEL_COUNT_SLOT)
}

fn channel_count_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(count) = offline_destination_channel_count(scope, args.this())
        .or_else(|| web_audio_number_slot(scope, args.this(), CHANNEL_COUNT))
    {
        rv.set(v8::Number::new(scope, count).into());
    }
}

fn max_channel_count_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    // The partial realtime backend currently advertises two channels.
    let count = offline_destination_channel_count(scope, args.this()).unwrap_or(2.0);
    rv.set(v8::Number::new(scope, count).into());
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioNode.channelCount")]
struct ChannelCountArgs {
    #[webidl(required)]
    value: u32,
}

fn channel_count_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ChannelCountArgs>(scope, &args) else {
        return;
    };
    let node = args.this();
    let value = parsed.value;
    if let Some(count) = offline_destination_channel_count(scope, node) {
        if f64::from(value) != count {
            throw_dom_exception(
                scope,
                "NotSupportedError",
                9,
                "Offline destination channel count cannot be changed.",
            );
        }
        return;
    }
    if value == 0 {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Audio nodes require at least one channel.",
        );
        return;
    }
    if web_api_interfaces::AudioDestinationNode::is_instance(scope, node) && value > 2 {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "Destination channel count exceeds the backend limit.",
        );
        return;
    }
    if value > 32
        || (web_api_interfaces::DynamicsCompressorNode::is_instance(scope, node) && value > 2)
    {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Audio node channel count exceeds its limit.",
        );
        return;
    }
    set_web_audio_number_slot(scope, node, CHANNEL_COUNT, f64::from(value));
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "ChannelCountMode")]
enum ChannelCountMode {
    #[webidl(token = "max")]
    Max,
    #[webidl(token = "clamped-max")]
    ClampedMax,
    #[webidl(token = "explicit")]
    Explicit,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioNode.channelCountMode")]
struct ChannelModeArgs {
    #[webidl(required)]
    value: String,
}

fn channel_mode_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ChannelModeArgs>(scope, &args) else {
        return;
    };
    // Enum attribute setters convert to DOMString and ignore unknown tokens;
    // enum operation arguments instead throw on an unknown token.
    let Some(value) = <ChannelCountMode as webidl::WebIdlEnum>::parse_token(&parsed.value) else {
        return;
    };
    let node = args.this();
    if offline_destination_channel_count(scope, node).is_some()
        && !matches!(value, ChannelCountMode::Explicit)
    {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "Offline destination channel count mode cannot be changed.",
        );
        return;
    }
    if web_api_interfaces::DynamicsCompressorNode::is_instance(scope, node)
        && matches!(value, ChannelCountMode::Max)
    {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Dynamics compressor does not support max channel count mode.",
        );
        return;
    }
    let value = match value {
        ChannelCountMode::Max => "max",
        ChannelCountMode::ClampedMax => "clamped-max",
        ChannelCountMode::Explicit => "explicit",
    };
    set_private_value(scope, node, CHANNEL_MODE, v8str(scope, value).into());
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "ChannelInterpretation")]
enum ChannelInterpretation {
    #[webidl(token = "speakers")]
    Speakers,
    #[webidl(token = "discrete")]
    Discrete,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioNode.channelInterpretation")]
struct InterpretationArgs {
    #[webidl(required)]
    value: String,
}

fn interpretation_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<InterpretationArgs>(scope, &args) else {
        return;
    };
    let Some(value) = <ChannelInterpretation as webidl::WebIdlEnum>::parse_token(&parsed.value)
    else {
        return;
    };
    let value = match value {
        ChannelInterpretation::Speakers => "speakers",
        ChannelInterpretation::Discrete => "discrete",
    };
    set_private_value(
        scope,
        args.this(),
        INTERPRETATION,
        v8str(scope, value).into(),
    );
}
