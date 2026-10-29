//! SQL results use the accepting realm's existing IndexedDB task queue.

use super::*;
use moli_indexeddb::sqlite::service::{Command, Completion, DatabaseClient, DatabaseId};
use parking_lot::Mutex;
use std::sync::Arc;

pub(in crate::context_bootstrap::indexed_db) const SQL_ABORT_REQUEST: &str =
    "moli.IndexedDb.sql.abortRequest";

type SqlReply = Result<Completion, IndexedDbError>;
type Continuation =
    Box<dyn for<'s, 'i> FnOnce(&mut v8::PinScope<'s, 'i>, v8::Local<'s, v8::Object>, SqlReply)>;

/// Owner retirement rolls back on the existing worker FIFO. This guard owns
/// the transaction identity until a terminal native receipt.
pub(super) struct SqlTransactionRollback {
    client: DatabaseClient,
    transaction: TransactionHandle,
    armed: bool,
}

impl SqlTransactionRollback {
    pub(super) fn new(client: DatabaseClient, transaction: TransactionHandle) -> Self {
        Self {
            client,
            transaction,
            armed: true,
        }
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

pub(super) fn configure_sql_upgrade<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    rollback: SqlTransactionRollback,
) -> Result<(), IndexedDbError> {
    configure_sql_transaction(
        scope,
        transaction,
        rollback.client.clone(),
        rollback.transaction,
    )?;
    let id = indexed_db_typed_state_id(scope, transaction)
        .ok_or_else(|| IndexedDbError::InvalidState("SQL upgrade was released".into()))?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table
        .borrow_mut()
        .transactions
        .get_mut(&id)
        .expect("configured SQL upgrade")
        .sql_rollback = Some(rollback);
    Ok(())
}

/// The rollback guard stays armed until the native abort receipt is consumed.
pub(in crate::context_bootstrap::indexed_db) fn submit_sql_transaction_abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    client: &DatabaseClient,
    handle: TransactionHandle,
    continuation: impl for<'a, 'i> FnOnce(
        &mut v8::PinScope<'a, 'i>,
        v8::Local<'a, v8::Object>,
        Completion,
    ) + 'static,
) -> Result<(), IndexedDbError> {
    if !sql_transaction_rollback_is_armed(scope, transaction) {
        return Err(IndexedDbError::TransactionInactive(
            "SQL transaction was released".into(),
        ));
    }
    submit_indexed_db_sql_completion_task(
        scope,
        transaction,
        move |complete| {
            client.submit(
                Command::Abort {
                    transaction: handle,
                },
                complete,
            )?;
            Ok(client.clone())
        },
        continuation,
    )
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
        rollback.armed = false;
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
    table.borrow().transactions.get(&id).is_some_and(|state| {
        state
            .sql_rollback
            .as_ref()
            .is_some_and(|rollback| rollback.armed)
    })
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
        if self.armed {
            let _ = self.client.submit(
                Command::Abort {
                    transaction: self.transaction,
                },
                drop,
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
        transaction.sql_rollback = Some(SqlTransactionRollback::new(client, handle));
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
    completion: Arc<Mutex<Option<SqlReply>>>,
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

pub(in crate::context_bootstrap::indexed_db) fn submit_indexed_db_sql_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    client: &DatabaseClient,
    command: Command,
    continuation: impl for<'a, 'i> FnOnce(
        &mut v8::PinScope<'a, 'i>,
        v8::Local<'a, v8::Object>,
        Completion,
    ) + 'static,
) -> Result<(), IndexedDbError> {
    submit_indexed_db_sql_completion_task(
        scope,
        source,
        move |complete| {
            client.submit(command, complete)?;
            Ok(client.clone())
        },
        continuation,
    )
}

/// Resolve/upgrade and transaction commands share the same accepting-owner
/// task transport. Only the submission differs; no second result queue exists.
pub(in crate::context_bootstrap::indexed_db) fn submit_indexed_db_sql_completion_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    submit: impl FnOnce(
        Box<dyn FnOnce(Completion) + Send>,
    ) -> Result<DatabaseClient, moli_indexeddb::sqlite::service::DispatchError>,
    continuation: impl for<'a, 'i> FnOnce(
        &mut v8::PinScope<'a, 'i>,
        v8::Local<'a, v8::Object>,
        Completion,
    ) + 'static,
) -> Result<(), IndexedDbError> {
    submit_sql_reply_task(
        scope,
        source,
        move |complete| submit(Box::new(move |reply| complete(Ok(reply)))),
        move |scope, source, reply| match reply {
            Ok(reply) => continuation(scope, source, reply),
            Err(error) => abort_sql_dispatch(scope, source, error),
        },
    )
}

/// Quota sampling returns through the same owner completion. Admission errors carry no
/// rollback receipt and must not disarm the transaction's emergency guard.
pub(in crate::context_bootstrap::indexed_db) fn submit_indexed_db_sql_quota_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    client: &DatabaseClient,
    command: Command,
    continuation: impl for<'a, 'i> FnOnce(
        &mut v8::PinScope<'a, 'i>,
        v8::Local<'a, v8::Object>,
        SqlReply,
    ) + 'static,
) -> Result<(), IndexedDbError> {
    let quota = super::super::storage_bucket::sql_quota_owner(scope, source)?;
    let client = client.clone();
    submit_sql_reply_task(
        scope,
        source,
        move |complete| {
            if let Some(quota) = quota {
                quota
                    .dispatch_indexed_db(client.clone(), command, complete)
                    .map_err(|error| {
                        moli_indexeddb::sqlite::service::DispatchError::Storage(
                            IndexedDbError::Resource(error.to_string()),
                        )
                    })?;
            } else {
                client.submit_checked(command, Ok(()), move |reply| complete(Ok(reply)))?;
            }
            Ok(client)
        },
        continuation,
    )
}

pub(in crate::context_bootstrap::indexed_db) fn abort_sql_dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    error: IndexedDbError,
) {
    let Some(transaction) = source_transaction(scope, source) else {
        return;
    };
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
        .unwrap_or(false)
    {
        if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
            .unwrap_or(false)
            && !indexed_db_typed_wrapper_is(scope, source, IndexedDbWrapperKind::Transaction)
        {
            let error = dom_exception_value(scope, "The transaction was aborted.", "AbortError");
            store_request_error(scope, source, error);
            complete_indexed_db_transaction_operation(scope, transaction);
        }
        return;
    }
    if !indexed_db_typed_wrapper_is(scope, source, IndexedDbWrapperKind::Transaction) {
        // The original completion is about to be consumed. Retain its request
        // through rollback even when no JavaScript variable references it.
        set_private_value(scope, transaction, SQL_ABORT_REQUEST, source.into());
    }
    let error = request_error_object(scope, &error);
    super::super::finish_transaction_abort(scope, transaction, error);
}

pub(in crate::context_bootstrap::indexed_db) fn submit_sql_reply_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    submit: impl FnOnce(
        Box<dyn FnOnce(SqlReply) + Send>,
    ) -> Result<DatabaseClient, moli_indexeddb::sqlite::service::DispatchError>,
    continuation: impl for<'a, 'i> FnOnce(
        &mut v8::PinScope<'a, 'i>,
        v8::Local<'a, v8::Object>,
        SqlReply,
    ) + 'static,
) -> Result<(), IndexedDbError> {
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
    let source = RealmValueHandle::new(scope, source.into());
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
    let submitted = submit(Box::new(move |result| {
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
    }));
    let client = match submitted {
        Ok(client) => client,
        Err(error) => {
            discard_indexed_db_task_by_id(scope, id);
            return Err(match error {
                moli_indexeddb::sqlite::service::DispatchError::Storage(error) => error,
                moli_indexeddb::sqlite::service::DispatchError::Closed => {
                    IndexedDbError::InvalidState(error.to_string())
                }
                error => IndexedDbError::Resource(error.to_string()),
            });
        }
    };
    // A worker may already have published a wake, but this realm cannot dispatch
    // it until this accepting call returns. The readiness predicate still sees
    // no payload before registration finishes.
    table.borrow_mut().sql_tasks.insert(
        id,
        SqlTaskPayload {
            transaction,
            source,
            completion,
            database: client.id(),
            generation: client.identity().generation,
            continuation: Box::new(continuation),
        },
    );
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_task_is_ready<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> bool {
    if indexed_db_typed_task_kind(scope, task) == Some(IndexedDbTaskKind::FactoryRequest) {
        let Some(id) = indexed_db_typed_task_id(scope, task) else {
            return false;
        };
        let table = indexed_db_runtime_state_table_for_object(scope, task);
        return super::factory::factory_task_is_ready(&table.borrow(), id);
    }
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
    let Some(reply) = payload.completion.lock().take() else {
        return;
    };
    let Ok(source) = v8::Local::<v8::Object>::try_from(payload.source.to_local(scope)) else {
        if let Some(transaction) = payload.transaction {
            cancel_sql_transaction(&mut table.borrow_mut(), transaction);
        }
        return;
    };
    let mut completion = match reply {
        Ok(reply) => reply,
        Err(error) => {
            (payload.continuation)(scope, source, Err(error));
            return;
        }
    };
    if completion.database != payload.database || completion.generation != payload.generation {
        if let Some(transaction) = payload.transaction {
            cancel_sql_transaction(&mut table.borrow_mut(), transaction);
        }
        completion.result = Err(IndexedDbError::InvalidState(
            "IndexedDB completion belongs to another database generation".into(),
        ));
    }
    if !indexed_db_typed_wrapper_is(scope, source, IndexedDbWrapperKind::Transaction)
        && let Some(transaction) = source_transaction(scope, source)
        && let Some(handle) = indexed_db_native_transaction_handle(scope, transaction)
        && completion.active_transaction != Some(handle)
        && !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
            .unwrap_or(false)
        && let Err(error) = &completion.result
        && !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
            .unwrap_or(false)
    {
        // A lost native transaction is not a recoverable request SAVEPOINT
        // failure. Its receipt proves the old SQL owner is gone, so no owner
        // event (including preventDefault) can revive it or release it twice.
        let error = match error {
            IndexedDbError::TransactionInactive(message) => IndexedDbError::Io(message.clone()),
            error => error.clone(),
        };
        let error = request_error_object(scope, &error);
        disarm_sql_transaction_rollback(scope, transaction);
        set_indexed_db_slot_value(
            scope,
            transaction,
            INDEXED_DB_TRANSACTION_ABORTED_SLOT,
            v8::Boolean::new(scope, true).into(),
        );
        set_indexed_db_slot_value(
            scope,
            transaction,
            INDEXED_DB_TRANSACTION_ACTIVE_SLOT,
            v8::Boolean::new(scope, false).into(),
        );
        // Preserve request-list order: settle this in-flight request before
        // draining later queued requests and queuing the transaction abort.
        let request_error =
            dom_exception_value(scope, "The transaction was aborted.", "AbortError");
        store_request_error(scope, source, request_error);
        crate::context_bootstrap::indexed_db::finish_transaction_abort_after_rollback(
            scope,
            transaction,
            error,
        );
        complete_indexed_db_transaction_operation(scope, transaction);
        return;
    }
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
    (payload.continuation)(scope, source, Ok(completion));
}

/// Rollback is confirmed. Retract deliveries still waiting for quota or SQL,
/// then fail their requests before the transaction's terminal event.
pub(in crate::context_bootstrap::indexed_db) fn abort_in_flight_sql_requests<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    let Some(context) = transaction.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(transaction_id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    loop {
        let next = table.borrow().sql_tasks.iter().find_map(|(id, payload)| {
            if payload.transaction != Some(transaction_id) {
                return None;
            }
            let request = v8::Local::<v8::Object>::try_from(payload.source.to_local(scope)).ok()?;
            indexed_db_typed_wrapper_is(scope, request, IndexedDbWrapperKind::Request)
                .then_some((*id, request))
        });
        let Some((id, request)) = next else {
            break;
        };
        let request_id = indexed_db_typed_state_id(scope, request);
        let (payload, pending) = {
            let mut table = table.borrow_mut();
            let payload = table.sql_tasks.remove(&id);
            let pending = request_id
                .and_then(|id| table.requests.get(&id))
                .is_some_and(|state| {
                    state.ready_state == "pending" && !state.error_pending_dispatch
                });
            (payload, pending)
        };
        // Remove the completion first: generic task retirement would otherwise
        // cancel the transaction and erase its remaining queued requests.
        drop(payload);
        discard_indexed_db_task_by_id(scope, id);
        if pending {
            store_request_error(scope, request, error);
        }
        complete_indexed_db_transaction_operation(scope, transaction);
    }
}

#[cfg(test)]
mod tests;
