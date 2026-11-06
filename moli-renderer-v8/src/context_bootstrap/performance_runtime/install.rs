use super::super::performance_observer_runtime::queue_matching_performance_observers;
use super::super::window_runtime::performance_now_callback;
use super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, serialize_v8_array,
    set_private_value,
};
use crate::web_api_interfaces;
use anyhow::{Result, anyhow};
use moli_webapi::performance::event_counts::{
    EVENT_COUNTS_TRACKED_TYPES, event_counts_tracked_index, event_counts_values_array,
};
pub(super) use moli_webapi::performance::legacy::*;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const DOM_CONTENT_LOADED_START_INDEX: usize = 0;
pub(super) const DOM_CONTENT_LOADED_END_INDEX: usize = 1;
const LOAD_START_INDEX: usize = 2;
pub(super) const LOAD_END_INDEX: usize = 3;
const LIFECYCLE_TIMESTAMP_COUNT: usize = 4;

fn initial_performance_entry_id() -> f64 {
    fastrand::u64(100..=10_000) as f64
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Performance)]
struct PerformanceObjectDeclaration<'scope> {
    #[webapi(slot = PERFORMANCE_TIME_ORIGIN_SLOT)]
    time_origin: f64,

    #[webapi(slot = PERFORMANCE_LAST_ENTRY_ID_SLOT)]
    last_entry_id: f64,

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
#[webapi(interface = web_api_interfaces::Performance)]
struct WorkerPerformanceObjectDeclaration {
    #[webapi(slot = PERFORMANCE_TIME_ORIGIN_SLOT)]
    time_origin: f64,

    #[webapi(slot = PERFORMANCE_LAST_ENTRY_ID_SLOT)]
    last_entry_id: f64,

    #[webapi(slot = PERFORMANCE_ENTRIES_SLOT, init = "array")]
    entries: (),
}

#[derive(Default, WebApiObject)]
#[webapi(fragment)]
struct PerformanceObserverRuntimeStateDeclaration {
    #[webapi(slot = PERFORMANCE_OBSERVER_QUEUE_SLOT, init = "array")]
    queue: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Performance, enumerable)]
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
#[webapi(interface = web_api_interfaces::Performance, enumerable)]
struct PerformancePrototypeCommonAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = performance_attribute_getter_callback,
        data = callback_data_index_value(scope, 0)
    )]
    time_origin: (),
}

#[derive(Default, WebApiObject)]
#[webapi(fragment, prototype = "Performance", enumerable, receiver = web_api_interfaces::Performance::is_instance)]
struct PerformancePrototypeWindowAccessorsDeclaration {
    #[webapi(accessor_property, getter = super::memory::performance_memory_getter)]
    memory: (),

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

pub(in crate::context_bootstrap) fn install_performance_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    install_performance_entry_template_bindings(scope, template, interface_name);
    super::measurement_interfaces::install(scope, template, interface_name);
    let prototype = template.prototype_template(scope);
    moli_webapi::performance::legacy::install(scope, template, interface_name);
    match interface_name {
        "Performance" => {
            PerformancePrototypeMethodsDeclaration::initialize_prototype_template(scope, prototype);
            PerformancePrototypeCommonAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            super::resource_buffer::install_resource_timing_buffer_template_bindings(
                scope, template,
            );
        }
        "EventCounts" => {
            moli_webapi::performance::event_counts::install(scope, template);
        }
        _ => {}
    }
}

pub(in crate::context_bootstrap) fn install_worker_performance_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    install_performance_observer_runtime_state(
        scope,
        global,
        WORKER_PERFORMANCE_OBSERVER_SUPPORTED_ENTRY_TYPES,
    )?;
    super::super::exposed_interfaces::ensure_intrinsic_interface_constructor(scope, "Performance")?;

    let performance = WorkerPerformanceObjectDeclaration::new(
        unix_epoch_millis(),
        initial_performance_entry_id(),
    )
    .bind(scope)
    .map_err(|error| anyhow!("failed to create worker performance: {error}"))?;
    initialize_simple_event_target(scope, performance);

    set_private_value(scope, global, WINDOW_PERFORMANCE_SLOT, performance.into());
    Ok(())
}

pub(super) fn install_performance_observer_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    supported_entry_types: &'static [&'static str],
) -> Result<()> {
    PerformanceObserverRuntimeStateDeclaration::default().initialize(scope, global)?;
    let supported_entry_types_array = serialize_v8_array(scope, supported_entry_types)
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    set_private_value(
        scope,
        global,
        PERFORMANCE_OBSERVER_SUPPORTED_ENTRY_TYPES_SLOT,
        supported_entry_types_array.into(),
    );
    Ok(())
}

pub(super) fn create_performance_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: Option<v8::Local<'s, v8::Object>>,
    navigation_type: &str,
    time_origin: f64,
) -> v8::Local<'s, v8::Object> {
    let performance = PerformanceObjectDeclaration::new(
        time_origin,
        initial_performance_entry_id(),
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
    initialize_simple_event_target(scope, performance);

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
            moli_webapi::performance::event_counts::new_event_counts(scope, values)?.into()
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
    ] = super::window_state::take_projected_lifecycle_timestamps(scope, window);
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

pub(crate) fn finalize_window_performance_realm_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> Result<()> {
    PerformancePrototypeWindowAccessorsDeclaration::default().initialize(scope, prototype)?;
    Ok(())
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
    performance: v8::Local<'s, v8::Object>,
    navigation_type: &str,
    name: &str,
) -> v8::Local<'s, v8::Object> {
    let entry = create_performance_entry(
        scope,
        performance,
        "navigation",
        name.into(),
        0.0,
        0.0,
        None,
    );
    super::entries::initialize_resource_timing_slots(
        scope,
        entry,
        "navigation",
        0.0,
        0.0,
        0.0,
        "non-blocking",
        0.0,
        "",
    );
    PerformanceNavigationTimingSlotDeclaration::new(performance_navigation_timing_type(
        navigation_type,
    ))
    .initialize(scope, entry)
    .expect("PerformanceNavigationTiming slot declaration should initialize");
    super::navigation_timing::refresh_navigation_timing(scope, entry);
    entry
}

/// Update JavaScript projections from the native Document clock. This never
/// reads the current time or creates a Performance object.
pub(crate) fn project_document_lifecycle_timing(
    scope: &mut v8::PinScope<'_, '_>,
    timestamps: [Option<f64>; 4],
) {
    let window = scope.get_current_context().global(scope);
    project_window_lifecycle_timing(scope, window, timestamps, true);
}

pub(super) fn project_window_lifecycle_timing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    timestamps: [Option<f64>; 4],
    notify_observers: bool,
) {
    let Some(performance) = get_private_value(scope, window, WINDOW_PERFORMANCE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        super::window_state::store_projected_lifecycle_timestamps(scope, window, timestamps);
        return;
    };
    let load_already_ended =
        performance_lifecycle_timestamp(scope, performance, LOAD_END_INDEX).is_some();
    if let Some(timestamp) = timestamps[DOM_CONTENT_LOADED_START_INDEX] {
        apply_dom_content_loaded_start(scope, performance, timestamp);
    }
    if let Some(timestamp) = timestamps[DOM_CONTENT_LOADED_END_INDEX] {
        apply_dom_content_loaded_end(scope, performance, timestamp);
    }
    if let Some(timestamp) = timestamps[LOAD_START_INDEX] {
        apply_load_start(scope, performance, timestamp);
    }
    if let Some(timestamp) = timestamps[LOAD_END_INDEX] {
        apply_load_end(
            scope,
            performance,
            timestamp,
            notify_observers && !load_already_ended,
        );
    }
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
            queue_matching_performance_observers(scope, performance, entry, "navigation");
        }
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
        set_performance_timing_value(
            scope,
            timing,
            name,
            legacy_timing_epoch_millis(time_origin + high_res_timestamp) as f64,
        );
    }
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
) {
    if navigation_performance_entry(scope, performance).is_some()
        || !is_window_performance(scope, performance)
    {
        return;
    }
    let navigation_type = performance_navigation_type_seed(scope, performance);
    let name = get_private_value(scope, performance, PERFORMANCE_NAVIGATION_NAME_SEED_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "document".to_owned());
    let entry = create_navigation_performance_entry(scope, performance, &navigation_type, &name);
    super::entries::assign_navigation_performance_entry_identity(scope, performance, entry);
    set_private_value(
        scope,
        performance,
        PERFORMANCE_NAVIGATION_ENTRY_SLOT,
        entry.into(),
    );
    apply_lifecycle_to_navigation_entry(scope, performance, entry);
    append_performance_entry(scope, performance, entry);
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

pub(super) fn is_window_performance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, performance, PERFORMANCE_NAVIGATION_TYPE_SEED_SLOT).is_some()
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

pub(in crate::context_bootstrap) fn performance_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let performance = args.this();
    let mut timing = None;
    let mut navigation = None;
    if is_window_performance(scope, performance) {
        use super::lazy_subobjects::{PerformanceSubobject, ensure_performance_subobject};
        for (subobject, output) in [
            (PerformanceSubobject::Timing, &mut timing),
            (PerformanceSubobject::Navigation, &mut navigation),
        ] {
            match ensure_performance_subobject(scope, performance, subobject) {
                Ok(value) => *output = Some(value),
                Err(error) => {
                    throw_type_error(scope, &error.to_string());
                    return;
                }
            }
        }
    }
    // Default toJSON uses IDL attribute values. Object-valued attributes retain
    // their native identity; JSON.stringify invokes their own toJSON later.
    let time_origin = get_private_value(scope, performance, PERFORMANCE_TIME_ORIGIN_SLOT);
    let output = new_performance_json_snapshot(scope, time_origin, timing, navigation);
    rv.set(output.into());
}
