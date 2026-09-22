//! Routes a JavaScript `AbortSignal` to the store that owns it.
//!
//! Window and worker AbortSignals have deliberately separate lifetime owners:
//! the Window `JsContextHost` and the exact worker run. Shared Web API
//! algorithms may accept either kind, but must not infer ownership from a
//! public prototype or maintain a third abort-algorithm list. Resolving once
//! produces a short-lived capability that delegates state and algorithm
//! registration back to the owning store.

use crate::context_bootstrap::context_host_ptr_from_global_bridge;
use crate::util::{throw_type_error, v8str};

#[derive(Clone, Copy)]
enum AbortSignalOwner {
    Window,
    Worker,
}

/// A scope-local capability for one validated Window- or worker-owned signal.
///
/// This value never outlives the V8 callback in which it was resolved. It is
/// not signal state and does not own abort algorithms; those remain in the
/// Window `AbortStore` or worker-run `WorkerAbortStore`.
#[derive(Clone, Copy)]
pub(crate) struct ResolvedAbortSignal<'s> {
    signal: v8::Local<'s, v8::Object>,
    owner: AbortSignalOwner,
}

impl<'s> ResolvedAbortSignal<'s> {
    pub(crate) fn resolve(
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
            && unsafe { &mut *host_ptr }.is_abort_signal(scope, signal)
        {
            return Some(Self {
                signal,
                owner: AbortSignalOwner::Window,
            });
        }
        crate::worker::abort::worker_abort_signal_id(scope, signal).map(|_| Self {
            signal,
            owner: AbortSignalOwner::Worker,
        })
    }

    pub(crate) fn value(self) -> v8::Local<'s, v8::Object> {
        self.signal
    }

    pub(crate) fn is_aborted(self, scope: &mut v8::PinScope<'s, '_>) -> bool {
        match self.owner {
            AbortSignalOwner::Window => {
                let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
                    return false;
                };
                unsafe { &mut *host_ptr }.abort_signal_aborted(scope, self.signal)
            }
            AbortSignalOwner::Worker => {
                crate::worker::abort::worker_abort_signal_aborted(scope, self.signal)
            }
        }
    }

    pub(crate) fn abort(self, scope: &mut v8::PinScope<'s, '_>, reason: v8::Local<'s, v8::Value>) {
        match self.owner {
            AbortSignalOwner::Window => {
                let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
                    return;
                };
                unsafe { &mut *host_ptr }.abort_signal(scope, self.signal, reason);
            }
            AbortSignalOwner::Worker => {
                let Some(signal_id) =
                    crate::worker::abort::worker_abort_signal_id(scope, self.signal)
                else {
                    return;
                };
                crate::worker::abort::abort_worker_signal_by_id(scope, signal_id, reason);
            }
        }
    }

    pub(crate) fn register_algorithm(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        algorithm: v8::Local<'s, v8::Function>,
    ) -> bool {
        match self.owner {
            AbortSignalOwner::Window => {
                let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
                    return false;
                };
                unsafe { &mut *host_ptr }.register_abort_signal_algorithm(
                    scope,
                    self.signal,
                    algorithm,
                )
            }
            AbortSignalOwner::Worker => {
                crate::worker::abort::register_worker_abort_signal_algorithm(
                    scope,
                    self.signal,
                    algorithm,
                )
            }
        }
    }

    pub(crate) fn unregister_algorithm(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        algorithm: v8::Local<'s, v8::Function>,
    ) -> bool {
        match self.owner {
            AbortSignalOwner::Window => {
                let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
                    return false;
                };
                unsafe { &mut *host_ptr }.unregister_abort_signal_algorithm(
                    scope,
                    self.signal,
                    algorithm,
                )
            }
            AbortSignalOwner::Worker => {
                crate::worker::abort::unregister_worker_abort_signal_algorithm(
                    scope,
                    self.signal,
                    algorithm,
                )
            }
        }
    }
}

/// Parses the `signal` member of `AddEventListenerOptions`.
///
/// The outer `Option` distinguishes abrupt conversion from an absent member;
/// the inner `Option` distinguishes no signal from a validated Window/worker
/// signal capability.
pub(crate) fn event_listener_signal_from_options_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<Option<ResolvedAbortSignal<'s>>> {
    if value.is_null_or_undefined() || !value.is_object() {
        return Some(None);
    }
    let Ok(options) = v8::Local::<v8::Object>::try_from(value) else {
        return Some(None);
    };
    let signal_key = v8str(scope, "signal");
    let signal_value = options.get(scope, signal_key.into())?;
    if signal_value.is_undefined() {
        return Some(None);
    }
    let Ok(signal) = v8::Local::<v8::Object>::try_from(signal_value) else {
        throw_type_error(
            scope,
            "Failed to execute 'addEventListener': options.signal must be an AbortSignal.",
        );
        return None;
    };
    let Some(signal) = ResolvedAbortSignal::resolve(scope, signal) else {
        throw_type_error(
            scope,
            "Failed to execute 'addEventListener': options.signal must be an AbortSignal.",
        );
        return None;
    };
    Some(Some(signal))
}
