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
const EDGE_SOURCE: &str = "__moliAudioConnectionSource";
const EDGE_DESTINATION: &str = "__moliAudioConnectionDestination";
const EDGE_OUTPUT: &str = "__moliAudioConnectionOutput";
const EDGE_INPUT: &str = "__moliAudioConnectionInput";
const START_TIME: &str = "__moliAudioSourceStartTime";
const STOP_TIME: &str = "__moliAudioSourceStopTime";
const RENDERED_INPUT: &str = "__moliAudioNodeRenderedInput";
const UNSUPPORTED_PROCESSOR: &str = "__moliAudioNodeNeedsProcessingBackend";

pub(super) fn mark_unsupported_processor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) {
    let required = v8::Boolean::new(scope, true);
    set_private_value(scope, node, UNSUPPORTED_PROCESSOR, required.into());
}

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

pub(super) fn require_node_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !web_api_interfaces::AudioNode::is_instance(scope, node) {
        throw_type_error(scope, "Illegal invocation: expected an AudioNode.");
        return None;
    }
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

// Both endpoints retain the same GC-managed record. Port identity is necessary
// for duplicate connections and for disconnecting only one of several routes.
struct Connection<'s> {
    record: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Object>,
    destination: v8::Local<'s, v8::Object>,
    output: u32,
    input: u32,
}

fn connections<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Vec<Connection<'s>> {
    objects(scope, node, slot)
        .into_iter()
        .filter_map(|record| {
            Some(Connection {
                record,
                source: web_audio_object_slot(scope, record, EDGE_SOURCE)?,
                destination: web_audio_object_slot(scope, record, EDGE_DESTINATION)?,
                output: web_audio_number_slot(scope, record, EDGE_OUTPUT)? as u32,
                input: web_audio_number_slot(scope, record, EDGE_INPUT)? as u32,
            })
        })
        .collect()
}

fn input_nodes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    connections(scope, node, INPUTS)
        .into_iter()
        .map(|connection| connection.source)
        .collect()
}

fn port_in_range(scope: &mut v8::PinScope<'_, '_>, port: u32, count: u32) -> bool {
    if port >= count {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "Audio node port index is out of range.",
        );
        return false;
    }
    true
}

pub(super) fn connect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let source = args.this();
    let context = require_node_context(scope, source)?;
    let Ok(destination) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_type_error(
            scope,
            "AudioNode.connect requires an AudioNode destination.",
        );
        return None;
    };
    let target_context = require_node_context(scope, destination)?;
    // Convert all Web IDL arguments before checking graph state or port ranges.
    let output = args.get(1).uint32_value(scope)?;
    let input = args.get(2).uint32_value(scope)?;
    let output_count = node::output_count(scope, source);
    let input_count = node::input_count(scope, destination);
    if !port_in_range(scope, output, output_count) || !port_in_range(scope, input, input_count) {
        return None;
    }
    if context != target_context {
        throw_dom_exception(
            scope,
            "InvalidAccessError",
            15,
            "Audio nodes must belong to the same context.",
        );
        return None;
    }
    if connections(scope, source, OUTPUTS)
        .iter()
        .any(|connection| {
            connection.destination == destination
                && connection.output == output
                && connection.input == input
        })
    {
        return Some(destination);
    }
    let record = v8::Object::new(scope);
    set_private_value(scope, record, EDGE_SOURCE, source.into());
    set_private_value(scope, record, EDGE_DESTINATION, destination.into());
    set_web_audio_number_slot(scope, record, EDGE_OUTPUT, f64::from(output));
    set_web_audio_number_slot(scope, record, EDGE_INPUT, f64::from(input));
    add_edge(scope, source, OUTPUTS, record);
    add_edge(scope, destination, INPUTS, record);
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
    let source = args.this();
    let Some(context) = require_node_context(scope, source) else {
        return;
    };
    let Some(selection) = DisconnectSelection::parse(scope, args) else {
        return;
    };
    if let Some(output) = selection.output {
        let count = node::output_count(scope, source);
        if !port_in_range(scope, output, count) {
            return;
        }
    }
    if let (Some(destination), Some(input)) = (selection.destination, selection.input) {
        let count = node::input_count(scope, destination);
        if !port_in_range(scope, input, count) {
            return;
        }
    }
    let selected: Vec<_> = connections(scope, source, OUTPUTS)
        .into_iter()
        .filter(|connection| selection.matches(connection))
        .collect();
    if selection.destination.is_some() && selected.is_empty() {
        throw_dom_exception(
            scope,
            "InvalidAccessError",
            15,
            "The selected audio ports are not connected.",
        );
        return;
    }
    for connection in selected {
        remove_edge(scope, source, OUTPUTS, connection.record);
        remove_edge(scope, connection.destination, INPUTS, connection.record);
        if get_private_value(scope, connection.destination, ANALYSER_FFT_SIZE_SLOT).is_some()
            && objects(scope, connection.destination, INPUTS).is_empty()
        {
            remove_edge(scope, context, ANALYSERS, connection.destination);
        }
    }
}

#[derive(Default)]
struct DisconnectSelection<'s> {
    destination: Option<v8::Local<'s, v8::Object>>,
    output: Option<u32>,
    input: Option<u32>,
}

impl<'s> DisconnectSelection<'s> {
    fn parse(
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
    ) -> Option<Self> {
        if args.length() == 0 {
            return Some(Self::default());
        }
        let value = args.get(0);
        // The single-argument overload accepts a number, including objects
        // convertible to a number. Author Proxies do not acquire native brands.
        let interface = v8::Local::<v8::Object>::try_from(value)
            .ok()
            .is_some_and(|object| {
                web_api_interfaces::AudioNode::is_instance(scope, object)
                    || web_api_interfaces::AudioParam::is_instance(scope, object)
            });
        if args.length() == 1 && !interface {
            return Some(Self {
                output: Some(value.uint32_value(scope)?),
                ..Self::default()
            });
        }
        let Ok(destination) = v8::Local::<v8::Object>::try_from(value) else {
            throw_type_error(
                scope,
                "AudioNode.disconnect requires an AudioNode destination.",
            );
            return None;
        };
        require_node_context(scope, destination)?;
        // Complete every conversion before checking ranges or connection state.
        let output = if args.length() >= 2 {
            Some(args.get(1).uint32_value(scope)?)
        } else {
            None
        };
        let input = if args.length() >= 3 {
            Some(args.get(2).uint32_value(scope)?)
        } else {
            None
        };
        Some(Self {
            destination: Some(destination),
            output,
            input,
        })
    }

    fn matches(&self, connection: &Connection<'s>) -> bool {
        self.destination
            .is_none_or(|destination| destination == connection.destination)
            && self.output.is_none_or(|output| output == connection.output)
            && self.input.is_none_or(|input| input == connection.input)
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
        pending.extend(input_nodes(scope, node));
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
        pending.extend(input_nodes(scope, node));
        let has_input = has_started_source(scope, node, end_time)?;
        if has_input
            && get_private_value(scope, node, UNSUPPORTED_PROCESSOR)
                .is_some_and(|value| value.boolean_value(scope))
        {
            throw_dom_exception(
                scope,
                "NotSupportedError",
                9,
                "Signal processing for this audio node is not implemented.",
            );
            return None;
        }
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
