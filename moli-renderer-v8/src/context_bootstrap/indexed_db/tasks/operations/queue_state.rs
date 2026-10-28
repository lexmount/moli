use super::*;

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

pub(in crate::context_bootstrap::indexed_db) fn abort_queued_transaction_requests<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    for operation in take_indexed_db_transaction_operations(scope, transaction) {
        let request = operation.request(scope);
        store_request_error(scope, request, error);
    }
}
