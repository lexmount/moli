use super::*;

const BIQUAD_FREQUENCY_SLOT: &str = "__moliBiquadFrequency";

#[derive(WebApiObject)]
#[webapi(interface = "BiquadFilterNode")]
struct BiquadFilterNodeObjectDeclaration<'scope> {
    #[webapi(data_property = "type")]
    kind: &'static str,
    #[webapi(data_property, readonly)]
    frequency: v8::Local<'scope, v8::Object>,
    #[webapi(slot = BIQUAD_FREQUENCY_SLOT)]
    internal_frequency: v8::Local<'scope, v8::Object>,
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
#[webapi(name = "BiquadFilterNode", enumerable)]
struct BiquadFilterNodePrototypeDeclaration {
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
    // Match Blink's BiquadFilterNode parameter defaults and float bounds.
    let nyquist = audio_context_sample_rate(scope, args.this()) / 2.0;
    let frequency = audio_param(scope, 350.0, 0.0, nyquist);
    let detune = detune_param(scope);
    let q = audio_param(scope, 1.0, f32::MIN as f64, f32::MAX as f64);
    let gain_max = (40.0_f32 * f32::MAX.log10()) as f64;
    let gain = audio_param(scope, 0.0, f32::MIN as f64, gain_max);
    let node =
        BiquadFilterNodeObjectDeclaration::new("lowpass", frequency, frequency, detune, q, gain)
            .bind(scope)
            .expect("BiquadFilterNode declaration should bind");
    graph::initialize_node(scope, node, args.this());
    rv.set(node.into());
}

fn get_frequency_response<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if get_private_value(scope, args.this(), BIQUAD_FREQUENCY_SLOT).is_none() {
        throw_type_error(scope, "Illegal invocation: expected a BiquadFilterNode.");
        return;
    }
    // Node/AudioParam metadata is available independently of the DSP backend.
    // Do not silently return fabricated response curves for an unimplemented filter.
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "Biquad frequency response calculation is not implemented.",
    );
}
