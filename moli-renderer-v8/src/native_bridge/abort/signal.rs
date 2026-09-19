use super::AbortStore;
use crate::util::context_host_ptr_from_global_bridge;

pub(crate) fn abort_signal_aborted_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_bool(false);
        return;
    };
    let signal = args.this();
    if AbortStore::signal_id_from_object(scope, signal).is_none() {
        rv.set_bool(false);
        return;
    }
    // SAFETY: as_ptr() — this getter may be called during event dispatch
    // (re-entrant from another callback holding borrow_mut). See util.rs.
    let aborted = AbortStore::signal_id_from_object(scope, signal)
        .and_then(|id| {
            unsafe { &mut *host_ptr }
                .native_bridge_mut()
                .abort
                .signal_state(id)
        })
        .is_some_and(|state| state.aborted);
    rv.set_bool(aborted);
}

pub(crate) fn abort_signal_reason_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let signal = args.this();
    if AbortStore::signal_id_from_object(scope, signal).is_none() {
        rv.set_undefined();
        return;
    }
    let Some(reason) = AbortStore::signal_id_from_object(scope, signal)
        .and_then(|id| {
            unsafe { &mut *host_ptr }
                .native_bridge_mut()
                .abort
                .signal_state(id)
        })
        .and_then(|state| state.reason.as_ref())
        .map(|reason| v8::Local::new(scope, reason))
    else {
        rv.set_undefined();
        return;
    };
    rv.set(reason);
}

pub(crate) fn abort_signal_throw_if_aborted_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let signal = args.this();
    if AbortStore::signal_id_from_object(scope, signal).is_none() {
        rv.set_undefined();
        return;
    }
    let Some(reason) = AbortStore::signal_id_from_object(scope, signal)
        .and_then(|id| {
            unsafe { &mut *host_ptr }
                .native_bridge_mut()
                .abort
                .signal_state(id)
        })
        .filter(|state| state.aborted)
        .and_then(|state| state.reason.as_ref())
        .map(|reason| v8::Local::new(scope, reason))
    else {
        rv.set_undefined();
        return;
    };
    // DOM requires throwing the stored abort reason itself. In particular, a
    // string reason remains a string rather than being wrapped in `Error`.
    scope.throw_exception(reason);
}
