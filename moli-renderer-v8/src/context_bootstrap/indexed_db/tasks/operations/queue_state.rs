use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbWrapperKind, indexed_db_typed_wrapper_is,
    mark_indexed_db_request_awaiting_operation_result,
    take_indexed_db_request_awaiting_operation_result,
};

pub(in crate::context_bootstrap::indexed_db) fn queue_transaction_request(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
    request: v8::Local<'_, v8::Object>,
) {
    mark_indexed_db_request_awaiting_operation_result(scope, request);
    increment_pending_transaction_requests(scope, transaction);
    set_indexed_db_slot_value(
        scope,
        request,
        INDEXED_DB_REQUEST_TRANSACTION_SLOT,
        transaction.into(),
    );
}

fn increment_pending_transaction_requests(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
) {
    let pending = object_number_property(scope, transaction, INDEXED_DB_TRANSACTION_PENDING_SLOT)
        .unwrap_or(0.0);
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_PENDING_SLOT,
        v8::Number::new(scope, pending + 1.0).into(),
    );
}

pub(in crate::context_bootstrap::indexed_db) fn queue_transaction_request_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) {
    if !indexed_db_typed_wrapper_is(scope, request, IndexedDbWrapperKind::Request)
        || take_indexed_db_request_awaiting_operation_result(scope, request)
    {
        return;
    }
    if let Some(transaction) = indexed_db_request_transaction_object(scope, request) {
        // Immediate operations become accepted when their result is queued.
        // Synchronous validation failures never increment the pending count.
        // Deferred operations and cursor iteration already counted acceptance.
        increment_pending_transaction_requests(scope, transaction);
    }
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
    let source_context = source
        .get_creation_context(scope)
        .expect("registered IndexedDB operation sources retain their creation context");
    let request_context = request
        .get_creation_context(scope)
        .expect("registered IndexedDB requests retain their creation context");
    assert!(
        transaction_context == source_context && transaction_context == request_context,
        "IndexedDB transaction operations must remain in the receiver's relevant realm"
    );

    let owner = indexed_db_typed_owner_scope(scope, request)
        .expect("IDB transaction operation requests should have typed owner state");
    let operation =
        IndexedDbPendingTransactionOperation::new(scope, owner, source, request, store_name, input);
    queue_transaction_request(scope, transaction, request);
    push_indexed_db_operation_waiting_for_start(scope, transaction, operation)
        .expect("IDB transaction operation should bind to its transaction state");
}

pub(in crate::context_bootstrap::indexed_db) fn abort_queued_transaction_requests<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    for operation in take_indexed_db_operations_waiting_for_start(scope, transaction) {
        let request = operation.request(scope);
        let error = dom_exception_value(scope, "The transaction was aborted.", "AbortError");
        store_request_error(scope, request, error);
    }
}
