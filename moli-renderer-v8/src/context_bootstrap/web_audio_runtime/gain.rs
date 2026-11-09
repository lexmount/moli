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

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "GainOptions")]
struct GainOptions {
    #[webidl(inherit)]
    node: node::AudioNodeOptions,
    #[webidl(default = 1.0)]
    gain: f32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "GainNode")]
struct ConstructorArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::BaseAudioContext)]
    context: v8::Local<'s, v8::Object>,
    #[webidl(dictionary)]
    options: GainOptions,
}

pub(in crate::context_bootstrap) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "GainNode constructor requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    if !initialize(scope, args.this(), parsed.context, parsed.options.gain)
        || !parsed.options.node.apply(scope, args.this())
    {
        return;
    }
    crate::util::apply_webidl_constructor_prototype_fallback(
        scope,
        args.this(),
        args.new_target(),
        "GainNode",
    );
    rv.set(args.this().into());
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
    let Ok(node) =
        super::super::exposed_interfaces::build_intrinsic_interface_instance(scope, "GainNode")
    else {
        return;
    };
    if initialize(scope, node, args.this(), 1.0) {
        rv.set(node.into());
    }
}

fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
    value: f32,
) -> bool {
    let gain = audio_param(scope, 1.0, f32::MIN as f64, f32::MAX as f64);
    if gain.create_data_property(
        scope,
        v8str(scope, "value").into(),
        v8::Number::new(scope, f64::from(value)).into(),
    ) != Some(true)
        || GainNodeObjectDeclaration::new(gain)
            .initialize(scope, node)
            .is_err()
    {
        return false;
    }
    graph::initialize_node(scope, node, context);
    true
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
