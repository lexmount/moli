//! SQL results use the accepting realm's existing IndexedDB task queue.

use super::*;
use moli_indexeddb::sqlite::service::{
    Command, Completion, DatabaseClient, DatabaseId, TaskPermit,
};
use parking_lot::Mutex;
use std::sync::Arc;

type Continuation =
    Box<dyn for<'s, 'i> FnOnce(&mut v8::PinScope<'s, 'i>, v8::Local<'s, v8::Object>, Completion)>;

/// Reserves native rollback capacity for owner/task retirement. Operation
/// scheduling belongs to the common transaction queue, independent of backend.
pub(super) struct SqlTransactionRollback {
    client: DatabaseClient,
    transaction: TransactionHandle,
    rollback: Option<TaskPermit>,
    finish: Option<TaskPermit>,
}

impl SqlTransactionRollback {
    fn new(client: DatabaseClient, transaction: TransactionHandle) -> Result<Self, IndexedDbError> {
        let command = Command::Abort { transaction };
        let rollback = client
            .try_reserve(command.kind(), command.input_bytes())
            .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
        let finish = client
            .try_reserve(command.kind(), command.input_bytes())
            .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
        Ok(Self {
            client,
            transaction,
            rollback: Some(rollback),
            finish: Some(finish),
        })
    }
}

pub(in crate::context_bootstrap::indexed_db) fn configure_sql_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    client: DatabaseClient,
    handle: TransactionHandle,
) -> Result<(), IndexedDbError> {
    let id = indexed_db_typed_state_id(scope, transaction)
        .ok_or_else(|| IndexedDbError::InvalidState("SQL transaction was released".into()))?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let mut table = table.borrow_mut();
    let state = table
        .transactions
        .get_mut(&id)
        .ok_or_else(|| IndexedDbError::InvalidState("SQL transaction was released".into()))?;
    state.handle = Some(handle);
    state.sql_client = Some(client);
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn take_sql_transaction_finish_permit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> Option<TaskPermit> {
    let id = indexed_db_typed_state_id(scope, transaction)?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let mut table = table.borrow_mut();
    let rollback = table.transactions.get_mut(&id)?.sql_rollback.as_mut()?;
    rollback.finish.take()
}

pub(in crate::context_bootstrap::indexed_db) fn disarm_sql_transaction_rollback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    if let Some(state) = table.borrow_mut().transactions.get_mut(&id)
        && let Some(mut rollback) = state.sql_rollback.take()
    {
        rollback.rollback = None;
    }
}

pub(in crate::context_bootstrap::indexed_db) fn sql_transaction_rollback_is_armed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return false;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table
        .borrow()
        .transactions
        .get(&id)
        .is_some_and(|state| state.sql_rollback.is_some())
}

pub(in crate::context_bootstrap::indexed_db) fn source_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if indexed_db_typed_wrapper_is(scope, source, IndexedDbWrapperKind::Transaction) {
        Some(source)
    } else {
        indexed_db_request_transaction_object(scope, source)
    }
}

impl Drop for SqlTransactionRollback {
    fn drop(&mut self) {
        if let Some(permit) = self.rollback.take() {
            let _ = self.client.submit_reserved(
                permit,
                Command::Abort {
                    transaction: self.transaction,
                },
                0,
                |_| {},
            );
        }
    }
}

pub(super) fn cancel_sql_transaction(
    table: &mut IndexedDbRuntimeStateTable,
    id: IndexedDbObjectId,
) {
    if let Some(transaction) = table.transactions.get_mut(&id) {
        if transaction.finished && !transaction.aborted {
            return;
        }
        transaction.aborted = true;
        transaction.finished = true;
        transaction.active = false;
        transaction.operations.clear();
        transaction.operation_in_flight = false;
        transaction.sql_rollback = None;
    }
}

pub(in crate::context_bootstrap::indexed_db) fn ensure_sql_transaction_rollback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> Result<(), IndexedDbError> {
    let transaction = source_transaction(scope, request).ok_or_else(|| {
        IndexedDbError::TransactionInactive("SQL request lost its transaction".into())
    })?;
    let id = indexed_db_typed_state_id(scope, transaction).ok_or_else(|| {
        IndexedDbError::TransactionInactive("SQL transaction was released".into())
    })?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let mut table = table.borrow_mut();
    let transaction = table.transactions.get_mut(&id).ok_or_else(|| {
        IndexedDbError::TransactionInactive("SQL transaction was released".into())
    })?;
    if transaction.finished || transaction.aborted {
        return Err(IndexedDbError::TransactionInactive(
            "SQL transaction is no longer running".into(),
        ));
    }
    if transaction.sql_rollback.is_none() {
        let client = transaction
            .sql_client
            .clone()
            .ok_or_else(|| IndexedDbError::InvalidState("SQL transaction has no client".into()))?;
        let handle = transaction.handle.ok_or_else(|| {
            IndexedDbError::TransactionInactive("SQL transaction has no handle".into())
        })?;
        transaction.sql_rollback = Some(SqlTransactionRollback::new(client, handle)?);
    }
    Ok(())
}

/// Capture the native state before running V8. Termination cleanup must not
/// inspect or allocate JavaScript objects after rethrowing the termination.
pub(in crate::context_bootstrap::indexed_db) fn sql_transaction_cancellation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> impl FnOnce() + 'static {
    capture_cancellation(scope, request, true)
}

/// An interrupted success listener cannot revoke an explicit COMMIT. Its
/// accepted native operations keep running on later owner tasks. Interrupting
/// the operation itself (serialization/backfill) instead aborts even then.
pub(in crate::context_bootstrap::indexed_db) fn sql_transaction_event_cancellation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> impl FnOnce() + 'static {
    capture_cancellation(scope, request, false)
}

fn capture_cancellation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    abort_committing: bool,
) -> impl FnOnce() + 'static {
    let state = source_transaction(scope, request).and_then(|transaction| {
        let id = indexed_db_typed_state_id(scope, transaction)?;
        let table = indexed_db_runtime_state_table_for_object(scope, transaction);
        Some((Rc::downgrade(&table), id))
    });
    move || {
        if let Some((table, id)) = state
            && let Some(table) = table.upgrade()
        {
            let mut table = table.borrow_mut();
            if !abort_committing
                && table
                    .transactions
                    .get(&id)
                    .is_some_and(|state| state.committing)
            {
                return;
            }
            cancel_sql_transaction(&mut table, id);
        }
    }
}

pub(super) struct SqlTaskPayload {
    pub(super) transaction: Option<IndexedDbObjectId>,
    source: RealmValueHandle,
    completion: Arc<Mutex<Option<Completion>>>,
    database: DatabaseId,
    generation: [u8; 16],
    continuation: Continuation,
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_request_sql_client<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> Option<DatabaseClient> {
    let transaction = source_transaction(scope, request)?;
    let id = indexed_db_typed_state_id(scope, transaction)?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table.borrow().transactions.get(&id)?.sql_client.clone()
}

#[cfg(test)]
fn submit_indexed_db_sql_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    client: &DatabaseClient,
    command: Command,
    result_bytes: usize,
    continuation: impl for<'a, 'i> FnOnce(
        &mut v8::PinScope<'a, 'i>,
        v8::Local<'a, v8::Object>,
        Completion,
    ) + 'static,
) -> Result<(), IndexedDbError> {
    let bytes = command
        .input_bytes()
        .checked_add(result_bytes)
        .ok_or_else(|| IndexedDbError::Resource("SQL input size overflow".into()))?;
    let permit = client
        .try_reserve(command.kind(), bytes)
        .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
    submit_indexed_db_sql_task_reserved(
        scope,
        source,
        client,
        permit,
        command,
        result_bytes,
        continuation,
    )
}

pub(in crate::context_bootstrap::indexed_db) fn submit_indexed_db_sql_task_reserved<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    client: &DatabaseClient,
    mut permit: TaskPermit,
    command: Command,
    result_bytes: usize,
    continuation: impl for<'a, 'i> FnOnce(
        &mut v8::PinScope<'a, 'i>,
        v8::Local<'a, v8::Object>,
        Completion,
    ) + 'static,
) -> Result<(), IndexedDbError> {
    let bytes = command
        .input_bytes()
        .checked_add(result_bytes)
        .ok_or_else(|| IndexedDbError::Resource("SQL input size overflow".into()))?;
    permit
        .try_resize(permit.reserved_bytes().max(bytes))
        .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
    let context = source.get_creation_context(scope).ok_or_else(|| {
        IndexedDbError::InvalidState("IndexedDB SQL task lost its accepting realm".into())
    })?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let owner = indexed_db_typed_execution_owner(scope, source)
        .ok_or_else(|| IndexedDbError::InvalidState("IndexedDB SQL task lost its owner".into()))?;
    let storage = indexed_db_typed_storage_scope(scope, source);
    let task = v8::Object::new(scope);
    let id = register_indexed_db_task_with_owner(
        scope,
        task,
        IndexedDbTaskKind::SqlCompletion,
        owner,
        storage,
    );
    let completion = Arc::new(Mutex::new(None));
    let receiver = Arc::downgrade(&completion);
    let wake = super::super::runtime::indexed_db_sql_completion_wake(scope, owner, id);
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let transaction = source_transaction(scope, source)
        .and_then(|transaction| indexed_db_typed_state_id(scope, transaction));
    let payload = SqlTaskPayload {
        transaction,
        source: RealmValueHandle::new(scope, source.into()),
        completion,
        database: client.id(),
        generation: client.identity().generation,
        continuation: Box::new(continuation),
    };
    table.borrow_mut().sql_tasks.insert(id, payload);
    let queued =
        indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue).is_some_and(|queue| {
            moli_webapi_declare::define_array_data_property(
                scope,
                queue,
                queue.length(),
                task.into(),
            )
            .is_some()
        });
    if !queued {
        table.borrow_mut().sql_tasks.remove(&id);
        unregister_indexed_db_task(scope, task);
        return Err(IndexedDbError::InvalidState(
            "Unable to retain IndexedDB SQL task in its accepting realm".into(),
        ));
    }
    let submitted = client.submit_reserved(permit, command, result_bytes, move |result| {
        let Some(receiver) = receiver.upgrade() else {
            return;
        };
        // Publish the ticket under the readiness lock. An explicit owner pump
        // cannot consume the result before its producer has published the wake.
        let mut slot = receiver.lock();
        *slot = Some(result);
        let discarded = if wake() { None } else { slot.take() };
        drop(slot);
        drop(discarded);
    });
    if let Err(error) = submitted {
        // Nothing reached the worker. Let the caller settle or cancel its
        // operation without treating a rejected submission as owner loss.
        table.borrow_mut().sql_tasks.remove(&id);
        discard_indexed_db_task_by_id(scope, id);
        return Err(IndexedDbError::Resource(error.to_string()));
    }
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_task_is_ready<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> bool {
    if indexed_db_typed_task_kind(scope, task) != Some(IndexedDbTaskKind::SqlCompletion) {
        return true;
    }
    let Some(id) = indexed_db_typed_task_id(scope, task) else {
        return false;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    table
        .borrow()
        .sql_tasks
        .get(&id)
        .is_some_and(|task| task.completion.lock().is_some())
}

pub(in crate::context_bootstrap::indexed_db) fn flush_indexed_db_sql_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_task_id(scope, task) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let Some(payload) = table.borrow_mut().sql_tasks.remove(&id) else {
        return;
    };
    let Some(mut completion) = payload.completion.lock().take() else {
        return;
    };
    if completion.database != payload.database || completion.generation != payload.generation {
        if let Some(transaction) = payload.transaction {
            cancel_sql_transaction(&mut table.borrow_mut(), transaction);
        }
        completion.result = Err(IndexedDbError::InvalidState(
            "IndexedDB completion belongs to another database generation".into(),
        ));
    }
    let Ok(source) = v8::Local::<v8::Object>::try_from(payload.source.to_local(scope)) else {
        if let Some(transaction) = payload.transaction {
            cancel_sql_transaction(&mut table.borrow_mut(), transaction);
        }
        return;
    };
    if !indexed_db_typed_wrapper_is(scope, source, IndexedDbWrapperKind::Transaction)
        && let Some(transaction) = source_transaction(scope, source)
        && object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
            .unwrap_or(false)
    {
        let error = dom_exception_value(scope, "The transaction was aborted.", "AbortError");
        store_request_error(scope, source, error);
        complete_indexed_db_transaction_operation(scope, transaction);
        return;
    }
    (payload.continuation)(scope, source, completion);
}

#[cfg(test)]
mod tests;
