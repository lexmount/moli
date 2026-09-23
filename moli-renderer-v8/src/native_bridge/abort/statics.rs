use super::{AbortStore, abort_error_value, create_signal, timeout_error_value};
use crate::context_bootstrap::abort_signal;
use crate::util::context_host_ptr_from_global_bridge;
use crate::webidl;

pub(crate) fn abort_signal_static_abort_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_null();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let reason = if args.length() > 0 && !args.get(0).is_undefined() {
        Some(args.get(0))
    } else {
        Some(abort_error_value(scope))
    };
    let Some(signal) = create_signal(scope, host, true, reason) else {
        rv.set_null();
        return;
    };
    rv.set(signal.into());
}

pub(crate) fn abort_signal_timeout_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<abort_signal::TimeoutArgs>(scope, &args) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_null();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(signal) = create_signal(scope, host, false, None) else {
        rv.set_null();
        return;
    };
    let Some(signal_id) = AbortStore::signal_id_from_object(scope, signal) else {
        rv.set_null();
        return;
    };
    let callback = v8::FunctionTemplate::builder(abort_signal_timeout_fire_native_callback)
        .data(v8::Number::new(scope, signal_id as f64).into())
        .build(scope)
        .get_function(scope);
    let Some(callback) = callback else {
        rv.set(signal.into());
        return;
    };
    let timeout_id = host.queue_timeout(
        scope,
        callback,
        parsed.milliseconds,
        crate::host::HostTimerOwner::Window,
        Vec::new(),
    );
    // AbortSignal.timeout() intentionally exposes no cancel handle.
    let _ = timeout_id;
    rv.set(signal.into());
}

pub(crate) fn abort_signal_any_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<abort_signal::AnyArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_null();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(signal) = create_signal(scope, host, false, None) else {
        rv.set_null();
        return;
    };
    let Some(composite_signal_id) = AbortStore::signal_id_from_object(scope, signal) else {
        rv.set_null();
        return;
    };

    let signals = parsed.signals;

    for source_signal in &signals {
        let Some(source_signal_id) = AbortStore::signal_id_from_object(scope, *source_signal)
        else {
            continue;
        };
        let Some(reason) = host
            .native_bridge_mut()
            .abort
            .signal_state(source_signal_id)
            .filter(|state| state.aborted)
            .and_then(|state| state.reason.as_ref())
            .map(|reason| v8::Local::new(scope, reason))
        else {
            continue;
        };
        crate::native_bridge::abort::abort_signal(scope, signal, reason);
        rv.set(signal.into());
        return;
    }

    for source_signal in signals {
        let Some(source_signal_id) = AbortStore::signal_id_from_object(scope, source_signal) else {
            continue;
        };
        host.native_bridge_mut()
            .abort
            .link_dependent_signal(source_signal_id, composite_signal_id);
    }

    rv.set(signal.into());
}

fn abort_signal_timeout_fire_native_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let data = args.data();
    let Some(signal_id) = data
        .number_value(scope)
        .filter(|value: &f64| value.is_finite() && *value >= 1.0)
        .map(|value| value as u32)
    else {
        rv.set_undefined();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(signal) = host
        .native_bridge_mut()
        .abort
        .signal_object(scope, signal_id)
    else {
        rv.set_undefined();
        return;
    };
    let reason = timeout_error_value(scope);
    crate::native_bridge::abort::abort_signal(scope, signal, reason);
    rv.set_undefined();
}
