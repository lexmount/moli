use super::*;
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value,
    throw_type_error,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PerformanceEntry, scope_lifetime = 'scope)]
struct PerformanceEntryObjectDeclaration<'scope, 'entry_type> {
    #[webapi(slot = PERFORMANCE_ENTRY_ID_SLOT)]
    id: f64,

    #[webapi(slot = PERFORMANCE_ENTRY_NAME_SLOT)]
    name: PerformanceEntryName,

    #[webapi(slot = PERFORMANCE_ENTRY_TYPE_SLOT)]
    entry_type: &'entry_type str,

    #[webapi(slot = PERFORMANCE_ENTRY_START_TIME_SLOT)]
    start_time: f64,

    #[webapi(slot = PERFORMANCE_ENTRY_DURATION_SLOT)]
    duration: f64,

    #[webapi(slot = PERFORMANCE_ENTRY_NAVIGATION_ID_SLOT)]
    navigation_id: f64,

    #[webapi(slot = PERFORMANCE_ENTRY_DETAIL_SLOT)]
    detail: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct PerformanceEntryJsonSnapshotDeclaration {
    id: f64,
    name: PerformanceEntryName,
    entry_type: String,
    start_time: f64,
    duration: f64,
    navigation_id: f64,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PerformanceResourceTiming)]
struct PerformanceResourceTimingSlotDeclaration {
    #[webapi(slot = PERFORMANCE_RESOURCE_INITIATOR_TYPE_SLOT)]
    initiator_type: String,

    #[webapi(slot = PERFORMANCE_RESOURCE_INITIATOR_URL_SLOT, constructor_default = "")]
    initiator_url: &'static str,

    #[webapi(slot = PERFORMANCE_RESOURCE_NEXT_HOP_PROTOCOL_SLOT, constructor_default = "")]
    next_hop_protocol: &'static str,

    #[webapi(slot = PERFORMANCE_RESOURCE_WORKER_START_SLOT, constructor_default)]
    worker_start: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_REDIRECT_START_SLOT, constructor_default)]
    redirect_start: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_REDIRECT_END_SLOT, constructor_default)]
    redirect_end: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_FETCH_START_SLOT, constructor_default)]
    fetch_start: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_DOMAIN_LOOKUP_START_SLOT, constructor_default)]
    domain_lookup_start: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_DOMAIN_LOOKUP_END_SLOT, constructor_default)]
    domain_lookup_end: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_CONNECT_START_SLOT, constructor_default)]
    connect_start: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_CONNECT_END_SLOT, constructor_default)]
    connect_end: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_SECURE_CONNECTION_START_SLOT, constructor_default)]
    secure_connection_start: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_REQUEST_START_SLOT, constructor_default)]
    request_start: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_RESPONSE_START_SLOT, constructor_default)]
    response_start: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_RESPONSE_END_SLOT, constructor_default)]
    response_end: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_TRANSFER_SIZE_SLOT)]
    transfer_size: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_ENCODED_BODY_SIZE_SLOT)]
    encoded_body_size: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_DECODED_BODY_SIZE_SLOT)]
    decoded_body_size: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_RENDER_BLOCKING_STATUS_SLOT)]
    render_blocking_status: String,

    #[webapi(slot = PERFORMANCE_RESOURCE_RESPONSE_STATUS_SLOT)]
    response_status: f64,

    #[webapi(slot = PERFORMANCE_RESOURCE_CONTENT_TYPE_SLOT)]
    content_type: String,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct PerformanceResourceTimingJsonSnapshotDeclaration {
    id: f64,
    name: PerformanceEntryName,
    entry_type: String,
    start_time: f64,
    duration: f64,
    navigation_id: f64,
    initiator_type: String,
    initiator_url: String,
    next_hop_protocol: String,
    worker_start: f64,
    redirect_start: f64,
    redirect_end: f64,
    fetch_start: f64,
    domain_lookup_start: f64,
    domain_lookup_end: f64,
    connect_start: f64,
    connect_end: f64,
    secure_connection_start: f64,
    request_start: f64,
    response_start: f64,
    response_end: f64,
    transfer_size: f64,
    encoded_body_size: f64,
    decoded_body_size: f64,
    render_blocking_status: String,
    response_status: f64,
    content_type: String,
}

impl PerformanceResourceTimingJsonSnapshotDeclaration {
    fn from_entry<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        entry: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        Some(Self::new(
            performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_ID_SLOT)?,
            PerformanceEntryName::from_entry(scope, entry)?,
            performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_TYPE_SLOT)?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_START_TIME_SLOT)?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_DURATION_SLOT)?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_NAVIGATION_ID_SLOT)?,
            performance_entry_slot_string(scope, entry, PERFORMANCE_RESOURCE_INITIATOR_TYPE_SLOT)?,
            performance_entry_slot_string(scope, entry, PERFORMANCE_RESOURCE_INITIATOR_URL_SLOT)?,
            performance_entry_slot_string(
                scope,
                entry,
                PERFORMANCE_RESOURCE_NEXT_HOP_PROTOCOL_SLOT,
            )?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_RESOURCE_WORKER_START_SLOT)?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_RESOURCE_REDIRECT_START_SLOT)?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_RESOURCE_REDIRECT_END_SLOT)?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_RESOURCE_FETCH_START_SLOT)?,
            performance_entry_slot_number(
                scope,
                entry,
                PERFORMANCE_RESOURCE_DOMAIN_LOOKUP_START_SLOT,
            )?,
            performance_entry_slot_number(
                scope,
                entry,
                PERFORMANCE_RESOURCE_DOMAIN_LOOKUP_END_SLOT,
            )?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_RESOURCE_CONNECT_START_SLOT)?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_RESOURCE_CONNECT_END_SLOT)?,
            performance_entry_slot_number(
                scope,
                entry,
                PERFORMANCE_RESOURCE_SECURE_CONNECTION_START_SLOT,
            )?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_RESOURCE_REQUEST_START_SLOT)?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_RESOURCE_RESPONSE_START_SLOT)?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_RESOURCE_RESPONSE_END_SLOT)?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_RESOURCE_TRANSFER_SIZE_SLOT)?,
            performance_entry_slot_number(
                scope,
                entry,
                PERFORMANCE_RESOURCE_ENCODED_BODY_SIZE_SLOT,
            )?,
            performance_entry_slot_number(
                scope,
                entry,
                PERFORMANCE_RESOURCE_DECODED_BODY_SIZE_SLOT,
            )?,
            performance_entry_slot_string(
                scope,
                entry,
                PERFORMANCE_RESOURCE_RENDER_BLOCKING_STATUS_SLOT,
            )?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_RESOURCE_RESPONSE_STATUS_SLOT)?,
            performance_entry_slot_string(scope, entry, PERFORMANCE_RESOURCE_CONTENT_TYPE_SLOT)?,
        ))
    }
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PerformanceEntry, receiver)]
struct PerformanceEntryPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = performance_entry_base_attribute_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    id: (),
    #[webapi(
        accessor_property,
        getter = performance_entry_base_attribute_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    name: (),
    #[webapi(
        accessor_property,
        getter = performance_entry_base_attribute_getter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    entry_type: (),
    #[webapi(
        accessor_property,
        getter = performance_entry_base_attribute_getter_callback,
        data = callback_data_index_value(scope, 3),
        enumerable
    )]
    start_time: (),
    #[webapi(
        accessor_property,
        getter = performance_entry_base_attribute_getter_callback,
        data = callback_data_index_value(scope, 4),
        enumerable
    )]
    duration: (),
    #[webapi(
        accessor_property,
        getter = performance_entry_base_attribute_getter_callback,
        data = callback_data_index_value(scope, 5),
        enumerable
    )]
    navigation_id: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PerformanceEntry, enumerable, receiver)]
struct PerformanceEntryPrototypeMethodsDeclaration {
    #[webapi(
        method,
        name = "toJSON",
        length = 0,
        callback = performance_entry_to_json_callback
    )]
    to_json: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "PerformanceEntryDetail")]
struct PerformanceEntryDetailPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = performance_entry_detail_attribute_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    detail: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::PerformanceResourceTiming,
    receiver
)]
struct PerformanceResourceTimingPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 20),
        enumerable
    )]
    initiator_url: (),

    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    initiator_type: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    next_hop_protocol: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    worker_start: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 3),
        enumerable
    )]
    redirect_start: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 4),
        enumerable
    )]
    redirect_end: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 5),
        enumerable
    )]
    fetch_start: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 6),
        enumerable
    )]
    domain_lookup_start: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 7),
        enumerable
    )]
    domain_lookup_end: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 8),
        enumerable
    )]
    connect_start: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 9),
        enumerable
    )]
    connect_end: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 10),
        enumerable
    )]
    secure_connection_start: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 11),
        enumerable
    )]
    request_start: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 12),
        enumerable
    )]
    response_start: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 13),
        enumerable
    )]
    response_end: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 14),
        enumerable
    )]
    transfer_size: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 15),
        enumerable
    )]
    encoded_body_size: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 16),
        enumerable
    )]
    decoded_body_size: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 17),
        enumerable
    )]
    render_blocking_status: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 18),
        enumerable
    )]
    response_status: (),
    #[webapi(
        accessor_property,
        getter = performance_resource_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 19),
        enumerable
    )]
    content_type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::PerformanceResourceTiming,
    enumerable,
    receiver
)]
struct PerformanceResourceTimingPrototypeMethodsDeclaration {
    #[webapi(
        method,
        name = "toJSON",
        length = 0,
        callback = performance_resource_timing_to_json_callback
    )]
    to_json: (),
}

pub fn install_performance_entry_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "PerformanceEntry" => {
            PerformanceEntryPrototypeMethodsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            PerformanceEntryPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "PerformanceMark" | "PerformanceMeasure" => {
            PerformanceEntryDetailPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "PerformanceResourceTiming" => {
            PerformanceResourceTimingPrototypeMethodsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            PerformanceResourceTimingPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

fn performance_entry_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    let Some(id) = performance_entry_slot_number(scope, receiver, PERFORMANCE_ENTRY_ID_SLOT) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let Some(name) = PerformanceEntryName::from_entry(scope, receiver) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let Some(entry_type) =
        performance_entry_slot_string(scope, receiver, PERFORMANCE_ENTRY_TYPE_SLOT)
    else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let Some(start_time) =
        performance_entry_slot_number(scope, receiver, PERFORMANCE_ENTRY_START_TIME_SLOT)
    else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let Some(duration) =
        performance_entry_slot_number(scope, receiver, PERFORMANCE_ENTRY_DURATION_SLOT)
    else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let Some(navigation_id) =
        performance_entry_slot_number(scope, receiver, PERFORMANCE_ENTRY_NAVIGATION_ID_SLOT)
    else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let snapshot = PerformanceEntryJsonSnapshotDeclaration::new(
        id,
        name,
        entry_type,
        start_time,
        duration,
        navigation_id,
    )
    .bind(scope)
    .expect("PerformanceEntry toJSON snapshot declaration should bind");
    rv.set(snapshot.into());
}

fn performance_resource_timing_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(snapshot) = resource_timing_json_snapshot(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(snapshot.into());
}

pub fn resource_timing_json_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let snapshot = PerformanceResourceTimingJsonSnapshotDeclaration::from_entry(scope, entry)?;
    Some(
        snapshot
            .bind(scope)
            .expect("PerformanceResourceTiming toJSON snapshot declaration should bind"),
    )
}

fn performance_entry_base_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        PERFORMANCE_ENTRY_BASE_ATTRIBUTE_SLOTS,
        "PerformanceEntry base attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    performance_entry_attribute_getter(scope, args.this(), slot, &mut rv);
}

fn performance_entry_detail_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        PERFORMANCE_ENTRY_DETAIL_ATTRIBUTE_SLOTS,
        "PerformanceEntry detail attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    performance_entry_attribute_getter(scope, args.this(), slot, &mut rv);
}

fn performance_resource_timing_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        PERFORMANCE_RESOURCE_TIMING_ATTRIBUTE_SLOTS,
        "PerformanceResourceTiming attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    performance_entry_attribute_getter(scope, args.this(), slot, &mut rv);
}

fn performance_entry_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = performance_entry_slot_value(scope, object, slot) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(value);
}

pub fn performance_entry_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, object, slot)
}

pub fn performance_entry_slot_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<String> {
    let value = performance_entry_slot_value(scope, object, slot)?;
    if value.is_null_or_undefined() {
        return None;
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

pub fn performance_entry_slot_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<f64> {
    performance_entry_slot_value(scope, object, slot).and_then(|value| value.number_value(scope))
}

pub fn set_performance_entry_slot_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: f64,
) {
    let stored = v8::Number::new(scope, value);
    set_private_value(scope, object, slot, stored.into());
}

const PERFORMANCE_ENTRY_BASE_ATTRIBUTE_SLOTS: &[&str] = &[
    PERFORMANCE_ENTRY_ID_SLOT,
    PERFORMANCE_ENTRY_NAME_SLOT,
    PERFORMANCE_ENTRY_TYPE_SLOT,
    PERFORMANCE_ENTRY_START_TIME_SLOT,
    PERFORMANCE_ENTRY_DURATION_SLOT,
    PERFORMANCE_ENTRY_NAVIGATION_ID_SLOT,
];

const PERFORMANCE_ENTRY_DETAIL_ATTRIBUTE_SLOTS: &[&str] = &[PERFORMANCE_ENTRY_DETAIL_SLOT];

const PERFORMANCE_RESOURCE_TIMING_ATTRIBUTE_SLOTS: &[&str] = &[
    PERFORMANCE_RESOURCE_INITIATOR_TYPE_SLOT,
    PERFORMANCE_RESOURCE_NEXT_HOP_PROTOCOL_SLOT,
    PERFORMANCE_RESOURCE_WORKER_START_SLOT,
    PERFORMANCE_RESOURCE_REDIRECT_START_SLOT,
    PERFORMANCE_RESOURCE_REDIRECT_END_SLOT,
    PERFORMANCE_RESOURCE_FETCH_START_SLOT,
    PERFORMANCE_RESOURCE_DOMAIN_LOOKUP_START_SLOT,
    PERFORMANCE_RESOURCE_DOMAIN_LOOKUP_END_SLOT,
    PERFORMANCE_RESOURCE_CONNECT_START_SLOT,
    PERFORMANCE_RESOURCE_CONNECT_END_SLOT,
    PERFORMANCE_RESOURCE_SECURE_CONNECTION_START_SLOT,
    PERFORMANCE_RESOURCE_REQUEST_START_SLOT,
    PERFORMANCE_RESOURCE_RESPONSE_START_SLOT,
    PERFORMANCE_RESOURCE_RESPONSE_END_SLOT,
    PERFORMANCE_RESOURCE_TRANSFER_SIZE_SLOT,
    PERFORMANCE_RESOURCE_ENCODED_BODY_SIZE_SLOT,
    PERFORMANCE_RESOURCE_DECODED_BODY_SIZE_SLOT,
    PERFORMANCE_RESOURCE_RENDER_BLOCKING_STATUS_SLOT,
    PERFORMANCE_RESOURCE_RESPONSE_STATUS_SLOT,
    PERFORMANCE_RESOURCE_CONTENT_TYPE_SLOT,
    PERFORMANCE_RESOURCE_INITIATOR_URL_SLOT,
];

pub fn create_performance_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry_type: &str,
    name: PerformanceEntryName,
    start_time: f64,
    duration: f64,
    detail: Option<v8::Local<'s, v8::Value>>,
) -> v8::Local<'s, v8::Object> {
    let context = performance
        .get_creation_context(scope)
        .expect("native Performance must retain its relevant Realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let detail = detail.unwrap_or_else(|| v8::null(scope).into());
    let entry = PerformanceEntryObjectDeclaration {
        id: 0.0,
        name,
        entry_type,
        start_time,
        duration,
        navigation_id: 0.0,
        detail,
    }
    .bind(scope)
    .expect("PerformanceEntry declaration should bind");
    let prototype_name = match entry_type {
        "navigation" => "PerformanceNavigationTiming",
        "mark" => "PerformanceMark",
        "measure" => "PerformanceMeasure",
        "resource" => "PerformanceResourceTiming",
        _ => "PerformanceEntry",
    };
    web_api_interfaces::initialize(scope, entry, prototype_name)
        .expect("PerformanceEntry primary interface should initialize");
    if let Some(prototype) = moli_v8_util::global_constructor_prototype(scope, prototype_name) {
        let _ = entry.set_prototype(scope, prototype.into());
    }
    entry
}

pub fn initialize_performance_entry_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    entry_type: &str,
    name: PerformanceEntryName,
    start_time: f64,
    duration: f64,
    detail: Option<v8::Local<'s, v8::Value>>,
) {
    let detail = detail.unwrap_or_else(|| v8::null(scope).into());
    PerformanceEntryObjectDeclaration {
        id: 0.0,
        name,
        entry_type,
        start_time,
        duration,
        navigation_id: 0.0,
        detail,
    }
    .initialize(scope, entry)
    .expect("PerformanceEntry declaration should initialize object");
}

pub fn initialize_resource_timing_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    initiator_type: &str,
    transfer_size: f64,
    encoded_body_size: f64,
    decoded_body_size: f64,
    render_blocking_status: &str,
    response_status: f64,
    content_type: &str,
) {
    PerformanceResourceTimingSlotDeclaration::new(
        initiator_type.to_owned(),
        transfer_size,
        encoded_body_size,
        decoded_body_size,
        render_blocking_status.to_owned(),
        response_status,
        content_type.to_owned(),
    )
    .initialize(scope, entry)
    .expect("PerformanceResourceTiming slot declaration should initialize");
}
