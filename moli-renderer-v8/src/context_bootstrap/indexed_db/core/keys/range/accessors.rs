use super::*;
use crate::util::get_private_value;

pub(in crate::context_bootstrap::indexed_db) fn idb_key_range_lower_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    bound_getter(scope, args.this(), LOWER, rv);
}

pub(in crate::context_bootstrap::indexed_db) fn idb_key_range_upper_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    bound_getter(scope, args.this(), UPPER, rv);
}

fn bound_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    slot: &str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    // Private keys contain only native dense arrays, dates and fixed buffers.
    // Reconstruct a value in the getter's realm without exposing the snapshot.
    if let Some(value) = get_private_value(scope, range, slot)
        && let Ok(Some(key)) = parse_idb_key(scope, value)
    {
        rv.set(key_to_js_value(scope, &key));
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_key_range_lower_open_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), LOWER_OPEN) {
        rv.set(value);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_key_range_upper_open_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), UPPER_OPEN) {
        rv.set(value);
    }
}
