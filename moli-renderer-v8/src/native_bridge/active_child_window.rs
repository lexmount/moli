use super::{ACTIVE_CHILD_WINDOW_HANDLE_SLOT, JsContextHost};
use crate::{
    document_runtime::DomHandle,
    util::{context_host_ptr_from_global_bridge, get_private_value, set_private_value},
};

pub(crate) fn active_child_window_handle(scope: &mut v8::PinScope<'_, '_>) -> Option<DomHandle> {
    child_window_handle_from_slot(scope, ACTIVE_CHILD_WINDOW_HANDLE_SLOT)
}

pub(crate) fn enter_active_child_window_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: Option<DomHandle>,
) -> v8::Local<'s, v8::Value> {
    let global = scope.get_current_context().global(scope);
    let previous = get_private_value(scope, global, ACTIVE_CHILD_WINDOW_HANDLE_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let next = handle
        .map(|handle| v8::BigInt::new_from_u64(scope, handle.index() as u64).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(scope, global, ACTIVE_CHILD_WINDOW_HANDLE_SLOT, next);
    previous
}

pub(crate) fn restore_active_child_window_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    previous: v8::Local<'s, v8::Value>,
) {
    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, ACTIVE_CHILD_WINDOW_HANDLE_SLOT, previous);
}

pub(crate) fn defer_active_child_window_restore<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    previous: v8::Local<'s, v8::Value>,
) {
    let previous_handle = child_window_handle_from_marker_data(scope, previous);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        restore_active_child_window_scope(scope, previous);
        return;
    };
    unsafe { &mut *host_ptr }.defer_active_child_window_restore_after_microtasks(previous_handle);
}

pub(crate) fn restore_deferred_active_child_window_scope_if_present(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
) -> bool {
    let Some(previous) = host.take_deferred_active_child_window_restore() else {
        return false;
    };
    restore_active_child_window_scope_to_handle(scope, previous);
    true
}

fn restore_active_child_window_scope_to_handle(
    scope: &mut v8::PinScope<'_, '_>,
    handle: Option<DomHandle>,
) {
    let value = handle
        .map(|handle| v8::BigInt::new_from_u64(scope, handle.index() as u64).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, ACTIVE_CHILD_WINDOW_HANDLE_SLOT, value);
}

pub(crate) fn child_window_handle_from_marker_data(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<DomHandle> {
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (index, lossless) = big.u64_value();
        return lossless.then(|| DomHandle::new(index as usize));
    }
    value
        .number_value(scope)
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| DomHandle::new(value as usize))
}

fn child_window_handle_from_slot(
    scope: &mut v8::PinScope<'_, '_>,
    slot: &str,
) -> Option<DomHandle> {
    let global = scope.get_current_context().global(scope);
    let value = get_private_value(scope, global, slot)?;
    child_window_handle_from_marker_data(scope, value)
}
