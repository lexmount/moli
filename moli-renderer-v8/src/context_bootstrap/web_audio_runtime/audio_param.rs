use super::*;
use crate::web_api_interfaces;

const DEFAULT_VALUE: &str = "__moliAudioParamDefaultValue";
const MIN_VALUE: &str = "__moliAudioParamMinValue";
const MAX_VALUE: &str = "__moliAudioParamMaxValue";
const AUTOMATION_RATE: &str = "__moliAudioParamAutomationRate";
const FIXED_AUTOMATION_RATE: &str = "__moliAudioParamFixedAutomationRate";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::AudioParam)]
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
#[webapi(interface = web_api_interfaces::AudioParam, enumerable, receiver)]
struct AudioParamPrototypeDeclaration {
    #[webapi(accessor_property, getter = default_value)]
    default_value: (),
    #[webapi(accessor_property, getter = min_value)]
    min_value: (),
    #[webapi(accessor_property, getter = max_value)]
    max_value: (),
    #[webapi(accessor_property, getter = automation_rate_getter, setter = automation_rate_setter)]
    automation_rate: (),
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
    let param =
        AudioParamObjectDeclaration::new(value, value, min as f32 as f64, max as f32 as f64)
            .bind(scope)
            .expect("AudioParam declaration should bind");
    initialize_rate(scope, param, "a-rate", false);
    param
}

pub(super) fn initialize_rate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    param: v8::Local<'s, v8::Object>,
    rate: &'static str,
    fixed: bool,
) {
    let rate = v8str(scope, rate);
    let fixed = v8::Boolean::new(scope, fixed);
    set_private_value(scope, param, AUTOMATION_RATE, rate.into());
    set_private_value(scope, param, FIXED_AUTOMATION_RATE, fixed.into());
}

fn automation_rate_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(value) = get_private_value(scope, args.this(), AUTOMATION_RATE) {
        rv.set(value);
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioParam.automationRate")]
struct AutomationRateArgs {
    #[webidl(required)]
    value: String,
}

fn automation_rate_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<AutomationRateArgs>(scope, &args) else {
        return;
    };
    let rate = match parsed.value.as_str() {
        "a-rate" => "a-rate",
        "k-rate" => "k-rate",
        _ => return,
    };
    let param = args.this();
    let fixed = get_private_value(scope, param, FIXED_AUTOMATION_RATE)
        .is_some_and(|value| value.boolean_value(scope));
    let current = get_private_value(scope, param, AUTOMATION_RATE)
        .map(|value| value.to_rust_string_lossy(scope));
    if fixed && current.as_deref() != Some(parsed.value.as_str()) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The AudioParam automation rate cannot be changed.",
        );
        return;
    }
    let value = v8str(scope, rate);
    set_private_value(scope, param, AUTOMATION_RATE, value.into());
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
