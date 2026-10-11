//! Script-constructed device events. Sensor delivery and permission prompting
//! are separate from these event payloads.

use super::{
    define_event_property, event_private_value, initialize_event_object_with_type,
    initialize_event_wrapper, new_event_state,
};
use crate::context_bootstrap::events::EventInit;
use crate::{
    util::{get_private_value, new_null_prototype_object, throw_type_error, v8str},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use webidl::{SnapshotPrototype, WebIdlDictionarySerialize};

mod permission;
pub(crate) use permission::DevicePermissionTask;

const ALPHA: &str = "__moliDeviceAlpha";
const BETA: &str = "__moliDeviceBeta";
const GAMMA: &str = "__moliDeviceGamma";
const ABSOLUTE: &str = "__moliDeviceAbsolute";
const X: &str = "__moliDeviceAccelerationX";
const Y: &str = "__moliDeviceAccelerationY";
const Z: &str = "__moliDeviceAccelerationZ";
const ACCELERATION: &str = "__moliDeviceAcceleration";
const ACCELERATION_WITH_GRAVITY: &str = "__moliDeviceAccelerationIncludingGravity";
const ROTATION_RATE: &str = "__moliDeviceRotationRate";
const INTERVAL: &str = "__moliDeviceInterval";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DeviceOrientationEvent, enumerable, receiver)]
struct OrientationPrototype {
    #[webapi(accessor_property, getter = event_payload_getter, data = v8str(scope, ALPHA))]
    alpha: (),
    #[webapi(accessor_property, getter = event_payload_getter, data = v8str(scope, BETA))]
    beta: (),
    #[webapi(accessor_property, getter = event_payload_getter, data = v8str(scope, GAMMA))]
    gamma: (),
    #[webapi(accessor_property, getter = event_payload_getter, data = v8str(scope, ABSOLUTE))]
    absolute: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DeviceMotionEvent, enumerable, receiver)]
struct MotionPrototype {
    #[webapi(accessor_property, getter = event_payload_getter, data = v8str(scope, ACCELERATION))]
    acceleration: (),
    #[webapi(accessor_property, getter = event_payload_getter, data = v8str(scope, ACCELERATION_WITH_GRAVITY))]
    acceleration_including_gravity: (),
    #[webapi(accessor_property, getter = event_payload_getter, data = v8str(scope, ROTATION_RATE))]
    rotation_rate: (),
    #[webapi(accessor_property, getter = event_payload_getter, data = v8str(scope, INTERVAL))]
    interval: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::DeviceMotionEventAcceleration)]
struct AccelerationObject<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DeviceMotionEventAcceleration, enumerable, receiver)]
struct AccelerationPrototype {
    #[webapi(accessor_property, getter = vector_getter, data = v8str(scope, X))]
    x: (),
    #[webapi(accessor_property, getter = vector_getter, data = v8str(scope, Y))]
    y: (),
    #[webapi(accessor_property, getter = vector_getter, data = v8str(scope, Z))]
    z: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::DeviceMotionEventRotationRate)]
struct RotationRateObject<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DeviceMotionEventRotationRate, enumerable, receiver)]
struct RotationRatePrototype {
    #[webapi(accessor_property, getter = vector_getter, data = v8str(scope, ALPHA))]
    alpha: (),
    #[webapi(accessor_property, getter = vector_getter, data = v8str(scope, BETA))]
    beta: (),
    #[webapi(accessor_property, getter = vector_getter, data = v8str(scope, GAMMA))]
    gamma: (),
}

// Convert inherited EventInit members first, followed by the derived members
// in Web IDL's lexicographic order, including each nested dictionary in place.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "DeviceOrientationEventInit", snapshot)]
struct OrientationInit {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(default = false)]
    #[webidl(snapshot_slot = ABSOLUTE)]
    absolute: bool,
    #[webidl(nullable, converter = "double")]
    #[webidl(snapshot_slot = ALPHA)]
    alpha: Option<f64>,
    #[webidl(nullable, converter = "double")]
    #[webidl(snapshot_slot = BETA)]
    beta: Option<f64>,
    #[webidl(nullable, converter = "double")]
    #[webidl(snapshot_slot = GAMMA)]
    gamma: Option<f64>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "DeviceMotionEventInit", snapshot)]
struct MotionInit {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(dictionary)]
    #[webidl(snapshot_slot = ACCELERATION, snapshot_with = acceleration_value, snapshot_kind = webidl::SnapshotKind::Identity, snapshot_default = v8::null(scope).into())]
    acceleration: Option<AccelerationInit>,
    #[webidl(name = "accelerationIncludingGravity", dictionary)]
    #[webidl(snapshot_slot = ACCELERATION_WITH_GRAVITY, snapshot_with = acceleration_value, snapshot_kind = webidl::SnapshotKind::Identity, snapshot_default = v8::null(scope).into())]
    acceleration_including_gravity: Option<AccelerationInit>,
    #[webidl(converter = "double", default = 0.0)]
    #[webidl(snapshot_slot = INTERVAL)]
    interval: f64,
    #[webidl(name = "rotationRate", dictionary)]
    #[webidl(snapshot_slot = ROTATION_RATE, snapshot_with = rotation_rate_value, snapshot_kind = webidl::SnapshotKind::Identity, snapshot_default = v8::null(scope).into())]
    rotation_rate: Option<RotationRateInit>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "DeviceMotionEventAccelerationInit", snapshot)]
struct AccelerationInit {
    #[webidl(nullable, converter = "double")]
    #[webidl(snapshot_slot = X)]
    x: Option<f64>,
    #[webidl(nullable, converter = "double")]
    #[webidl(snapshot_slot = Y)]
    y: Option<f64>,
    #[webidl(nullable, converter = "double")]
    #[webidl(snapshot_slot = Z)]
    z: Option<f64>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "DeviceMotionEventRotationRateInit", snapshot)]
struct RotationRateInit {
    #[webidl(nullable, converter = "double")]
    #[webidl(snapshot_slot = ALPHA)]
    alpha: Option<f64>,
    #[webidl(nullable, converter = "double")]
    #[webidl(snapshot_slot = BETA)]
    beta: Option<f64>,
    #[webidl(nullable, converter = "double")]
    #[webidl(snapshot_slot = GAMMA)]
    gamma: Option<f64>,
}

pub(in crate::context_bootstrap) fn install_device_event_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "DeviceOrientationEvent" => {
            permission::install_orientation(scope, template);
            OrientationPrototype::initialize_prototype_template(scope, prototype);
        }
        "DeviceMotionEvent" => {
            permission::install_motion(scope, template);
            MotionPrototype::initialize_prototype_template(scope, prototype);
        }
        "DeviceMotionEventAcceleration" => {
            AccelerationPrototype::initialize_prototype_template(scope, prototype)
        }
        "DeviceMotionEventRotationRate" => {
            RotationRatePrototype::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

fn event_payload_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let name = args.data().to_rust_string_lossy(scope);
    if let Some(value) = event_private_value(scope, args.this(), &name) {
        rv.set(value);
    }
}

fn vector_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let name = args.data().to_rust_string_lossy(scope);
    if let Some(value) = get_private_value(scope, args.this(), &name) {
        rv.set(value);
    }
}

// Nested motion dictionaries produce platform objects, retaining their intrinsic
// prototypes and native brands while their payload slots come from the schema.
fn acceleration_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &AccelerationInit,
    _: SnapshotPrototype,
) -> Option<v8::Local<'s, v8::Value>> {
    let prototype = super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "DeviceMotionEventAcceleration",
    )
    .ok()?;
    let object = AccelerationObject::new(prototype).bind(scope).ok()?;
    value.initialize_snapshot(scope, object)?;
    Some(object.into())
}

fn rotation_rate_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &RotationRateInit,
    _: SnapshotPrototype,
) -> Option<v8::Local<'s, v8::Value>> {
    let prototype = super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "DeviceMotionEventRotationRate",
    )
    .ok()?;
    let object = RotationRateObject::new(prototype).bind(scope).ok()?;
    value.initialize_snapshot(scope, object)?;
    Some(object.into())
}

#[derive(Clone, Copy)]
enum DeviceEventKind {
    Orientation,
    Motion,
}

impl DeviceEventKind {
    fn name(self) -> &'static str {
        match self {
            Self::Orientation => "DeviceOrientationEvent",
            Self::Motion => "DeviceMotionEvent",
        }
    }
}

fn initialize_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: DeviceEventKind,
    dictionary: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Object>,
) -> Result<(bool, bool, bool), webidl::WebIdlError> {
    match kind {
        DeviceEventKind::Orientation => {
            let parsed = webidl::parse_dictionary_object::<OrientationInit>(scope, dictionary)?;
            parsed
                .initialize_snapshot(scope, state)
                .expect("orientation event state");
            Ok((
                parsed.base.bubbles,
                parsed.base.cancelable,
                parsed.base.composed,
            ))
        }
        DeviceEventKind::Motion => {
            let parsed = webidl::parse_dictionary_object::<MotionInit>(scope, dictionary)?;
            parsed
                .initialize_snapshot(scope, state)
                .expect("motion event state");
            Ok((
                parsed.base.bubbles,
                parsed.base.cancelable,
                parsed.base.composed,
            ))
        }
    }
}

fn device_event_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: DeviceEventKind,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, &format!("{} requires 'new'.", kind.name()));
        return;
    }
    if args.length() == 0 {
        throw_type_error(scope, &format!("{} requires a type argument.", kind.name()));
        return;
    }
    let Some(event_type) = args.get(0).to_string(scope) else {
        return;
    };
    let dictionary =
        match webidl::dictionary_arg(&args, 1, webidl::Context::argument(kind.name(), 2)) {
            Ok(dictionary) => dictionary.unwrap_or_else(|| new_null_prototype_object(scope)),
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
        };
    let state = new_event_state(scope);
    let (bubbles, cancelable, composed) = match initialize_payload(scope, kind, dictionary, state) {
        Ok(flags) => flags,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    initialize_event_object_with_type(scope, state, event_type, bubbles, cancelable);
    define_event_property(
        scope,
        state,
        "composed",
        v8::Boolean::new(scope, composed).into(),
    );
    web_api_interfaces::initialize(scope, state, kind.name())
        .expect("device event brand should initialize");
    if initialize_event_wrapper(scope, args.this(), state).is_some() {
        rv.set(args.this().into());
    }
}

pub(in crate::context_bootstrap) fn device_orientation_event_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    device_event_constructor(scope, args, rv, DeviceEventKind::Orientation);
}

pub(in crate::context_bootstrap) fn device_motion_event_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    device_event_constructor(scope, args, rv, DeviceEventKind::Motion);
}
