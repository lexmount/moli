use super::*;

pub(in crate::context_bootstrap::indexed_db) fn execute_index_count_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &str,
    query: Option<&IdbKeyRangeQuery>,
) {
    let Some(index_info) = index_info_from_index_object(scope, index) else {
        let error =
            dom_exception_value(scope, "The requested index was not found.", "NotFoundError");
        store_request_error(scope, request, error);
        return;
    };
    match scan_index_entries(scope, handle, store_name, &index_info, query) {
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
