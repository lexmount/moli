use super::*;
use crate::context_bootstrap::indexed_db::stores::collection::CollectionRequest;
use crate::util::serialize_v8_iter_array;
mod parse;

pub(in crate::context_bootstrap::indexed_db) fn execute_index_get_all_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &IndexedDbName,
    collection: &CollectionRequest,
) {
    let Some(index_info) = parse::index_info_for_collection(scope, index, request) else {
        return;
    };
    match scan_index_entries(
        scope,
        handle,
        store_name,
        &index_info,
        collection.query.as_ref(),
    ) {
        Ok(entries) => {
            let entries = apply_index_collection_direction(entries, collection.direction);
            let limit = collection
                .count
                .filter(|count| *count != 0)
                .unwrap_or(entries.len());
            let values = entries
                .iter()
                .take(limit)
                .map(|entry| {
                    collection.kind.result(
                        scope,
                        &entry.index_key,
                        &entry.primary_key,
                        &entry.value,
                    )
                })
                .collect::<Option<Vec<_>>>();
            if let Some(values) = values.and_then(|values| serialize_v8_iter_array(scope, values)) {
                store_request_success(scope, request, values.into());
            }
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
}
