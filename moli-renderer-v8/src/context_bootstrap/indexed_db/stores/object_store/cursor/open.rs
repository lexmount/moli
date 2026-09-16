use super::*;
use crate::context_bootstrap::indexed_db::stores::cursor_open_parse::parse_open_cursor_args;

fn idb_object_store_open_cursor_impl<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    key_only: bool,
) {
    let store = args.this();
    let Some((request, transaction, store_name)) = object_store_operation_common(scope, store)
    else {
        return;
    };
    let operation_name = if key_only {
        "IDBObjectStore.openKeyCursor"
    } else {
        "IDBObjectStore.openCursor"
    };
    let Some(parsed) = parse_open_cursor_args(scope, args, operation_name) else {
        return;
    };
    let operation =
        IndexedDbCursorOpenOperation::object_store(parsed.query, parsed.direction, key_only);
    submit_cursor_open_operation(scope, transaction, store, request, &store_name, operation);
    rv.set(request.into());
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_open_cursor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    idb_object_store_open_cursor_impl(scope, args, rv, false);
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_open_key_cursor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    idb_object_store_open_cursor_impl(scope, args, rv, true);
}
