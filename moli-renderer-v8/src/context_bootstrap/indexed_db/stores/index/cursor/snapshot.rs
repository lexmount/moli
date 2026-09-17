use super::*;

pub(in crate::context_bootstrap::indexed_db) fn index_cursor_snapshot(
    scope: &mut v8::PinScope<'_, '_>,
    handle: TransactionHandle,
    store_name: &IndexedDbName,
    index_info: &IndexInfo,
    query: Option<&IdbKeyRangeQuery>,
    direction: CursorDirection,
    key_only: bool,
) -> std::result::Result<Vec<CursorSnapshotEntry>, IndexedDbError> {
    let entries = scan_index_entries(scope, handle, store_name, index_info, query)?;
    let snapshot = entries
        .into_iter()
        .map(|entry| CursorSnapshotEntry {
            key: entry.index_key,
            primary_key: entry.primary_key,
            value: (!key_only).then_some(entry.value),
        })
        .collect::<Vec<_>>();
    Ok(apply_cursor_direction(snapshot, direction))
}
