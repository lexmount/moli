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
const RENDERED_INPUT: &str = "__moliAudioNodeRenderedInput";

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "AudioNode", enumerable)]
struct AudioNodePrototypeDeclaration {
    #[webapi(accessor_property, getter = context_getter)]
    context: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    AudioNodePrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

fn context_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(context) = require_node_context(scope, args.this()) {
        rv.set(context.into());
    }
}

pub(super) fn initialize_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
) {
    set_private_value(scope, node, CONTEXT, context.into());
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
}

pub(super) fn set_destination<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
) {
    set_private_value(scope, context, DESTINATION, node.into());
}

fn require_node_context<'s>(
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
    let _ = array.set_index(scope, array.length(), other.into());
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
    for index in 1..args.length().min(3) {
        if args.get(index).uint32_value(scope)? != 0 {
            throw_dom_exception(
                scope,
                "IndexSizeError",
                1,
                "Audio node port index is out of range.",
            );
            return None;
        }
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
    let source = args.this();
    let Some(context) = require_node_context(scope, source) else {
        return;
    };
    let outputs = objects(scope, source, OUTPUTS);
    let selected = if args.length() == 0 {
        None
    } else if args.get(0).is_object() {
        let destination = v8::Local::<v8::Object>::try_from(args.get(0)).unwrap();
        if require_node_context(scope, destination).is_none() {
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
        Some(destination)
    } else {
        let Some(port) = args.get(0).uint32_value(scope) else {
            return;
        };
        if port != 0 {
            throw_dom_exception(
                scope,
                "IndexSizeError",
                1,
                "Audio node port index is out of range.",
            );
            return;
        }
        None
    };
    for index in 1..args.length().min(3) {
        let Some(port) = args.get(index).uint32_value(scope) else {
            return;
        };
        if port != 0 {
            throw_dom_exception(
                scope,
                "IndexSizeError",
                1,
                "Audio node port index is out of range.",
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

pub(super) fn start_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) {
    let node = args.this();
    let Some(start) = web_audio_number_slot(scope, node, START_TIME) else {
        throw_type_error(scope, "Illegal invocation: expected an OscillatorNode.");
        return;
    };
    let when = if args.get(0).is_undefined() {
        0.0
    } else {
        let Some(when) = args.get(0).number_value(scope) else {
            return;
        };
        when
    };
    if !when.is_finite() {
        throw_type_error(scope, "Audio source start time must be finite.");
        return;
    }
    if when < 0.0 {
        throw_range_error(scope, "Audio source start time must not be negative.");
        return;
    }
    if start.is_finite() {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The audio source has already been started.",
        );
        return;
    }
    set_web_audio_number_slot(scope, node, START_TIME, when);
}

fn has_started_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    end_time: f64,
) -> bool {
    let mut pending = vec![node];
    let mut visited = Vec::new();
    while let Some(node) = pending.pop() {
        if visited.contains(&node) {
            continue;
        }
        visited.push(node);
        if web_audio_number_slot(scope, node, START_TIME).is_some_and(|start| start < end_time) {
            return true;
        }
        pending.extend(objects(scope, node, INPUTS));
    }
    false
}

pub(super) fn prepare_offline_render<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    end_time: f64,
) -> bool {
    let destination = web_audio_object_slot(scope, context, DESTINATION);
    let mut pending: Vec<_> = objects(scope, context, ANALYSERS)
        .into_iter()
        .filter(|node| objects(scope, *node, OUTPUTS).is_empty())
        .collect();
    pending.extend(destination);
    let mut visited = Vec::new();
    while let Some(node) = pending.pop() {
        if visited.contains(&node) {
            continue;
        }
        visited.push(node);
        pending.extend(objects(scope, node, INPUTS));
        let has_input = has_started_source(scope, node, end_time);
        let flag = v8::Boolean::new(scope, has_input);
        set_private_value(scope, node, RENDERED_INPUT, flag.into());
    }
    destination.is_some_and(|node| rendered_with_input(scope, node))
}

pub(super) fn rendered_with_input<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, node, RENDERED_INPUT).is_some_and(|value| value.boolean_value(scope))
}
