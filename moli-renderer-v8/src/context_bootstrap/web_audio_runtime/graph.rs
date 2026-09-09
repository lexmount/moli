//! WebIDL connection validation and GC-traced wrapper relationships.
//! Signal processing and scheduling belong to the native audio graph.

use super::*;

const CONTEXT: &str = "__moliAudioNodeContext";
const ANALYSERS: &str = "__moliAudioContextPullAnalysers";
const DEMANDED: &str = "__moliAudioContextDemandedCompressors";
const DESTINATION: &str = "__moliAudioContextDestination";
const INPUTS: &str = "__moliAudioNodeInputs";
const OUTPUTS: &str = "__moliAudioNodeOutputs";
const START_TIME: &str = "__moliAudioSourceStartTime";

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
    let mut values = objects(scope, node, slot);
    if values.contains(&other) {
        return;
    }
    values.push(other);
    let values: Vec<v8::Local<v8::Value>> = values.into_iter().map(Into::into).collect();
    // Define dense own elements without invoking Array.prototype setters.
    let array = v8::Array::new_with_elements(scope, &values);
    set_private_value(scope, node, slot, array.into());
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
    if !with_connection(scope, source, destination, |source, destination| {
        source.audio_node().connect(destination.audio_node());
    }) {
        return None;
    }
    add_edge(scope, source, OUTPUTS, destination);
    add_edge(scope, destination, INPUTS, source);
    if is_analyser(scope, destination) {
        add_edge(scope, context, ANALYSERS, destination);
    }
    update_pull_demand(scope, context);
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
    let selected = if args.length() == 0 {
        None
    } else if args.get(0).is_object() {
        let destination = v8::Local::<v8::Object>::try_from(args.get(0)).unwrap();
        if require_node_context(scope, destination).is_none() {
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
    // Port conversion can reenter connect/disconnect. Never pass a stale edge
    // to the backend, whose targeted-disconnect contract requires it to exist.
    let outputs = objects(scope, source, OUTPUTS);
    if selected.is_some_and(|destination| !outputs.contains(&destination)) {
        throw_dom_exception(
            scope,
            "InvalidAccessError",
            15,
            "The audio nodes are not connected.",
        );
        return;
    }
    for destination in outputs {
        if selected.is_none_or(|selected| selected == destination) {
            if !with_connection(scope, source, destination, |source, destination| {
                source
                    .audio_node()
                    .disconnect_dest(destination.audio_node());
            }) {
                return;
            }
            remove_edge(scope, source, OUTPUTS, destination);
            remove_edge(scope, destination, INPUTS, source);
            if is_analyser(scope, destination) && objects(scope, destination, INPUTS).is_empty() {
                remove_edge(scope, context, ANALYSERS, destination);
            }
        }
    }
    update_pull_demand(scope, context);
}

pub(super) fn start_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) {
    let node = args.this();
    if web_audio_number_slot(scope, node, START_TIME).is_none() {
        throw_type_error(scope, "Illegal invocation: expected an OscillatorNode.");
        return;
    }
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
    // WebIDL conversion above may run page code and start this same source.
    // Check the live state afterwards, before the algorithm's range check.
    if web_audio_number_slot(scope, node, START_TIME).is_some_and(f64::is_finite) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The audio source has already been started.",
        );
        return;
    }
    if when < 0.0 {
        throw_range_error(scope, "Audio source start time must not be negative.");
        return;
    }
    let Some(state) = backend::get(scope, node) else {
        return;
    };
    if let State::Node(Node::Oscillator(native)) = &mut *state.borrow_mut() {
        native.start_at(when);
    }
    set_web_audio_number_slot(scope, node, START_TIME, when);
}

fn is_analyser<'s>(scope: &mut v8::PinScope<'s, '_>, node: v8::Local<'s, v8::Object>) -> bool {
    backend::get(scope, node)
        .is_some_and(|state| matches!(&*state.borrow(), State::Node(Node::Analyser(_))))
}

fn set_demand<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    active: bool,
) -> bool {
    if let Some(state) = backend::get(scope, node)
        && let State::Node(Node::Compressor { demanded, .. }) = &*state.borrow()
    {
        demanded.store(active, std::sync::atomic::Ordering::Release);
        return true;
    }
    false
}

// Demand and input availability are different: a pulled compressor processes
// silent input too. This only selects processors; it never substitutes PCM or
// decides whether the output is silent from graph reachability.
fn update_pull_demand<'s>(scope: &mut v8::PinScope<'s, '_>, context: v8::Local<'s, v8::Object>) {
    let previous = objects(scope, context, DEMANDED);
    let mut pending = objects(scope, context, ANALYSERS);
    pending.extend(web_audio_object_slot(scope, context, DESTINATION));
    let mut visited = Vec::new();
    let mut demanded = Vec::new();
    while let Some(node) = pending.pop() {
        if visited.contains(&node) {
            continue;
        }
        visited.push(node);
        if backend::get(scope, node)
            .is_some_and(|state| matches!(*state.borrow(), State::ModuleWorklet))
        {
            // Module-only worklets have no DSP pull edge yet. Do not propagate
            // native processor demand across a control-only connection.
            continue;
        }
        pending.extend(objects(scope, node, INPUTS));
        if set_demand(scope, node, true) {
            demanded.push(node.into());
        }
    }
    // Retained processors must not briefly observe false on the audio thread
    // just because an unrelated edge changed in the control thread.
    for node in previous {
        if !visited.contains(&node) {
            set_demand(scope, node, false);
        }
    }
    let nodes = v8::Array::new_with_elements(scope, &demanded);
    set_private_value(scope, context, DEMANDED, nodes.into());
}

fn with_connection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    destination: v8::Local<'s, v8::Object>,
    operation: impl FnOnce(&Node, &Node),
) -> bool {
    if let Some(source) = backend::get(scope, source)
        && let Some(destination) = backend::get(scope, destination)
    {
        match (&*source.borrow(), &*destination.borrow()) {
            (State::Node(source), State::Node(destination)) => {
                operation(source, destination);
                return true;
            }
            (State::ModuleWorklet, State::Node(_) | State::ModuleWorklet)
            | (State::Node(_), State::ModuleWorklet) => {
                // Keep the pre-existing module/MessagePort surface usable.
                // The caller records the logical edge; unlike native-to-native
                // edges, it does not imply that JS worklet DSP is implemented.
                return true;
            }
            _ => {}
        }
    }
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "This AudioNode has no native audio processor.",
    );
    false
}
