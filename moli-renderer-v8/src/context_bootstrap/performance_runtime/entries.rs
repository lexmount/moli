use super::super::performance_observer_runtime::{
    filtered_entry_list_entries, queue_matching_performance_observers,
};
use super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value,
    throw_type_error,
};
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.getEntriesByType")]
struct PerformanceGetEntriesByTypeArgs {
    #[webidl(required)]
    entry_type: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.getEntriesByName")]
struct PerformanceGetEntriesByNameArgs {
    #[webidl(required)]
    name: String,
    entry_type: Option<String>,
}

#[derive(WebApiObject)]
#[webapi(interface = "PerformanceEntry", scope_lifetime = 'scope)]
struct PerformanceEntryObjectDeclaration<'scope, 'name, 'entry_type> {
    #[webapi(slot = PERFORMANCE_ENTRY_ID_SLOT)]
    id: f64,

    #[webapi(slot = PERFORMANCE_ENTRY_NAME_SLOT)]
    name: &'name str,

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
#[webapi(interface = "Object", data_properties, enumerable)]
struct PerformanceEntryJsonSnapshotDeclaration {
    id: f64,
    name: String,
    entry_type: String,
    start_time: f64,
    duration: f64,
    navigation_id: f64,
}

#[derive(WebApiObject)]
#[webapi(interface = "PerformanceResourceTiming")]
struct PerformanceResourceTimingSlotDeclaration {
    #[webapi(slot = PERFORMANCE_RESOURCE_INITIATOR_TYPE_SLOT)]
    initiator_type: String,

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
#[webapi(interface = "Object", data_properties, enumerable)]
struct PerformanceResourceTimingJsonSnapshotDeclaration {
    id: f64,
    name: String,
    entry_type: String,
    start_time: f64,
    duration: f64,
    navigation_id: f64,
    initiator_type: String,
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
            performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_NAME_SLOT)?,
            performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_TYPE_SLOT)?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_START_TIME_SLOT)?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_DURATION_SLOT)?,
            performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_NAVIGATION_ID_SLOT)?,
            performance_entry_slot_string(scope, entry, PERFORMANCE_RESOURCE_INITIATOR_TYPE_SLOT)?,
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
#[webapi(name = "PerformanceEntry")]
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
#[webapi(name = "PerformanceEntry", enumerable)]
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
#[webapi(name = "PerformanceResourceTiming")]
struct PerformanceResourceTimingPrototypeAccessorsDeclaration {
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
#[webapi(name = "PerformanceResourceTiming", enumerable)]
struct PerformanceResourceTimingPrototypeMethodsDeclaration {
    #[webapi(
        method,
        name = "toJSON",
        length = 0,
        callback = performance_resource_timing_to_json_callback
    )]
    to_json: (),
}

pub(in crate::context_bootstrap) fn install_performance_entry_template_bindings<'s>(
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
    let Some(name) = performance_entry_slot_string(scope, receiver, PERFORMANCE_ENTRY_NAME_SLOT)
    else {
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
    let Some(snapshot) =
        PerformanceResourceTimingJsonSnapshotDeclaration::from_entry(scope, args.this())
    else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let snapshot = snapshot
        .bind(scope)
        .expect("PerformanceResourceTiming toJSON snapshot declaration should bind");
    rv.set(snapshot.into());
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

pub(in crate::context_bootstrap) fn performance_entry_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, object, slot)
}

pub(crate) fn is_performance_entry_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    PERFORMANCE_ENTRY_BASE_ATTRIBUTE_SLOTS
        .iter()
        .all(|slot| get_private_value(scope, object, slot).is_some())
}

pub(in crate::context_bootstrap) fn performance_entry_slot_string<'s>(
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

pub(in crate::context_bootstrap) fn performance_entry_slot_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<f64> {
    performance_entry_slot_value(scope, object, slot).and_then(|value| value.number_value(scope))
}

pub(in crate::context_bootstrap) fn set_performance_entry_slot_number<'s>(
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
];

pub(in crate::context_bootstrap) fn performance_get_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    super::install::ensure_navigation_performance_entry(scope, args.this());
    rv.set(filtered_performance_entries(scope, args.this(), None, None).into());
}

pub(in crate::context_bootstrap) fn performance_get_entries_by_type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<PerformanceGetEntriesByTypeArgs>(scope, &args) else {
        return;
    };
    if parsed.entry_type == "navigation" {
        super::install::ensure_navigation_performance_entry(scope, args.this());
    }
    rv.set(filtered_performance_entries(scope, args.this(), Some(&parsed.entry_type), None).into());
}

pub(in crate::context_bootstrap) fn performance_get_entries_by_name_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<PerformanceGetEntriesByNameArgs>(scope, &args) else {
        return;
    };
    if parsed
        .entry_type
        .as_deref()
        .is_none_or(|value| value == "navigation")
    {
        super::install::ensure_navigation_performance_entry(scope, args.this());
    }
    rv.set(
        filtered_performance_entries(
            scope,
            args.this(),
            parsed.entry_type.as_deref(),
            Some(&parsed.name),
        )
        .into(),
    );
}

pub(super) fn filtered_performance_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    expected_type: Option<&str>,
    expected_name: Option<&str>,
) -> v8::Local<'s, v8::Array> {
    let Some(entries) = performance_slot_array(scope, performance, PERFORMANCE_ENTRIES_SLOT) else {
        return v8::Array::new(scope, 0);
    };
    filtered_entry_list_entries(scope, entries, expected_type, expected_name)
}

pub(super) fn find_latest_performance_mark_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<f64> {
    let entries = performance_slot_array(scope, performance, PERFORMANCE_ENTRIES_SLOT)?;
    let mut found = None;
    for index in 0..entries.length() {
        let entry = entries.get_index(scope, index)?;
        let entry = v8::Local::<v8::Object>::try_from(entry).ok()?;
        if performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_TYPE_SLOT).as_deref()
            == Some("mark")
            && performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_NAME_SLOT).as_deref()
                == Some(name)
        {
            found = performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_START_TIME_SLOT);
        }
    }
    found
}

pub(super) fn create_performance_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry_type: &str,
    name: &str,
    start_time: f64,
    duration: f64,
    detail: Option<v8::Local<'s, v8::Value>>,
) -> v8::Local<'s, v8::Object> {
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
    if let Ok(prototype) =
        crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, prototype_name)
    {
        let _ = entry.set_prototype(scope, prototype.into());
    }
    entry
}

pub(super) fn initialize_performance_entry_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    entry_type: &str,
    name: &str,
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

pub(super) fn push_performance_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) {
    if super::install::is_window_performance(scope, performance) {
        super::install::ensure_navigation_performance_entry(scope, performance);
    }
    assign_queued_performance_entry_identity(scope, performance, entry);
    let entry_type = performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_TYPE_SLOT)
        .unwrap_or_default();
    if entry_type == "resource" {
        queue_matching_performance_observers(scope, entry, &entry_type);
        super::resource_buffer::add_resource_timing_entry(scope, performance, entry);
        return;
    }
    append_performance_entry(scope, performance, entry);
    queue_matching_performance_observers(scope, entry, &entry_type);
}

pub(super) fn assign_navigation_performance_entry_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) {
    let id = ensure_performance_entry_id(scope, performance, entry);
    set_performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_NAVIGATION_ID_SLOT, id);
}

fn assign_queued_performance_entry_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) {
    ensure_performance_entry_id(scope, performance, entry);
    let navigation_id = get_private_value(scope, performance, PERFORMANCE_NAVIGATION_ENTRY_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|navigation| {
            performance_entry_slot_number(scope, navigation, PERFORMANCE_ENTRY_ID_SLOT)
        })
        .unwrap_or(0.0);
    set_performance_entry_slot_number(
        scope,
        entry,
        PERFORMANCE_ENTRY_NAVIGATION_ID_SLOT,
        navigation_id,
    );
}

fn ensure_performance_entry_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) -> f64 {
    if let Some(id) = performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_ID_SLOT)
        .filter(|id| *id > 0.0)
    {
        return id;
    }
    let last_id = get_private_value(scope, performance, PERFORMANCE_LAST_ENTRY_ID_SLOT)
        .and_then(|value| value.number_value(scope))
        .unwrap_or_else(|| fastrand::u64(100..=10_000) as f64);
    let id = last_id + fastrand::u64(1..=10) as f64;
    let id_value = v8::Number::new(scope, id);
    set_private_value(
        scope,
        performance,
        PERFORMANCE_LAST_ENTRY_ID_SLOT,
        id_value.into(),
    );
    set_performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_ID_SLOT, id);
    id
}

pub(super) fn append_performance_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) {
    let Some(entries) = performance_slot_array(scope, performance, PERFORMANCE_ENTRIES_SLOT) else {
        return;
    };
    let _ = entries.set_index(scope, entries.length(), entry.into());
}

pub(in crate::context_bootstrap) fn initialize_resource_timing_slots<'s>(
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
