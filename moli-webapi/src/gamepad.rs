//! Device interface shells and native Gamepad polling without a device backend.
//!
//! Polling returns no devices. Do not create fake device snapshots or actuator
//! results; Event payloads are implemented by the shared value-event bindings.

use crate::interfaces as web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;
use moli_webidl as webidl;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Gamepad, enumerable, receiver)]
struct GamepadPrototypeDeclaration {
    #[webapi(accessor_property, getter = device_backend_unavailable)]
    id: (),
    #[webapi(accessor_property, getter = device_backend_unavailable)]
    index: (),
    #[webapi(accessor_property, getter = device_backend_unavailable)]
    connected: (),
    #[webapi(accessor_property, getter = device_backend_unavailable)]
    timestamp: (),
    #[webapi(accessor_property, getter = device_backend_unavailable)]
    mapping: (),
    #[webapi(accessor_property, getter = device_backend_unavailable)]
    axes: (),
    #[webapi(accessor_property, getter = device_backend_unavailable)]
    buttons: (),
    #[webapi(accessor_property, getter = device_backend_unavailable)]
    touches: (),
    #[webapi(accessor_property, getter = device_backend_unavailable)]
    vibration_actuator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::GamepadButton, enumerable, receiver)]
struct GamepadButtonPrototypeDeclaration {
    #[webapi(accessor_property, getter = device_backend_unavailable)]
    pressed: (),
    #[webapi(accessor_property, getter = device_backend_unavailable)]
    touched: (),
    #[webapi(accessor_property, getter = device_backend_unavailable)]
    value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::GamepadHapticActuator, enumerable, receiver)]
struct GamepadHapticActuatorPrototypeDeclaration {
    #[webapi(accessor_property, getter = device_backend_unavailable)]
    effects: (),
    #[webapi(method, returns_promise, length = 1, callback = device_backend_unavailable)]
    play_effect: (),
    #[webapi(method, returns_promise, length = 0, callback = device_backend_unavailable)]
    reset: (),
}

pub fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Gamepad" => GamepadPrototypeDeclaration::initialize_prototype_template(scope, prototype),
        "GamepadButton" => {
            GamepadButtonPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "GamepadHapticActuator" => {
            GamepadHapticActuatorPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
        _ => {}
    }
}

fn device_backend_unavailable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    webidl::throw_dom_exception(
        scope,
        "NotSupportedError",
        "Gamepad device and haptic backends are not implemented.",
    );
}
