use super::*;
use crate::util::{callback_data_index_value, get_private_value};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};
use webidl::WebIdlDictionarySerialize;

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

const TOUCH_EVENT_TOUCHES_SLOT: &str = "__lmTouchEventTouches";
const TOUCH_EVENT_TARGET_TOUCHES_SLOT: &str = "__lmTouchEventTargetTouches";
const TOUCH_EVENT_CHANGED_TOUCHES_SLOT: &str = "__lmTouchEventChangedTouches";
const TOUCH_EVENT_ALT_KEY_SLOT: &str = "__lmTouchEventAltKey";
const TOUCH_EVENT_META_KEY_SLOT: &str = "__lmTouchEventMetaKey";
const TOUCH_EVENT_CTRL_KEY_SLOT: &str = "__lmTouchEventCtrlKey";
const TOUCH_EVENT_SHIFT_KEY_SLOT: &str = "__lmTouchEventShiftKey";

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
    #[webapi(slot = TOUCH_EVENT_ALT_KEY_SLOT)]
    alt_key: bool,
    #[webapi(slot = TOUCH_EVENT_META_KEY_SLOT)]
    meta_key: bool,
    #[webapi(slot = TOUCH_EVENT_CTRL_KEY_SLOT)]
    ctrl_key: bool,
    #[webapi(slot = TOUCH_EVENT_SHIFT_KEY_SLOT)]
    shift_key: bool,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TouchEvent, receiver)]
struct TouchEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = touch_event_getter, data = callback_data_index_value(scope, 0), enumerable)]
    touches: (),
    #[webapi(accessor_property, getter = touch_event_getter, data = callback_data_index_value(scope, 1), enumerable)]
    target_touches: (),
    #[webapi(accessor_property, getter = touch_event_getter, data = callback_data_index_value(scope, 2), enumerable)]
    changed_touches: (),
    #[webapi(accessor_property, getter = touch_event_getter, data = callback_data_index_value(scope, 3), enumerable)]
    alt_key: (),
    #[webapi(accessor_property, getter = touch_event_getter, data = callback_data_index_value(scope, 4), enumerable)]
    meta_key: (),
    #[webapi(accessor_property, getter = touch_event_getter, data = callback_data_index_value(scope, 5), enumerable)]
    ctrl_key: (),
    #[webapi(accessor_property, getter = touch_event_getter, data = callback_data_index_value(scope, 6), enumerable)]
    shift_key: (),
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

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "TouchInit", snapshot)]
struct TouchInitMembers<'s> {
    #[webidl(required)]
    #[webidl(snapshot_slot = TOUCH_IDENTIFIER_SLOT)]
    identifier: i32,
    #[webidl(required, interface = web_api_interfaces::EventTarget)]
    #[webidl(snapshot_slot = TOUCH_TARGET_SLOT)]
    target: v8::Local<'s, v8::Object>,
    #[webidl(converter = "double", default = 0.0)]
    #[webidl(snapshot_slot = TOUCH_SCREEN_X_SLOT)]
    screen_x: f64,
    #[webidl(converter = "double", default = 0.0)]
    #[webidl(snapshot_slot = TOUCH_SCREEN_Y_SLOT)]
    screen_y: f64,
    #[webidl(converter = "double", default = 0.0)]
    #[webidl(snapshot_slot = TOUCH_CLIENT_X_SLOT)]
    client_x: f64,
    #[webidl(converter = "double", default = 0.0)]
    #[webidl(snapshot_slot = TOUCH_CLIENT_Y_SLOT)]
    client_y: f64,
    #[webidl(converter = "double", default = 0.0)]
    #[webidl(snapshot_slot = TOUCH_PAGE_X_SLOT)]
    page_x: f64,
    #[webidl(converter = "double", default = 0.0)]
    #[webidl(snapshot_slot = TOUCH_PAGE_Y_SLOT)]
    page_y: f64,
    #[webidl(converter = "double", default = 0.0)]
    #[webidl(snapshot_slot = TOUCH_RADIUS_X_SLOT)]
    radius_x: f64,
    #[webidl(converter = "double", default = 0.0)]
    #[webidl(snapshot_slot = TOUCH_RADIUS_Y_SLOT)]
    radius_y: f64,
    #[webidl(converter = "double", default = 0.0)]
    #[webidl(snapshot_slot = TOUCH_ROTATION_ANGLE_SLOT)]
    rotation_angle: f64,
    #[webidl(converter = "double", default = 0.0)]
    #[webidl(snapshot_slot = TOUCH_FORCE_SLOT)]
    force: f64,
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
    let receiver = args.this();
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
    let receiver = crate::context_bootstrap::event_backing(scope, args.this());
    match get_private_value(scope, receiver, slot) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
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
        .initialize_snapshot(scope, touch)
        .expect("Touch state");
    web_api_interfaces::initialize(scope, touch, "Touch").expect("Touch brand");

    rv.set(touch.into());
}

pub(in crate::context_bootstrap) fn initialize_touch_event_lists<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    touches: &[v8::Local<'s, v8::Object>],
    target_touches: &[v8::Local<'s, v8::Object>],
    changed_touches: &[v8::Local<'s, v8::Object>],
    (alt_key, meta_key, ctrl_key, shift_key): (bool, bool, bool, bool),
) {
    let touches = list::build(scope, touches);
    let target_touches = list::build(scope, target_touches);
    let changed_touches = list::build(scope, changed_touches);
    TouchEventObjectDeclaration::new(
        touches,
        target_touches,
        changed_touches,
        alt_key,
        meta_key,
        ctrl_key,
        shift_key,
    )
    .initialize(scope, event)
    .expect("TouchEvent lists");
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
        "TouchEvent" => {
            TouchEventPrototypeDeclaration::initialize_prototype_template(scope, prototype);
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
];

const TOUCH_EVENT_PROPERTY_SLOTS: &[&str] = &[
    TOUCH_EVENT_TOUCHES_SLOT,
    TOUCH_EVENT_TARGET_TOUCHES_SLOT,
    TOUCH_EVENT_CHANGED_TOUCHES_SLOT,
    TOUCH_EVENT_ALT_KEY_SLOT,
    TOUCH_EVENT_META_KEY_SLOT,
    TOUCH_EVENT_CTRL_KEY_SLOT,
    TOUCH_EVENT_SHIFT_KEY_SLOT,
];
