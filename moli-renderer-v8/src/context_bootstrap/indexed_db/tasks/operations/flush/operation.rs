use super::*;
use crate::context_bootstrap::indexed_db::indexed_db_typed_storage_scope;

pub(super) struct TransactionOperation<'s> {
    pub(super) source: v8::Local<'s, v8::Object>,
    pub(super) handle: TransactionHandle,
    pub(super) store_name: moli_indexeddb::IndexedDbName,
    pub(super) kind: IndexedDbTransactionOperation,
}

pub(in crate::context_bootstrap::indexed_db) fn pump_transaction_operations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    pump_operations(scope, transaction, false);
}

fn pump_operations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    restore_immediately: bool,
) {
    let context = transaction
        .get_creation_context(scope)
        .expect("IDB transactions retain their relevant realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    while let Some(pending) =
        crate::context_bootstrap::indexed_db::pop_indexed_db_transaction_operation(
            scope,
            transaction,
        )
    {
        let owner = pending.owner();
        let operation = pending.into_locals(scope);
        let restore = owner.enter(scope);
        let in_flight = match operation.kind {
            IndexedDbTransactionOperation::InternalError(error) => {
                enqueue_transaction_operation_error(scope, transaction, error);
                true
            }
            IndexedDbTransactionOperation::Sql { start } => start(scope, operation.request),
            _ => unreachable!(
                "IndexedDB requests must be prepared before entering the operation queue"
            ),
        };
        if scope.is_execution_terminating() {
            return;
        }
        if restore_immediately {
            owner.restore(scope, restore);
        } else {
            owner.defer_restore(scope, restore);
        }
        if in_flight {
            return;
        }
        crate::context_bootstrap::indexed_db::complete_indexed_db_transaction_operation(
            scope,
            transaction,
        );
    }
    if crate::context_bootstrap::indexed_db::indexed_db_transaction_can_commit(scope, transaction) {
        enqueue_transaction_commit_task(scope, transaction);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn finish_transaction_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) {
    if scope.is_execution_terminating() {
        return;
    }
    let Some(transaction) =
        crate::context_bootstrap::indexed_db::typed_state::sql::source_transaction(scope, request)
    else {
        return;
    };
    crate::context_bootstrap::indexed_db::complete_indexed_db_transaction_operation(
        scope,
        transaction,
    );
    pump_transaction_operations(scope, transaction);
}

/// Internal schema/backfill work shares the request queue and its in-flight
/// barrier, without allocating a dummy IDBRequest or incrementing event count.
pub(in crate::context_bootstrap::indexed_db) fn schedule_internal_transaction_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    kind: IndexedDbTransactionOperation,
) -> Result<(), IndexedDbError> {
    let owner = indexed_db_typed_owner_scope(scope, transaction).ok_or_else(|| {
        IndexedDbError::InvalidState("Internal IndexedDB work lost its owner".into())
    })?;
    let operation = IndexedDbPendingTransactionOperation::new(scope, owner, transaction, kind);
    push_indexed_db_transaction_operation(scope, transaction, operation)?;
    pump_operations(scope, transaction, true);
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn schedule_transaction_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    store_name: &moli_indexeddb::IndexedDbName,
    input: IndexedDbTransactionOperation,
) {
    let transaction_context = transaction
        .get_creation_context(scope)
        .expect("registered IndexedDB transactions retain their creation context");
    let request_context = request
        .get_creation_context(scope)
        .expect("registered IndexedDB requests retain their creation context");
    let scope = &mut v8::ContextScope::new(scope, transaction_context);
    // Borrowed methods can create a source wrapper in another realm. Its
    // database execution owner and storage scope still belong to the transaction.
    let execution_owner = indexed_db_typed_execution_owner(scope, transaction)
        .expect("IDB transactions retain their execution owner");
    let storage_scope = indexed_db_typed_storage_scope(scope, transaction)
        .expect("IDB transactions retain their storage scope");
    assert!(
        transaction_context == request_context,
        "IDB requests inherit their transaction's relevant realm"
    );
    for value in [source, request] {
        assert_eq!(
            indexed_db_typed_execution_owner(scope, value),
            Some(execution_owner),
            "IDB operation objects must share their transaction's execution owner"
        );
        assert_eq!(
            indexed_db_typed_storage_scope(scope, value).as_ref(),
            Some(&storage_scope),
            "IDB operation objects must share their transaction's storage scope"
        );
    }
    let owner = indexed_db_typed_owner_scope(scope, transaction)
        .expect("IDB transaction operations retain their transaction's dispatch scope");
    let Some(client) =
        crate::context_bootstrap::indexed_db::typed_state::sql::indexed_db_request_sql_client(
            scope, request,
        )
    else {
        let error = request_error_object(
            scope,
            &IndexedDbError::InvalidState("IndexedDB transaction is unavailable".into()),
        );
        store_request_error(scope, request, error);
        return;
    };
    let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
        let error = dom_exception_value(
            scope,
            "The transaction is not active.",
            "TransactionInactiveError",
        );
        store_request_error(scope, request, error);
        return;
    };
    let kind = match sql::prepare_operation(
        scope,
        &client,
        TransactionOperation {
            source,
            handle,
            store_name: store_name.to_owned(),
            kind: input,
        },
    ) {
        Ok(kind) => kind,
        Err(error) => IndexedDbTransactionOperation::sql(move |scope, request| {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
            false
        }),
    };
    let operation = IndexedDbPendingTransactionOperation::new(scope, owner, request, kind);
    if let Err(error) = push_indexed_db_transaction_operation(scope, transaction, operation) {
        let error = request_error_object(scope, &error);
        store_request_error(scope, request, error);
        return;
    }
    // The API path restores its caller immediately. Transaction-start and SQL
    // completion tasks keep their existing deferred owner restoration.
    pump_operations(scope, transaction, true);
}
