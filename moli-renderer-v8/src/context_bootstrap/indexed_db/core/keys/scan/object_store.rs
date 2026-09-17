use super::*;

pub(in crate::context_bootstrap::indexed_db) fn scan_object_store_entries(
    scope: &mut v8::PinScope<'_, '_>,
    handle: TransactionHandle,
    store_name: &IndexedDbName,
    query: Option<&IdbKeyRangeQuery>,
) -> std::result::Result<Vec<(Key, IndexedDbValue)>, IndexedDbError> {
    let mut entries =
        with_indexed_db_manager(scope, |manager| manager.entries(handle, store_name))?;
    if let Some(range) = query {
        entries.retain(|(key, _)| key_in_range(key, range));
    }
    Ok(entries)
}
