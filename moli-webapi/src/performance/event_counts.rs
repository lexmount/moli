//! Event-count map values. Event collection and lazy Performance ownership stay with the embedder.
use crate::interfaces as web_api_interfaces;
use crate::webidl_iterator::{
    SnapshotWebIdlIteratorKind, invoke_webidl_collection_for_each_callback,
    new_snapshot_webidl_iterator, prepare_webidl_collection_for_each_callback,
};
use moli_v8_util::{get_private_value, v8str};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, serialize_v8_array};
const PERFORMANCE_EVENT_COUNTS_VALUES_SLOT: &str = "__moliPerformanceEventCountsValues";
pub const EVENT_COUNTS_TRACKED_TYPES: &[&str] = &[
    "auxclick",
    "click",
    "contextmenu",
    "dblclick",
    "mousedown",
    "mouseenter",
    "mouseleave",
    "mouseout",
    "mouseover",
    "mouseup",
    "pointerover",
    "pointerenter",
    "pointerdown",
    "pointerup",
    "pointercancel",
    "pointerout",
    "pointerleave",
    "gotpointercapture",
    "lostpointercapture",
    "touchstart",
    "touchend",
    "touchcancel",
    "keydown",
    "keypress",
    "keyup",
    "beforeinput",
    "input",
    "compositionstart",
    "compositionupdate",
    "compositionend",
    "dragstart",
    "dragend",
    "dragenter",
    "dragleave",
    "dragover",
    "drop",
];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::EventCounts)]
struct EventCountsObjectDeclaration {
    #[webapi(slot = PERFORMANCE_EVENT_COUNTS_VALUES_SLOT)]
    values: Vec<u32>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::EventCounts, enumerable)]
struct EventCountsPrototypeMethodsDeclaration {
    #[webapi(accessor_property, getter = event_counts_size_getter)]
    size: (),

    #[webapi(method, length = 1, callback = event_counts_get_callback)]
    get: (),

    #[webapi(method, length = 1, callback = event_counts_has_callback)]
    has: (),

    #[webapi(method, length = 0, callback = event_counts_keys_callback)]
    keys: (),

    #[webapi(method, length = 0, callback = event_counts_values_callback)]
    values: (),

    #[webapi(method, length = 0, callback = event_counts_entries_callback)]
    entries: (),

    #[webapi(method, length = 1, callback = event_counts_for_each_callback)]
    for_each: (),

    #[webapi(alias = "entries", symbol = "iterator")]
    iterator: (),
}

fn event_counts_size_getter(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Integer::new(scope, EVENT_COUNTS_TRACKED_TYPES.len() as i32).into());
}

fn event_counts_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(index) = event_counts_arg_tracked_index(scope, args.get(0)) else {
        rv.set(v8::Integer::new(scope, 0).into());
        return;
    };
    rv.set(event_counts_value_at(scope, args.this(), index));
}

fn event_counts_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let tracked = event_counts_arg_is_tracked(scope, args.get(0));
    rv.set(v8::Boolean::new(scope, tracked).into());
}

fn event_counts_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let array = serialize_v8_array(scope, EVENT_COUNTS_TRACKED_TYPES)
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    set_event_counts_iterator(scope, array, &mut rv);
}

fn event_counts_values_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let values = (0..EVENT_COUNTS_TRACKED_TYPES.len())
        .map(|index| event_counts_value_at(scope, args.this(), index))
        .collect::<Vec<_>>();
    let array =
        serialize_v8_array(scope, values.as_slice()).unwrap_or_else(|| v8::Array::new(scope, 0));
    set_event_counts_iterator(scope, array, &mut rv);
}

fn event_counts_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let array = event_counts_entries_array(scope, args.this());
    set_event_counts_iterator(scope, array, &mut rv);
}

fn event_counts_for_each_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(callback) =
        prepare_webidl_collection_for_each_callback(scope, args.get(0), "EventCounts forEach")
    else {
        return;
    };
    let this_arg = if args.length() > 1 {
        args.get(1)
    } else {
        v8::undefined(scope).into()
    };
    for event_type in EVENT_COUNTS_TRACKED_TYPES {
        let index = event_counts_tracked_index(event_type).unwrap_or(0);
        let value = event_counts_value_at(scope, args.this(), index);
        let key = v8str(scope, event_type);
        if invoke_webidl_collection_for_each_callback(
            scope,
            &callback,
            this_arg,
            value,
            key.into(),
            args.this(),
        )
        .is_none()
        {
            return;
        }
    }
    rv.set_undefined();
}

fn event_counts_arg_is_tracked(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    event_counts_arg_tracked_index(scope, value).is_some()
}

fn event_counts_arg_tracked_index(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<usize> {
    let value = value.to_string(scope)?;
    let value = value.to_rust_string_lossy(scope);
    event_counts_tracked_index(&value)
}

pub fn event_counts_tracked_index(event_type: &str) -> Option<usize> {
    EVENT_COUNTS_TRACKED_TYPES
        .iter()
        .position(|candidate| *candidate == event_type)
}

fn event_counts_value_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_counts: v8::Local<'s, v8::Object>,
    index: usize,
) -> v8::Local<'s, v8::Value> {
    event_counts_values_array(scope, event_counts)
        .and_then(|values| values.get_index(scope, index as u32))
        .unwrap_or_else(|| v8::Integer::new(scope, 0).into())
}

pub fn event_counts_values_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_counts: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, event_counts, PERFORMANCE_EVENT_COUNTS_VALUES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn event_counts_entries_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_counts: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    let entries: Vec<(&str, v8::Local<'s, v8::Value>)> = EVENT_COUNTS_TRACKED_TYPES
        .iter()
        .copied()
        .enumerate()
        .map(|(index, event_type)| {
            (
                event_type,
                event_counts_value_at(scope, event_counts, index),
            )
        })
        .collect();
    serialize_v8_array(scope, entries.as_slice()).unwrap_or_else(|| v8::Array::new(scope, 0))
}

fn set_event_counts_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(iterator) =
        new_snapshot_webidl_iterator(scope, array, SnapshotWebIdlIteratorKind::EventCounts)
    {
        rv.set(iterator.into());
    } else {
        rv.set_undefined();
    }
}

pub fn new_event_counts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: Vec<u32>,
) -> anyhow::Result<v8::Local<'s, v8::Object>> {
    EventCountsObjectDeclaration { values }
        .bind(scope)
        .map_err(|error| anyhow::anyhow!("failed to bind EventCounts object: {error}"))
}
pub fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    EventCountsPrototypeMethodsDeclaration::initialize_prototype_template(scope, prototype);
}
