use super::observer::{
    array_contains_string, performance_observer_active, performance_observer_entry_types,
    performance_observer_observed_type, performance_observer_pending,
    performance_observer_scheduled, set_performance_observer_pending,
    set_performance_observer_scheduled,
};
use super::registration::{
    PerformanceObserverRegistration, active_performance_observers,
    performance_observer_callback_is_current,
};
use super::*;
use crate::web_api_interfaces;
use crate::webidl::{self, SnapshotPrototype, WebIdlDictionarySerialize};
use moli_webapi_declare::WebApiObject;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PerformanceObserverCallbackOptions", snapshot)]
struct PerformanceObserverCallbackOptions {
    #[webidl(converter = "unsigned_long_long")]
    dropped_entries_count: Option<u64>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PerformanceObserverEntryList)]
struct PerformanceObserverEntryListObjectDeclaration<'scope> {
    #[webapi(slot = PERFORMANCE_ENTRY_LIST_ENTRIES_SLOT)]
    entries: v8::Local<'scope, v8::Array>,
}

fn performance_observer_flush_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(observer) = pop_first_object_from_global_queue(scope, PERFORMANCE_OBSERVER_QUEUE_SLOT)
    else {
        return;
    };
    set_performance_observer_scheduled(scope, observer, false);
    if !performance_observer_active(scope, observer) {
        return;
    }
    let Some(pending) = performance_observer_pending(scope, observer) else {
        return;
    };
    if pending.length() == 0 {
        return;
    }
    let next_pending = v8::Array::new(scope, 0);
    set_performance_observer_pending(scope, observer, next_pending);
    let Some(delivery) = PerformanceObserverRegistration::from_observer(scope, observer)
        .and_then(|registration| registration.prepare_delivery(scope))
    else {
        return;
    };
    let dropped_entries_count =
        super::observer::take_performance_observer_dropped_entries_count(scope, observer);
    // Callback arguments are wrapped in the callback's relevant Realm, while
    // subscriptions and cumulative counters belong to the observer's global.
    let (list, options) = {
        let context = delivery.relevant_context(scope);
        let scope = &mut v8::ContextScope::new(scope, context);
        let list = PerformanceObserverEntryListObjectDeclaration::new(pending)
            .bind(scope)
            .expect("PerformanceObserverEntryList declaration should bind");
        let options = PerformanceObserverCallbackOptions {
            dropped_entries_count,
        }
        .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
        .expect("PerformanceObserver callback options should serialize");
        (list, options)
    };
    let observer_value: v8::Local<'_, v8::Value> = observer.into();
    delivery.invoke_and_report(
        scope,
        "performanceobserver",
        "PerformanceObserver callback",
        observer_value,
        &[list.into(), observer_value, options.into()],
    );
}

pub(super) fn enqueue_buffered_performance_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    observer: v8::Local<'s, v8::Object>,
) {
    let Some(entries) = performance_slot_array(scope, performance, PERFORMANCE_ENTRIES_SLOT) else {
        return;
    };
    let pending = v8::Array::new(scope, 0);
    for index in 0..entries.length() {
        let Some(entry) = entries.get_index(scope, index) else {
            continue;
        };
        let Ok(entry_obj) = v8::Local::<v8::Object>::try_from(entry) else {
            continue;
        };
        let entry_type =
            performance_entry_slot_string(scope, entry_obj, PERFORMANCE_ENTRY_TYPE_SLOT);
        if performance_observer_matches_entry_type(scope, observer, entry_type.as_deref()) {
            let _ = pending.set_index(scope, pending.length(), entry_obj.into());
        }
    }
    set_performance_observer_pending(scope, observer, pending);
}

pub(in crate::context_bootstrap) fn queue_matching_performance_observers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
    entry_type: &str,
) {
    let Some(timeline_context) = performance.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, timeline_context);
    for observer in active_performance_observers(scope, timeline_context) {
        if !performance_observer_callback_is_current(scope, observer) {
            let pending = v8::Array::new(scope, 0);
            set_performance_observer_pending(scope, observer, pending);
            continue;
        }
        if !performance_observer_active(scope, observer) {
            continue;
        }
        if !performance_observer_matches_entry_type(scope, observer, Some(entry_type)) {
            continue;
        }
        let Some(pending) = performance_observer_pending(scope, observer) else {
            continue;
        };
        let _ = pending.set_index(scope, pending.length(), entry.into());
        queue_performance_observer_delivery(scope, observer);
    }
}

pub(super) fn queue_performance_observer_delivery<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) {
    let Some(context) = observer.get_creation_context(scope) else {
        return;
    };
    // The queue and native flush trampoline belong to the observer's global,
    // even when observe() was borrowed from another Realm. Callback Realm
    // entry remains the shared Web IDL execution layer's responsibility.
    let scope = &mut v8::ContextScope::new(scope, context);
    if performance_observer_scheduled(scope, observer) {
        return;
    }
    if !performance_observer_callback_is_current(scope, observer) {
        let pending = v8::Array::new(scope, 0);
        set_performance_observer_pending(scope, observer, pending);
        return;
    }
    set_performance_observer_scheduled(scope, observer, true);
    push_object_to_global_queue(scope, PERFORMANCE_OBSERVER_QUEUE_SLOT, observer);
    schedule_scope_callback(scope, performance_observer_flush_callback, None);
}

fn performance_observer_matches_entry_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    entry_type: Option<&str>,
) -> bool {
    let Some(entry_type) = entry_type else {
        return false;
    };
    if performance_observer_observed_type(scope, observer).as_deref() == Some(entry_type) {
        return true;
    }
    let Some(entry_types) = performance_observer_entry_types(scope, observer) else {
        return false;
    };
    array_contains_string(scope, entry_types, entry_type)
}
