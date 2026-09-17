use super::*;

pub(in crate::context_bootstrap::indexed_db) fn object_store_cursor_snapshot(
    scope: &mut v8::PinScope<'_, '_>,
    handle: TransactionHandle,
    store_name: &IndexedDbName,
    query: Option<&IdbKeyRangeQuery>,
    direction: CursorDirection,
    key_only: bool,
) -> std::result::Result<Vec<CursorSnapshotEntry>, IndexedDbError> {
    let entries = scan_object_store_entries(scope, handle, store_name, query)?;
    let snapshot = entries
        .into_iter()
        .map(|(key, value)| CursorSnapshotEntry {
            key: key.clone(),
            primary_key: key,
            value: (!key_only).then_some(value),
        })
        .collect::<Vec<_>>();
    Ok(apply_cursor_direction(snapshot, direction))
}
