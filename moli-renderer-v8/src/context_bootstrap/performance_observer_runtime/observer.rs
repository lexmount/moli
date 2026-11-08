use super::delivery::{enqueue_buffered_performance_entries, queue_performance_observer_delivery};
use super::registration::{
    PerformanceObserverRegistration, register_performance_observer_callback,
};
use super::*;
use crate::util::{
    define_v8_array_data_property, get_private_value, serialize_v8_iter_array, set_private_value,
};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PerformanceObserver)]
struct PerformanceObserverObjectDeclaration<'s> {
    #[webapi(slot = PERFORMANCE_OBSERVER_CALLBACK_ID_SLOT)]
    callback_id: u32,
    #[webapi(slot = PERFORMANCE_OBSERVER_CALLBACK_VALUE_SLOT)]
    callback_value: v8::Local<'s, v8::Object>,
    #[webapi(slot = PERFORMANCE_OBSERVER_CALLBACK_RELEVANT_GLOBAL_SLOT)]
    callback_relevant_global: v8::Local<'s, v8::Object>,
    #[webapi(slot = PERFORMANCE_OBSERVER_CALLBACK_INCUMBENT_GLOBAL_SLOT)]
    callback_incumbent_global: v8::Local<'s, v8::Object>,
    #[webapi(slot = PERFORMANCE_OBSERVER_PENDING_SLOT, init = "array")]
    pending: (),
    #[webapi(slot = PERFORMANCE_OBSERVER_TYPE_SLOT, init = "null")]
    observed_type: (),
    #[webapi(slot = PERFORMANCE_OBSERVER_MODE_SLOT, init = 0)]
    observe_mode: (),
    #[webapi(slot = PERFORMANCE_OBSERVER_ENTRY_TYPES_SLOT, init = "array")]
    entry_types: (),
    #[webapi(slot = PERFORMANCE_OBSERVER_ACTIVE_SLOT, init = false)]
    active: (),
    #[webapi(slot = PERFORMANCE_OBSERVER_SCHEDULED_SLOT, init = false)]
    scheduled: (),
    #[webapi(slot = PERFORMANCE_OBSERVER_REQUIRES_DROPPED_ENTRIES_SLOT, init = false)]
    requires_dropped_entries: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PerformanceObserver")]
struct PerformanceObserverConstructorArgs {
    #[webidl(
        required,
        converter = "callback_function",
        missing_message = "Failed to construct 'PerformanceObserver': parameter 1 is not a function."
    )]
    callback: webidl::WebIdlCallbackFunction,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PerformanceObserver.observe")]
struct PerformanceObserverObserveArgs {
    #[webidl(required, dictionary)]
    options: PerformanceObserverInit,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PerformanceObserverInit")]
struct PerformanceObserverInit {
    buffered: Option<bool>,
    #[webidl(converter = "double")]
    duration_threshold: Option<f64>,
    #[webidl(converter = "raw")]
    entry_types: Option<webidl::Sequence<webidl::DomString>>,
    #[webidl(name = "type")]
    observed_type: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
#[repr(u32)]
enum ObserveMode {
    Undefined = 0,
    Single = 1,
    Multiple = 2,
}

pub(in crate::context_bootstrap) fn performance_observer_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'PerformanceObserver': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<PerformanceObserverConstructorArgs>(scope, &args)
    else {
        return;
    };
    let Some((callback_id, callback, relevant_global, incumbent_global)) =
        register_performance_observer_callback(scope, args.this(), parsed.callback)
    else {
        throw_type_error(
            scope,
            "PerformanceObserver is unavailable outside a Window or Worker realm.",
        );
        return;
    };
    PerformanceObserverObjectDeclaration::new(
        callback_id,
        callback,
        relevant_global,
        incumbent_global,
    )
    .initialize(scope, args.this())
    .expect("PerformanceObserver declaration should initialize object");
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn performance_observer_observe_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<PerformanceObserverObserveArgs>(scope, &args) else {
        return;
    };
    let init = parsed.options;
    let has_entry_types_member = init.entry_types.is_some();
    let has_type_member = init.observed_type.is_some();
    if has_entry_types_member
        && (has_type_member || init.buffered.is_some() || init.duration_threshold.is_some())
    {
        throw_type_error(
            scope,
            "PerformanceObserver.observe entryTypes must not be combined with other options.",
        );
        return;
    }
    if !has_entry_types_member && !has_type_member {
        throw_type_error(
            scope,
            "PerformanceObserver.observe options must include either type or entryTypes.",
        );
        return;
    }

    let requested_mode = if has_entry_types_member {
        ObserveMode::Multiple
    } else {
        ObserveMode::Single
    };
    let current_mode = performance_observer_mode(scope, args.this());
    if current_mode == ObserveMode::Single && requested_mode == ObserveMode::Multiple {
        webidl::throw_dom_exception(
            scope,
            "InvalidModificationError",
            "This PerformanceObserver has performed observe({type: ...}) and cannot observe entryTypes.",
        );
        return;
    }
    if current_mode == ObserveMode::Multiple && requested_mode == ObserveMode::Single {
        webidl::throw_dom_exception(
            scope,
            "InvalidModificationError",
            "This PerformanceObserver has performed observe({entryTypes: ...}) and cannot observe type.",
        );
        return;
    }
    if current_mode == ObserveMode::Undefined {
        // Choosing a mode precedes support filtering, which may abort registration.
        let value = v8::Integer::new_from_unsigned(scope, requested_mode as u32);
        set_performance_observer_slot_value(
            scope,
            args.this(),
            PERFORMANCE_OBSERVER_MODE_SLOT,
            value.into(),
        );
    }

    // observe() rearms reporting before support filtering. An unsupported
    // type or empty entryTypes keeps an existing registration, but still
    // requests its cumulative count on the next nonempty delivery.
    set_performance_observer_bool_slot(
        scope,
        args.this(),
        PERFORMANCE_OBSERVER_REQUIRES_DROPPED_ENTRIES_SLOT,
        true,
    );
    let observed_entry_types = if let Some(observed_type) = init.observed_type.as_deref() {
        if !performance_observer_entry_type_supported(scope, observed_type) {
            rv.set_undefined();
            return;
        }
        let entry_types = performance_observer_entry_types(scope, args.this())
            .unwrap_or_else(|| v8::Array::new(scope, 0));
        if !array_contains_string(scope, entry_types, observed_type) {
            let value = v8_string(scope, observed_type)
                .unwrap_or_else(|| v8::String::empty(scope))
                .into();
            let _ = define_v8_array_data_property(scope, entry_types, entry_types.length(), value);
        }
        entry_types
    } else {
        let supported_entry_types = init
            .entry_types
            .map(|entry_types| entry_types.0)
            .unwrap_or_default()
            .into_iter()
            .map(|entry_type| entry_type.0)
            .filter(|entry_type| performance_observer_entry_type_supported(scope, entry_type))
            .collect::<Vec<_>>();
        performance_entry_types_array_from_strings(scope, supported_entry_types)
    };
    if observed_entry_types.length() == 0 {
        rv.set_undefined();
        return;
    }
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    // Web IDL conversion and option validation above use the method Realm.
    // Registration, buffered records and delivery use the observer's global.
    let scope = &mut v8::ContextScope::new(scope, context);
    let performance = super::super::performance_runtime::ensure_current_performance_for_api(scope);
    let observes_navigation = (0..observed_entry_types.length()).any(|index| {
        observed_entry_types
            .get_index(scope, index)
            .and_then(|value| value.to_string(scope))
            .is_some_and(|value| value.to_rust_string_lossy(scope) == "navigation")
    });
    if observes_navigation && let Some(performance) = performance {
        super::super::performance_runtime::ensure_navigation_performance_entry_for_api(
            scope,
            performance,
        );
    }
    let observed_type = init
        .observed_type
        .as_deref()
        .and_then(|value| v8_string(scope, value))
        .map(|value| value.into())
        .unwrap_or_else(|| v8::null(scope).into());
    set_performance_observer_type(scope, args.this(), observed_type);
    set_performance_observer_entry_types(scope, args.this(), observed_entry_types);
    let activated = PerformanceObserverRegistration::from_observer(scope, args.this())
        .is_some_and(|registration| registration.activate(scope));
    if !activated {
        set_performance_observer_active(scope, args.this(), false);
        let pending = v8::Array::new(scope, 0);
        set_performance_observer_pending(scope, args.this(), pending);
        rv.set_undefined();
        return;
    }
    set_performance_observer_active(scope, args.this(), true);
    if init.buffered.unwrap_or(false) && has_type_member {
        if let Some(performance) = performance {
            enqueue_buffered_performance_entries(scope, performance, args.this());
        }
        queue_performance_observer_delivery(scope, args.this());
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn performance_observer_disconnect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_performance_observer_active(scope, args.this(), false);
    if let Some(registration) = PerformanceObserverRegistration::from_observer(scope, args.this()) {
        registration.deactivate(scope);
    }
    let pending = v8::Array::new(scope, 0);
    set_performance_observer_pending(scope, args.this(), pending);
    let entry_types = v8::Array::new(scope, 0);
    set_performance_observer_entry_types(scope, args.this(), entry_types);
    set_performance_observer_type(scope, args.this(), v8::null(scope).into());
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn performance_observer_take_records_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let records = performance_observer_pending(scope, args.this())
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    let pending = v8::Array::new(scope, 0);
    set_performance_observer_pending(scope, args.this(), pending);
    rv.set(records.into());
}

fn performance_entry_types_array_from_strings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: impl IntoIterator<Item = String>,
) -> v8::Local<'s, v8::Array> {
    let mut entry_types = Vec::new();
    for entry_type in values {
        if entry_types.iter().any(|existing| existing == &entry_type) {
            continue;
        }
        entry_types.push(entry_type);
    }
    serialize_v8_iter_array(scope, entry_types).unwrap_or_else(|| v8::Array::new(scope, 0))
}

fn performance_observer_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> ObserveMode {
    match performance_observer_slot_value(scope, observer, PERFORMANCE_OBSERVER_MODE_SLOT)
        .and_then(|value| v8::Local::<v8::Uint32>::try_from(value).ok())
        .map(|value| value.value())
    {
        Some(1) => ObserveMode::Single,
        Some(2) => ObserveMode::Multiple,
        _ => ObserveMode::Undefined,
    }
}

fn performance_observer_entry_type_supported(
    scope: &mut v8::PinScope<'_, '_>,
    entry_type: &str,
) -> bool {
    let global = scope.get_current_context().global(scope);
    get_private_value(
        scope,
        global,
        PERFORMANCE_OBSERVER_SUPPORTED_ENTRY_TYPES_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    .is_some_and(|entry_types| array_contains_string(scope, entry_types, entry_type))
}

pub(super) fn array_contains_string(
    scope: &mut v8::PinScope<'_, '_>,
    array: v8::Local<'_, v8::Array>,
    expected: &str,
) -> bool {
    (0..array.length()).any(|index| {
        array
            .get_index(scope, index)
            .and_then(|value| value.to_string(scope))
            .is_some_and(|value| value.to_rust_string_lossy(scope) == expected)
    })
}

pub(super) fn performance_observer_pending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    performance_observer_slot_value(scope, observer, PERFORMANCE_OBSERVER_PENDING_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(super) fn take_performance_observer_dropped_entries_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    if !performance_observer_slot_bool(
        scope,
        observer,
        PERFORMANCE_OBSERVER_REQUIRES_DROPPED_ENTRIES_SLOT,
    )
    .unwrap_or(false)
    {
        return None;
    }
    // Resource Timing is the bounded entry buffer currently exposed by
    // Moli; mark/measure and the other timelines do not discard queued entries.
    let observes_resource = performance_observer_entry_types(scope, observer)
        .is_some_and(|types| array_contains_string(scope, types, "resource"));
    let count = if observes_resource {
        super::super::performance_runtime::ensure_current_performance_for_api(scope).map_or(
            0,
            |performance| {
                super::super::performance_runtime::resource_timing_dropped_entries_count(
                    scope,
                    performance,
                )
            },
        )
    } else {
        0
    };
    set_performance_observer_bool_slot(
        scope,
        observer,
        PERFORMANCE_OBSERVER_REQUIRES_DROPPED_ENTRIES_SLOT,
        false,
    );
    Some(count)
}

pub(super) fn set_performance_observer_pending(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    pending: v8::Local<'_, v8::Array>,
) {
    set_performance_observer_slot_value(
        scope,
        observer,
        PERFORMANCE_OBSERVER_PENDING_SLOT,
        pending.into(),
    );
}

pub(super) fn performance_observer_entry_types<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    performance_observer_slot_value(scope, observer, PERFORMANCE_OBSERVER_ENTRY_TYPES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn set_performance_observer_entry_types(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    entry_types: v8::Local<'_, v8::Array>,
) {
    set_performance_observer_slot_value(
        scope,
        observer,
        PERFORMANCE_OBSERVER_ENTRY_TYPES_SLOT,
        entry_types.into(),
    );
}

pub(super) fn performance_observer_observed_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<String> {
    performance_observer_slot_value(scope, observer, PERFORMANCE_OBSERVER_TYPE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn set_performance_observer_type(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    observed_type: v8::Local<'_, v8::Value>,
) {
    set_performance_observer_slot_value(
        scope,
        observer,
        PERFORMANCE_OBSERVER_TYPE_SLOT,
        observed_type,
    );
}

pub(super) fn performance_observer_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> bool {
    performance_observer_slot_bool(scope, observer, PERFORMANCE_OBSERVER_ACTIVE_SLOT)
        .unwrap_or(false)
}

fn set_performance_observer_active(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    active: bool,
) {
    set_performance_observer_bool_slot(scope, observer, PERFORMANCE_OBSERVER_ACTIVE_SLOT, active);
}

pub(super) fn performance_observer_scheduled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> bool {
    performance_observer_slot_bool(scope, observer, PERFORMANCE_OBSERVER_SCHEDULED_SLOT)
        .unwrap_or(false)
}

pub(super) fn set_performance_observer_scheduled(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    scheduled: bool,
) {
    set_performance_observer_bool_slot(
        scope,
        observer,
        PERFORMANCE_OBSERVER_SCHEDULED_SLOT,
        scheduled,
    );
}

fn performance_observer_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, observer, slot)
}

fn performance_observer_slot_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<bool> {
    performance_observer_slot_value(scope, observer, slot).map(|value| value.boolean_value(scope))
}

fn set_performance_observer_slot_value(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    set_private_value(scope, observer, slot, value);
}

fn set_performance_observer_bool_slot(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: bool,
) {
    let value = v8::Boolean::new(scope, value);
    set_performance_observer_slot_value(scope, observer, slot, value.into());
}
