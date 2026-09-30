//! Minimal connection/start bookkeeping for the existing audio backend.
//!
//! Reachability can establish silence without pretending to implement DSP. Keep
//! connections in native slots, handle cycles, and snapshot input availability
//! before completion callbacks can disconnect or reconnect nodes.

use super::*;

const CONTEXT: &str = "__moliAudioNodeContext";
const ANALYSERS: &str = "__moliAudioContextActiveAnalysers";
const DESTINATION: &str = "__moliAudioContextDestination";
const INPUTS: &str = "__moliAudioNodeInputs";
const OUTPUTS: &str = "__moliAudioNodeOutputs";
const START_TIME: &str = "__moliAudioSourceStartTime";
const STOP_TIME: &str = "__moliAudioSourceStopTime";
const RENDERED_INPUT: &str = "__moliAudioNodeRenderedInput";

pub(super) fn initialize_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
) {
    set_private_value(scope, node, CONTEXT, context.into());
    node::initialize(scope, node);
    for slot in [INPUTS, OUTPUTS] {
        let array = v8::Array::new(scope, 0);
        set_private_value(scope, node, slot, array.into());
    }
}

pub(super) fn initialize_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) {
    set_web_audio_number_slot(scope, node, START_TIME, f64::INFINITY);
    set_web_audio_number_slot(scope, node, STOP_TIME, f64::INFINITY);
}

pub(super) fn set_destination<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
) {
    set_private_value(scope, context, DESTINATION, node.into());
}

// Receivers are validated by the AudioNode declaration; destination arguments
// are validated by the derived parsers below. This helper only reads graph state.
pub(super) fn node_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let context = web_audio_object_slot(scope, node, CONTEXT);
    if context.is_none() {
        throw_type_error(scope, "Illegal invocation: expected an AudioNode.");
    }
    context
}

fn objects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(array) = web_audio_array_slot(scope, owner, slot) else {
        return Vec::new();
    };
    (0..array.length())
        .filter_map(|index| {
            array
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        })
        .collect()
}

fn add_edge<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    slot: &'static str,
    other: v8::Local<'s, v8::Object>,
) {
    if objects(scope, node, slot).contains(&other) {
        return;
    }
    let array = web_audio_array_slot(scope, node, slot).unwrap_or_else(|| {
        let array = v8::Array::new(scope, 0);
        set_private_value(scope, node, slot, array.into());
        array
    });
    let index = v8::Integer::new_from_unsigned(scope, array.length())
        .to_string(scope)
        .expect("native array index should stringify");
    let _ = array.create_data_property(scope, index.into(), other.into());
}

fn remove_edge<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    slot: &'static str,
    other: v8::Local<'s, v8::Object>,
) {
    let values: Vec<v8::Local<v8::Value>> = objects(scope, node, slot)
        .into_iter()
        .filter(|value| *value != other)
        .map(Into::into)
        .collect();
    let array = v8::Array::new_with_elements(scope, &values);
    set_private_value(scope, node, slot, array.into());
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioNode.connect")]
struct ConnectArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::AudioNode)]
    destination: v8::Local<'s, v8::Object>,
    #[webidl(default = 0)]
    output: u32,
    #[webidl(default = 0)]
    input: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioNode.disconnect")]
struct DisconnectNodeArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::AudioNode)]
    destination: v8::Local<'s, v8::Object>,
    #[webidl(default = 0)]
    output: u32,
    #[webidl(default = 0)]
    input: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioNode.disconnect")]
struct DisconnectOutputArgs {
    #[webidl(required)]
    output: u32,
}

pub(super) fn connect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let parsed = webidl::parse_args::<ConnectArgs>(scope, args)?;
    let source = args.this();
    let destination = parsed.destination;
    let context = node_context(scope, source)?;
    let target_context = node_context(scope, destination)?;
    if context != target_context {
        throw_dom_exception(
            scope,
            "InvalidAccessError",
            15,
            "Audio nodes must belong to the same context.",
        );
        return None;
    }
    if web_audio_object_slot(scope, context, DESTINATION) == Some(source)
        || get_private_value(scope, destination, START_TIME).is_some()
    {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "The audio node has no port in this direction.",
        );
        return None;
    }
    // The remaining node implementations each expose a single output/input port.
    if parsed.output != 0 || parsed.input != 0 {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "Audio node port index is out of range.",
        );
        return None;
    }
    add_edge(scope, source, OUTPUTS, destination);
    add_edge(scope, destination, INPUTS, source);
    if get_private_value(scope, destination, ANALYSER_FFT_SIZE_SLOT).is_some() {
        // Track candidates for offline automatic pull: analysers with input
        // can process even without an output connection.
        add_edge(scope, context, ANALYSERS, destination);
    }
    Some(destination)
}

pub(super) fn disconnect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) {
    // Only the one-argument overload can select an output number. Platform
    // interface identity distinguishes it from a destination; ordinary objects
    // follow unsigned-long conversion, including their numeric coercion hooks.
    let destination_overload = if args.length() == 1 {
        match v8::Local::<v8::Object>::try_from(args.get(0)) {
            Ok(object) => {
                web_api_interfaces::AudioNode::is_instance(scope, object)
                    || web_api_interfaces::AudioParam::is_instance(scope, object)
            }
            Err(_) => false,
        }
    } else {
        args.length() > 1
    };
    let (selected, output, input) = if destination_overload {
        let Some(parsed) = webidl::parse_args::<DisconnectNodeArgs>(scope, args) else {
            return;
        };
        (Some(parsed.destination), parsed.output, parsed.input)
    } else if args.length() == 1 {
        let Some(parsed) = webidl::parse_args::<DisconnectOutputArgs>(scope, args) else {
            return;
        };
        (None, parsed.output, 0)
    } else {
        (None, 0, 0)
    };
    // Finish conversion before observing graph state, validating ports, or
    // removing any edge. A later conversion exception must leave edges intact.
    let source = args.this();
    let Some(context) = node_context(scope, source) else {
        return;
    };
    if output != 0 || input != 0 {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "Audio node port index is out of range.",
        );
        return;
    }
    let outputs = objects(scope, source, OUTPUTS);
    if let Some(destination) = selected {
        if node_context(scope, destination).is_none() {
            return;
        }
        if !outputs.contains(&destination) {
            throw_dom_exception(
                scope,
                "InvalidAccessError",
                15,
                "The audio nodes are not connected.",
            );
            return;
        }
    }
    for destination in outputs {
        if selected.is_none_or(|selected| selected == destination) {
            remove_edge(scope, source, OUTPUTS, destination);
            remove_edge(scope, destination, INPUTS, source);
            if get_private_value(scope, destination, ANALYSER_FFT_SIZE_SLOT).is_some()
                && objects(scope, destination, INPUTS).is_empty()
            {
                remove_edge(scope, context, ANALYSERS, destination);
            }
        }
    }
}

pub(super) fn source_started<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    web_audio_number_slot(scope, node, START_TIME).is_some_and(f64::is_finite)
}

pub(super) fn start_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    when: f64,
    extra_times: &[f64],
) -> bool {
    // The bindings have already converted every restricted-double argument.
    // The source-started state is checked before the operation's range rules.
    if source_started(scope, node) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The audio source has already been started.",
        );
        return false;
    }
    if when < 0.0 || extra_times.iter().any(|value| *value < 0.0) {
        throw_range_error(scope, "Audio source start arguments must not be negative.");
        return false;
    }
    set_web_audio_number_slot(scope, node, START_TIME, when);
    true
}

pub(super) fn stop_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    when: f64,
) {
    if !source_started(scope, node) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The audio source has not been started.",
        );
        return;
    }
    if when < 0.0 {
        throw_range_error(scope, "Audio source stop time must not be negative.");
        return;
    }
    set_web_audio_number_slot(scope, node, STOP_TIME, when);
}

fn has_started_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    end_time: f64,
) -> Option<bool> {
    let mut pending = vec![node];
    let mut has_input = false;
    let mut visited = Vec::new();
    while let Some(node) = pending.pop() {
        if visited.contains(&node) {
            continue;
        }
        visited.push(node);
        let active = web_audio_number_slot(scope, node, START_TIME).is_some_and(|start| {
            start < end_time
                && web_audio_number_slot(scope, node, STOP_TIME).is_some_and(|stop| stop > start)
        });
        if active {
            if source::needs_rendering_backend(scope, node) {
                throw_dom_exception(
                    scope,
                    "NotSupportedError",
                    9,
                    "PCM rendering for this audio source is not implemented.",
                );
                return None;
            }
            // Only the pre-existing oscillator backend supplies its synthetic
            // signal. A null-buffer source must continue to render silence.
            has_input |= web_api_interfaces::OscillatorNode::is_instance(scope, node);
        }
        pending.extend(objects(scope, node, INPUTS));
    }
    Some(has_input)
}

pub(super) fn prepare_offline_render<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    end_time: f64,
) -> Option<bool> {
    let destination = web_audio_object_slot(scope, context, DESTINATION);
    let mut pending: Vec<_> = objects(scope, context, ANALYSERS)
        .into_iter()
        .filter(|node| objects(scope, *node, OUTPUTS).is_empty())
        .collect();
    pending.extend(destination);
    let mut visited = Vec::new();
    let mut rendered_inputs = Vec::new();
    while let Some(node) = pending.pop() {
        if visited.contains(&node) {
            continue;
        }
        visited.push(node);
        pending.extend(objects(scope, node, INPUTS));
        let has_input = has_started_source(scope, node, end_time)?;
        rendered_inputs.push((node, has_input));
    }
    // Reject unsupported processing before committing any render snapshot.
    for (node, has_input) in rendered_inputs {
        let flag = v8::Boolean::new(scope, has_input);
        set_private_value(scope, node, RENDERED_INPUT, flag.into());
    }
    Some(destination.is_some_and(|node| rendered_with_input(scope, node)))
}

pub(super) fn rendered_with_input<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, node, RENDERED_INPUT).is_some_and(|value| value.boolean_value(scope))
}
