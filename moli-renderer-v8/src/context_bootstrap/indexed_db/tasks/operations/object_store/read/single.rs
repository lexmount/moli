use super::*;

pub(in crate::context_bootstrap::indexed_db) fn execute_object_store_get_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &str,
    query: &IdbKeyRangeQuery,
) {
    match scan_object_store_entries(scope, handle, store_name, Some(query)) {
        Ok(entries) => {
            let result = entries
                .first()
                .and_then(|(_, bytes)| deserialize_js_value(scope, bytes))
                .unwrap_or_else(|| v8::undefined(scope).into());
            store_request_success(scope, request, result);
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn execute_object_store_get_key_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &str,
    query: &IdbKeyRangeQuery,
) {
    match scan_object_store_entries(scope, handle, store_name, Some(query)) {
        Ok(entries) => {
            let result = entries
                .first()
                .map(|(key, _)| key_to_js_value(scope, key))
                .unwrap_or_else(|| v8::undefined(scope).into());
            store_request_success(scope, request, result);
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
}
