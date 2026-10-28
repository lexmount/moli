use super::*;
use crate::context_bootstrap::indexed_db::indexed_db_typed_storage_scope;

pub(super) struct TransactionOperation<'s> {
    pub(super) source: v8::Local<'s, v8::Object>,
    pub(super) request: v8::Local<'s, v8::Object>,
    pub(super) handle: TransactionHandle,
    pub(super) store_name: String,
    pub(super) kind: IndexedDbTransactionOperation,
}

impl<'s> TransactionOperation<'s> {
    fn from_pending(
        scope: &mut v8::PinScope<'s, '_>,
        transaction: v8::Local<'s, v8::Object>,
        pending: IndexedDbPendingTransactionOperation,
    ) -> Self {
        let operation = pending.into_locals(scope);
        Self {
            source: operation.source,
            request: operation.request,
            handle: transaction_handle_from_value(scope, transaction.into())
                .expect("started IDB transactions should retain their backend handle"),
            store_name: operation.store_name,
            kind: operation.kind,
        }
    }
}

pub(super) fn flush_queued_transaction_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    pending: IndexedDbPendingTransactionOperation,
) {
    let owner = pending.owner();
    let operation = TransactionOperation::from_pending(scope, transaction, pending);
    let owner_restore = owner.enter(scope);
    dispatch_operation(scope, operation);
    owner.defer_restore(scope, owner_restore);
}

fn dispatch_operation<'s>(scope: &mut v8::PinScope<'s, '_>, operation: TransactionOperation<'s>) {
    let dispatched = cursor::try_dispatch_cursor_operation(scope, &operation)
        || object_store::try_dispatch_object_store_operation(scope, &operation)
        || index::try_dispatch_index_operation(scope, &operation);
    assert!(dispatched, "typed IDB operation should have a dispatcher");
}

pub(in crate::context_bootstrap::indexed_db) fn schedule_transaction_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    store_name: &str,
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
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_STARTED_SLOT)
        .unwrap_or(false)
    {
        let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
            let error = dom_exception_value(
                scope,
                "The transaction is not active.",
                "TransactionInactiveError",
            );
            store_request_error(scope, request, error);
            return;
        };
        let owner_restore = owner.enter(scope);
        dispatch_operation(
            scope,
            TransactionOperation {
                source,
                request,
                handle,
                store_name: store_name.to_owned(),
                kind: input,
            },
        );
        // A synchronous API call must restore the caller before returning.
        owner.restore(scope, owner_restore);
        return;
    }
    let operation =
        IndexedDbPendingTransactionOperation::new(scope, owner, source, request, store_name, input);
    push_indexed_db_operation_waiting_for_start(scope, transaction, operation)
        .expect("IDB transaction operation should bind to its transaction state");
}
