use super::*;
use crate::util::serialize_v8_iter_array;

pub(in crate::context_bootstrap::indexed_db) fn execute_index_get_all_keys_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &str,
    query: Option<&IdbKeyRangeQuery>,
    count: Option<usize>,
    direction: CursorDirection,
) {
    let Some(index_info) = parse::index_info_for_collection(scope, index, request) else {
        return;
    };
    match scan_index_entries(scope, handle, store_name, &index_info, query) {
        Ok(entries) => {
            let entries = apply_index_collection_direction(entries, direction);
            let limit = count.unwrap_or(entries.len());
            let keys = entries
                .iter()
                .take(limit)
                .map(|entry| key_to_js_value(scope, &entry.primary_key))
                .collect::<Vec<_>>();
            let array =
                serialize_v8_iter_array(scope, keys).unwrap_or_else(|| v8::Array::new(scope, 0));
            store_request_success(scope, request, array.into());
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
}
