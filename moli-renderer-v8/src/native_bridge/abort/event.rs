use super::{AbortAlgorithm, AbortDispatchSnapshot, AbortStore};
use crate::context_bootstrap::{
    EVENT_PASSIVE_SLOT, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT, event_internal_bool_flag,
    set_event_internal_flag,
};
use crate::exception_reporting::invoke_callback;
use crate::host::invoke_prepared_event_callback_on_object;
use crate::native_bridge::JsContextHost;
use crate::util::{v8_string, v8str};

pub(super) fn invoke_abort_algorithms<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'_, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
    abort_algorithms: Vec<AbortAlgorithm>,
) {
    let signal = local_object_in_scope(scope, signal);
    for algorithm in abort_algorithms {
        let algorithm = v8::Local::new(scope, &algorithm.callback);
        let _ = invoke_callback(
            scope,
            "AbortSignal abort algorithm",
            algorithm,
            signal.into(),
            &[reason],
        );
    }
}

pub(super) fn dispatch_abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    signal: v8::Local<'_, v8::Object>,
    signal_id: u32,
    dispatch_snapshot: AbortDispatchSnapshot,
) {
    // Abort listener/event-handler delivery is entered from both Rust-side
    // state transitions and V8 native binding shims. Like XHR, that means the
    // incoming `signal` local is not guaranteed to share the exact scope
    // lifetime required by the structured exception reporter. Normalize it up
    // front so listeners and `onabort` retain local `TryCatch`, structured
    // stderr, and no stdout pollution.
    let signal = local_object_in_scope(scope, signal);
    let global = scope.get_current_context().global(scope);
    let Some(event_ctor) = global
        .get(scope, v8str(scope, "Event").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let Some(event_type) = v8_string(scope, "abort") else {
        return;
    };
    let Some(event) = event_ctor.new_instance(scope, &[event_type.into()]) else {
        return;
    };
    AbortStore::define_hidden_value(scope, event, "target", signal.into());
    AbortStore::define_hidden_value(scope, event, "currentTarget", signal.into());

    invoke_abort_event_callbacks(
        scope,
        host_ptr,
        signal,
        signal_id,
        dispatch_snapshot,
        "abort",
        event,
    );
}

pub(super) fn local_object_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let global = v8::Global::new(scope, object);
    v8::Local::new(scope, global)
}

pub(super) fn invoke_abort_event_callbacks<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    signal: v8::Local<'s, v8::Object>,
    signal_id: u32,
    dispatch_snapshot: AbortDispatchSnapshot,
    event_type: &str,
    event: v8::Local<'s, v8::Object>,
) {
    for listener in dispatch_snapshot.listeners {
        let Some(listener) = (unsafe { &mut *host_ptr })
            .claim_abort_signal_event_listener_for_dispatch(
                scope,
                signal_id,
                event_type,
                listener.callback_id,
            )
        else {
            continue;
        };
        let callback_name = format!("AbortSignal {event_type} listener");
        set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, listener.passive);
        let _ = invoke_prepared_event_callback_on_object(
            scope,
            host_ptr,
            event_type,
            &callback_name,
            listener.callback,
            signal,
            event,
        );
        set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, false);
        if event_internal_bool_flag(scope, event, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT) {
            break;
        }
    }
    if event_internal_bool_flag(scope, event, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT) {
        return;
    }
    if event_type == "abort"
        && let Some(onabort) = dispatch_snapshot.onabort
    {
        let onabort = v8::Local::new(scope, &onabort);
        let _ = invoke_callback(
            scope,
            "AbortSignal.onabort",
            onabort,
            signal.into(),
            &[event.into()],
        );
    }
}
