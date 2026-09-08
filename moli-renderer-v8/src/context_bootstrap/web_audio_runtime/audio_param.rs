use super::*;

const DEFAULT_VALUE: &str = "__moliAudioParamDefaultValue";
const MIN_VALUE: &str = "__moliAudioParamMinValue";
const MAX_VALUE: &str = "__moliAudioParamMaxValue";

#[derive(WebApiObject)]
#[webapi(interface = "AudioParam")]
struct AudioParamObjectDeclaration {
    #[webapi(data_property)]
    value: f64,
    #[webapi(slot = DEFAULT_VALUE)]
    default_value: f64,
    #[webapi(slot = MIN_VALUE)]
    min_value: f64,
    #[webapi(slot = MAX_VALUE)]
    max_value: f64,
    #[webapi(method, length = 2, callback = audio_param_set_value_at_time_callback)]
    set_value_at_time: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "AudioParam", enumerable)]
struct AudioParamPrototypeDeclaration {
    #[webapi(accessor_property, getter = default_value)]
    default_value: (),
    #[webapi(accessor_property, getter = min_value)]
    min_value: (),
    #[webapi(accessor_property, getter = max_value)]
    max_value: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    AudioParamPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

pub(super) fn audio_param<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: f64,
    min: f64,
    max: f64,
) -> v8::Local<'s, v8::Object> {
    let value = value as f32 as f64;
    AudioParamObjectDeclaration::new(value, value, min as f32 as f64, max as f32 as f64)
        .bind(scope)
        .expect("AudioParam declaration should bind")
}

pub(super) fn detune_param<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let limit = (1200.0_f32 * f32::MAX.log2()) as f64;
    audio_param(scope, 0.0, -limit, limit)
}

fn read_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    slot: &'static str,
) {
    let Some(value) = web_audio_number_slot(scope, args.this(), slot) else {
        throw_type_error(scope, "Illegal invocation: expected an AudioParam.");
        return;
    };
    rv.set(v8::Number::new(scope, value).into());
}

fn default_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    read_metadata(scope, args, rv, DEFAULT_VALUE);
}

fn min_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    read_metadata(scope, args, rv, MIN_VALUE);
}

fn max_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    read_metadata(scope, args, rv, MAX_VALUE);
}
