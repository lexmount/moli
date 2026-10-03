//! Browser task payloads and invocation abort algorithms.

use super::bindings::js_string;
use super::state::AbortRegistration;
use crate::native_bridge::JsContextHost;
use crate::util::context_host_ptr_from_global_bridge;

pub(super) fn invocation_context(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Option<(*mut JsContextHost, u64)> {
    let id = v8::Local::<v8::BigInt>::try_from(args.data())
        .ok()?
        .u64_value()
        .0;
    Some((context_host_ptr_from_global_bridge(scope)?, id))
}

// Native Function data is never exposed to author code. Initialize every array
// element at construction, so indexed prototype setters cannot intercept it.
pub(super) fn task_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    id: u64,
    name: Option<&str>,
) -> v8::Local<'s, v8::Array> {
    let id = v8::BigInt::new_from_u64(scope, id);
    let name = name
        .map(|name| js_string(scope, name).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    v8::Array::new_with_elements(scope, &[target.into(), id.into(), name])
}

pub(super) fn task_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(v8::Local<'s, v8::Object>, u64, v8::Local<'s, v8::Value>)> {
    let data = v8::Local::<v8::Array>::try_from(args.data()).ok()?;
    let target = v8::Local::<v8::Object>::try_from(data.get_index(scope, 0)?).ok()?;
    let id = v8::Local::<v8::BigInt>::try_from(data.get_index(scope, 1)?)
        .ok()?
        .u64_value()
        .0;
    Some((target, id, data.get_index(scope, 2)?))
}

pub(super) fn queue_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    callback: v8::Local<'s, v8::Function>,
) {
    unsafe { &mut *host_ptr }.queue_internal_task(scope, callback);
}

pub(super) fn signal_reason<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    crate::abort_signal_route::ResolvedAbortSignal::resolve(scope, signal)
        .and_then(|signal| signal.reason(scope))
        .unwrap_or_else(|| crate::native_bridge::abort::abort_error_value(scope))
}

pub(super) fn remove_abort_registration(
    scope: &mut v8::PinScope<'_, '_>,
    abort: Option<AbortRegistration>,
) {
    if let Some(abort) = abort {
        let signal = v8::Local::new(scope, &abort.signal);
        let algorithm = v8::Local::new(scope, &abort.algorithm);
        if let Some(signal) = crate::abort_signal_route::ResolvedAbortSignal::resolve(scope, signal)
        {
            signal.unregister_algorithm(scope, algorithm);
        }
    }
}
