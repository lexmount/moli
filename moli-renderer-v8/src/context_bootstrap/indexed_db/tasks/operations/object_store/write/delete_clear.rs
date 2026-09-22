use super::*;

pub(in crate::context_bootstrap::indexed_db) fn execute_object_store_delete_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &str,
    query: &IdbKeyRangeQuery,
) {
    let result = with_indexed_db_manager(scope, |manager| {
        // Preserve the direct lookup for a single key. Ranges are evaluated
        // when the operation executes, after earlier writes in the transaction.
        if let Some(key) = &query.lower
            && query.upper.as_ref() == Some(key)
            && !query.lower_open
            && !query.upper_open
        {
            return manager.delete(handle, store_name, key);
        }
        let entries = manager.entries(handle, store_name)?;
        for (key, _) in entries {
            if key_in_range(&key, query) {
                manager.delete(handle, store_name, &key)?;
            }
        }
        Ok(())
    });
    match result {
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
