use super::*;
use crate::util::{get_private_value, set_private_value};

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
    let range = moli_webapi_declare::web_api_object_target(scope, range)
        .expect("generated IDBKeyRange getter validates native identity");
    let cache = if slot == LOWER {
        LOWER_VALUE
    } else {
        UPPER_VALUE
    };
    if let Some(value) = get_private_value(scope, range, cache) {
        rv.set(value);
        return;
    }
    // Keep the immutable query snapshot separate from the cached mutable value.
    // First access creates the public value in the getter's realm; later reads
    // return that same value without changing the range used by queries.
    if let Some(value) = get_private_value(scope, range, slot)
        && let Ok(Some(key)) = parse_idb_key(scope, value)
    {
        let value = key_to_js_value(scope, &key);
        set_private_value(scope, range, cache, value);
        rv.set(value);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_key_range_lower_open_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let range = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated IDBKeyRange getter validates native identity");
    if let Some(value) = get_private_value(scope, range, LOWER_OPEN) {
        rv.set(value);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_key_range_upper_open_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let range = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated IDBKeyRange getter validates native identity");
    if let Some(value) = get_private_value(scope, range, UPPER_OPEN) {
        rv.set(value);
    }
}
