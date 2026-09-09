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
    if let Some(destination) = selected
        && node_context(scope, destination).is_none()
    {
        return;
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
