use super::*;

pub(in crate::context_bootstrap::indexed_db) fn scan_index_entries(
    scope: &mut v8::PinScope<'_, '_>,
    handle: TransactionHandle,
    store_name: &IndexedDbName,
    index: &IndexInfo,
    query: Option<&IdbKeyRangeQuery>,
) -> std::result::Result<Vec<IndexEntry>, IndexedDbError> {
    let entries = with_indexed_db_manager(scope, |manager| manager.entries(handle, store_name))?;
    let mut matches = Vec::new();
    for (primary_key, value) in entries {
        let Some(decoded) = deserialize_js_value(scope, &value) else {
            continue;
        };
        for index_key in
            extract_index_keys_from_value(scope, decoded, &index.key_path, index.multi_entry)
        {
            if query.is_some_and(|range| !key_in_range(&index_key, range)) {
                continue;
            }
            matches.push(IndexEntry {
                index_key,
                primary_key: primary_key.clone(),
                value: value.clone(),
            });
        }
    }
    matches.sort_by(|left, right| {
        left.index_key
            .cmp(&right.index_key)
            .then_with(|| left.primary_key.cmp(&right.primary_key))
    });
    Ok(matches)
}
