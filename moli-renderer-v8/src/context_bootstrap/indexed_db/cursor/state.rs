use super::*;

pub(in crate::context_bootstrap::indexed_db) fn cursor_request_and_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    let request = object_hidden_value(scope, cursor, INDEXED_DB_CURSOR_REQUEST_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let transaction = indexed_db_request_transaction_object(scope, request)?;
    Some((request, transaction))
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_position(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
) -> i32 {
    object_number_property(scope, cursor, INDEXED_DB_CURSOR_POSITION_SLOT).unwrap_or(-1.0) as i32
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_entries_len<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> usize {
    object_hidden_value(scope, cursor, INDEXED_DB_CURSOR_ENTRIES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .map(|entries| entries.length() as usize)
        .unwrap_or(0)
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_key_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    position: usize,
) -> Option<Key> {
    let entry = cursor_entry_object(scope, cursor, position)?;
    parse_idb_key(scope, entry.get(scope, v8str(scope, "key").into())?).ok()?
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_primary_key_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    position: usize,
) -> Option<Key> {
    let entry = cursor_entry_object(scope, cursor, position)?;
    parse_idb_key(scope, entry.get(scope, v8str(scope, "primaryKey").into())?).ok()?
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_store_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let source = object_property_as_object(scope, cursor, "source")?;
    if cursor_source_is_index(scope, cursor) {
        return object_property_as_object(scope, source, "objectStore");
    }
    Some(source)
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_store_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let store = cursor_store_object(scope, cursor)?;
    object_string_property(scope, store, INDEXED_DB_OBJECT_STORE_NAME_SLOT)
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_current_position(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
) -> Option<usize> {
    let position = cursor_position(scope, cursor);
    (position >= 0).then_some(position as usize)
}

pub(in crate::context_bootstrap::indexed_db) fn create_cursor_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, TransactionHandle, String)> {
    let (_, transaction) = cursor_request_and_transaction(scope, cursor)?;
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
        .unwrap_or(false)
    {
        return None;
    }
    let request = create_request_object(scope, cursor.into(), transaction)?;
    let handle = transaction_handle_from_value(scope, transaction.into())?;
    let store_name = cursor_store_name(scope, cursor)?;
    Some((request, handle, store_name))
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_source_is_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> bool {
    object_property_as_object(scope, cursor, "source")
        .and_then(|source| object_bool_property(scope, source, INDEXED_DB_INDEX_MARKER_SLOT))
        .unwrap_or(false)
}
