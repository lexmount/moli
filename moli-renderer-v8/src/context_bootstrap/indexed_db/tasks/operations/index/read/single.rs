use super::*;

pub(in crate::context_bootstrap::indexed_db) fn execute_index_get_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &IndexedDbName,
    query: &IdbKeyRangeQuery,
) {
    let Some(index_info) = index_info_from_index_object(scope, index) else {
        let error =
            dom_exception_value(scope, "The requested index was not found.", "NotFoundError");
        store_request_error(scope, request, error);
        return;
    };
    match scan_index_entries(scope, handle, store_name, &index_info, Some(query)) {
        Ok(entries) => {
            let result = entries
                .first()
                .and_then(|entry| deserialize_js_value(scope, &entry.value))
                .unwrap_or_else(|| v8::undefined(scope).into());
            store_request_success(scope, request, result);
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn execute_index_get_key_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &IndexedDbName,
    query: &IdbKeyRangeQuery,
) {
    let Some(index_info) = index_info_from_index_object(scope, index) else {
        let error =
            dom_exception_value(scope, "The requested index was not found.", "NotFoundError");
        store_request_error(scope, request, error);
        return;
    };
    match scan_index_entries(scope, handle, store_name, &index_info, Some(query)) {
        Ok(entries) => {
            let result = entries
                .first()
                .map(|entry| key_to_js_value(scope, &entry.primary_key))
                .unwrap_or_else(|| v8::undefined(scope).into());
            store_request_success(scope, request, result);
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
}
