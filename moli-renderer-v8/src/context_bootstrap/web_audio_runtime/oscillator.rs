use super::*;
use web_audio_api::node::OscillatorType;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::OscillatorNode, enumerable)]
struct OscillatorPrototype {
    #[webapi(accessor_property = "type", getter = kind, setter = set_kind)]
    kind: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    OscillatorPrototype::initialize_prototype_template(scope, template.prototype_template(scope));
}

fn kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(state) = backend::get(scope, args.this())
        && let State::Node(Node::Oscillator(node)) = &*state.borrow()
    {
        let name = match node.type_() {
            OscillatorType::Sine => "sine",
            OscillatorType::Square => "square",
            OscillatorType::Sawtooth => "sawtooth",
            OscillatorType::Triangle => "triangle",
            OscillatorType::Custom => "custom",
        };
        rv.set(v8str(scope, name).into());
        return;
    }
    throw_type_error(scope, "Illegal invocation: expected an OscillatorNode.");
}

fn set_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(state) = backend::get(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation: expected an OscillatorNode.");
        return;
    };
    if !matches!(&*state.borrow(), State::Node(Node::Oscillator(_))) {
        throw_type_error(scope, "Illegal invocation: expected an OscillatorNode.");
        return;
    }
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    let value = value.to_rust_string_lossy(scope);
    let kind = match value.as_str() {
        "sine" => OscillatorType::Sine,
        "square" => OscillatorType::Square,
        "sawtooth" => OscillatorType::Sawtooth,
        "triangle" => OscillatorType::Triangle,
        "custom" => {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "Use setPeriodicWave to select a custom waveform.",
            );
            return;
        }
        _ => {
            throw_type_error(scope, "Invalid OscillatorType.");
            return;
        }
    };
    if let State::Node(Node::Oscillator(node)) = &mut *state.borrow_mut() {
        node.set_type(kind);
    }
}
