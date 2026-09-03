use super::observer::{
    array_contains_string, performance_observer_active, performance_observer_callback_id,
    performance_observer_callback_residence, performance_observer_entry_types,
    performance_observer_observed_type, performance_observer_pending,
    performance_observer_scheduled, prepare_worker_performance_observer_callback,
    set_performance_observer_pending, set_performance_observer_scheduled,
};
use super::*;
use crate::host::report_event_callback_exception;
use crate::window_webidl_callback::WindowWebIdlCallbackFunctionOutcome;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "PerformanceObserverEntryList")]
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
    let list = PerformanceObserverEntryListObjectDeclaration::new(pending)
        .bind(scope)
        .expect("PerformanceObserverEntryList declaration should bind");
    let next_pending = v8::Array::new(scope, 0);
    set_performance_observer_pending(scope, observer, next_pending);
    // Performance Timeline defines a third optional callback-options
    // dictionary. Moli does not currently drop buffered entries, so an
    // empty object is the exact observable value; Chromium likewise only sets
    // droppedEntriesCount when a drop count exists.
    let options = v8::Object::new(scope);
    let observer_value: v8::Local<'_, v8::Value> = observer.into();
    let callback_arguments = [list.into(), observer_value, options.into()];
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let Some(callback_residence) = performance_observer_callback_residence(scope, observer)
        else {
            return;
        };
        let Some(callback) =
            crate::observer_runtime::prepare_callback(scope, host_ptr, callback_residence)
        else {
            return;
        };
        match callback.invoke(
            scope,
            host_ptr,
            "PerformanceObserver callback",
            observer_value,
            &callback_arguments,
        ) {
            WindowWebIdlCallbackFunctionOutcome::Threw(report) => {
                report_event_callback_exception(
                    scope,
                    host_ptr,
                    "performanceobserver",
                    callback.relevant_identity(),
                    None,
                    &report,
                );
            }
            WindowWebIdlCallbackFunctionOutcome::Returned
            | WindowWebIdlCallbackFunctionOutcome::Retired => {}
        }
        return;
    }

    if crate::worker::get_worker_state(scope).is_none() {
        return;
    }
    let Some(callback) = prepare_worker_performance_observer_callback(scope, observer) else {
        return;
    };
    let _ = moli_webidl_callback::invoke_webidl_callback_function(
        scope,
        &callback,
        observer_value,
        &callback_arguments,
        |scope, callback, receiver, arguments| {
            crate::exception_reporting::invoke_callback(
                scope,
                "PerformanceObserver callback",
                callback,
                receiver,
                arguments,
            )
        },
    );
}

pub(super) fn enqueue_buffered_performance_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) {
    let Some(performance) =
        super::super::performance_runtime::ensure_current_performance_for_api(scope)
    else {
        return;
    };
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
    entry: v8::Local<'s, v8::Object>,
    entry_type: &str,
) {
    let host_ptr = context_host_ptr_from_global_bridge(scope);
    let observers = if let Some(host_ptr) = host_ptr {
        crate::observer_runtime::active_performance_observer_callbacks(scope, host_ptr)
    } else if crate::worker::get_worker_state(scope).is_some() {
        let Some(registry) = global_queue_array(scope, PERFORMANCE_OBSERVER_REGISTRY_SLOT) else {
            return;
        };
        (0..registry.length())
            .filter_map(|index| registry.get_index(scope, index))
            .filter_map(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .collect()
    } else {
        return;
    };
    for observer in observers {
        if let Some(host_ptr) = host_ptr {
            let Some(callback_id) = performance_observer_callback_id(scope, observer) else {
                continue;
            };
            if !crate::observer_runtime::callback_is_current(host_ptr, callback_id) {
                let pending = v8::Array::new(scope, 0);
                set_performance_observer_pending(scope, observer, pending);
                continue;
            }
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
    if performance_observer_scheduled(scope, observer) {
        return;
    }
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let Some(callback_id) = performance_observer_callback_id(scope, observer) else {
            return;
        };
        if !crate::observer_runtime::callback_is_current(host_ptr, callback_id) {
            let pending = v8::Array::new(scope, 0);
            set_performance_observer_pending(scope, observer, pending);
            return;
        }
    } else if crate::worker::get_worker_state(scope).is_none() {
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
