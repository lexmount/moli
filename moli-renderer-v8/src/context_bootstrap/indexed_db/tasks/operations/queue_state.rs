use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbRequestValue, set_request_value, update_indexed_db_transaction_status,
};

pub(in crate::context_bootstrap::indexed_db) fn queue_transaction_request(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
    request: v8::Local<'_, v8::Object>,
) {
    let _ = update_indexed_db_transaction_status(scope, transaction, |state| {
        state.pending = state.pending.saturating_add(1);
    });
    set_request_value(
        scope,
        request,
        IndexedDbRequestValue::Transaction,
        transaction.into(),
    );
}

pub(in crate::context_bootstrap::indexed_db) fn abort_queued_transaction_requests<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    for operation in take_indexed_db_transaction_operations(scope, transaction) {
        if let Some(request) = operation.request(scope) {
            store_request_error(scope, request, error);
        }
    }
}
