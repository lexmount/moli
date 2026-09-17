use super::*;

pub(in crate::context_bootstrap::indexed_db) fn cursor_request_and_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    let request = cursor_request(scope, cursor)?;
    let transaction = indexed_db_request_transaction_object(scope, request)?;
    Some((request, transaction))
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_key_at(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
    position: usize,
) -> Option<Key> {
    Some(
        indexed_db_cursor_state(scope, cursor)?
            .snapshot
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
            .snapshot
            .entries
            .get(position)?
            .primary_key
            .clone(),
    )
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_iteration_position(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
) -> Option<usize> {
    let state = indexed_db_cursor_state(scope, cursor)?;
    if state.got_value {
        return state.position;
    }
    let error = dom_exception_value(
        scope,
        "The cursor is being iterated or has iterated past its end.",
        "InvalidStateError",
    );
    scope.throw_exception(error);
    None
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_active_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let (_, transaction) = cursor_request_and_transaction(scope, cursor)?;
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
    Some(transaction)
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_effective_object_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let source = cursor_source(scope, cursor)?;
    let store = indexed_db_index_object_store(scope, source).unwrap_or(source);
    if indexed_db_object_store_is_deleted(scope, store)
        || (source != store && indexed_db_index_is_deleted(scope, source))
    {
        let error = dom_exception_value(
            scope,
            "The cursor's source or effective object store has been deleted.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return None;
    }
    Some(store)
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_mutation_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>, Key)> {
    let transaction = cursor_active_transaction(scope, cursor)?;
    if indexed_db_transaction_mode(scope, transaction) == Some(TransactionMode::ReadOnly) {
        let error = dom_exception_value(scope, "The transaction is readonly.", "ReadOnlyError");
        scope.throw_exception(error);
        return None;
    }
    let store = cursor_effective_object_store(scope, cursor)?;
    let position = cursor_iteration_position(scope, cursor)?;
    if indexed_db_cursor_state(scope, cursor)?.key_only {
        let error = dom_exception_value(
            scope,
            "A key-only cursor cannot modify records.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return None;
    }
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
