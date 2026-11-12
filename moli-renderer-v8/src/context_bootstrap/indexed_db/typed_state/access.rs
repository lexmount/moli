use super::*;

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_transaction_status(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
) -> Option<IndexedDbTransactionStatus> {
    let id = indexed_db_typed_state_id(scope, transaction)?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table
        .borrow()
        .transactions
        .get(&id)
        .map(|state| state.status)
}

pub(in crate::context_bootstrap::indexed_db) fn update_indexed_db_transaction_status<R>(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
    update: impl FnOnce(&mut IndexedDbTransactionStatus) -> R,
) -> Option<R> {
    let id = indexed_db_typed_state_id(scope, transaction)?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    Some(update(
        &mut table.borrow_mut().transactions.get_mut(&id)?.status,
    ))
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_request_ready_state(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
) -> Option<IndexedDbRequestReadyState> {
    let id = indexed_db_typed_state_id(scope, request)?;
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    table
        .borrow()
        .requests
        .get(&id)
        .map(|state| state.ready_state)
}

pub(in crate::context_bootstrap::indexed_db) fn set_indexed_db_request_ready_state(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
    ready_state: IndexedDbRequestReadyState,
) {
    let Some(id) = indexed_db_typed_state_id(scope, request) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    if let Some(state) = table.borrow_mut().requests.get_mut(&id) {
        state.ready_state = ready_state;
    }
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_request_cursor_position(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
) -> Option<usize> {
    let id = indexed_db_typed_state_id(scope, request)?;
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    table.borrow().requests.get(&id)?.pending_cursor_position
}

pub(in crate::context_bootstrap::indexed_db) fn set_indexed_db_request_cursor_position(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
    position: Option<usize>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, request) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    if let Some(state) = table.borrow_mut().requests.get_mut(&id) {
        state.pending_cursor_position = position;
    }
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_database_is_closed(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(id) = indexed_db_typed_state_id(scope, database) else {
        return false;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    table
        .borrow()
        .databases
        .get(&id)
        .is_some_and(|state| state.closed)
}

pub(in crate::context_bootstrap::indexed_db) fn mark_indexed_db_database_closed(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, database) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    if let Some(state) = table.borrow_mut().databases.get_mut(&id) {
        state.closed = true;
    }
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_database_upgrade_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'_, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = indexed_db_typed_state_id(scope, database)?;
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let table = table.borrow();
    let transaction = table
        .databases
        .get(&id)?
        .upgrade_transaction
        .as_ref()?
        .to_local(scope);
    v8::Local::<v8::Object>::try_from(transaction).ok()
}

pub(in crate::context_bootstrap::indexed_db) fn set_indexed_db_database_upgrade_transaction(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
    transaction: Option<v8::Local<'_, v8::Object>>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, database) else {
        return;
    };
    let transaction = transaction.map(|value| RealmValueHandle::new(scope, value.into()));
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    if let Some(state) = table.borrow_mut().databases.get_mut(&id) {
        if transaction.is_none()
            && let Some(mirror) = state.mirror.upgrade()
        {
            mirror.borrow_mut().upgrade_metadata = None;
        }
        state.upgrade_transaction = transaction;
    }
}
