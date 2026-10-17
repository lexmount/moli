use super::*;
use crate::context_bootstrap::indexed_db::stores::cursor_open_parse::parse_open_cursor_args;

fn idb_index_open_cursor_impl<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    key_only: bool,
) {
    let index = args.this();
    let Some((request, transaction, store_name, index_info)) = create_index_request(scope, index)
    else {
        return;
    };
    let operation_name = if key_only {
        "IDBIndex.openKeyCursor"
    } else {
        "IDBIndex.openCursor"
    };
    let Some(parsed) = parse_open_cursor_args(scope, args, operation_name) else {
        return;
    };
    let operation =
        IndexedDbCursorOpenOperation::index(index_info, parsed.query, parsed.direction, key_only);
    submit_cursor_open_operation(scope, transaction, index, request, &store_name, operation);
    rv.set(request.into());
}

pub(in crate::context_bootstrap::indexed_db) fn idb_index_open_cursor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    idb_index_open_cursor_impl(scope, args, rv, false);
}

pub(in crate::context_bootstrap::indexed_db) fn idb_index_open_key_cursor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    idb_index_open_cursor_impl(scope, args, rv, true);
}
