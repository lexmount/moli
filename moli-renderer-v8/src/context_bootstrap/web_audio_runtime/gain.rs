use super::*;

const GAIN: &str = "__moliGainNodeGain";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::GainNode)]
struct GainNodeObjectDeclaration<'scope> {
    #[webapi(slot = GAIN)]
    gain: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::GainNode, enumerable, receiver)]
struct GainNodePrototypeDeclaration {
    #[webapi(accessor_property, getter = gain)]
    gain: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    GainNodePrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_base_audio_context(scope, args.this()) {
        return;
    }
    let gain = audio_param(scope, 1.0, f32::MIN as f64, f32::MAX as f64);
    let node = GainNodeObjectDeclaration::new(gain)
        .bind(scope)
        .expect("GainNode declaration should bind");
    graph::initialize_node(scope, node, args.this());
    rv.set(node.into());
}

fn gain<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), GAIN) {
        rv.set(value);
    }
}
