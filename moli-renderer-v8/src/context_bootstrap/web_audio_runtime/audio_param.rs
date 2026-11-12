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
    #[webapi(slot = DEFAULT_VALUE)]
    default_value: f64,
    #[webapi(slot = MIN_VALUE)]
    min_value: f64,
    #[webapi(slot = MAX_VALUE)]
    max_value: f64,
    #[webapi(method, length = 2, callback = set_value_at_time)]
    set_value_at_time: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AudioParam, enumerable, receiver)]
struct AudioParamPrototypeDeclaration {
    #[webapi(accessor_property, getter = value, setter = set_value)]
    value: (),
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
    let param = AudioParamObjectDeclaration::new(value, min as f32 as f64, max as f32 as f64)
        .bind(scope)
        .expect("AudioParam declaration should bind");
    define_non_enumerable_number_property(scope, param, "value", value);
    initialize_rate(scope, param, "a-rate", false);
    param
}

pub(super) fn wrap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    native: backend::Parameter,
) -> v8::Local<'s, v8::Object> {
    let param = AudioParamObjectDeclaration::new(
        f64::from(native.default_value()),
        f64::from(native.min_value()),
        f64::from(native.max_value()),
    )
    .bind(scope)
    .expect("AudioParam declaration should bind");
    backend::initialize(scope, param, State::Param(native));
    param
}

fn native_parameter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    param: v8::Local<'s, v8::Object>,
) -> Option<backend::Parameter> {
    backend::get(scope, param)?;
    backend::param(scope, param)
}

fn value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(param) = native_parameter(scope, args.this()) {
        rv.set_double(f64::from(param.value()));
    } else {
        let key = v8str(scope, "value");
        let descriptor = args
            .this()
            .get_own_property_descriptor(scope, key.into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
        if let Some(descriptor) = descriptor
            && descriptor.has_own_property(scope, key.into()) == Some(true)
            && let Some(value) = descriptor.get(scope, key.into())
        {
            rv.set(value);
        } else {
            rv.set_double(web_audio_number_slot(scope, args.this(), DEFAULT_VALUE).unwrap_or(0.0));
        }
    }
}

fn set_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(value) = args.get(0).number_value(scope) else {
        return;
    };
    let value = value as f32;
    if !value.is_finite() {
        throw_type_error(scope, "AudioParam value must be a finite float.");
        return;
    }
    if let Some(param) = native_parameter(scope, args.this()) {
        param.set_value(value);
    } else {
        define_non_enumerable_number_property(scope, args.this(), "value", f64::from(value));
    }
}

fn set_value_at_time<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if !web_api_interfaces::AudioParam::is_instance(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation: expected an AudioParam.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<AudioParamSetValueAtTimeArgs>(scope, &args) else {
        return;
    };
    let value = parsed.value as f32;
    if !value.is_finite() || !parsed.start_time.is_finite() {
        throw_type_error(scope, "AudioParam value and time must be finite.");
        return;
    }
    if parsed.start_time < 0.0 {
        throw_range_error(scope, "AudioParam start time must not be negative.");
        return;
    }
    if let Some(param) = native_parameter(scope, args.this()) {
        param.set_value_at_time(value, parsed.start_time);
    } else {
        define_non_enumerable_number_property(scope, args.this(), "value", f64::from(value));
    }
    rv.set(args.this().into());
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
    if let Some(param) = native_parameter(scope, args.this()) {
        rv.set(
            v8str(
                scope,
                match param.automation_rate() {
                    web_audio_api::AutomationRate::A => "a-rate",
                    web_audio_api::AutomationRate::K => "k-rate",
                },
            )
            .into(),
        );
        return;
    }
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
    if let Some(param) = native_parameter(scope, param) {
        let native_rate = match rate {
            "a-rate" => web_audio_api::AutomationRate::A,
            _ => web_audio_api::AutomationRate::K,
        };
        if !param.set_automation_rate(native_rate) {
            throw_dom_exception(
                scope,
                "InvalidStateError",
                11,
                "The AudioParam automation rate cannot be changed.",
            );
        }
        return;
    }
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
