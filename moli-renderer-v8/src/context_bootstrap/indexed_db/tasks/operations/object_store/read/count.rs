use super::*;

pub(in crate::context_bootstrap::indexed_db) fn execute_object_store_count_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &str,
    query: Option<&IdbKeyRangeQuery>,
) {
    match scan_object_store_entries(scope, handle, store_name, query) {
        Ok(entries) => store_request_success(
            scope,
            request,
            v8::Number::new(scope, entries.len() as f64).into(),
        ),
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
}
