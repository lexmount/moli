use super::*;

const LISTENER: &str = "__moliAudioContextListener";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::AudioListener)]
struct AudioListenerObjectDeclaration {}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AudioListener, enumerable, receiver)]
struct AudioListenerPrototypeDeclaration {
    #[webapi(accessor_property = "positionX", getter = parameter, data = v8str(scope, "positionX"))]
    position_x: (),
    #[webapi(accessor_property = "positionY", getter = parameter, data = v8str(scope, "positionY"))]
    position_y: (),
    #[webapi(accessor_property = "positionZ", getter = parameter, data = v8str(scope, "positionZ"))]
    position_z: (),
    #[webapi(accessor_property = "forwardX", getter = parameter, data = v8str(scope, "forwardX"))]
    forward_x: (),
    #[webapi(accessor_property = "forwardY", getter = parameter, data = v8str(scope, "forwardY"))]
    forward_y: (),
    #[webapi(accessor_property = "forwardZ", getter = parameter, data = v8str(scope, "forwardZ"))]
    forward_z: (),
    #[webapi(accessor_property = "upX", getter = parameter, data = v8str(scope, "upX"))]
    up_x: (),
    #[webapi(accessor_property = "upY", getter = parameter, data = v8str(scope, "upY"))]
    up_y: (),
    #[webapi(accessor_property = "upZ", getter = parameter, data = v8str(scope, "upZ"))]
    up_z: (),
    #[webapi(method, length = 3, callback = set_position)]
    set_position: (),
    #[webapi(method, length = 6, callback = set_orientation)]
    set_orientation: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioListener.setPosition")]
struct SetPositionArgs {
    #[webidl(required)]
    x: f32,
    #[webidl(required)]
    y: f32,
    #[webidl(required)]
    z: f32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioListener.setOrientation")]
struct SetOrientationArgs {
    #[webidl(required)]
    x: f32,
    #[webidl(required)]
    y: f32,
    #[webidl(required)]
    z: f32,
    #[webidl(required)]
    up_x: f32,
    #[webidl(required)]
    up_y: f32,
    #[webidl(required)]
    up_z: f32,
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    AudioListenerPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let context = args.this();
    if let Some(listener) = get_private_value(scope, context, LISTENER) {
        rv.set(listener);
        return;
    }
    // A getter borrowed from another realm still creates the listener and its
    // AudioParams in the owning context's realm.
    let creation_context = context
        .get_creation_context(scope)
        .expect("AudioContext has a creation context");
    let scope = &mut v8::ContextScope::new(scope, creation_context);
    let listener = AudioListenerObjectDeclaration::new()
        .bind(scope)
        .expect("AudioListener declaration should bind");
    for (name, value) in [
        ("positionX", 0.0),
        ("positionY", 0.0),
        ("positionZ", 0.0),
        ("forwardX", 0.0),
        ("forwardY", 0.0),
        ("forwardZ", -1.0),
        ("upX", 0.0),
        ("upY", 1.0),
        ("upZ", 0.0),
    ] {
        let param = audio_param(scope, value, f32::MIN as f64, f32::MAX as f64);
        set_private_value(scope, listener, name, param.into());
    }
    set_private_value(scope, context, LISTENER, listener.into());
    rv.set(listener.into());
}

fn parameter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let name = args.data().to_rust_string_lossy(scope);
    let param =
        get_private_value(scope, args.this(), &name).expect("AudioListener has native parameters");
    rv.set(param);
}

fn set_parameters<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    listener: v8::Local<'s, v8::Object>,
    values: &[(&str, f32)],
) {
    for (name, value) in values {
        let param = get_private_value(scope, listener, name)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .expect("AudioListener has native AudioParams");
        define_non_enumerable_number_property(scope, param, "value", f64::from(*value));
    }
}

fn set_position<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SetPositionArgs>(scope, &args) else {
        return;
    };
    set_parameters(
        scope,
        args.this(),
        &[
            ("positionX", parsed.x),
            ("positionY", parsed.y),
            ("positionZ", parsed.z),
        ],
    );
}

fn set_orientation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SetOrientationArgs>(scope, &args) else {
        return;
    };
    set_parameters(
        scope,
        args.this(),
        &[
            ("forwardX", parsed.x),
            ("forwardY", parsed.y),
            ("forwardZ", parsed.z),
            ("upX", parsed.up_x),
            ("upY", parsed.up_y),
            ("upZ", parsed.up_z),
        ],
    );
}
