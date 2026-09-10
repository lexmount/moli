use super::*;

// The wrapper retains the actual graph parameter. Public property overrides do
// not replace the value consumed by the audio render thread.
#[derive(WebApiObject)]
#[webapi(interface = "AudioParam")]
struct AudioParamObjectDeclaration {
    #[webapi(method, length = 2, callback = set_value_at_time)]
    set_value_at_time: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "AudioParam", enumerable)]
struct AudioParamPrototypeDeclaration {
    #[webapi(accessor_property, getter = automation_rate, setter = set_automation_rate)]
    automation_rate: (),
    #[webapi(accessor_property, getter = value, setter = set_value)]
    value: (),
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

pub(super) fn wrap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    param: backend::Parameter,
) -> v8::Local<'s, v8::Object> {
    let object = AudioParamObjectDeclaration::new()
        .bind(scope)
        .expect("AudioParam declaration should bind");
    backend::initialize(scope, object, backend::State::Param(param));
    object
}

fn automation_rate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(param) = backend::param(scope, args.this()) {
        let value = match param.automation_rate() {
            web_audio_api::AutomationRate::A => "a-rate",
            web_audio_api::AutomationRate::K => "k-rate",
        };
        rv.set(v8str(scope, value).into());
    }
}

fn set_automation_rate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(param) = backend::param(scope, args.this()) else {
        return;
    };
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    let value = value.to_rust_string_lossy(scope);
    let rate = match value.as_str() {
        "a-rate" => web_audio_api::AutomationRate::A,
        "k-rate" => web_audio_api::AutomationRate::K,
        _ => {
            throw_type_error(scope, "Invalid AutomationRate.");
            return;
        }
    };
    if !param.set_automation_rate(rate) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "This AudioParam has a fixed automation rate.",
        );
    }
}

fn read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    get: fn(&backend::Parameter) -> f32,
) {
    if let Some(param) = backend::param(scope, args.this()) {
        rv.set(v8::Number::new(scope, f64::from(get(&param))).into());
    }
}

fn value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    read(scope, args, rv, backend::Parameter::value);
}
fn default_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    read(scope, args, rv, backend::Parameter::default_value);
}
fn min_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    read(scope, args, rv, backend::Parameter::min_value);
}
fn max_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    read(scope, args, rv, backend::Parameter::max_value);
}

fn finite_float<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<f32> {
    let value = value.number_value(scope)? as f32;
    if !value.is_finite() {
        throw_type_error(scope, "AudioParam value must be a finite float.");
        return None;
    }
    Some(value)
}

fn set_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(param) = backend::param(scope, args.this()) else {
        return;
    };
    let Some(value) = finite_float(scope, args.get(0)) else {
        return;
    };
    param.set_value(value);
}

fn set_value_at_time<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(param) = backend::param(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<AudioParamSetValueAtTimeArgs>(scope, &args) else {
        return;
    };
    if !(parsed.value as f32).is_finite() || !parsed.start_time.is_finite() {
        throw_type_error(scope, "AudioParam value and time must be finite.");
        return;
    }
    if parsed.start_time < 0.0 {
        throw_range_error(scope, "AudioParam start time must not be negative.");
        return;
    }
    param.set_value_at_time(parsed.value as f32, parsed.start_time);
    rv.set(args.this().into());
}
