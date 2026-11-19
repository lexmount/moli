//! Geolocation result values. Permissions, overrides, clocks, and callbacks belong to the embedder.

/// The coordinate snapshot returned by a position provider.
pub struct Coordinates {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: Option<f64>,
    pub accuracy: f64,
    pub altitude_accuracy: Option<f64>,
    pub heading: Option<f64>,
    pub speed: Option<f64>,
}

use moli_webidl as webidl;

use crate::interfaces as web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use moli_v8_util::{
    callback_data_index_value, callback_data_item, get_private_value, throw_type_error, v8str,
};

const COORDINATES_SLOT: &str = "__moliGeolocationCoordinatesValues";
const POSITION_COORDS_SLOT: &str = "__moliGeolocationPositionCoords";
const POSITION_TIME_SLOT: &str = "__moliGeolocationPositionTime";
const COORDINATE_NAMES: &[&str] = &[
    "latitude",
    "longitude",
    "altitude",
    "accuracy",
    "altitudeAccuracy",
    "heading",
    "speed",
];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::GeolocationCoordinates)]
struct CoordinatesDeclaration<'s> {
    #[webapi(slot = COORDINATES_SLOT)]
    values: v8::Local<'s, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::GeolocationPosition)]
struct PositionDeclaration<'s> {
    #[webapi(slot = POSITION_COORDS_SLOT)]
    coords: v8::Local<'s, v8::Object>,
    #[webapi(slot = POSITION_TIME_SLOT)]
    timestamp: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::GeolocationCoordinates, enumerable)]
struct CoordinatesPrototype {
    #[webapi(accessor_property, getter = coordinate_getter, data = callback_data_index_value(scope, 0))]
    latitude: (),
    #[webapi(accessor_property, getter = coordinate_getter, data = callback_data_index_value(scope, 1))]
    longitude: (),
    #[webapi(accessor_property, getter = coordinate_getter, data = callback_data_index_value(scope, 2))]
    altitude: (),
    #[webapi(accessor_property, getter = coordinate_getter, data = callback_data_index_value(scope, 3))]
    accuracy: (),
    #[webapi(accessor_property = "altitudeAccuracy", getter = coordinate_getter, data = callback_data_index_value(scope, 4))]
    altitude_accuracy: (),
    #[webapi(accessor_property, getter = coordinate_getter, data = callback_data_index_value(scope, 5))]
    heading: (),
    #[webapi(accessor_property, getter = coordinate_getter, data = callback_data_index_value(scope, 6))]
    speed: (),
    #[webapi(method, name = "toJSON", length = 0, callback = coordinates_to_json)]
    to_json: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::GeolocationPosition, enumerable)]
struct PositionPrototype {
    #[webapi(accessor_property, getter = position_getter, data = callback_data_index_value(scope, 0))]
    coords: (),
    #[webapi(accessor_property, getter = position_getter, data = callback_data_index_value(scope, 1))]
    timestamp: (),
    #[webapi(method, name = "toJSON", length = 0, callback = position_to_json)]
    to_json: (),
}

pub fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "GeolocationCoordinates" => {
            CoordinatesPrototype::initialize_prototype_template(scope, prototype)
        }
        "GeolocationPosition" => PositionPrototype::initialize_prototype_template(scope, prototype),
        "GeolocationPositionError" => {
            GeolocationPositionErrorPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            GeolocationPositionErrorConstantsDeclaration::initialize_template(scope, template);
            GeolocationPositionErrorConstantsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

pub fn build_position<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    position: &Coordinates,
    timestamp: f64,
) -> v8::Local<'s, v8::Object> {
    let values = [
        Some(position.latitude),
        Some(position.longitude),
        position.altitude,
        Some(position.accuracy),
        position.altitude_accuracy,
        position.heading,
        position.speed,
    ]
    .map(|value| {
        value
            .map(|value| v8::Number::new(scope, value).into())
            .unwrap_or_else(|| v8::null(scope).into())
    });
    let values = v8::Array::new_with_elements(scope, &values);
    let coords = CoordinatesDeclaration::new(values)
        .bind(scope)
        .expect("native coordinates");
    PositionDeclaration::new(coords, timestamp)
        .bind(scope)
        .expect("native position")
}

fn require_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let value = get_private_value(scope, receiver, slot);
    if value.is_none() {
        throw_type_error(scope, "Illegal invocation");
    }
    value
}

fn coordinate_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(values) = require_slot(scope, args.this(), COORDINATES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return;
    };
    let Some(index) = callback_data_item(
        scope,
        &args,
        &[0, 1, 2, 3, 4, 5, 6],
        "GeolocationCoordinates",
    ) else {
        return;
    };
    if let Some(value) = values.get_index(scope, index) {
        rv.set(value);
    }
}

fn position_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        &[POSITION_COORDS_SLOT, POSITION_TIME_SLOT],
        "GeolocationPosition",
    ) else {
        return;
    };
    if let Some(value) = require_slot(scope, args.this(), slot) {
        rv.set(value);
    }
}

fn coordinate_json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    coords: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let values =
        v8::Local::<v8::Array>::try_from(require_slot(scope, coords, COORDINATES_SLOT)?).ok()?;
    let result = v8::Object::new(scope);
    for index in [3, 0, 1, 2, 4, 5, 6] {
        let name = COORDINATE_NAMES[index];
        let value = values.get_index(scope, index as u32)?;
        result
            .create_data_property(scope, v8str(scope, name).into(), value)?
            .then_some(())?;
    }
    Some(result)
}

fn coordinates_to_json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(result) = coordinate_json(scope, args.this()) {
        rv.set(result.into());
    }
}

fn position_to_json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(coords) = require_slot(scope, args.this(), POSITION_COORDS_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(coords) = coordinate_json(scope, coords) else {
        return;
    };
    let Some(timestamp) = require_slot(scope, args.this(), POSITION_TIME_SLOT) else {
        return;
    };
    let result = v8::Object::new(scope);
    let _ = result.create_data_property(scope, v8str(scope, "coords").into(), coords.into());
    let _ = result.create_data_property(scope, v8str(scope, "timestamp").into(), timestamp);
    rv.set(result.into());
}

const GEOLOCATION_POSITION_ERROR_CODE_SLOT: &str = "__moliGeolocationPositionErrorCode";
const GEOLOCATION_POSITION_ERROR_MESSAGE_SLOT: &str = "__moliGeolocationPositionErrorMessage";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::GeolocationPositionError)]
struct GeolocationPositionErrorObjectDeclaration {
    #[webapi(slot = GEOLOCATION_POSITION_ERROR_CODE_SLOT)]
    code: u16,

    #[webapi(slot = GEOLOCATION_POSITION_ERROR_MESSAGE_SLOT)]
    message: String,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::GeolocationPositionError, enumerable)]
struct GeolocationPositionErrorPrototypeDeclaration {
    #[webapi(accessor_property, getter = geolocation_position_error_code_getter_callback)]
    code: (),

    #[webapi(accessor_property, getter = geolocation_position_error_message_getter_callback)]
    message: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::GeolocationPositionError, enumerable)]
struct GeolocationPositionErrorConstantsDeclaration {
    #[webapi(constant = "PERMISSION_DENIED", value = 1u32)]
    permission_denied: (),

    #[webapi(constant = "POSITION_UNAVAILABLE", value = 2u32)]
    position_unavailable: (),

    #[webapi(constant = "TIMEOUT", value = 3u32)]
    timeout: (),
}

fn geolocation_position_error_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if web_api_interfaces::GeolocationPositionError::is_instance(scope, receiver) {
        Some(receiver)
    } else {
        throw_type_error(scope, "Illegal invocation");
        None
    }
}

fn geolocation_position_error_code_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = geolocation_position_error_receiver(scope, args.this()) else {
        return;
    };
    let code = get_private_value(scope, receiver, GEOLOCATION_POSITION_ERROR_CODE_SLOT)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or_default();
    rv.set_uint32(code);
}

fn geolocation_position_error_message_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = geolocation_position_error_receiver(scope, args.this()) else {
        return;
    };
    let message = get_private_value(scope, receiver, GEOLOCATION_POSITION_ERROR_MESSAGE_SLOT)
        .unwrap_or_else(|| v8::String::empty(scope).into());
    rv.set(message);
}

pub fn build_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    code: u16,
    message: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    GeolocationPositionErrorObjectDeclaration::new(code, message.to_owned())
        .bind(scope)
        .ok()
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PositionOptions")]
pub struct PositionOptions {
    #[webidl(name = "enableHighAccuracy", default = false)]
    pub enable_high_accuracy: bool,

    #[webidl(with = clamped_unsigned_long_position_option)]
    pub timeout: u32,

    #[webidl(name = "maximumAge", with = clamped_unsigned_long_position_option)]
    pub maximum_age: u32,
}

fn clamped_unsigned_long_position_option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> std::result::Result<u32, webidl::WebIdlError> {
    let default = if name == "timeout" { u32::MAX } else { 0 };
    let value = webidl::optional_member::<webidl::UnrestrictedDouble>(
        scope,
        object,
        name,
        webidl::Context::member("PositionOptions", name),
    )?;
    Ok(value.map_or(default, |value| clamp_unsigned_long(value.0)))
}

fn clamp_unsigned_long(value: f64) -> u32 {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    if value >= f64::from(u32::MAX) {
        return u32::MAX;
    }
    value.round_ties_even() as u32
}
