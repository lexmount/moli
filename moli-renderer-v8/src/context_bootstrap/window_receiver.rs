use super::shared::WINDOW_NAVIGATOR_SLOT;
use crate::util::{
    callback_data_index_value, context_host_ptr_from_context_slot,
    context_host_ptr_from_window_object, get_private_value, throw_type_error,
};

/// Recognizes a native Window receiver for WebIDL brand checks.
///
/// A retained, detached global proxy can lose both its creation context and
/// observable private slots. When one of its own methods is called, however,
/// V8 still enters that function's context. That exact current-global
/// association is enough to preserve the Window brand while the operation
/// itself subsequently reports that its LocalWindow has shut down.
pub(crate) fn is_window_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    if is_live_window_receiver(scope, receiver) {
        return true;
    }
    let current_context = scope.get_current_context();
    receiver.strict_equals(current_context.global(scope).into())
        && context_host_ptr_from_context_slot(current_context).is_some()
}

/// Recognizes a Window whose V8 object still exposes its live realm markers.
///
/// This is deliberately narrower than the WebIDL brand check above. Bound
/// Window accessors use the distinction to return their captured SameObject
/// fallback after V8 detaches the old global proxy. Treating the retained
/// current global as live would make `navigator`, `performance`, and similar
/// surfaces incorrectly become `undefined` after child navigation.
fn is_live_window_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    if context_host_ptr_from_window_object(scope, receiver).is_some() {
        return true;
    }
    // The shared helper maps an `undefined` rollback marker to `None`.
    if get_private_value(scope, receiver, WINDOW_NAVIGATOR_SLOT).is_some() {
        return true;
    }
    let Some(context) = receiver.get_creation_context(scope) else {
        return false;
    };
    receiver.strict_equals(context.global(scope).into())
        && context_host_ptr_from_context_slot(context).is_some()
}

pub(in crate::context_bootstrap) fn bound_callback_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: usize,
    receiver: v8::Local<'s, v8::Object>,
    detached_fallback: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let index = callback_data_index_value(scope, index);
    v8::Array::new_with_elements(scope, &[index, receiver.into(), detached_fallback]).into()
}

pub(in crate::context_bootstrap) fn bound_callback_data_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    items: &'static [&'static str],
    context: &'static str,
) -> Option<(&'static str, Option<v8::Local<'s, v8::Value>>)> {
    let data = v8::Local::<v8::Array>::try_from(args.data()).ok();
    let index = data
        .and_then(|data| data.get_index(scope, 0))
        .and_then(|value| value.uint32_value(scope))
        .and_then(|index| usize::try_from(index).ok());
    let expected_receiver = data
        .and_then(|data| data.get_index(scope, 1))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let live_receiver = is_live_window_receiver(scope, args.this());
    let expected_receiver =
        expected_receiver.is_some_and(|expected| args.this().strict_equals(expected.into()));
    if !live_receiver && !expected_receiver {
        throw_type_error(scope, "Window getter called on incompatible receiver.");
        return None;
    }
    let current = scope.get_current_context();
    let receiver_context = args.this().get_creation_context(scope);
    let current_is_retired = context_host_ptr_from_context_slot(current)
        .map(|host| {
            let host = unsafe { &*host };
            host.window_execution_context_identity_for_access_check(current)
                .map(|identity| !host.window_execution_context_identity_is_current(identity))
                .unwrap_or_else(|| {
                    current
                        .get_slot::<crate::native_bridge::WindowEnvironmentSettings>()
                        .is_some()
                })
        })
        .unwrap_or(true);
    let original_retired_receiver = (expected_receiver && receiver_context.is_none())
        || (live_receiver && receiver_context == Some(current) && current_is_retired);
    // The original getter can return its captured SameObject after retirement.
    // A live receiver, including a reused WindowProxy, must pass authorization
    // before any private cache read or entry into its relevant Realm.
    if !original_retired_receiver
        && !crate::native_bridge::caller_can_access_window(scope, args.this())
    {
        crate::native_bridge::throw_cross_origin_location_security_error(scope);
        return None;
    }
    let Some(item) = index.and_then(|index| items.get(index)).copied() else {
        throw_type_error(scope, &format!("Invalid callback data for {context}."));
        return None;
    };
    let detached_fallback = (!live_receiver || original_retired_receiver)
        .then(|| data.and_then(|data| data.get_index(scope, 2)))
        .flatten();
    Some((item, detached_fallback))
}
