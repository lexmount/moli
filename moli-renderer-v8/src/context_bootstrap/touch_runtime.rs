use super::*;
use crate::util::{callback_data_index_value, get_private_value};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};

mod list;

const TOUCH_IDENTIFIER_SLOT: &str = "__lmTouchIdentifier";
const TOUCH_TARGET_SLOT: &str = "__lmTouchTarget";
const TOUCH_SCREEN_X_SLOT: &str = "__lmTouchScreenX";
const TOUCH_SCREEN_Y_SLOT: &str = "__lmTouchScreenY";
const TOUCH_CLIENT_X_SLOT: &str = "__lmTouchClientX";
const TOUCH_CLIENT_Y_SLOT: &str = "__lmTouchClientY";
const TOUCH_PAGE_X_SLOT: &str = "__lmTouchPageX";
const TOUCH_PAGE_Y_SLOT: &str = "__lmTouchPageY";
const TOUCH_RADIUS_X_SLOT: &str = "__lmTouchRadiusX";
const TOUCH_RADIUS_Y_SLOT: &str = "__lmTouchRadiusY";
const TOUCH_ROTATION_ANGLE_SLOT: &str = "__lmTouchRotationAngle";
const TOUCH_FORCE_SLOT: &str = "__lmTouchForce";
const TOUCH_ALTITUDE_ANGLE_SLOT: &str = "__lmTouchAltitudeAngle";
const TOUCH_AZIMUTH_ANGLE_SLOT: &str = "__lmTouchAzimuthAngle";
const TOUCH_TYPE_SLOT: &str = "__lmTouchType";

const TOUCH_EVENT_TOUCHES_SLOT: &str = "__lmTouchEventTouches";
const TOUCH_EVENT_TARGET_TOUCHES_SLOT: &str = "__lmTouchEventTargetTouches";
const TOUCH_EVENT_CHANGED_TOUCHES_SLOT: &str = "__lmTouchEventChangedTouches";
#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Touch)]
struct TouchObjectDeclaration<'scope> {
    #[webapi(slot = TOUCH_IDENTIFIER_SLOT)]
    identifier: i32,
    #[webapi(slot = TOUCH_TARGET_SLOT)]
    target: v8::Local<'scope, v8::Object>,
    #[webapi(slot = TOUCH_SCREEN_X_SLOT)]
    screen_x: f64,
    #[webapi(slot = TOUCH_SCREEN_Y_SLOT)]
    screen_y: f64,
    #[webapi(slot = TOUCH_CLIENT_X_SLOT)]
    client_x: f64,
    #[webapi(slot = TOUCH_CLIENT_Y_SLOT)]
    client_y: f64,
    #[webapi(slot = TOUCH_PAGE_X_SLOT)]
    page_x: f64,
    #[webapi(slot = TOUCH_PAGE_Y_SLOT)]
    page_y: f64,
    #[webapi(slot = TOUCH_RADIUS_X_SLOT)]
    radius_x: f64,
    #[webapi(slot = TOUCH_RADIUS_Y_SLOT)]
    radius_y: f64,
    #[webapi(slot = TOUCH_ROTATION_ANGLE_SLOT)]
    rotation_angle: f64,
    #[webapi(slot = TOUCH_FORCE_SLOT)]
    force: f64,
    #[webapi(slot = TOUCH_ALTITUDE_ANGLE_SLOT)]
    altitude_angle: f64,
    #[webapi(slot = TOUCH_AZIMUTH_ANGLE_SLOT)]
    azimuth_angle: f64,
    #[webapi(slot = TOUCH_TYPE_SLOT)]
    touch_type: v8::Local<'scope, v8::String>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Touch, receiver)]
struct TouchPrototypeDeclaration {
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 0), enumerable)]
    identifier: (),
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 1), enumerable)]
    target: (),
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 2), enumerable)]
    screen_x: (),
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 3), enumerable)]
    screen_y: (),
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 4), enumerable)]
    client_x: (),
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 5), enumerable)]
    client_y: (),
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 6), enumerable)]
    page_x: (),
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 7), enumerable)]
    page_y: (),
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 8), enumerable)]
    radius_x: (),
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 9), enumerable)]
    radius_y: (),
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 10), enumerable)]
    rotation_angle: (),
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 11), enumerable)]
    force: (),
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 12), enumerable)]
    altitude_angle: (),
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 13), enumerable)]
    azimuth_angle: (),
    #[webapi(accessor_property, getter = touch_getter, data = callback_data_index_value(scope, 14), enumerable)]
    touch_type: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TouchEvent)]
struct TouchEventObjectDeclaration<'scope> {
    #[webapi(slot = TOUCH_EVENT_TOUCHES_SLOT)]
    touches: v8::Local<'scope, v8::Object>,
    #[webapi(slot = TOUCH_EVENT_TARGET_TOUCHES_SLOT)]
    target_touches: v8::Local<'scope, v8::Object>,
    #[webapi(slot = TOUCH_EVENT_CHANGED_TOUCHES_SLOT)]
    changed_touches: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TouchEvent, receiver)]
pub(super) struct TouchEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = touch_event_getter, data = callback_data_index_value(scope, 0), enumerable)]
    touches: (),
    #[webapi(accessor_property, getter = touch_event_getter, data = callback_data_index_value(scope, 1), enumerable)]
    target_touches: (),
    #[webapi(accessor_property, getter = touch_event_getter, data = callback_data_index_value(scope, 2), enumerable)]
    changed_touches: (),
    #[webapi(accessor_property, getter = touch_event_modifier_getter, data = v8str(scope, "altKey"), enumerable)]
    alt_key: (),
    #[webapi(accessor_property, getter = touch_event_modifier_getter, data = v8str(scope, "metaKey"), enumerable)]
    meta_key: (),
    #[webapi(accessor_property, getter = touch_event_modifier_getter, data = v8str(scope, "ctrlKey"), enumerable)]
    ctrl_key: (),
    #[webapi(accessor_property, getter = touch_event_modifier_getter, data = v8str(scope, "shiftKey"), enumerable)]
    shift_key: (),

    #[webapi(method, length = 1, callback = events::event_get_modifier_state_callback, enumerable)]
    get_modifier_state: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Touch")]
struct TouchConstructorArgs<'s> {
    #[webidl(
        required,
        dictionary,
        missing_message = "Failed to construct 'Touch': 1 argument required, but only 0 present."
    )]
    init: TouchInitMembers<'s>,
}

#[derive(Default, webidl::WebIdlEnum)]
#[webidl(name = "TouchType")]
enum TouchType {
    #[default]
    Direct,
    Stylus,
}

impl TouchType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Stylus => "stylus",
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "TouchInit")]
struct TouchInitMembers<'s> {
    #[webidl(required)]
    identifier: i32,
    #[webidl(required, interface = web_api_interfaces::EventTarget)]
    target: v8::Local<'s, v8::Object>,
    #[webidl(converter = "double", default = 0.0)]
    screen_x: f64,
    #[webidl(converter = "double", default = 0.0)]
    screen_y: f64,
    #[webidl(converter = "double", default = 0.0)]
    client_x: f64,
    #[webidl(converter = "double", default = 0.0)]
    client_y: f64,
    #[webidl(converter = "double", default = 0.0)]
    page_x: f64,
    #[webidl(converter = "double", default = 0.0)]
    page_y: f64,
    #[webidl(converter = "float", default = 0.0)]
    radius_x: f32,
    #[webidl(converter = "float", default = 0.0)]
    radius_y: f32,
    #[webidl(converter = "float", default = 0.0)]
    rotation_angle: f32,
    #[webidl(converter = "float", default = 0.0)]
    force: f32,
    #[webidl(converter = "double", default = 0.0)]
    altitude_angle: f64,
    #[webidl(converter = "double", default = 0.0)]
    azimuth_angle: f64,
    #[webidl(converter = "enum", default = TouchType::Direct)]
    touch_type: TouchType,
}

impl<'s> TouchInitMembers<'s> {
    fn declaration(self, scope: &mut v8::PinScope<'s, '_>) -> TouchObjectDeclaration<'s> {
        TouchObjectDeclaration::new(
            self.identifier,
            self.target,
            self.screen_x,
            self.screen_y,
            self.client_x,
            self.client_y,
            self.page_x,
            self.page_y,
            f64::from(self.radius_x),
            f64::from(self.radius_y),
            f64::from(self.rotation_angle),
            f64::from(self.force),
            self.altitude_angle,
            self.azimuth_angle,
            v8str(scope, self.touch_type.as_str()),
        )
    }
}

pub(crate) fn construct_native_touch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    identifier: i32,
    target: v8::Local<'s, v8::Object>,
    x: f64,
    y: f64,
) -> Option<v8::Local<'s, v8::Object>> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    // Native input is already typed. Avoid public constructors and dictionary
    // conversion so author getters cannot run while producing trusted input.
    TouchInitMembers {
        identifier,
        target,
        screen_x: 0.0,
        screen_y: 0.0,
        client_x: x,
        client_y: y,
        page_x: x,
        page_y: y,
        radius_x: 0.0,
        radius_y: 0.0,
        rotation_angle: 0.0,
        force: 0.0,
        altitude_angle: 0.0,
        azimuth_angle: 0.0,
        touch_type: TouchType::Direct,
    }
    .declaration(scope)
    .bind(scope)
    .ok()
}

fn touch_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(slot) = callback_data_item(scope, &args, TOUCH_PROPERTY_SLOTS, "Touch property slots")
    else {
        rv.set_undefined();
        return;
    };
    let receiver = web_api_object_target(scope, args.this()).expect("native Touch receiver");
    match get_private_value(scope, receiver, slot) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

fn touch_event_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        TOUCH_EVENT_PROPERTY_SLOTS,
        "TouchEvent property slots",
    ) else {
        rv.set_undefined();
        return;
    };
    let receiver = web_api_object_target(scope, args.this()).expect("native TouchEvent receiver");
    let receiver = crate::context_bootstrap::event_backing(scope, receiver);
    match get_private_value(scope, receiver, slot) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

fn touch_event_modifier_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = web_api_object_target(scope, args.this()).expect("native TouchEvent receiver");
    let state = events::event_backing(scope, receiver);
    if let Some(value) = state.get(scope, args.data()) {
        rv.set(value);
    }
}

pub(in crate::context_bootstrap) fn touch_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'Touch': Please use the 'new' operator, this DOM object constructor cannot be called as a function.",
        );
        return;
    }
    let touch = args.this();
    let Some(parsed_args) = webidl::parse_args::<TouchConstructorArgs>(scope, &args) else {
        return;
    };
    parsed_args
        .init
        .declaration(scope)
        .initialize(scope, touch)
        .expect("Touch declaration should initialize constructed object");

    rv.set(touch.into());
}

pub(in crate::context_bootstrap) fn initialize_touch_event_lists<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    touches: &[v8::Local<'s, v8::Object>],
    target_touches: &[v8::Local<'s, v8::Object>],
    changed_touches: &[v8::Local<'s, v8::Object>],
) {
    let touches = list::build(scope, touches);
    let target_touches = list::build(scope, target_touches);
    let changed_touches = list::build(scope, changed_touches);
    TouchEventObjectDeclaration::new(touches, target_touches, changed_touches)
        .initialize(scope, event)
        .expect("TouchEvent declaration should initialize object");
}

pub(in crate::context_bootstrap) fn install_touch_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Touch" => TouchPrototypeDeclaration::initialize_prototype_template(scope, prototype),
        "TouchList" => {
            list::install(scope, template);
        }
        _ => {}
    }
}

const TOUCH_PROPERTY_SLOTS: &[&str] = &[
    TOUCH_IDENTIFIER_SLOT,
    TOUCH_TARGET_SLOT,
    TOUCH_SCREEN_X_SLOT,
    TOUCH_SCREEN_Y_SLOT,
    TOUCH_CLIENT_X_SLOT,
    TOUCH_CLIENT_Y_SLOT,
    TOUCH_PAGE_X_SLOT,
    TOUCH_PAGE_Y_SLOT,
    TOUCH_RADIUS_X_SLOT,
    TOUCH_RADIUS_Y_SLOT,
    TOUCH_ROTATION_ANGLE_SLOT,
    TOUCH_FORCE_SLOT,
    TOUCH_ALTITUDE_ANGLE_SLOT,
    TOUCH_AZIMUTH_ANGLE_SLOT,
    TOUCH_TYPE_SLOT,
];

const TOUCH_EVENT_PROPERTY_SLOTS: &[&str] = &[
    TOUCH_EVENT_TOUCHES_SLOT,
    TOUCH_EVENT_TARGET_TOUCHES_SLOT,
    TOUCH_EVENT_CHANGED_TOUCHES_SLOT,
];
