//! Routes a JavaScript `AbortSignal` to the store that owns it.
//!
//! Window and worker AbortSignals have deliberately separate lifetime owners:
//! the Window `PageScriptHost` and the exact worker run. Shared Web API
//! algorithms may accept either kind, but must not infer ownership from a
//! public prototype or maintain a third abort-algorithm list. Resolving once
//! produces a short-lived capability that delegates state and algorithm
//! registration back to the owning store.

use crate::context_bootstrap::context_host_ptr_from_global_bridge;
use crate::webidl;

use crate::worker::abort as worker_abort;
use moli_webapi::abort_signal::AbortSignalOwner;
pub(crate) use moli_webapi::abort_signal::ResolvedAbortSignal;

static WINDOW_SIGNALS: AbortSignalOwner = AbortSignalOwner {
    is_aborted: |scope, signal| {
        context_host_ptr_from_global_bridge(scope)
            .is_some_and(|host| unsafe { &mut *host }.abort_signal_aborted(scope, signal))
    },
    reason: |scope, signal| {
        let host = context_host_ptr_from_global_bridge(scope)?;
        unsafe { &mut *host }.abort_signal_reason(scope, signal)
    },
    abort: crate::native_bridge::abort::abort_signal,
    register_algorithm: |scope, signal, algorithm| {
        context_host_ptr_from_global_bridge(scope).is_some_and(|host| {
            unsafe { &mut *host }.register_abort_signal_algorithm(scope, signal, algorithm)
        })
    },
    unregister_algorithm: |scope, signal, algorithm| {
        context_host_ptr_from_global_bridge(scope).is_some_and(|host| {
            unsafe { &mut *host }.unregister_abort_signal_algorithm(scope, signal, algorithm)
        })
    },
};

static WORKER_SIGNALS: AbortSignalOwner = AbortSignalOwner {
    is_aborted: worker_abort::worker_abort_signal_aborted,
    reason: worker_abort::worker_abort_signal_reason,
    abort: |scope, signal, reason| {
        if let Some(id) = worker_abort::worker_abort_signal_id(scope, signal) {
            worker_abort::abort_worker_signal_by_id(scope, id, reason);
        }
    },
    register_algorithm: worker_abort::register_worker_abort_signal_algorithm,
    unregister_algorithm: worker_abort::unregister_worker_abort_signal_algorithm,
};

pub(crate) fn resolve_abort_signal_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
) -> Option<&'static AbortSignalOwner> {
    if context_host_ptr_from_global_bridge(scope)
        .is_some_and(|host| unsafe { &mut *host }.is_abort_signal(scope, signal))
    {
        return Some(&WINDOW_SIGNALS);
    }
    worker_abort::worker_abort_signal_id(scope, signal).map(|_| &WORKER_SIGNALS)
}

/// Creates a signal in the current realm using its native abort store.
pub(crate) fn new_abort_signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<ResolvedAbortSignal<'s>> {
    let signal = if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        crate::native_bridge::abort::create_signal(scope, unsafe { &mut *host }, false, None)?
    } else {
        worker_abort::new_worker_abort_signal(scope)?
    };
    ResolvedAbortSignal::resolve(scope, signal)
}

/// Converts the final AddEventListenerOptions member after capture/once/passive.
/// A dictionary uses Get, not HasProperty: Proxy traps and inherited getters
/// must run exactly once, and their original exceptions must escape unchanged.
pub(crate) fn event_listener_signal_from_options_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Option<ResolvedAbortSignal<'s>>, webidl::WebIdlError> {
    let Ok(options) = v8::Local::<v8::Object>::try_from(value) else {
        return Ok(None);
    };
    let context = webidl::Context::member("AddEventListenerOptions", "signal");
    let Some(signal_value) = webidl::property_result(scope, options, "signal", context)? else {
        return Ok(None);
    };
    if signal_value.is_undefined() {
        return Ok(None);
    }
    let signal = v8::Local::<v8::Object>::try_from(signal_value)
        .ok()
        .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))
        .ok_or_else(|| {
            webidl::WebIdlError::custom_message(
                "Failed to execute 'addEventListener': options.signal must be an AbortSignal.",
            )
        })?;
    Ok(Some(signal))
}
