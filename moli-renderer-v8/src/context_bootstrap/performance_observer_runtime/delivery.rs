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
use crate::util::context_host_ptr_from_context_slot;
use moli_webapi::performance::observer::build_performance_observer_callback_options;

fn performance_observer_flush_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    deliver_next_performance_observer(scope);
}

/// Snapshot the Window's registered observers before invoking author code.
pub(crate) fn notify_performance_observers(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let context = scope.get_current_context();
    let observers = active_performance_observers(scope, context);
    let mut invoked = false;
    for observer in observers {
        invoked |= deliver_performance_observer(scope, observer);
    }
    invoked
}

fn deliver_next_performance_observer(scope: &mut v8::PinScope<'_, '_>) {
    if let Some(observer) =
        pop_first_object_from_global_queue(scope, PERFORMANCE_OBSERVER_QUEUE_SLOT)
    {
        set_performance_observer_scheduled(scope, observer, false);
        deliver_performance_observer(scope, observer);
    }
}

fn deliver_performance_observer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> bool {
    if !performance_observer_active(scope, observer) {
        return false;
    }
    let Some(pending) = performance_observer_pending(scope, observer) else {
        return false;
    };
    if pending.length() == 0 {
        return false;
    }
    let next_pending = v8::Array::new(scope, 0);
    set_performance_observer_pending(scope, observer, next_pending);
    let Some(delivery) = PerformanceObserverRegistration::from_observer(scope, observer)
        .and_then(|registration| registration.prepare_delivery(scope))
    else {
        return false;
    };
    let dropped_entries_count =
        super::observer::take_performance_observer_dropped_entries_count(scope, observer);
    // Callback arguments are wrapped in the callback's relevant Realm, while
    // subscriptions and cumulative counters belong to the observer's global.
    let (list, options) = {
        let context = delivery.relevant_context(scope);
        let scope = &mut v8::ContextScope::new(scope, context);
        let list = moli_webapi::performance::entry_list::build(scope, pending)
            .expect("PerformanceObserverEntryList declaration should bind");
        let options = build_performance_observer_callback_options(scope, dropped_entries_count);
        (list, options)
    };
    let observer_value: v8::Local<'_, v8::Value> = observer.into();
    delivery.invoke_and_report(
        scope,
        "performanceobserver",
        "PerformanceObserver callback",
        observer_value,
        &[list.into(), observer_value, options.into()],
    )
}

pub(super) fn enqueue_buffered_performance_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    observer: v8::Local<'s, v8::Object>,
) {
    let buffer =
        if performance_observer_observed_type(scope, observer).as_deref() == Some("longtask") {
            super::super::performance_runtime::LONG_TASK_BUFFER_SLOT
        } else {
            PERFORMANCE_ENTRIES_SLOT
        };
    let Some(entries) = performance_slot_array(scope, performance, buffer) else {
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
    if !performance_observer_callback_is_current(scope, observer) {
        let pending = v8::Array::new(scope, 0);
        set_performance_observer_pending(scope, observer, pending);
        return;
    }
    if let Some(host) = context_host_ptr_from_context_slot(context) {
        unsafe { &mut *host }.queue_performance_observer_task(scope, context);
        return;
    }
    if performance_observer_scheduled(scope, observer) {
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
