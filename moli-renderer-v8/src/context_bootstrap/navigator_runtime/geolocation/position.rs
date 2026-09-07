use moli_page_types::GeolocationPositionOverride;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use crate::util::{
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
#[webapi(interface = "GeolocationCoordinates")]
struct CoordinatesDeclaration<'s> {
    #[webapi(slot = COORDINATES_SLOT)]
    values: v8::Local<'s, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(interface = "GeolocationPosition")]
struct PositionDeclaration<'s> {
    #[webapi(slot = POSITION_COORDS_SLOT)]
    coords: v8::Local<'s, v8::Object>,
    #[webapi(slot = POSITION_TIME_SLOT)]
    timestamp: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "GeolocationCoordinates", enumerable)]
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
#[webapi(name = "GeolocationPosition", enumerable)]
struct PositionPrototype {
    #[webapi(accessor_property, getter = position_getter, data = callback_data_index_value(scope, 0))]
    coords: (),
    #[webapi(accessor_property, getter = position_getter, data = callback_data_index_value(scope, 1))]
    timestamp: (),
    #[webapi(method, name = "toJSON", length = 0, callback = position_to_json)]
    to_json: (),
}

pub(super) fn install<'s>(
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
        _ => {}
    }
}

pub(super) fn build<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    position: &GeolocationPositionOverride,
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
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or_default();
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
        let _ = result.create_data_property(scope, v8str(scope, name).into(), value);
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
