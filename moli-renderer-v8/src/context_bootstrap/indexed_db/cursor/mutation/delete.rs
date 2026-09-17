use super::*;

pub(in crate::context_bootstrap::indexed_db) fn idb_cursor_delete_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let cursor = args.this();
    let Some((store, transaction, primary_key)) = cursor_mutation_state(scope, cursor) else {
        return;
    };
    let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
        return;
    };
    let Some(store_name) = indexed_db_object_store_name(scope, store) else {
        return;
    };
    let Some(request) = create_request_object(scope, cursor.into(), transaction) else {
        return;
    };
    match with_indexed_db_manager(scope, |manager| {
        manager.delete(handle, &store_name, &primary_key)
    }) {
        Ok(()) => store_request_success(scope, request, v8::undefined(scope).into()),
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
    rv.set(request.into());
}
