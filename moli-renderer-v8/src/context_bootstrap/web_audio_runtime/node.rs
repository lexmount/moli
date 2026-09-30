//! Shared AudioNode bindings and native channel metadata for the partial backend.

use super::*;

const INPUT_COUNT: &str = "__moliAudioNodeInputCount";
const OUTPUT_COUNT: &str = "__moliAudioNodeOutputCount";
const CHANNEL_COUNT: &str = "__moliAudioNodeChannelCount";
const CHANNEL_MODE: &str = "__moliAudioNodeChannelMode";
const INTERPRETATION: &str = "__moliAudioNodeChannelInterpretation";
const LISTENERS: &str = "__moliAudioNodeListeners";

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "AudioNodeOptions")]
pub(super) struct AudioNodeOptions {
    channel_count: Option<u32>,
    #[webidl(converter = "enum")]
    channel_count_mode: Option<ChannelCountMode>,
    #[webidl(converter = "enum")]
    channel_interpretation: Option<ChannelInterpretation>,
}

pub(super) fn apply_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    options: AudioNodeOptions,
) -> bool {
    if let Some(count) = options.channel_count
        && !set_channel_count(scope, node, count)
    {
        return false;
    }
    if let Some(mode) = options.channel_count_mode
        && !set_channel_mode(scope, node, mode)
    {
        return false;
    }
    if let Some(interpretation) = options.channel_interpretation
        && !set_interpretation(scope, node, interpretation)
    {
        return false;
    }
    true
}

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
    let source = web_api_interfaces::AudioScheduledSourceNode::is_instance(scope, node);
    let destination = web_api_interfaces::AudioDestinationNode::is_instance(scope, node);
    let clamped = has_stereo_channel_limit(scope, node);
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
    } else if clamped {
        "clamped-max"
    } else {
        "max"
    };
    set_private_value(scope, node, CHANNEL_MODE, v8str(scope, mode).into());
    set_private_value(scope, node, INTERPRETATION, v8str(scope, "speakers").into());
    super::super::media_queries::mark_simple_event_target_slot(scope, node, LISTENERS);
}

pub(super) enum ChannelLayout {
    Merger(u32),
    Splitter(u32),
}

pub(super) fn initialize_channel_layout<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    layout: ChannelLayout,
) {
    let (inputs, outputs, channels, interpretation) = match layout {
        ChannelLayout::Merger(count) => (count, 1, 1, "speakers"),
        ChannelLayout::Splitter(count) => (1, count, count, "discrete"),
    };
    set_web_audio_number_slot(scope, node, INPUT_COUNT, f64::from(inputs));
    set_web_audio_number_slot(scope, node, OUTPUT_COUNT, f64::from(outputs));
    set_web_audio_number_slot(scope, node, CHANNEL_COUNT, f64::from(channels));
    set_private_value(scope, node, CHANNEL_MODE, v8str(scope, "explicit").into());
    set_private_value(
        scope,
        node,
        INTERPRETATION,
        v8str(scope, interpretation).into(),
    );
}

pub(super) fn input_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> u32 {
    web_audio_number_slot(scope, node, INPUT_COUNT).unwrap_or(0.0) as u32
}

pub(super) fn output_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> u32 {
    web_audio_number_slot(scope, node, OUTPUT_COUNT).unwrap_or(0.0) as u32
}

fn fixed_channel_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    if web_api_interfaces::ChannelMergerNode::is_instance(scope, node) {
        Some(1)
    } else if web_api_interfaces::ChannelSplitterNode::is_instance(scope, node) {
        Some(output_count(scope, node))
    } else {
        None
    }
}

fn context_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(context) = graph::require_node_context(scope, args.this()) {
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
    let context = graph::require_node_context(scope, node)?;
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
    set_channel_count(scope, args.this(), parsed.value);
}

fn has_stereo_channel_limit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::DynamicsCompressorNode::is_instance(scope, node)
        || web_api_interfaces::StereoPannerNode::is_instance(scope, node)
}

fn set_channel_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    value: u32,
) -> bool {
    if let Some(count) = fixed_channel_count(scope, node) {
        if value != count {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "The channel count of this node is fixed.",
            );
        }
        return value == count;
    }
    if let Some(count) = offline_destination_channel_count(scope, node) {
        if f64::from(value) != count {
            throw_dom_exception(
                scope,
                "NotSupportedError",
                9,
                "Offline destination channel count cannot be changed.",
            );
        }
        return f64::from(value) == count;
    }
    if value == 0 {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Audio nodes require at least one channel.",
        );
        return false;
    }
    if web_api_interfaces::AudioDestinationNode::is_instance(scope, node) && value > 2 {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "Destination channel count exceeds the backend limit.",
        );
        return false;
    }
    if value > 32 || (has_stereo_channel_limit(scope, node) && value > 2) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Audio node channel count exceeds its limit.",
        );
        return false;
    }
    set_web_audio_number_slot(scope, node, CHANNEL_COUNT, f64::from(value));
    true
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "ChannelCountMode")]
pub(super) enum ChannelCountMode {
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
    set_channel_mode(scope, args.this(), value);
}

fn set_channel_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    value: ChannelCountMode,
) -> bool {
    if (fixed_channel_count(scope, node).is_some()
        || offline_destination_channel_count(scope, node).is_some())
        && !matches!(value, ChannelCountMode::Explicit)
    {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "This audio node requires explicit channel count mode.",
        );
        return false;
    }
    if has_stereo_channel_limit(scope, node) && matches!(value, ChannelCountMode::Max) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "This audio node does not support max channel count mode.",
        );
        return false;
    }
    let value = match value {
        ChannelCountMode::Max => "max",
        ChannelCountMode::ClampedMax => "clamped-max",
        ChannelCountMode::Explicit => "explicit",
    };
    set_private_value(scope, node, CHANNEL_MODE, v8str(scope, value).into());
    true
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "ChannelInterpretation")]
pub(super) enum ChannelInterpretation {
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
    set_interpretation(scope, args.this(), value);
}

fn set_interpretation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    value: ChannelInterpretation,
) -> bool {
    if web_api_interfaces::ChannelSplitterNode::is_instance(scope, node)
        && !matches!(value, ChannelInterpretation::Discrete)
    {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "A channel splitter requires discrete channel interpretation.",
        );
        return false;
    }
    let value = match value {
        ChannelInterpretation::Speakers => "speakers",
        ChannelInterpretation::Discrete => "discrete",
    };
    set_private_value(scope, node, INTERPRETATION, v8str(scope, value).into());
    true
}
