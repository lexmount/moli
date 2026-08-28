use super::{AbortStore, abort_error_value, create_signal_with_prototype, timeout_error_value};
use crate::util::{context_host_ptr_from_global_bridge, v8_string, v8str};
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
    let Some(signal) = create_signal_with_prototype(scope, args.this(), host, true, reason) else {
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
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_null();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let delay = webidl::non_negative_milliseconds_arg(scope, &args, 0, "AbortSignal.timeout");
    let Some(signal) = create_signal_with_prototype(scope, args.this(), host, false, None) else {
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
        delay,
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
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_null();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(signal) = create_signal_with_prototype(scope, args.this(), host, false, None) else {
        rv.set_null();
        return;
    };
    let Some(composite_signal_id) = AbortStore::signal_id_from_object(scope, signal) else {
        rv.set_null();
        return;
    };

    let signals = match collect_abort_signal_iterable(scope, args.get(0)) {
        Ok(signals) => signals,
        Err(message) => {
            if let Some(message) = v8_string(scope, &message) {
                scope.throw_exception(v8::Exception::type_error(scope, message));
            }
            return;
        }
    };

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
            .and_then(|_| AbortStore::signal_reason_from_object(scope, *source_signal))
        else {
            continue;
        };
        host.abort_signal(scope, signal, reason);
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

fn collect_abort_signal_iterable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterable: v8::Local<'s, v8::Value>,
) -> Result<Vec<v8::Local<'s, v8::Object>>, String> {
    if iterable.is_null_or_undefined() {
        return Err(
            "Failed to execute 'any' on 'AbortSignal': parameter 1 is not iterable.".to_owned(),
        );
    }
    let Ok(iterable_object) = v8::Local::<v8::Object>::try_from(iterable) else {
        return Err(
            "Failed to execute 'any' on 'AbortSignal': parameter 1 is not iterable.".to_owned(),
        );
    };
    let iterator_symbol = v8::Symbol::get_iterator(scope);
    let Some(iterator_method) = iterable_object
        .get(scope, iterator_symbol.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return Err(
            "Failed to execute 'any' on 'AbortSignal': parameter 1 is not iterable.".to_owned(),
        );
    };
    let Some(iterator) = iterator_method
        .call(scope, iterable, &[])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return Err(
            "Failed to execute 'any' on 'AbortSignal': parameter 1 is not iterable.".to_owned(),
        );
    };
    let Some(next_method) = iterator
        .get(scope, v8str(scope, "next").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return Err(
            "Failed to execute 'any' on 'AbortSignal': parameter 1 is not iterable.".to_owned(),
        );
    };
    let mut signals = Vec::new();
    loop {
        let Some(step) = next_method
            .call(scope, iterator.into(), &[])
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            return Err(
                "Failed to execute 'any' on 'AbortSignal': parameter 1 is not iterable.".to_owned(),
            );
        };
        let done = step
            .get(scope, v8str(scope, "done").into())
            .is_some_and(|value| value.boolean_value(scope));
        if done {
            break;
        }
        let Some(value) = step.get(scope, v8str(scope, "value").into()) else {
            return Err(
                "Failed to execute 'any' on 'AbortSignal': iterable yielded a non-AbortSignal value."
                    .to_owned(),
            );
        };
        let Ok(signal) = v8::Local::<v8::Object>::try_from(value) else {
            return Err(
                "Failed to execute 'any' on 'AbortSignal': iterable yielded a non-AbortSignal value."
                    .to_owned(),
            );
        };
        if AbortStore::signal_id_from_object(scope, signal).is_none() {
            return Err(
                "Failed to execute 'any' on 'AbortSignal': iterable yielded a non-AbortSignal value."
                    .to_owned(),
            );
        }
        signals.push(signal);
    }
    Ok(signals)
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
    host.abort_signal(scope, signal, reason);
    rv.set_undefined();
}
