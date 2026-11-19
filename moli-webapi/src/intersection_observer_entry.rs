pub mod options;

use crate::interfaces as web_api_interfaces;
use moli_v8_util::{
    callback_data_index_value, callback_data_item, get_private_value, throw_type_error, v8str,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::IntersectionObserverEntry)]
pub struct IntersectionObserverEntryDeclaration<'scope> {
    #[webapi(slot = "__moliIntersectionEntryTarget")]
    pub target: v8::Local<'scope, v8::Value>,
    #[webapi(slot = "__moliIntersectionEntryIntersecting")]
    pub is_intersecting: bool,
    #[webapi(slot = "__moliIntersectionEntryVisible")]
    pub is_visible: bool,
    #[webapi(slot = "__moliIntersectionEntryRatio")]
    pub intersection_ratio: f64,
    #[webapi(slot = "__moliIntersectionEntryBoundingRect")]
    pub bounding_client_rect: v8::Local<'scope, v8::Value>,
    #[webapi(slot = "__moliIntersectionEntryIntersectionRect")]
    pub intersection_rect: v8::Local<'scope, v8::Value>,
    #[webapi(slot = "__moliIntersectionEntryRootBounds")]
    pub root_bounds: v8::Local<'scope, v8::Value>,
    #[webapi(slot = "__moliIntersectionEntryTime")]
    pub time: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::IntersectionObserverEntry,
    enumerable)]
struct IntersectionObserverEntryPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 0))]
    time: (),
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 1))]
    root_bounds: (),
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 2))]
    bounding_client_rect: (),
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 3))]
    intersection_rect: (),
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 4))]
    is_intersecting: (),
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 5))]
    is_visible: (),
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 6))]
    intersection_ratio: (),
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 7))]
    target: (),
}

pub fn intersection_observer_entry_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'IntersectionObserverEntry': Please use the 'new' operator.",
        );
        return;
    }
    let Some(init) = args.get(0).to_object(scope) else {
        throw_type_error(
            scope,
            "Failed to construct 'IntersectionObserverEntry': 1 argument required.",
        );
        return;
    };
    initialize_intersection_observer_entry_from_init(scope, args.this(), init);
    rv.set(args.this().into());
}

fn initialize_intersection_observer_entry_from_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    init: v8::Local<'s, v8::Object>,
) {
    let value = |scope: &mut v8::PinScope<'s, '_>, name: &'static str| {
        init.get(scope, v8str(scope, name).into())
            .unwrap_or_else(|| v8::undefined(scope).into())
    };
    let time = value(scope, "time").number_value(scope).unwrap_or(0.0);
    let root_bounds = value(scope, "rootBounds");
    let bounding_client_rect = value(scope, "boundingClientRect");
    let intersection_rect = value(scope, "intersectionRect");
    let target = value(scope, "target");
    let is_intersecting = value(scope, "isIntersecting").boolean_value(scope);
    let is_visible = value(scope, "isVisible").boolean_value(scope);
    let ratio = value(scope, "intersectionRatio")
        .number_value(scope)
        .unwrap_or(0.0);

    let _ = IntersectionObserverEntryDeclaration {
        time,
        root_bounds,
        bounding_client_rect,
        intersection_rect,
        target,
        is_intersecting,
        is_visible,
        intersection_ratio: ratio,
    }
    .initialize(scope, object);
}

fn intersection_observer_entry_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        INTERSECTION_OBSERVER_ENTRY_ATTRIBUTE_SLOTS,
        "IntersectionObserverEntry attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    let value =
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

const INTERSECTION_OBSERVER_ENTRY_ATTRIBUTE_SLOTS: &[&str] = &[
    "__moliIntersectionEntryTime",
    "__moliIntersectionEntryRootBounds",
    "__moliIntersectionEntryBoundingRect",
    "__moliIntersectionEntryIntersectionRect",
    "__moliIntersectionEntryIntersecting",
    "__moliIntersectionEntryVisible",
    "__moliIntersectionEntryRatio",
    "__moliIntersectionEntryTarget",
];
pub fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    IntersectionObserverEntryPrototypeAccessorsDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}
