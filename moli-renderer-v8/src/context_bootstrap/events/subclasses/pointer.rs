use super::*;
use moli_webapi_declare::WebApiObject;

use crate::context_bootstrap::events::{modifiers::EventModifierInitMembers, ui::UiEventInit};
use crate::context_bootstrap::file_api::is_branded_data_transfer_object;
use crate::web_api_interfaces;
use crate::webidl;

const POINTER_EVENT_COALESCED_EVENTS_SLOT: &str = "__moliPointerEventCoalescedEvents";
const POINTER_EVENT_PREDICTED_EVENTS_SLOT: &str = "__moliPointerEventPredictedEvents";

/// MouseEventInit includes the CSSOM View and Pointer Lock double members.
/// Each derived dictionary delegates to this base before reading its own members.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "MouseEventInit")]
struct MouseEventInitMembers<'s> {
    #[webidl(default = 0)]
    button: i16,
    #[webidl(default = 0)]
    buttons: u16,
    #[webidl(default = 0.0, converter = "double")]
    client_x: f64,
    #[webidl(default = 0.0, converter = "double")]
    client_y: f64,
    #[webidl(default = 0.0, converter = "double")]
    movement_x: f64,
    #[webidl(default = 0.0, converter = "double")]
    movement_y: f64,
    #[webidl(nullable, interface = web_api_interfaces::EventTarget)]
    related_target: Option<v8::Local<'s, v8::Object>>,
    #[webidl(default = 0.0, converter = "double")]
    screen_x: f64,
    #[webidl(default = 0.0, converter = "double")]
    screen_y: f64,
}

#[derive(Default)]
pub(super) struct MouseEventInit<'s> {
    ui: UiEventInit<'s>,
    modifiers: EventModifierInitMembers,
    members: MouseEventInitMembers<'s>,
}

impl<'s> webidl::WebIdlDictionary<'s> for MouseEventInit<'s> {
    fn parse_dictionary(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Result<Self, webidl::WebIdlError> {
        Ok(Self {
            ui: webidl::parse_dictionary_object(scope, object)?,
            modifiers: webidl::parse_dictionary_object(scope, object)?,
            members: webidl::parse_dictionary_object(scope, object)?,
        })
    }
}

impl<'s> MouseEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.ui.event_flags()
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.ui.initialize(scope, event);
        self.modifiers.initialize(scope, event);
        let members = self.members;
        MouseEventBaseInitDeclaration::new(
            members.screen_x,
            members.screen_y,
            members.client_x,
            members.client_y,
            members.client_x,
            members.client_y,
            members.client_x,
            members.client_y,
            members.button,
            members.buttons,
            members.movement_x,
            members.movement_y,
        )
        .initialize(scope, event)
        .expect("MouseEvent state should initialize");
        let related_target = members
            .related_target
            .map(|target| target.into())
            .unwrap_or_else(|| v8::null(scope).into());
        MouseEventRelatedTargetDeclaration::new(related_target)
            .initialize(scope, event)
            .expect("MouseEvent relatedTarget should initialize");
    }
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "WheelEventInit")]
struct WheelEventInitMembers {
    #[webidl(default = 0)]
    delta_mode: u32,
    #[webidl(default = 0.0, converter = "double")]
    delta_x: f64,
    #[webidl(default = 0.0, converter = "double")]
    delta_y: f64,
    #[webidl(default = 0.0, converter = "double")]
    delta_z: f64,
}

#[derive(Default)]
pub(super) struct WheelEventInit<'s> {
    mouse: MouseEventInit<'s>,
    members: WheelEventInitMembers,
}

impl<'s> webidl::WebIdlDictionary<'s> for WheelEventInit<'s> {
    fn parse_dictionary(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Result<Self, webidl::WebIdlError> {
        Ok(Self {
            mouse: webidl::parse_dictionary_object(scope, object)?,
            members: webidl::parse_dictionary_object(scope, object)?,
        })
    }
}

impl<'s> WheelEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.mouse.event_flags()
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.mouse.initialize(scope, event);
        WheelEventDeltaInitDeclaration::new(
            self.members.delta_x,
            self.members.delta_y,
            self.members.delta_z,
            self.members.delta_mode,
        )
        .initialize(scope, event)
        .expect("WheelEvent state should initialize");
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PointerEventInit")]
struct PointerEventInitMembers<'s> {
    #[webidl(converter = "double")]
    altitude_angle: Option<f64>,
    #[webidl(converter = "double")]
    azimuth_angle: Option<f64>,
    #[webidl(sequence, interface = web_api_interfaces::PointerEvent, default = Vec::new())]
    coalesced_events: Vec<v8::Local<'s, v8::Object>>,
    #[webidl(default = 1.0, converter = "double")]
    height: f64,
    #[webidl(default = false)]
    is_primary: bool,
    #[webidl(default = 0)]
    persistent_device_id: i32,
    #[webidl(default = 0)]
    pointer_id: i32,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    pointer_type: webidl::DomString16,
    #[webidl(sequence, interface = web_api_interfaces::PointerEvent, default = Vec::new())]
    predicted_events: Vec<v8::Local<'s, v8::Object>>,
    #[webidl(default = 0.0)]
    pressure: f32,
    #[webidl(default = 0.0)]
    tangential_pressure: f32,
    tilt_x: Option<i32>,
    tilt_y: Option<i32>,
    #[webidl(default = 0)]
    twist: i32,
    #[webidl(default = 1.0, converter = "double")]
    width: f64,
}

impl Default for PointerEventInitMembers<'_> {
    fn default() -> Self {
        Self {
            altitude_angle: None,
            azimuth_angle: None,
            coalesced_events: Vec::new(),
            height: 1.0,
            is_primary: false,
            persistent_device_id: 0,
            pointer_id: 0,
            pointer_type: webidl::DomString16(Vec::new()),
            predicted_events: Vec::new(),
            pressure: 0.0,
            tangential_pressure: 0.0,
            tilt_x: None,
            tilt_y: None,
            twist: 0,
            width: 1.0,
        }
    }
}

#[derive(Default)]
pub(super) struct PointerEventInit<'s> {
    mouse: MouseEventInit<'s>,
    members: PointerEventInitMembers<'s>,
}

impl<'s> webidl::WebIdlDictionary<'s> for PointerEventInit<'s> {
    fn parse_dictionary(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Result<Self, webidl::WebIdlError> {
        Ok(Self {
            mouse: webidl::parse_dictionary_object(scope, object)?,
            members: webidl::parse_dictionary_object(scope, object)?,
        })
    }
}

impl<'s> PointerEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.mouse.event_flags()
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.mouse.initialize(scope, event);
        let angles = pointer_event_angle_init(&self.members);
        let members = self.members;
        PointerEventNumberInitDeclaration::new(
            members.pointer_id,
            members.width,
            members.height,
            members.pressure,
            members.tangential_pressure,
            angles.tilt_x,
            angles.tilt_y,
            angles.azimuth_angle,
            angles.altitude_angle,
            members.twist,
            members.persistent_device_id,
        )
        .initialize(scope, event)
        .expect("PointerEvent state should initialize");
        let pointer_type = crate::util::v8_string_from_utf16_units(scope, &members.pointer_type.0)
            .expect("PointerEvent pointerType should fit in a V8 string");
        PointerEventTailInitDeclaration::new(members.is_primary, pointer_type)
            .initialize(scope, event)
            .expect("PointerEvent tail should initialize");
        store_pointer_event_sequence(
            scope,
            event,
            POINTER_EVENT_COALESCED_EVENTS_SLOT,
            &members.coalesced_events,
        );
        store_pointer_event_sequence(
            scope,
            event,
            POINTER_EVENT_PREDICTED_EVENTS_SLOT,
            &members.predicted_events,
        );
    }
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "DragEventInit")]
struct DragEventInitMembers<'s> {
    #[webidl(nullable, interface = web_api_interfaces::DataTransfer, brand_check = is_branded_data_transfer_object)]
    data_transfer: Option<v8::Local<'s, v8::Object>>,
}

#[derive(Default)]
pub(super) struct DragEventInit<'s> {
    mouse: MouseEventInit<'s>,
    members: DragEventInitMembers<'s>,
}

impl<'s> webidl::WebIdlDictionary<'s> for DragEventInit<'s> {
    fn parse_dictionary(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Result<Self, webidl::WebIdlError> {
        Ok(Self {
            mouse: webidl::parse_dictionary_object(scope, object)?,
            members: webidl::parse_dictionary_object(scope, object)?,
        })
    }
}

impl<'s> DragEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.mouse.event_flags()
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.mouse.initialize(scope, event);
        let data_transfer = self
            .members
            .data_transfer
            .map(|data| data.into())
            .unwrap_or_else(|| v8::null(scope).into());
        DragEventInitDeclaration::new(data_transfer)
            .initialize(scope, event)
            .expect("DragEvent state should initialize");
    }
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct DragEventInitDeclaration<'s> {
    data_transfer: v8::Local<'s, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct MouseEventBaseInitDeclaration {
    screen_x: f64,
    screen_y: f64,
    client_x: f64,
    client_y: f64,
    x: f64,
    y: f64,
    page_x: f64,
    page_y: f64,
    button: i16,
    buttons: u16,
    movement_x: f64,
    movement_y: f64,
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
    delta_mode: u32,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct PointerEventNumberInitDeclaration {
    pointer_id: i32,
    width: f64,
    height: f64,
    pressure: f32,
    tangential_pressure: f32,
    tilt_x: i32,
    tilt_y: i32,
    azimuth_angle: f64,
    altitude_angle: f64,
    twist: i32,
    persistent_device_id: i32,
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

fn pointer_event_angle_init(init: &PointerEventInitMembers<'_>) -> PointerEventAngleInit {
    let tilt_x = init.tilt_x;
    let tilt_y = init.tilt_y;
    let azimuth_angle = init.azimuth_angle;
    let altitude_angle = init.altitude_angle;

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
    angles
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
    let Some(stored) = crate::context_bootstrap::event_private_value(scope, event, slot)
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
) {
    rv.set(pointer_event_sequence_copy(scope, args.this(), slot).into());
}

pub(in crate::context_bootstrap) fn pointer_event_get_coalesced_events_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    pointer_event_sequence_callback(scope, args, rv, POINTER_EVENT_COALESCED_EVENTS_SLOT);
}

pub(in crate::context_bootstrap) fn pointer_event_get_predicted_events_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    pointer_event_sequence_callback(scope, args, rv, POINTER_EVENT_PREDICTED_EVENTS_SLOT);
}
