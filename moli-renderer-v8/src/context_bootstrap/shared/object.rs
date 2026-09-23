use super::*;
pub(in crate::context_bootstrap) use crate::util::{
    array_contains_strict, array_push_value, get_own_static_property, get_private_value,
    global_constructor_object, global_constructor_prototype, object_bool_property,
    object_defined_string_property as object_string_property_defined,
    object_own_static_bool_property, object_property_as_array, object_property_as_object,
    object_string_property, set_private_value,
};

pub(in crate::context_bootstrap) fn global_queue_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    slot_name: &'static str,
) -> Option<v8::Local<'s, v8::Array>> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, slot_name)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(in crate::context_bootstrap) fn push_object_to_global_queue(
    scope: &mut v8::PinScope<'_, '_>,
    slot_name: &'static str,
    object: v8::Local<'_, v8::Object>,
) {
    if let Some(queue) = global_queue_array(scope, slot_name) {
        array_push_value(scope, queue, object.into());
    }
}

pub(in crate::context_bootstrap) fn push_object_to_global_registry(
    scope: &mut v8::PinScope<'_, '_>,
    slot_name: &'static str,
    object: v8::Local<'_, v8::Object>,
) {
    if let Some(queue) = global_queue_array(scope, slot_name)
        && !array_contains_strict(scope, queue, object.into())
    {
        array_push_value(scope, queue, object.into());
    }
}

pub(in crate::context_bootstrap) fn pop_first_object_from_global_queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    slot_name: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    let queue = global_queue_array(scope, slot_name)?;
    let first = queue.get_index(scope, 0)?;
    let first = v8::Local::<v8::Object>::try_from(first).ok()?;
    let next = v8::Array::new(scope, 0);
    for index in 1..queue.length() {
        let Some(value) = queue.get_index(scope, index) else {
            continue;
        };
        array_push_value(scope, next, value);
    }
    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, slot_name, next.into());
    Some(first)
}

pub(in crate::context_bootstrap) fn window_performance_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, WINDOW_PERFORMANCE_SLOT)
        .or_else(|| global_hidden_value(scope, WINDOW_PERFORMANCE_SLOT))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(in crate::context_bootstrap) fn global_hidden_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    object_own_hidden_value(scope, global, key)
}

pub(in crate::context_bootstrap) fn object_hidden_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    object_own_hidden_value(scope, object, key)
}

pub(in crate::context_bootstrap) fn object_own_hidden_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_own_static_property(scope, object, key)
}

pub(in crate::context_bootstrap) fn object_hidden_bool(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &'static str,
) -> Option<bool> {
    object_own_static_bool_property(scope, object, key)
}
