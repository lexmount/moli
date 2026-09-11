use super::*;
use moli_webapi_declare::WebApiObject;

use crate::webidl;

const POINTER_EVENT_COALESCED_EVENTS_SLOT: &str = "__moliPointerEventCoalescedEvents";
const POINTER_EVENT_PREDICTED_EVENTS_SLOT: &str = "__moliPointerEventPredictedEvents";

struct PointerEventReference<'scope>(v8::Local<'scope, v8::Object>);

impl<'scope> webidl::WebIdlConverter<'scope> for PointerEventReference<'scope> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'scope, '_>,
        value: v8::Local<'scope, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        let event = webidl::convert::<v8::Local<'scope, v8::Object>>(scope, value, context)?;
        if super::super::event_subclass_kind(scope, event) != Some(EventSubclassKind::PointerEvent)
        {
            return Err(webidl::WebIdlError::custom_message(
                "PointerEventInit sequence members must be PointerEvent objects.",
            ));
        }
        Ok(Self(event))
    }
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct MouseEventBaseInitDeclaration<'scope> {
    view: v8::Local<'scope, v8::Value>,
    detail: f64,
    screen_x: f64,
    screen_y: f64,
    client_x: f64,
    client_y: f64,
    x: f64,
    y: f64,
    page_x: f64,
    page_y: f64,
    button: f64,
    buttons: f64,
    movement_x: f64,
    movement_y: f64,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct MouseEventModifierInitDeclaration {
    ctrl_key: bool,
    shift_key: bool,
    alt_key: bool,
    meta_key: bool,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct MouseEventRelatedTargetDeclaration<'scope> {
    related_target: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct WheelEventDeltaInitDeclaration {
    delta_x: f64,
    delta_y: f64,
    delta_z: f64,
    delta_mode: f64,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct PointerEventNumberInitDeclaration {
    pointer_id: f64,
    width: f64,
    height: f64,
    pressure: f64,
    tangential_pressure: f64,
    tilt_x: f64,
    tilt_y: f64,
    azimuth_angle: f64,
    altitude_angle: f64,
    twist: f64,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct PointerEventTailInitDeclaration<'scope> {
    is_primary: bool,
    pointer_type: v8::Local<'scope, v8::String>,
}

#[derive(Clone, Copy)]
struct PointerEventAngleInit {
    tilt_x: i32,
    tilt_y: i32,
    azimuth_angle: f64,
    altitude_angle: f64,
}

impl Default for PointerEventAngleInit {
    fn default() -> Self {
        Self {
            tilt_x: 0,
            tilt_y: 0,
            azimuth_angle: 0.0,
            altitude_angle: std::f64::consts::FRAC_PI_2,
        }
    }
}

fn normalized_tilt_degrees(tilt_degrees: i32) -> i32 {
    if (-90..=90).contains(&tilt_degrees) {
        return tilt_degrees;
    }
    let mut normalized = tilt_degrees % 180;
    if normalized > 90 {
        normalized -= 180;
    } else if normalized < -90 {
        normalized += 180;
    }
    normalized
}

fn normalized_azimuth_angle(azimuth_angle: f64) -> f64 {
    if (0.0..=std::f64::consts::TAU).contains(&azimuth_angle) {
        return azimuth_angle;
    }
    azimuth_angle.rem_euclid(std::f64::consts::TAU)
}

fn normalized_altitude_angle(altitude_angle: f64) -> f64 {
    if (0.0..=std::f64::consts::FRAC_PI_2).contains(&altitude_angle) {
        return altitude_angle;
    }
    altitude_angle.rem_euclid(std::f64::consts::FRAC_PI_2)
}

fn azimuth_angle_from_tilt(tilt_x_degrees: i32, tilt_y_degrees: i32) -> f64 {
    if tilt_x_degrees == 0 {
        return match tilt_y_degrees.cmp(&0) {
            std::cmp::Ordering::Greater => std::f64::consts::FRAC_PI_2,
            std::cmp::Ordering::Less => 3.0 * std::f64::consts::FRAC_PI_2,
            std::cmp::Ordering::Equal => 0.0,
        };
    }
    if tilt_y_degrees == 0 {
        return if tilt_x_degrees < 0 {
            std::f64::consts::PI
        } else {
            0.0
        };
    }
    if tilt_x_degrees.abs() == 90 || tilt_y_degrees.abs() == 90 {
        return 0.0;
    }

    let tilt_x_radians = f64::from(tilt_x_degrees).to_radians();
    let tilt_y_radians = f64::from(tilt_y_degrees).to_radians();
    tilt_y_radians
        .tan()
        .atan2(tilt_x_radians.tan())
        .rem_euclid(std::f64::consts::TAU)
}

fn altitude_angle_from_tilt(tilt_x_degrees: i32, tilt_y_degrees: i32) -> f64 {
    let tilt_x_radians = f64::from(tilt_x_degrees).to_radians();
    let tilt_y_radians = f64::from(tilt_y_degrees).to_radians();
    if tilt_x_degrees.abs() == 90 || tilt_y_degrees.abs() == 90 {
        return 0.0;
    }
    if tilt_x_degrees == 0 {
        return std::f64::consts::FRAC_PI_2 - tilt_y_radians.abs();
    }
    if tilt_y_degrees == 0 {
        return std::f64::consts::FRAC_PI_2 - tilt_x_radians.abs();
    }
    (1.0 / (tilt_x_radians.tan().powi(2) + tilt_y_radians.tan().powi(2)).sqrt()).atan()
}

fn javascript_round(value: f64) -> i32 {
    (value + 0.5).floor() as i32
}

fn tilt_x_from_spherical(azimuth_angle: f64, altitude_angle: f64) -> i32 {
    if altitude_angle != 0.0 {
        return javascript_round(
            (azimuth_angle.cos() / altitude_angle.tan())
                .atan()
                .to_degrees(),
        );
    }
    if azimuth_angle == std::f64::consts::FRAC_PI_2
        || azimuth_angle == 3.0 * std::f64::consts::FRAC_PI_2
    {
        0
    } else if !(std::f64::consts::FRAC_PI_2..=3.0 * std::f64::consts::FRAC_PI_2)
        .contains(&azimuth_angle)
    {
        90
    } else {
        -90
    }
}

fn tilt_y_from_spherical(azimuth_angle: f64, altitude_angle: f64) -> i32 {
    if altitude_angle != 0.0 {
        return javascript_round(
            (azimuth_angle.sin() / altitude_angle.tan())
                .atan()
                .to_degrees(),
        );
    }
    if azimuth_angle == 0.0
        || azimuth_angle == std::f64::consts::PI
        || azimuth_angle == std::f64::consts::TAU
    {
        0
    } else if azimuth_angle < std::f64::consts::PI {
        90
    } else {
        -90
    }
}

fn pointer_event_angle_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Result<PointerEventAngleInit, webidl::WebIdlError> {
    let Some(init) = init else {
        return Ok(PointerEventAngleInit::default());
    };
    let tilt_x = webidl::optional_member::<webidl::Long>(
        scope,
        init,
        "tiltX",
        webidl::Context::member("PointerEventInit", "tiltX"),
    )?
    .map(|value| value.0);
    let tilt_y = webidl::optional_member::<webidl::Long>(
        scope,
        init,
        "tiltY",
        webidl::Context::member("PointerEventInit", "tiltY"),
    )?
    .map(|value| value.0);
    let azimuth_angle = webidl::optional_member::<webidl::Double>(
        scope,
        init,
        "azimuthAngle",
        webidl::Context::member("PointerEventInit", "azimuthAngle"),
    )?
    .map(|value| value.0);
    let altitude_angle = webidl::optional_member::<webidl::Double>(
        scope,
        init,
        "altitudeAngle",
        webidl::Context::member("PointerEventInit", "altitudeAngle"),
    )?
    .map(|value| value.0);

    let has_tilt = tilt_x.is_some() || tilt_y.is_some();
    let has_spherical_angles = azimuth_angle.is_some() || altitude_angle.is_some();
    let mut angles = PointerEventAngleInit {
        tilt_x: tilt_x.unwrap_or(0),
        tilt_y: tilt_y.unwrap_or(0),
        azimuth_angle: azimuth_angle.unwrap_or(0.0),
        altitude_angle: altitude_angle.unwrap_or(std::f64::consts::FRAC_PI_2),
    };

    if has_tilt && !has_spherical_angles {
        let normalized_tilt_x = normalized_tilt_degrees(angles.tilt_x);
        let normalized_tilt_y = normalized_tilt_degrees(angles.tilt_y);
        angles.azimuth_angle = azimuth_angle_from_tilt(normalized_tilt_x, normalized_tilt_y);
        angles.altitude_angle = altitude_angle_from_tilt(normalized_tilt_x, normalized_tilt_y);
    } else if has_spherical_angles && !has_tilt {
        let normalized_azimuth = normalized_azimuth_angle(angles.azimuth_angle);
        let normalized_altitude = normalized_altitude_angle(angles.altitude_angle);
        angles.tilt_x = tilt_x_from_spherical(normalized_azimuth, normalized_altitude);
        angles.tilt_y = tilt_y_from_spherical(normalized_azimuth, normalized_altitude);
    }
    Ok(angles)
}

fn pointer_event_sequence_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
    key: &'static str,
) -> Result<Vec<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    let Some(init) = init else {
        return Ok(Vec::new());
    };
    webidl::optional_member_or::<webidl::Sequence<PointerEventReference<'s>>>(
        scope,
        init,
        key,
        webidl::Context::member("PointerEventInit", key),
        webidl::Sequence(Vec::new()),
    )
    .map(|sequence| sequence.0.into_iter().map(|event| event.0).collect())
}

fn store_pointer_event_sequence<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    slot: &'static str,
    entries: &[v8::Local<'s, v8::Object>],
) {
    let entries = entries
        .iter()
        .map(|entry| v8::Local::<v8::Value>::from(*entry))
        .collect::<Vec<_>>();
    let values = v8::Array::new_with_elements(scope, &entries);
    set_private_value(scope, event, slot, values.into());
}

fn pointer_event_sequence_copy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> v8::Local<'s, v8::Array> {
    let Some(stored) = get_private_value(scope, event, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return v8::Array::new(scope, 0);
    };
    let entries = (0..stored.length())
        .filter_map(|index| stored.get_index(scope, index))
        .collect::<Vec<_>>();
    v8::Array::new_with_elements(scope, &entries)
}

fn pointer_event_sequence_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    slot: &'static str,
    method: &'static str,
) {
    if super::super::event_subclass_kind(scope, args.this())
        != Some(EventSubclassKind::PointerEvent)
    {
        throw_type_error(
            scope,
            &format!("Failed to execute '{method}' on 'PointerEvent': Illegal invocation"),
        );
        return;
    }
    rv.set(pointer_event_sequence_copy(scope, args.this(), slot).into());
}

pub(in crate::context_bootstrap) fn pointer_event_get_coalesced_events_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    pointer_event_sequence_callback(
        scope,
        args,
        rv,
        POINTER_EVENT_COALESCED_EVENTS_SLOT,
        "getCoalescedEvents",
    );
}

pub(in crate::context_bootstrap) fn pointer_event_get_predicted_events_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    pointer_event_sequence_callback(
        scope,
        args,
        rv,
        POINTER_EVENT_PREDICTED_EVENTS_SLOT,
        "getPredictedEvents",
    );
}

fn define_mouse_event_base_fields<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
    constructor_name: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    let Ok(view) = init_window_view_property(scope, init, constructor_name) else {
        return None;
    };
    let client_x = init_number_property(scope, init, "clientX", 0.0);
    let client_y = init_number_property(scope, init, "clientY", 0.0);
    let detail = init_number_property(scope, init, "detail", 0.0);
    let related_target =
        init_value_property(scope, init, "relatedTarget").unwrap_or_else(|| v8::null(scope).into());

    MouseEventBaseInitDeclaration::new(
        view,
        detail,
        init_number_property(scope, init, "screenX", 0.0),
        init_number_property(scope, init, "screenY", 0.0),
        client_x,
        client_y,
        client_x,
        client_y,
        client_x,
        client_y,
        init_number_property(scope, init, "button", 0.0),
        init_number_property(scope, init, "buttons", 0.0),
        init_number_property(scope, init, "movementX", 0.0),
        init_number_property(scope, init, "movementY", 0.0),
    )
    .initialize(scope, event)
    .expect("MouseEvent base init declaration should initialize");
    Some(related_target)
}

fn define_mouse_event_modifier_fields<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) {
    MouseEventModifierInitDeclaration::new(
        init_bool_property(scope, init, "ctrlKey", false),
        init_bool_property(scope, init, "shiftKey", false),
        init_bool_property(scope, init, "altKey", false),
        init_bool_property(scope, init, "metaKey", false),
    )
    .initialize(scope, event)
    .expect("MouseEvent modifier init declaration should initialize");
}

fn define_mouse_event_related_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    related_target: v8::Local<'s, v8::Value>,
) {
    MouseEventRelatedTargetDeclaration::new(related_target)
        .initialize(scope, event)
        .expect("MouseEvent relatedTarget declaration should initialize");
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_mouse_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let Some(related_target) = define_mouse_event_base_fields(scope, event, init, "MouseEvent")
    else {
        return false;
    };
    define_mouse_event_modifier_fields(scope, event, init);
    define_mouse_event_related_target(scope, event, related_target);
    true
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_wheel_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let Some(related_target) = define_mouse_event_base_fields(scope, event, init, "WheelEvent")
    else {
        return false;
    };
    WheelEventDeltaInitDeclaration::new(
        init_number_property(scope, init, "deltaX", 0.0),
        init_number_property(scope, init, "deltaY", 0.0),
        init_number_property(scope, init, "deltaZ", 0.0),
        init_number_property(scope, init, "deltaMode", 0.0),
    )
    .initialize(scope, event)
    .expect("WheelEvent delta init declaration should initialize");
    define_mouse_event_modifier_fields(scope, event, init);
    define_mouse_event_related_target(scope, event, related_target);
    true
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_pointer_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let pointer_type = init_string_property(scope, init, "pointerType", "");
    let pointer_type = v8_string(scope, &pointer_type).expect("PointerEvent pointerType");
    let coalesced_events = match pointer_event_sequence_member(scope, init, "coalescedEvents") {
        Ok(events) => events,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return false;
        }
    };
    let predicted_events = match pointer_event_sequence_member(scope, init, "predictedEvents") {
        Ok(events) => events,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return false;
        }
    };
    let Some(related_target) = define_mouse_event_base_fields(scope, event, init, "PointerEvent")
    else {
        return false;
    };
    let angles = match pointer_event_angle_init(scope, init) {
        Ok(angles) => angles,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return false;
        }
    };
    PointerEventNumberInitDeclaration::new(
        init_number_property(scope, init, "pointerId", 0.0),
        init_number_property(scope, init, "width", 1.0),
        init_number_property(scope, init, "height", 1.0),
        init_number_property(scope, init, "pressure", 0.0),
        init_number_property(scope, init, "tangentialPressure", 0.0),
        f64::from(angles.tilt_x),
        f64::from(angles.tilt_y),
        angles.azimuth_angle,
        angles.altitude_angle,
        init_number_property(scope, init, "twist", 0.0),
    )
    .initialize(scope, event)
    .expect("PointerEvent number init declaration should initialize");
    define_mouse_event_modifier_fields(scope, event, init);
    PointerEventTailInitDeclaration::new(
        init_bool_property(scope, init, "isPrimary", false),
        pointer_type,
    )
    .initialize(scope, event)
    .expect("PointerEvent tail init declaration should initialize");
    define_mouse_event_related_target(scope, event, related_target);
    store_pointer_event_sequence(
        scope,
        event,
        POINTER_EVENT_COALESCED_EVENTS_SLOT,
        &coalesced_events,
    );
    store_pointer_event_sequence(
        scope,
        event,
        POINTER_EVENT_PREDICTED_EVENTS_SLOT,
        &predicted_events,
    );
    true
}
