use super::super::performance_observer_runtime::queue_matching_performance_observers;
use super::super::window_runtime::performance_now_callback;
use super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, serialize_v8_array,
    set_private_value,
};
use crate::webidl_iterator::{
    SnapshotWebIdlIteratorKind, invoke_webidl_collection_for_each_callback,
    new_snapshot_webidl_iterator, prepare_webidl_collection_for_each_callback,
};
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiFunctionTemplate, WebApiObject};

pub(super) const MIN_LIFECYCLE_TIMING_DELTA_MILLIS: f64 = 0.001;
pub(super) const EVENT_COUNTS_TRACKED_TYPES: &[&str] = &[
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

const DOM_CONTENT_LOADED_START_INDEX: usize = 0;
pub(super) const DOM_CONTENT_LOADED_END_INDEX: usize = 1;
const LOAD_START_INDEX: usize = 2;
pub(super) const LOAD_END_INDEX: usize = 3;
const LIFECYCLE_TIMESTAMP_COUNT: usize = 4;

#[derive(WebApiObject)]
#[webapi(interface = "Performance")]
struct PerformanceObjectDeclaration<'scope> {
    #[webapi(slot = PERFORMANCE_TIME_ORIGIN_SLOT)]
    time_origin: f64,

    #[webapi(slot = PERFORMANCE_ENTRIES_SLOT, init = "array")]
    entries: (),

    #[webapi(slot = PERFORMANCE_TIMING_SLOT)]
    timing: v8::Local<'scope, v8::Value>,

    #[webapi(slot = PERFORMANCE_NAVIGATION_SLOT)]
    navigation: v8::Local<'scope, v8::Value>,

    #[webapi(slot = PERFORMANCE_EVENT_COUNTS_SLOT)]
    event_counts: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct PerformanceJsonSnapshotDeclaration<'scope> {
    time_origin: Option<v8::Local<'scope, v8::Value>>,
    timing: Option<v8::Local<'scope, v8::Value>>,
    navigation: Option<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(interface = "PerformanceObserver")]
struct PerformanceObserverConstructorDeclaration {
    #[webapi(data_property = "supportedEntryTypes")]
    supported_entry_types: &'static [&'static str],
}

#[derive(WebApiObject)]
#[webapi(interface = "PerformanceNavigation")]
struct PerformanceNavigationObjectDeclaration {
    #[webapi(slot = PERFORMANCE_NAVIGATION_TYPE_SLOT)]
    navigation_type: f64,

    #[webapi(slot = PERFORMANCE_NAVIGATION_REDIRECT_COUNT_SLOT)]
    redirect_count: f64,

    #[webapi(method, name = "toJSON", length = 0, callback = performance_navigation_to_json_callback)]
    to_json: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "PerformanceNavigation")]
struct PerformanceNavigationConstantsDeclaration {
    #[webapi(constant = "TYPE_NAVIGATE", value = 0.0)]
    type_navigate: (),

    #[webapi(constant = "TYPE_RELOAD", value = 1.0)]
    type_reload: (),

    #[webapi(constant = "TYPE_BACK_FORWARD", value = 2.0)]
    type_back_forward: (),

    #[webapi(constant = "TYPE_RESERVED", value = 255.0)]
    type_reserved: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "EventCounts")]
struct EventCountsObjectDeclaration {
    #[webapi(slot = PERFORMANCE_EVENT_COUNTS_VALUES_SLOT)]
    values: Vec<u32>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Performance", enumerable)]
struct PerformancePrototypeMethodsDeclaration {
    #[webapi(method, length = 0, callback = performance_now_callback)]
    now: (),

    #[webapi(method, name = "toJSON", length = 0, callback = performance_to_json_callback)]
    to_json: (),

    #[webapi(method, length = 1, callback = performance_mark_callback)]
    mark: (),

    #[webapi(method, length = 0, callback = performance_clear_marks_callback)]
    clear_marks: (),

    #[webapi(method, length = 1, callback = performance_measure_callback)]
    measure: (),

    #[webapi(method, length = 0, callback = performance_clear_measures_callback)]
    clear_measures: (),

    #[webapi(method, length = 0, callback = performance_get_entries_callback)]
    get_entries: (),

    #[webapi(method, length = 1, callback = performance_get_entries_by_type_callback)]
    get_entries_by_type: (),

    #[webapi(method, length = 1, callback = performance_get_entries_by_name_callback)]
    get_entries_by_name: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Performance", enumerable)]
struct PerformancePrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = performance_attribute_getter_callback,
        data = callback_data_index_value(scope, 0)
    )]
    time_origin: (),

    #[webapi(
        accessor_property,
        getter = performance_attribute_getter_callback,
        data = callback_data_index_value(scope, 1)
    )]
    timing: (),

    #[webapi(
        accessor_property,
        getter = performance_attribute_getter_callback,
        data = callback_data_index_value(scope, 2)
    )]
    navigation: (),

    #[webapi(
        accessor_property,
        getter = performance_attribute_getter_callback,
        data = callback_data_index_value(scope, 3)
    )]
    event_counts: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "PerformanceNavigation", enumerable)]
struct PerformanceNavigationPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = performance_navigation_attribute_getter_callback,
        data = callback_data_index_value(scope, 0)
    )]
    r#type: (),

    #[webapi(
        accessor_property,
        getter = performance_navigation_attribute_getter_callback,
        data = callback_data_index_value(scope, 1)
    )]
    redirect_count: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "EventCounts", enumerable)]
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

#[derive(WebApiObject)]
#[webapi(interface = "PerformanceTiming")]
struct PerformanceTimingObjectDeclaration {
    #[webapi(data_property, enumerable)]
    navigation_start: u64,
    #[webapi(data_property, enumerable)]
    unload_event_start: u64,
    #[webapi(data_property, enumerable)]
    unload_event_end: u64,
    #[webapi(data_property, enumerable)]
    redirect_start: u64,
    #[webapi(data_property, enumerable)]
    redirect_end: u64,
    #[webapi(data_property, enumerable)]
    fetch_start: u64,
    #[webapi(data_property, enumerable)]
    domain_lookup_start: u64,
    #[webapi(data_property, enumerable)]
    domain_lookup_end: u64,
    #[webapi(data_property, enumerable)]
    connect_start: u64,
    #[webapi(data_property, enumerable)]
    connect_end: u64,
    #[webapi(data_property, enumerable)]
    secure_connection_start: u64,
    #[webapi(data_property, enumerable)]
    request_start: u64,
    #[webapi(data_property, enumerable)]
    response_start: u64,
    #[webapi(data_property, enumerable)]
    response_end: u64,
    #[webapi(data_property, enumerable)]
    dom_loading: u64,
    #[webapi(data_property, enumerable)]
    dom_interactive: u64,
    #[webapi(data_property, enumerable)]
    dom_content_loaded_event_start: u64,
    #[webapi(data_property, enumerable)]
    dom_content_loaded_event_end: u64,
    #[webapi(data_property, enumerable)]
    dom_complete: u64,
    #[webapi(data_property, enumerable)]
    load_event_start: u64,
    #[webapi(data_property, enumerable)]
    load_event_end: u64,
    #[webapi(method, name = "toJSON", length = 0, callback = performance_timing_to_json_callback)]
    to_json: (),
}

pub(in crate::context_bootstrap) fn install_performance_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    install_performance_entry_template_bindings(scope, template, interface_name);
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Performance" => {
            PerformancePrototypeMethodsDeclaration::initialize_prototype_template(scope, prototype);
            PerformancePrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            super::resource_buffer::install_resource_timing_buffer_template_bindings(
                scope, template,
            );
        }
        "PerformanceNavigation" => {
            PerformanceNavigationConstantsDeclaration::initialize_template(scope, template);
            PerformanceNavigationConstantsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            PerformanceNavigationPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "PerformanceNavigationTiming" => {
            PerformanceNavigationTimingPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "EventCounts" => {
            EventCountsPrototypeMethodsDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

pub(super) fn create_performance_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: Option<v8::Local<'s, v8::Object>>,
    navigation_type: &str,
    time_origin: f64,
) -> v8::Local<'s, v8::Object> {
    let performance = PerformanceObjectDeclaration::new(
        time_origin,
        v8::undefined(scope).into(),
        v8::undefined(scope).into(),
        v8::undefined(scope).into(),
    )
    .bind(scope)
    .expect("Performance declaration should bind");
    let navigation_type =
        v8_string(scope, navigation_type).unwrap_or_else(|| v8::String::empty(scope));
    set_private_value(
        scope,
        performance,
        PERFORMANCE_NAVIGATION_TYPE_SEED_SLOT,
        navigation_type.into(),
    );
    let navigation_entry_name = performance_navigation_entry_name(scope, window);
    let navigation_entry_name =
        v8_string(scope, &navigation_entry_name).unwrap_or_else(|| v8::String::empty(scope));
    set_private_value(
        scope,
        performance,
        PERFORMANCE_NAVIGATION_NAME_SEED_SLOT,
        navigation_entry_name.into(),
    );
    install_simple_event_target_methods(
        scope,
        performance,
        PERFORMANCE_EVENT_LISTENERS_SLOT,
        false,
    );
    install_simple_event_target_ordered_handlers(scope, performance);
    performance
}

pub(super) fn build_lazy_performance_subobject_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    subobject: super::lazy_subobjects::PerformanceSubobject,
) -> anyhow::Result<v8::Local<'s, v8::Value>> {
    let value: v8::Local<'s, v8::Value> = match subobject {
        super::lazy_subobjects::PerformanceSubobject::Timing => {
            PerformanceTimingObjectDeclaration::from(performance_timing_snapshot(
                scope,
                performance,
            ))
            .bind(scope)
            .map_err(|error| anyhow!("failed to bind PerformanceTiming object: {error}"))?
            .into()
        }
        super::lazy_subobjects::PerformanceSubobject::Navigation => {
            let navigation_type = performance_navigation_type_seed(scope, performance);
            PerformanceNavigationObjectDeclaration::new(
                performance_navigation_legacy_type(&navigation_type),
                0.0,
            )
            .bind(scope)
            .map_err(|error| anyhow!("failed to bind PerformanceNavigation object: {error}"))?
            .into()
        }
        super::lazy_subobjects::PerformanceSubobject::EventCounts => {
            let values = take_pending_performance_event_counts(scope, performance);
            EventCountsObjectDeclaration { values }
                .bind(scope)
                .map_err(|error| anyhow!("failed to bind EventCounts object: {error}"))?
                .into()
        }
    };
    Ok(value)
}

pub(super) fn apply_pending_window_performance_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    performance: v8::Local<'s, v8::Object>,
) {
    let [
        dom_content_loaded_start,
        dom_content_loaded_end,
        load_start,
        load_end,
    ] = super::window_state::take_pending_lifecycle_timestamps(scope, window);
    if let Some(timestamp) = dom_content_loaded_start {
        apply_dom_content_loaded_start(scope, performance, timestamp);
    }
    if let Some(timestamp) = dom_content_loaded_end {
        apply_dom_content_loaded_end(scope, performance, timestamp);
    }
    if let Some(timestamp) = load_start {
        apply_load_start(scope, performance, timestamp);
    }
    if let Some(timestamp) = load_end {
        apply_load_end(scope, performance, timestamp, false);
    }

    for entry in super::window_state::take_pending_resource_entries(scope, window) {
        super::append_resource_performance_entry(scope, performance, entry);
    }

    let pending_counts = super::window_state::take_pending_event_counts(scope, window);
    merge_pending_performance_event_counts(scope, performance, &pending_counts);
}

pub(crate) fn finalize_performance_observer_realm_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Object>,
) {
    PerformanceObserverConstructorDeclaration::new(PERFORMANCE_OBSERVER_SUPPORTED_ENTRY_TYPES)
        .initialize(scope, constructor)
        .expect("PerformanceObserver constructor declaration should initialize");
}

fn performance_navigation_timing_type(navigation_type: &str) -> &'static str {
    match navigation_type {
        "reload" => "reload",
        "traverse" => "back_forward",
        _ => "navigate",
    }
}

fn performance_navigation_legacy_type(navigation_type: &str) -> f64 {
    match navigation_type {
        "reload" => 1.0,
        "traverse" => 2.0,
        _ => 0.0,
    }
}

const NAV_INITIATOR_TYPE_SLOT: &str = "__moliPerformanceNavigationTimingInitiatorType";
const NAV_NEXT_HOP_PROTOCOL_SLOT: &str = "__moliPerformanceNavigationTimingNextHopProtocol";
const NAV_WORKER_START_SLOT: &str = "__moliPerformanceNavigationTimingWorkerStart";
const NAV_REDIRECT_START_SLOT: &str = "__moliPerformanceNavigationTimingRedirectStart";
const NAV_REDIRECT_END_SLOT: &str = "__moliPerformanceNavigationTimingRedirectEnd";
const NAV_FETCH_START_SLOT: &str = "__moliPerformanceNavigationTimingFetchStart";
const NAV_DOMAIN_LOOKUP_START_SLOT: &str = "__moliPerformanceNavigationTimingDomainLookupStart";
const NAV_DOMAIN_LOOKUP_END_SLOT: &str = "__moliPerformanceNavigationTimingDomainLookupEnd";
const NAV_CONNECT_START_SLOT: &str = "__moliPerformanceNavigationTimingConnectStart";
const NAV_CONNECT_END_SLOT: &str = "__moliPerformanceNavigationTimingConnectEnd";
const NAV_SECURE_CONNECTION_START_SLOT: &str =
    "__moliPerformanceNavigationTimingSecureConnectionStart";
const NAV_REQUEST_START_SLOT: &str = "__moliPerformanceNavigationTimingRequestStart";
const NAV_RESPONSE_START_SLOT: &str = "__moliPerformanceNavigationTimingResponseStart";
const NAV_RESPONSE_END_SLOT: &str = "__moliPerformanceNavigationTimingResponseEnd";
const NAV_TRANSFER_SIZE_SLOT: &str = "__moliPerformanceNavigationTimingTransferSize";
const NAV_ENCODED_BODY_SIZE_SLOT: &str = "__moliPerformanceNavigationTimingEncodedBodySize";
const NAV_DECODED_BODY_SIZE_SLOT: &str = "__moliPerformanceNavigationTimingDecodedBodySize";
const NAV_UNLOAD_EVENT_START_SLOT: &str = "__moliPerformanceNavigationTimingUnloadEventStart";
const NAV_UNLOAD_EVENT_END_SLOT: &str = "__moliPerformanceNavigationTimingUnloadEventEnd";
const NAV_DOM_INTERACTIVE_SLOT: &str = "__moliPerformanceNavigationTimingDomInteractive";
const NAV_DOM_CONTENT_LOADED_EVENT_START_SLOT: &str =
    "__moliPerformanceNavigationTimingDomContentLoadedEventStart";
const NAV_DOM_CONTENT_LOADED_EVENT_END_SLOT: &str =
    "__moliPerformanceNavigationTimingDomContentLoadedEventEnd";
const NAV_DOM_COMPLETE_SLOT: &str = "__moliPerformanceNavigationTimingDomComplete";
const NAV_LOAD_EVENT_START_SLOT: &str = "__moliPerformanceNavigationTimingLoadEventStart";
const NAV_LOAD_EVENT_END_SLOT: &str = "__moliPerformanceNavigationTimingLoadEventEnd";
const NAV_TYPE_SLOT: &str = "__moliPerformanceNavigationTimingType";
const NAV_REDIRECT_COUNT_SLOT: &str = "__moliPerformanceNavigationTimingRedirectCount";

const PERFORMANCE_NAVIGATION_TIMING_ATTRIBUTE_SLOTS: &[&str] = &[
    NAV_INITIATOR_TYPE_SLOT,
    NAV_NEXT_HOP_PROTOCOL_SLOT,
    NAV_WORKER_START_SLOT,
    NAV_REDIRECT_START_SLOT,
    NAV_REDIRECT_END_SLOT,
    NAV_FETCH_START_SLOT,
    NAV_DOMAIN_LOOKUP_START_SLOT,
    NAV_DOMAIN_LOOKUP_END_SLOT,
    NAV_CONNECT_START_SLOT,
    NAV_CONNECT_END_SLOT,
    NAV_SECURE_CONNECTION_START_SLOT,
    NAV_REQUEST_START_SLOT,
    NAV_RESPONSE_START_SLOT,
    NAV_RESPONSE_END_SLOT,
    NAV_TRANSFER_SIZE_SLOT,
    NAV_ENCODED_BODY_SIZE_SLOT,
    NAV_DECODED_BODY_SIZE_SLOT,
    NAV_UNLOAD_EVENT_START_SLOT,
    NAV_UNLOAD_EVENT_END_SLOT,
    NAV_DOM_INTERACTIVE_SLOT,
    NAV_DOM_CONTENT_LOADED_EVENT_START_SLOT,
    NAV_DOM_CONTENT_LOADED_EVENT_END_SLOT,
    NAV_DOM_COMPLETE_SLOT,
    NAV_LOAD_EVENT_START_SLOT,
    NAV_LOAD_EVENT_END_SLOT,
    NAV_TYPE_SLOT,
    NAV_REDIRECT_COUNT_SLOT,
];

#[derive(WebApiObject)]
#[webapi(interface = "PerformanceNavigationTiming")]
struct PerformanceNavigationTimingSlotDeclaration {
    #[webapi(slot = NAV_INITIATOR_TYPE_SLOT, constructor_default = "navigation")]
    initiator_type: &'static str,
    #[webapi(slot = NAV_NEXT_HOP_PROTOCOL_SLOT, constructor_default = "")]
    next_hop_protocol: &'static str,
    #[webapi(slot = NAV_WORKER_START_SLOT, constructor_default)]
    worker_start: f64,
    #[webapi(slot = NAV_REDIRECT_START_SLOT, constructor_default)]
    redirect_start: f64,
    #[webapi(slot = NAV_REDIRECT_END_SLOT, constructor_default)]
    redirect_end: f64,
    #[webapi(slot = NAV_FETCH_START_SLOT, constructor_default)]
    fetch_start: f64,
    #[webapi(slot = NAV_DOMAIN_LOOKUP_START_SLOT, constructor_default)]
    domain_lookup_start: f64,
    #[webapi(slot = NAV_DOMAIN_LOOKUP_END_SLOT, constructor_default)]
    domain_lookup_end: f64,
    #[webapi(slot = NAV_CONNECT_START_SLOT, constructor_default)]
    connect_start: f64,
    #[webapi(slot = NAV_CONNECT_END_SLOT, constructor_default)]
    connect_end: f64,
    #[webapi(slot = NAV_SECURE_CONNECTION_START_SLOT, constructor_default)]
    secure_connection_start: f64,
    #[webapi(slot = NAV_REQUEST_START_SLOT, constructor_default)]
    request_start: f64,
    #[webapi(slot = NAV_RESPONSE_START_SLOT, constructor_default)]
    response_start: f64,
    #[webapi(slot = NAV_RESPONSE_END_SLOT, constructor_default)]
    response_end: f64,
    #[webapi(slot = NAV_TRANSFER_SIZE_SLOT, constructor_default)]
    transfer_size: f64,
    #[webapi(slot = NAV_ENCODED_BODY_SIZE_SLOT, constructor_default)]
    encoded_body_size: f64,
    #[webapi(slot = NAV_DECODED_BODY_SIZE_SLOT, constructor_default)]
    decoded_body_size: f64,
    #[webapi(slot = NAV_UNLOAD_EVENT_START_SLOT, constructor_default)]
    unload_event_start: f64,
    #[webapi(slot = NAV_UNLOAD_EVENT_END_SLOT, constructor_default)]
    unload_event_end: f64,
    #[webapi(slot = NAV_DOM_INTERACTIVE_SLOT, constructor_default)]
    dom_interactive: f64,
    #[webapi(slot = NAV_DOM_CONTENT_LOADED_EVENT_START_SLOT, constructor_default)]
    dom_content_loaded_event_start: f64,
    #[webapi(slot = NAV_DOM_CONTENT_LOADED_EVENT_END_SLOT, constructor_default)]
    dom_content_loaded_event_end: f64,
    #[webapi(slot = NAV_DOM_COMPLETE_SLOT, constructor_default)]
    dom_complete: f64,
    #[webapi(slot = NAV_LOAD_EVENT_START_SLOT, constructor_default)]
    load_event_start: f64,
    #[webapi(slot = NAV_LOAD_EVENT_END_SLOT, constructor_default)]
    load_event_end: f64,
    #[webapi(slot = NAV_TYPE_SLOT)]
    navigation_type: &'static str,
    #[webapi(slot = NAV_REDIRECT_COUNT_SLOT, constructor_default)]
    redirect_count: f64,
    #[webapi(method, name = "toJSON", length = 0, callback = performance_navigation_timing_to_json_callback)]
    to_json: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "PerformanceNavigationTiming", enumerable)]
struct PerformanceNavigationTimingPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 0)
    )]
    initiator_type: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 1)
    )]
    next_hop_protocol: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 2)
    )]
    worker_start: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 3)
    )]
    redirect_start: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 4)
    )]
    redirect_end: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 5)
    )]
    fetch_start: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 6)
    )]
    domain_lookup_start: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 7)
    )]
    domain_lookup_end: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 8)
    )]
    connect_start: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 9)
    )]
    connect_end: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 10)
    )]
    secure_connection_start: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 11)
    )]
    request_start: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 12)
    )]
    response_start: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 13)
    )]
    response_end: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 14)
    )]
    transfer_size: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 15)
    )]
    encoded_body_size: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 16)
    )]
    decoded_body_size: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 17)
    )]
    unload_event_start: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 18)
    )]
    unload_event_end: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 19)
    )]
    dom_interactive: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 20)
    )]
    dom_content_loaded_event_start: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 21)
    )]
    dom_content_loaded_event_end: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 22)
    )]
    dom_complete: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 23)
    )]
    load_event_start: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 24)
    )]
    load_event_end: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 25)
    )]
    r#type: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 26)
    )]
    redirect_count: (),
}

fn performance_navigation_entry_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: Option<v8::Local<'s, v8::Object>>,
) -> String {
    let location = window
        .and_then(|window| {
            get_private_value(scope, window, WINDOW_LOCATION_SLOT)
                .filter(|value| !value.is_undefined())
        })
        .or_else(|| {
            let global = scope.get_current_context().global(scope);
            get_private_value(scope, global, WINDOW_LOCATION_SLOT)
                .filter(|value| !value.is_undefined())
        })
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    location
        .and_then(|location| object_string_property(scope, location, "href"))
        .unwrap_or_else(|| "document".to_owned())
}

fn create_navigation_performance_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation_type: &str,
    name: &str,
) -> v8::Local<'s, v8::Object> {
    let entry = create_performance_entry(scope, "navigation", name, 0.0, 0.0, None);
    PerformanceNavigationTimingSlotDeclaration::new(performance_navigation_timing_type(
        navigation_type,
    ))
    .initialize(scope, entry)
    .expect("PerformanceNavigationTiming slot declaration should initialize");
    entry
}

pub(crate) fn record_performance_dom_content_loaded_event_start(scope: &mut v8::PinScope<'_, '_>) {
    let Some(performance) = window_performance_value(scope) else {
        super::window_state::record_pending_dom_content_loaded_start(scope);
        return;
    };
    let previous = 0.0;
    let timestamp = monotonic_lifecycle_timestamp(scope, performance, previous);
    apply_dom_content_loaded_start(scope, performance, timestamp);
}

fn apply_dom_content_loaded_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    timestamp: f64,
) {
    set_performance_lifecycle_timestamp(
        scope,
        performance,
        DOM_CONTENT_LOADED_START_INDEX,
        timestamp,
    );
    update_legacy_timing(scope, performance, "domInteractive", timestamp);
    update_legacy_timing(scope, performance, "domContentLoadedEventStart", timestamp);
    if let Some(entry) = navigation_performance_entry(scope, performance) {
        set_performance_entry_slot_number(scope, entry, NAV_DOM_INTERACTIVE_SLOT, timestamp);
        set_performance_entry_slot_number(
            scope,
            entry,
            NAV_DOM_CONTENT_LOADED_EVENT_START_SLOT,
            timestamp,
        );
    }
}

pub(crate) fn record_performance_dom_content_loaded_event_end(scope: &mut v8::PinScope<'_, '_>) {
    let Some(performance) = window_performance_value(scope) else {
        super::window_state::record_pending_dom_content_loaded_end(scope);
        return;
    };
    let previous =
        performance_lifecycle_timestamp(scope, performance, DOM_CONTENT_LOADED_START_INDEX)
            .unwrap_or(0.0);
    let timestamp = monotonic_lifecycle_timestamp(scope, performance, previous);
    apply_dom_content_loaded_end(scope, performance, timestamp);
}

fn apply_dom_content_loaded_end<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    timestamp: f64,
) {
    set_performance_lifecycle_timestamp(
        scope,
        performance,
        DOM_CONTENT_LOADED_END_INDEX,
        timestamp,
    );
    update_legacy_timing(scope, performance, "domContentLoadedEventEnd", timestamp);
    if let Some(entry) = navigation_performance_entry(scope, performance) {
        set_performance_entry_slot_number(
            scope,
            entry,
            NAV_DOM_CONTENT_LOADED_EVENT_END_SLOT,
            timestamp,
        );
    }
}

pub(crate) fn record_performance_load_event_start(scope: &mut v8::PinScope<'_, '_>) {
    let Some(performance) = window_performance_value(scope) else {
        super::window_state::record_pending_load_start(scope);
        return;
    };
    let previous =
        performance_lifecycle_timestamp(scope, performance, DOM_CONTENT_LOADED_END_INDEX)
            .unwrap_or(0.0);
    let timestamp = monotonic_lifecycle_timestamp(scope, performance, previous);
    apply_load_start(scope, performance, timestamp);
}

fn apply_load_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    timestamp: f64,
) {
    set_performance_lifecycle_timestamp(scope, performance, LOAD_START_INDEX, timestamp);
    update_legacy_timing(scope, performance, "domComplete", timestamp);
    update_legacy_timing(scope, performance, "loadEventStart", timestamp);
    if let Some(entry) = navigation_performance_entry(scope, performance) {
        set_performance_entry_slot_number(scope, entry, NAV_DOM_COMPLETE_SLOT, timestamp);
        set_performance_entry_slot_number(scope, entry, NAV_LOAD_EVENT_START_SLOT, timestamp);
    }
}

pub(crate) fn record_performance_load_event_end(scope: &mut v8::PinScope<'_, '_>) {
    let Some(performance) = window_performance_value(scope) else {
        super::window_state::record_pending_load_end(scope);
        return;
    };
    if performance_lifecycle_timestamp(scope, performance, LOAD_END_INDEX).unwrap_or(0.0) > 0.0 {
        return;
    }
    let previous =
        performance_lifecycle_timestamp(scope, performance, LOAD_START_INDEX).unwrap_or(0.0);
    let timestamp = monotonic_lifecycle_timestamp(scope, performance, previous);
    apply_load_end(scope, performance, timestamp, true);
}

fn apply_load_end<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    timestamp: f64,
    notify_observers: bool,
) {
    set_performance_lifecycle_timestamp(scope, performance, LOAD_END_INDEX, timestamp);
    update_legacy_timing(scope, performance, "loadEventEnd", timestamp);
    if let Some(entry) = navigation_performance_entry(scope, performance) {
        set_performance_entry_slot_number(scope, entry, NAV_LOAD_EVENT_END_SLOT, timestamp);
        // PerformanceNavigationTiming.duration is loadEventEnd relative to timeOrigin.
        set_performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_DURATION_SLOT, timestamp);
        if notify_observers {
            queue_matching_performance_observers(scope, entry, "navigation");
        }
    }
}

fn monotonic_lifecycle_timestamp<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    previous: f64,
) -> f64 {
    let time_origin =
        performance_slot_number(scope, performance, PERFORMANCE_TIME_ORIGIN_SLOT).unwrap_or(0.0);
    let now = dom_time_since_origin_millis(time_origin).max(0.0);
    if now > previous {
        now
    } else {
        previous + MIN_LIFECYCLE_TIMING_DELTA_MILLIS
    }
}

fn update_legacy_timing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    name: &'static str,
    high_res_timestamp: f64,
) {
    let time_origin =
        performance_slot_number(scope, performance, PERFORMANCE_TIME_ORIGIN_SLOT).unwrap_or(0.0);
    if let Some(timing) = performance_slot_object(scope, performance, PERFORMANCE_TIMING_SLOT) {
        define_non_enumerable_number_property(
            scope,
            timing,
            name,
            legacy_timing_epoch_millis(time_origin + high_res_timestamp) as f64,
        );
    }
}

fn legacy_timing_epoch_millis(epoch_millis: f64) -> u64 {
    // PerformanceTiming is the legacy Navigation Timing 1 surface. Blink
    // projects its pseudo-wall timestamps through uint64_t while keeping
    // PerformanceNavigationTiming as a DOMHighResTimeStamp.
    epoch_millis as u64
}

fn navigation_performance_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, performance, PERFORMANCE_NAVIGATION_ENTRY_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn ensure_navigation_performance_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    if let Some(entry) = navigation_performance_entry(scope, performance) {
        return entry;
    }
    let navigation_type = performance_navigation_type_seed(scope, performance);
    let name = get_private_value(scope, performance, PERFORMANCE_NAVIGATION_NAME_SEED_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "document".to_owned());
    let entry = create_navigation_performance_entry(scope, &navigation_type, &name);
    set_private_value(
        scope,
        performance,
        PERFORMANCE_NAVIGATION_ENTRY_SLOT,
        entry.into(),
    );
    apply_lifecycle_to_navigation_entry(scope, performance, entry);
    append_performance_entry(scope, performance, entry);
    entry
}

fn apply_lifecycle_to_navigation_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) {
    if let Some(timestamp) =
        performance_lifecycle_timestamp(scope, performance, DOM_CONTENT_LOADED_START_INDEX)
    {
        set_performance_entry_slot_number(scope, entry, NAV_DOM_INTERACTIVE_SLOT, timestamp);
        set_performance_entry_slot_number(
            scope,
            entry,
            NAV_DOM_CONTENT_LOADED_EVENT_START_SLOT,
            timestamp,
        );
    }
    if let Some(timestamp) =
        performance_lifecycle_timestamp(scope, performance, DOM_CONTENT_LOADED_END_INDEX)
    {
        set_performance_entry_slot_number(
            scope,
            entry,
            NAV_DOM_CONTENT_LOADED_EVENT_END_SLOT,
            timestamp,
        );
    }
    if let Some(timestamp) = performance_lifecycle_timestamp(scope, performance, LOAD_START_INDEX) {
        set_performance_entry_slot_number(scope, entry, NAV_DOM_COMPLETE_SLOT, timestamp);
        set_performance_entry_slot_number(scope, entry, NAV_LOAD_EVENT_START_SLOT, timestamp);
    }
    if let Some(timestamp) = performance_lifecycle_timestamp(scope, performance, LOAD_END_INDEX) {
        set_performance_entry_slot_number(scope, entry, NAV_LOAD_EVENT_END_SLOT, timestamp);
        set_performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_DURATION_SLOT, timestamp);
    }
}

pub(super) fn performance_navigation_type_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) -> String {
    get_private_value(scope, performance, PERFORMANCE_NAVIGATION_TYPE_SEED_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "navigate".to_owned())
}

pub(super) fn performance_lifecycle_timestamp<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    index: usize,
) -> Option<f64> {
    performance_lifecycle_timestamps(scope, performance)?
        .get_index(scope, index as u32)
        .filter(|value| !value.is_undefined())
        .and_then(|value| value.number_value(scope))
}

fn set_performance_lifecycle_timestamp<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    index: usize,
    timestamp: f64,
) {
    let timestamps = performance_lifecycle_timestamps(scope, performance).unwrap_or_else(|| {
        let timestamps = v8::Array::new(scope, LIFECYCLE_TIMESTAMP_COUNT as i32);
        set_private_value(
            scope,
            performance,
            PERFORMANCE_LIFECYCLE_TIMESTAMPS_SLOT,
            timestamps.into(),
        );
        timestamps
    });
    let timestamp = v8::Number::new(scope, timestamp);
    let _ = timestamps.set_index(scope, index as u32, timestamp.into());
}

fn performance_lifecycle_timestamps<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, performance, PERFORMANCE_LIFECYCLE_TIMESTAMPS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

#[derive(Default)]
struct PerformanceTimingSnapshot {
    navigation_start: u64,
    unload_event_start: u64,
    unload_event_end: u64,
    redirect_start: u64,
    redirect_end: u64,
    fetch_start: u64,
    domain_lookup_start: u64,
    domain_lookup_end: u64,
    connect_start: u64,
    connect_end: u64,
    secure_connection_start: u64,
    request_start: u64,
    response_start: u64,
    response_end: u64,
    dom_loading: u64,
    dom_interactive: u64,
    dom_content_loaded_event_start: u64,
    dom_content_loaded_event_end: u64,
    dom_complete: u64,
    load_event_start: u64,
    load_event_end: u64,
}

impl PerformanceTimingSnapshot {
    fn new(time_origin: f64) -> Self {
        let time_origin = legacy_timing_epoch_millis(time_origin);
        Self {
            navigation_start: time_origin,
            unload_event_start: time_origin,
            unload_event_end: time_origin,
            redirect_start: time_origin,
            redirect_end: time_origin,
            fetch_start: time_origin,
            domain_lookup_start: time_origin,
            domain_lookup_end: time_origin,
            connect_start: time_origin,
            connect_end: time_origin,
            secure_connection_start: 0,
            request_start: time_origin,
            response_start: time_origin,
            response_end: time_origin,
            dom_loading: time_origin,
            dom_interactive: time_origin,
            dom_content_loaded_event_start: time_origin,
            dom_content_loaded_event_end: time_origin,
            dom_complete: time_origin,
            load_event_start: time_origin,
            load_event_end: time_origin,
        }
    }
}

fn performance_timing_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) -> PerformanceTimingSnapshot {
    let time_origin =
        performance_slot_number(scope, performance, PERFORMANCE_TIME_ORIGIN_SLOT).unwrap_or(0.0);
    let mut snapshot = PerformanceTimingSnapshot::new(time_origin);
    if let Some(timestamp) =
        performance_lifecycle_timestamp(scope, performance, DOM_CONTENT_LOADED_START_INDEX)
    {
        let timestamp = legacy_timing_epoch_millis(time_origin + timestamp);
        snapshot.dom_interactive = timestamp;
        snapshot.dom_content_loaded_event_start = timestamp;
    }
    if let Some(timestamp) =
        performance_lifecycle_timestamp(scope, performance, DOM_CONTENT_LOADED_END_INDEX)
    {
        snapshot.dom_content_loaded_event_end = legacy_timing_epoch_millis(time_origin + timestamp);
    }
    if let Some(timestamp) = performance_lifecycle_timestamp(scope, performance, LOAD_START_INDEX) {
        let timestamp = legacy_timing_epoch_millis(time_origin + timestamp);
        snapshot.dom_complete = timestamp;
        snapshot.load_event_start = timestamp;
    }
    if let Some(timestamp) = performance_lifecycle_timestamp(scope, performance, LOAD_END_INDEX) {
        snapshot.load_event_end = legacy_timing_epoch_millis(time_origin + timestamp);
    }
    snapshot
}

impl From<PerformanceTimingSnapshot> for PerformanceTimingObjectDeclaration {
    fn from(snapshot: PerformanceTimingSnapshot) -> Self {
        Self {
            navigation_start: snapshot.navigation_start,
            unload_event_start: snapshot.unload_event_start,
            unload_event_end: snapshot.unload_event_end,
            redirect_start: snapshot.redirect_start,
            redirect_end: snapshot.redirect_end,
            fetch_start: snapshot.fetch_start,
            domain_lookup_start: snapshot.domain_lookup_start,
            domain_lookup_end: snapshot.domain_lookup_end,
            connect_start: snapshot.connect_start,
            connect_end: snapshot.connect_end,
            secure_connection_start: snapshot.secure_connection_start,
            request_start: snapshot.request_start,
            response_start: snapshot.response_start,
            response_end: snapshot.response_end,
            dom_loading: snapshot.dom_loading,
            dom_interactive: snapshot.dom_interactive,
            dom_content_loaded_event_start: snapshot.dom_content_loaded_event_start,
            dom_content_loaded_event_end: snapshot.dom_content_loaded_event_end,
            dom_complete: snapshot.dom_complete,
            load_event_start: snapshot.load_event_start,
            load_event_end: snapshot.load_event_end,
            to_json: (),
        }
    }
}

fn performance_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        PERFORMANCE_ATTRIBUTE_SLOTS,
        "Performance attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    if let Some(subobject) = super::lazy_subobjects::PerformanceSubobject::from_slot(slot) {
        if performance_slot_value(scope, args.this(), PERFORMANCE_TIME_ORIGIN_SLOT).is_none() {
            rv.set_undefined();
            return;
        }
        match super::lazy_subobjects::ensure_performance_subobject(scope, args.this(), subobject) {
            Ok(value) => rv.set(value),
            Err(error) => throw_type_error(scope, &error.to_string()),
        }
        return;
    }
    rv.set(
        performance_slot_value(scope, args.this(), slot)
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

const PERFORMANCE_ATTRIBUTE_SLOTS: &[&str] = &[
    PERFORMANCE_TIME_ORIGIN_SLOT,
    PERFORMANCE_TIMING_SLOT,
    PERFORMANCE_NAVIGATION_SLOT,
    PERFORMANCE_EVENT_COUNTS_SLOT,
];

const PERFORMANCE_TIMING_JSON_KEYS: &[&str] = &[
    "navigationStart",
    "unloadEventStart",
    "unloadEventEnd",
    "redirectStart",
    "redirectEnd",
    "fetchStart",
    "domainLookupStart",
    "domainLookupEnd",
    "connectStart",
    "connectEnd",
    "secureConnectionStart",
    "requestStart",
    "responseStart",
    "responseEnd",
    "domLoading",
    "domInteractive",
    "domContentLoadedEventStart",
    "domContentLoadedEventEnd",
    "domComplete",
    "loadEventStart",
    "loadEventEnd",
];

const PERFORMANCE_NAVIGATION_JSON_KEYS: &[&str] = &["type", "redirectCount"];

const PERFORMANCE_NAVIGATION_ATTRIBUTE_SLOTS: &[&str] = &[
    PERFORMANCE_NAVIGATION_TYPE_SLOT,
    PERFORMANCE_NAVIGATION_REDIRECT_COUNT_SLOT,
];

fn performance_navigation_timing_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        PERFORMANCE_NAVIGATION_TIMING_ATTRIBUTE_SLOTS,
        "PerformanceNavigationTiming attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    rv.set(
        performance_entry_slot_value(scope, args.this(), slot)
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

fn performance_navigation_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        PERFORMANCE_NAVIGATION_ATTRIBUTE_SLOTS,
        "PerformanceNavigation attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

pub(in crate::context_bootstrap) fn event_counts_size_getter(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Integer::new(scope, EVENT_COUNTS_TRACKED_TYPES.len() as i32).into());
}

pub(in crate::context_bootstrap) fn event_counts_get_callback<'s>(
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

pub(in crate::context_bootstrap) fn event_counts_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let tracked = event_counts_arg_is_tracked(scope, args.get(0));
    rv.set(v8::Boolean::new(scope, tracked).into());
}

pub(in crate::context_bootstrap) fn event_counts_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let array = crate::util::serialize_v8_array(scope, EVENT_COUNTS_TRACKED_TYPES)
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    set_event_counts_iterator(scope, array, &mut rv);
}

pub(in crate::context_bootstrap) fn event_counts_values_callback<'s>(
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

pub(in crate::context_bootstrap) fn event_counts_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let array = event_counts_entries_array(scope, args.this());
    set_event_counts_iterator(scope, array, &mut rv);
}

pub(in crate::context_bootstrap) fn event_counts_for_each_callback<'s>(
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

pub(crate) fn increment_performance_event_count(
    scope: &mut v8::PinScope<'_, '_>,
    event_type: &str,
) {
    let Some(index) = event_counts_tracked_index(event_type) else {
        return;
    };
    let Some(performance) = window_performance_value(scope) else {
        super::window_state::increment_pending_event_count(scope, index);
        return;
    };
    let Some(event_counts) =
        performance_slot_object(scope, performance, PERFORMANCE_EVENT_COUNTS_SLOT)
    else {
        increment_pending_performance_event_count(scope, performance, index);
        return;
    };
    let Some(values) = event_counts_values_array(scope, event_counts) else {
        return;
    };
    let current = values
        .get_index(scope, index as u32)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    let next = v8::Integer::new_from_unsigned(scope, current.saturating_add(1));
    let _ = values.set_index(scope, index as u32, next.into());
}

fn increment_pending_performance_event_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    index: usize,
) {
    let values = ensure_pending_performance_event_counts(scope, performance);
    let current = values
        .get_index(scope, index as u32)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    let next = v8::Integer::new_from_unsigned(scope, current.saturating_add(1));
    let _ = values.set_index(scope, index as u32, next.into());
}

fn merge_pending_performance_event_counts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    pending: &[u32],
) {
    if pending.iter().all(|count| *count == 0) {
        return;
    }
    let values = ensure_pending_performance_event_counts(scope, performance);
    for (index, count) in pending.iter().copied().enumerate() {
        if count == 0 {
            continue;
        }
        let current = values
            .get_index(scope, index as u32)
            .and_then(|value| value.uint32_value(scope))
            .unwrap_or(0);
        let next = v8::Integer::new_from_unsigned(scope, current.saturating_add(count));
        let _ = values.set_index(scope, index as u32, next.into());
    }
}

fn take_pending_performance_event_counts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) -> Vec<u32> {
    let values = get_private_value(scope, performance, PERFORMANCE_PENDING_EVENT_COUNTS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok());
    set_private_value(
        scope,
        performance,
        PERFORMANCE_PENDING_EVENT_COUNTS_SLOT,
        v8::undefined(scope).into(),
    );
    (0..EVENT_COUNTS_TRACKED_TYPES.len())
        .map(|index| {
            values
                .and_then(|values| values.get_index(scope, index as u32))
                .and_then(|value| value.uint32_value(scope))
                .unwrap_or(0)
        })
        .collect()
}

fn ensure_pending_performance_event_counts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    if let Some(values) =
        get_private_value(scope, performance, PERFORMANCE_PENDING_EVENT_COUNTS_SLOT)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        return values;
    }
    let values = v8::Array::new(scope, EVENT_COUNTS_TRACKED_TYPES.len() as i32);
    set_private_value(
        scope,
        performance,
        PERFORMANCE_PENDING_EVENT_COUNTS_SLOT,
        values.into(),
    );
    values
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

fn event_counts_tracked_index(event_type: &str) -> Option<usize> {
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

fn event_counts_values_array<'s>(
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

pub(in crate::context_bootstrap) fn performance_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let timing = object_property_as_object(scope, args.this(), "timing")
        .map(|timing| object_json_snapshot(scope, timing, PERFORMANCE_TIMING_JSON_KEYS).into());
    let navigation =
        object_property_as_object(scope, args.this(), "navigation").map(|navigation| {
            object_json_snapshot(scope, navigation, PERFORMANCE_NAVIGATION_JSON_KEYS).into()
        });
    let output = PerformanceJsonSnapshotDeclaration {
        time_origin: args.this().get(scope, v8str(scope, "timeOrigin").into()),
        timing,
        navigation,
    }
    .bind(scope)
    .expect("Performance toJSON snapshot declaration should bind");
    rv.set(output.into());
}

fn performance_timing_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(object_json_snapshot(scope, args.this(), PERFORMANCE_TIMING_JSON_KEYS).into());
}

fn performance_navigation_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(object_json_snapshot(scope, args.this(), PERFORMANCE_NAVIGATION_JSON_KEYS).into());
}

fn performance_navigation_timing_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let output = object_json_snapshot(
        scope,
        args.this(),
        &[
            "name",
            "entryType",
            "startTime",
            "duration",
            "initiatorType",
            "nextHopProtocol",
            "workerStart",
            "redirectStart",
            "redirectEnd",
            "fetchStart",
            "domainLookupStart",
            "domainLookupEnd",
            "connectStart",
            "connectEnd",
            "secureConnectionStart",
            "requestStart",
            "responseStart",
            "responseEnd",
            "transferSize",
            "encodedBodySize",
            "decodedBodySize",
            "unloadEventStart",
            "unloadEventEnd",
            "domInteractive",
            "domContentLoadedEventStart",
            "domContentLoadedEventEnd",
            "domComplete",
            "loadEventStart",
            "loadEventEnd",
            "type",
            "redirectCount",
        ],
    );
    rv.set(output.into());
}

fn object_json_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    keys: &[&str],
) -> v8::Local<'s, v8::Object> {
    let output = ObjectLiteralDeclaration::bind(scope);
    for key in keys {
        output.copy_string_property(scope, source, key);
    }
    output.into_object()
}
