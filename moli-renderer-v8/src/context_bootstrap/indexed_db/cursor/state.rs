use super::*;

pub(in crate::context_bootstrap::indexed_db) fn cursor_request_and_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    let request = cursor_request(scope, cursor)?;
    let transaction = indexed_db_request_transaction_object(scope, request)?;
    Some((request, transaction))
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_entries_len(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
) -> usize {
    indexed_db_cursor_state(scope, cursor).map_or(0, |state| state.entries.len())
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_key_at(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
    position: usize,
) -> Option<Key> {
    Some(
        indexed_db_cursor_state(scope, cursor)?
            .entries
            .get(position)?
            .key
            .clone(),
    )
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_primary_key_at(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
    position: usize,
) -> Option<Key> {
    Some(
        indexed_db_cursor_state(scope, cursor)?
            .entries
            .get(position)?
            .primary_key
            .clone(),
    )
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_current_position(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
) -> Option<usize> {
    indexed_db_cursor_state(scope, cursor)?.position
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_mutation_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>, Key)> {
    let (request, transaction) = cursor_request_and_transaction(scope, cursor)?;
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
        .unwrap_or(false)
    {
        let error = dom_exception_value(
            scope,
            "The transaction is not active.",
            "TransactionInactiveError",
        );
        scope.throw_exception(error);
        return None;
    }
    if indexed_db_transaction_mode(scope, transaction) == Some(TransactionMode::ReadOnly) {
        let error = dom_exception_value(scope, "The transaction is readonly.", "ReadOnlyError");
        scope.throw_exception(error);
        return None;
    }
    let source = object_hidden_value(scope, request, INDEXED_DB_REQUEST_SOURCE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let store = indexed_db_index_object_store(scope, source).unwrap_or(source);
    if indexed_db_object_store_is_deleted(scope, store)
        || (source != store && indexed_db_index_is_deleted(scope, source))
        || object_string_property(scope, request, INDEXED_DB_REQUEST_READY_STATE_SLOT).as_deref()
            != Some("done")
        || indexed_db_cursor_state(scope, cursor).is_some_and(|state| state.key_only)
    {
        let error = dom_exception_value(
            scope,
            "The cursor cannot modify a record in its current state.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return None;
    }
    let Some(position) = cursor_current_position(scope, cursor) else {
        let error = dom_exception_value(scope, "The cursor is exhausted.", "InvalidStateError");
        scope.throw_exception(error);
        return None;
    };
    Some((
        store,
        transaction,
        cursor_primary_key_at(scope, cursor, position)?,
    ))
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_source_is_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> bool {
    cursor_source(scope, cursor)
        .and_then(|source| object_bool_property(scope, source, INDEXED_DB_INDEX_MARKER_SLOT))
        .unwrap_or(false)
}
