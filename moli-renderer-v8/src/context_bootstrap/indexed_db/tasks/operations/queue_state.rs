use super::*;
use crate::context_bootstrap::indexed_db::indexed_db_typed_storage_scope;

pub(in crate::context_bootstrap::indexed_db) fn queue_transaction_request(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
    request: v8::Local<'_, v8::Object>,
) {
    let pending = object_number_property(scope, transaction, INDEXED_DB_TRANSACTION_PENDING_SLOT)
        .unwrap_or(0.0);
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_PENDING_SLOT,
        v8::Number::new(scope, pending + 1.0).into(),
    );
    set_indexed_db_request_surface_value(
        scope,
        request,
        INDEXED_DB_REQUEST_TRANSACTION_SLOT,
        "transaction",
        transaction.into(),
    );
}

pub(in crate::context_bootstrap::indexed_db) fn enqueue_transaction_operation<'s>(
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
    let operation =
        IndexedDbPendingTransactionOperation::new(scope, owner, source, request, store_name, input);
    push_indexed_db_operation_waiting_for_start(scope, transaction, operation)
        .expect("IDB transaction operation should bind to its transaction state");
}

pub(in crate::context_bootstrap::indexed_db) fn abort_queued_transaction_requests<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    for operation in take_indexed_db_operations_waiting_for_start(scope, transaction) {
        let request = operation.request(scope);
        store_request_error(scope, request, error);
    }
}
