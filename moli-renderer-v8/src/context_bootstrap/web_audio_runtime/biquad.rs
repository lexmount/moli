use super::*;
use crate::web_api_interfaces;
use web_audio_api::node::BiquadFilterType;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::BiquadFilterNode)]
struct BiquadFilterNodeObjectDeclaration<'scope> {
    #[webapi(data_property, readonly)]
    frequency: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, readonly)]
    detune: v8::Local<'scope, v8::Object>,
    #[webapi(data_property = "Q", readonly)]
    q: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, readonly)]
    gain: v8::Local<'scope, v8::Object>,
    #[webapi(method, length = 1, callback = audio_node_connect_callback)]
    connect: (),
    #[webapi(method, length = 0, callback = audio_node_disconnect_callback)]
    disconnect: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::BiquadFilterNode, enumerable)]
struct BiquadFilterNodePrototypeDeclaration {
    #[webapi(accessor_property = "type", getter = kind, setter = set_kind)]
    kind: (),
    #[webapi(method, length = 3, callback = get_frequency_response)]
    get_frequency_response: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    BiquadFilterNodePrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

pub(super) fn create_biquad_filter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_base_audio_context(scope, args.this()) {
        return;
    }
    let native = backend::create_node(scope, args.this(), NodeKind::Biquad);
    let frequency = audio_param::wrap(scope, native.param("frequency"));
    let detune = audio_param::wrap(scope, native.param("detune"));
    let q = audio_param::wrap(scope, native.param("Q"));
    let gain = audio_param::wrap(scope, native.param("gain"));
    let node = BiquadFilterNodeObjectDeclaration::new(frequency, detune, q, gain)
        .bind(scope)
        .expect("BiquadFilterNode declaration should bind");
    graph::initialize_node(scope, node, args.this());
    backend::initialize(scope, node, State::Node(native));
    rv.set(node.into());
}

fn state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<backend::StateRef> {
    if let Some(state) = backend::get(scope, object)
        && matches!(&*state.borrow(), State::Node(Node::Biquad(_)))
    {
        return Some(state);
    }
    throw_type_error(scope, "Illegal invocation: expected a BiquadFilterNode.");
    None
}

const TYPES: [(&str, BiquadFilterType); 8] = [
    ("lowpass", BiquadFilterType::Lowpass),
    ("highpass", BiquadFilterType::Highpass),
    ("bandpass", BiquadFilterType::Bandpass),
    ("lowshelf", BiquadFilterType::Lowshelf),
    ("highshelf", BiquadFilterType::Highshelf),
    ("peaking", BiquadFilterType::Peaking),
    ("notch", BiquadFilterType::Notch),
    ("allpass", BiquadFilterType::Allpass),
];

fn kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(state) = state(scope, args.this()) else {
        return;
    };
    if let State::Node(Node::Biquad(node)) = &*state.borrow() {
        let name = TYPES
            .iter()
            .find(|(_, kind)| *kind == node.type_())
            .unwrap()
            .0;
        rv.set(v8str(scope, name).into());
    }
}

fn set_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(state) = state(scope, args.this()) else {
        return;
    };
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    let value = value.to_rust_string_lossy(scope);
    let Some((_, kind)) = TYPES.iter().find(|(name, _)| *name == value) else {
        throw_type_error(scope, "Invalid BiquadFilterType.");
        return;
    };
    if let State::Node(Node::Biquad(node)) = &mut *state.borrow_mut() {
        node.set_type(*kind);
    }
}

fn float_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Float32Array>> {
    if let Ok(array) = v8::Local::<v8::Float32Array>::try_from(value)
        && array
            .get_backing_store()
            .is_none_or(|store| !store.is_shared() && !store.is_resizable_by_user_javascript())
    {
        return Some(array);
    }
    throw_type_error(
        scope,
        "BiquadFilterNode requires non-shared, fixed-length Float32Arrays.",
    );
    None
}

fn get_frequency_response<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(state) = state(scope, args.this()) else {
        return;
    };
    let Some(frequencies) = float_array(scope, args.get(0)) else {
        return;
    };
    let Some(magnitudes) = float_array(scope, args.get(1)) else {
        return;
    };
    let Some(phases) = float_array(scope, args.get(2)) else {
        return;
    };
    let length = frequencies.length();
    if magnitudes.length() != length || phases.length() != length {
        throw_dom_exception(
            scope,
            "InvalidAccessError",
            15,
            "Frequency response arrays must have equal lengths.",
        );
        return;
    }
    let mut frequency_values = Vec::with_capacity(length);
    for index in 0..length {
        let Some(value) = frequencies
            .get_index(scope, index as u32)
            .and_then(|value| value.number_value(scope))
        else {
            return;
        };
        frequency_values.push(value as f32);
    }
    let mut magnitude_values = vec![0.0; length];
    let mut phase_values = vec![0.0; length];
    if let State::Node(Node::Biquad(node)) = &*state.borrow() {
        node.get_frequency_response(&frequency_values, &mut magnitude_values, &mut phase_values);
    }
    for index in 0..length {
        let magnitude = v8::Number::new(scope, f64::from(magnitude_values[index]));
        let phase = v8::Number::new(scope, f64::from(phase_values[index]));
        if magnitudes
            .set_index(scope, index as u32, magnitude.into())
            .is_none()
        {
            return;
        }
        if phases
            .set_index(scope, index as u32, phase.into())
            .is_none()
        {
            return;
        }
    }
}
