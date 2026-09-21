use super::shared::WINDOW_NAVIGATOR_SLOT;
use crate::util::{
    callback_data_index_value, context_host_ptr_from_context_slot, get_private_value,
    set_private_value, throw_type_error,
};

const WINDOW_BRAND_SLOT: &str = "__moliWindowBrand";

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
    is_current_window_receiver(scope, receiver)
        || is_live_window_receiver(scope, receiver)
        || get_private_value(scope, receiver, WINDOW_BRAND_SLOT).is_some()
}

fn is_current_window_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    let current_context = scope.get_current_context();
    receiver.strict_equals(current_context.global(scope).into())
        && context_host_ptr_from_context_slot(current_context).is_some()
}

/// Marks a Window-shaped native object that intentionally has no execution
/// context, such as an iframe owned by a DOMParser-created Document.
pub(crate) fn mark_window_receiver(
    scope: &mut v8::PinScope<'_, '_>,
    receiver: v8::Local<'_, v8::Object>,
) {
    let branded = v8::Boolean::new(scope, true);
    set_private_value(scope, receiver, WINDOW_BRAND_SLOT, branded.into());
}

pub(crate) fn require_same_origin_window_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    lenient_this: bool,
) -> bool {
    // A retained function still identifies its own native global after V8 has
    // detached that global proxy. Reading its private slots would throw before
    // the operation can observe that its captured Window has retired.
    if is_current_window_receiver(scope, receiver) {
        return true;
    }
    // An extracted method or accessor bypasses WindowProxy's property access check.
    // Authorize native globals before reading slots, which can themselves
    // trigger V8's cross-origin fallback. Leniency only applies to objects
    // without the Window brand, never to a cross-origin Window.
    if let Some(context) = receiver.get_creation_context(scope)
        && receiver.strict_equals(context.global(scope).into())
        && context_host_ptr_from_context_slot(context).is_some()
    {
        if crate::native_bridge::window_contexts_allow_access(scope.get_current_context(), context)
        {
            return true;
        }
    } else if !crate::native_bridge::is_cross_origin_top_window_proxy(scope, receiver) {
        if is_window_receiver(scope, receiver) {
            return true;
        }
        if !lenient_this {
            throw_type_error(scope, "Window member called on incompatible receiver.");
        }
        return false;
    }
    crate::native_bridge::throw_dom_exception(
        scope,
        "SecurityError",
        18,
        "Blocked access to a cross-origin Window.",
    );
    false
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
    // A host pointer recovered through the public __moliNativeBridge property
    // is not a brand: an author can copy that property to an ordinary object.
    // Native bridge Window wrappers already carry the shared Web IDL brand.
    if crate::web_api_interfaces::Window::is_instance(scope, receiver) {
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
    if !require_same_origin_window_receiver(scope, args.this(), false) {
        return None;
    }
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
    let Some(item) = index.and_then(|index| items.get(index)).copied() else {
        throw_type_error(scope, &format!("Invalid callback data for {context}."));
        return None;
    };
    let detached_fallback = (!live_receiver)
        .then(|| data.and_then(|data| data.get_index(scope, 2)))
        .flatten();
    Some((item, detached_fallback))
}
