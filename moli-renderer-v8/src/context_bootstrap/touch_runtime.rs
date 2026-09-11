use super::*;
use crate::util::{callback_data_index_value, get_private_value};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

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

const TOUCH_LIST_LENGTH_SLOT: &str = "__lmTouchListLength";

const TOUCH_EVENT_TOUCHES_SLOT: &str = "__lmTouchEventTouches";
const TOUCH_EVENT_TARGET_TOUCHES_SLOT: &str = "__lmTouchEventTargetTouches";
const TOUCH_EVENT_CHANGED_TOUCHES_SLOT: &str = "__lmTouchEventChangedTouches";
const TOUCH_EVENT_ALT_KEY_SLOT: &str = "__lmTouchEventAltKey";
const TOUCH_EVENT_META_KEY_SLOT: &str = "__lmTouchEventMetaKey";
const TOUCH_EVENT_CTRL_KEY_SLOT: &str = "__lmTouchEventCtrlKey";
const TOUCH_EVENT_SHIFT_KEY_SLOT: &str = "__lmTouchEventShiftKey";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Touch)]
struct TouchObjectDeclaration<'scope> {
    #[webapi(slot = TOUCH_IDENTIFIER_SLOT)]
    identifier: f64,
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
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Touch)]
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
#[webapi(interface = web_api_interfaces::TouchList)]
struct TouchListObjectDeclaration {
    #[webapi(slot = TOUCH_LIST_LENGTH_SLOT)]
    length: u32,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TouchList, enumerable)]
struct TouchListPrototypeDeclaration {
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),

    #[webapi(accessor_property, getter = touch_list_length_getter, enumerable)]
    length: (),

    #[webapi(method, length = 1, callback = touch_list_item_callback)]
    item: (),
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct TouchUiEventInitDeclaration<'scope> {
    view: v8::Local<'scope, v8::Value>,
    detail: f64,
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
#[webapi(interface = web_api_interfaces::TouchEvent)]
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
        missing_message = "Failed to construct 'Touch': 1 argument required, but only 0 present."
    )]
    init: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TouchList.item")]
struct TouchListItemArgs {
    #[webidl(required)]
    index: u32,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "TouchInit")]
struct TouchInitMembers<'s> {
    #[webidl(required)]
    identifier: i32,
    #[webidl(required, with = touch_event_target_member)]
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
    #[webidl(converter = "double", default = 0.0)]
    radius_x: f64,
    #[webidl(converter = "double", default = 0.0)]
    radius_y: f64,
    #[webidl(converter = "double", default = 0.0)]
    rotation_angle: f64,
    #[webidl(converter = "double", default = 0.0)]
    force: f64,
}

fn touch_event_target_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    let context = webidl::Context::member("TouchInit", name);
    let Some(value) = webidl::property_result(scope, object, name, context)? else {
        return Err(webidl::WebIdlError::missing_required(context));
    };
    if value.is_undefined() {
        return Err(webidl::WebIdlError::missing_required(context));
    }
    let Ok(target) = v8::Local::<v8::Object>::try_from(value) else {
        return Err(webidl::WebIdlError::custom_message(
            "TouchInit member target is not of type EventTarget.",
        ));
    };
    if !super::event_template::object_is_event_target(scope, target) {
        return Err(webidl::WebIdlError::custom_message(
            "TouchInit member target is not of type EventTarget.",
        ));
    }
    Ok(target)
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
    let receiver = args.this();
    match get_private_value(scope, receiver, slot) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

fn touch_list_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    let length = get_private_value(scope, receiver, TOUCH_LIST_LENGTH_SLOT)
        .and_then(|value| value.integer_value(scope))
        .unwrap_or(0);
    rv.set(v8::Integer::new(scope, length as i32).into());
}

pub(in crate::context_bootstrap) fn touch_list_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<TouchListItemArgs>(scope, &args) else {
        return;
    };
    match args.this().get_index(scope, parsed.index) {
        Some(value) if !value.is_undefined() => rv.set(value),
        None => rv.set(v8::null(scope).into()),
        _ => rv.set(v8::null(scope).into()),
    }
}

fn touch_event_init_bool(
    scope: &mut v8::PinScope<'_, '_>,
    init: Option<v8::Local<'_, v8::Object>>,
    key: &'static str,
    default: bool,
) -> bool {
    init.and_then(|object| object.get(scope, v8str(scope, key).into()))
        .filter(|value| !value.is_null_or_undefined())
        .map(|value| value.boolean_value(scope))
        .unwrap_or(default)
}

fn touch_event_init_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
    key: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    init.and_then(|object| object.get(scope, v8str(scope, key).into()))
        .filter(|value| !value.is_undefined())
}

fn initialize_touch_ui_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) {
    let view = touch_event_init_value(scope, init, "view")
        .unwrap_or_else(|| scope.get_current_context().global(scope).into());
    let detail = init
        .and_then(|object| object.get(scope, v8str(scope, "detail").into()))
        .filter(|value| !value.is_null_or_undefined())
        .and_then(|value| value.number_value(scope))
        .filter(|value| !value.is_nan())
        .unwrap_or(0.0);
    TouchUiEventInitDeclaration::new(view, detail)
        .initialize(scope, event)
        .expect("Touch UIEvent init declaration should initialize");
}

fn touch_init_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    if value.is_null_or_undefined() || !value.is_object() {
        None
    } else {
        value.to_object(scope)
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
    let init = match webidl::parse_dictionary_object::<TouchInitMembers>(scope, parsed_args.init) {
        Ok(init) => init,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };

    TouchObjectDeclaration::new(
        f64::from(init.identifier),
        init.target,
        init.screen_x,
        init.screen_y,
        init.client_x,
        init.client_y,
        init.page_x,
        init.page_y,
        init.radius_x,
        init.radius_y,
        init.rotation_angle,
        init.force,
    )
    .bind_into(scope, touch)
    .expect("Touch declaration should bind into constructed object");

    rv.set(touch.into());
}

pub(in crate::context_bootstrap) fn build_touch_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<v8::Local<'s, v8::Value>>,
) -> v8::Local<'s, v8::Object> {
    let source = value.and_then(|value| touch_init_object(scope, value));
    let length = source
        .and_then(|source| source.get(scope, v8str(scope, "length").into()))
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as u32)
        .unwrap_or(0);

    let list = TouchListObjectDeclaration::new(length)
        .bind(scope)
        .expect("TouchList declaration should bind");
    for index in 0..length {
        if let Some(item) = source.and_then(|source| source.get_index(scope, index)) {
            let _ = list.set_index(scope, index, item);
        }
    }

    list
}

pub(in crate::context_bootstrap) fn initialize_touch_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) {
    initialize_touch_ui_event(scope, event, init);

    let touches_value = touch_event_init_value(scope, init, "touches");
    let target_touches_value = touch_event_init_value(scope, init, "targetTouches");
    let changed_touches_value = touch_event_init_value(scope, init, "changedTouches");
    let touches = build_touch_list(scope, touches_value);
    let target_touches = build_touch_list(scope, target_touches_value);
    let changed_touches = build_touch_list(scope, changed_touches_value);
    let alt_key = touch_event_init_bool(scope, init, "altKey", false);
    let meta_key = touch_event_init_bool(scope, init, "metaKey", false);
    let ctrl_key = touch_event_init_bool(scope, init, "ctrlKey", false);
    let shift_key = touch_event_init_bool(scope, init, "shiftKey", false);
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
            TouchListPrototypeDeclaration::initialize_prototype_template(scope, prototype);
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
