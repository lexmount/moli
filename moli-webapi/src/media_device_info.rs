//! Device info interface shims. The current media backend enumerates no devices;
//! exposing these types does not manufacture a camera, microphone or permission.

use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, v8str};
use moli_webapi_declare::ObjectLiteralDeclaration;
use moli_webapi_declare::WebApiFunctionTemplate;

const DEVICE_ID: &str = "__moliMediaDeviceId";
const KIND: &str = "__moliMediaDeviceKind";
const LABEL: &str = "__moliMediaDeviceLabel";
const GROUP_ID: &str = "__moliMediaDeviceGroupId";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaDeviceInfo, enumerable, receiver)]
struct DeviceInfoAttributes {
    #[webapi(accessor_property, getter = info_getter, data = v8str(scope, DEVICE_ID))]
    device_id: (),
    #[webapi(accessor_property, getter = info_getter, data = v8str(scope, KIND))]
    kind: (),
    #[webapi(accessor_property, getter = info_getter, data = v8str(scope, LABEL))]
    label: (),
    #[webapi(accessor_property, getter = info_getter, data = v8str(scope, GROUP_ID))]
    group_id: (),
    #[webapi(method = "toJSON", length = 0, callback = to_json)]
    to_json: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::InputDeviceInfo, enumerable, receiver)]
struct InputDeviceInfoMethods {
    #[webapi(method, length = 0, callback = get_capabilities)]
    get_capabilities: (),
}

pub fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "MediaDeviceInfo" => DeviceInfoAttributes::initialize_prototype_template(scope, prototype),
        "InputDeviceInfo" => {
            InputDeviceInfoMethods::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

fn info_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    if let Some(value) = get_private_value(scope, args.this(), &slot) {
        rv.set(value);
    }
}

fn to_json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let object = ObjectLiteralDeclaration::bind(scope);
    for (name, slot) in [
        ("deviceId", DEVICE_ID),
        ("kind", KIND),
        ("label", LABEL),
        ("groupId", GROUP_ID),
    ] {
        let value = get_private_value(scope, args.this(), slot)
            .unwrap_or_else(|| v8::undefined(scope).into());
        object.set_string_property(scope, name, value);
    }
    rv.set(object.into_value());
}

fn get_capabilities<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    // There is no capture backend or device permission grant yet. An empty
    // capabilities dictionary also represents privacy-filtered input devices.
    rv.set(ObjectLiteralDeclaration::bind(scope).into_value());
}
