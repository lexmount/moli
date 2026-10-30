use super::*;

pub(super) fn start_regular_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    transaction_handle: TransactionHandle,
) {
    crate::context_bootstrap::indexed_db::mark_indexed_db_transaction_started(
        scope,
        transaction,
        transaction_handle,
    );
    crate::context_bootstrap::indexed_db::pump_transaction_operations(scope, transaction);
    if object_number_property(scope, transaction, INDEXED_DB_TRANSACTION_PENDING_SLOT)
        .unwrap_or(0.0)
        == 0.0
    {
        enqueue_transaction_commit_task(scope, transaction);
    }
}
