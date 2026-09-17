use super::*;

pub(in crate::context_bootstrap::indexed_db) fn execute_object_store_delete_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &str,
    key_value: v8::Local<'s, v8::Value>,
) {
    let key = match parse_idb_key(scope, key_value) {
        Ok(Some(key)) => key,
        Ok(None) => {
            let error = dom_exception_value(
                scope,
                "Failed to execute 'delete': invalid key.",
                "DataError",
            );
            store_request_error(scope, request, error);
            return;
        }
        Err(error) => {
            let value = error.into_value(scope);
            store_request_error(scope, request, value);
            return;
        }
    };
    match with_indexed_db_manager(scope, |manager| manager.delete(handle, store_name, &key)) {
        Ok(()) => store_request_success(scope, request, v8::undefined(scope).into()),
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn execute_object_store_clear_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &str,
) {
    match with_indexed_db_manager(scope, |manager| manager.clear(handle, store_name)) {
        Ok(()) => store_request_success(scope, request, v8::undefined(scope).into()),
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
}
