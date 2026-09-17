use super::*;

pub(super) fn start_regular_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    transaction_handle: TransactionHandle,
) {
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_HANDLE_SLOT,
        v8::Number::new(scope, transaction_handle.into_raw() as f64).into(),
    );
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_STARTED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    run_waiting::run_operations_waiting_for_start(scope, transaction);
    if object_number_property(scope, transaction, INDEXED_DB_TRANSACTION_PENDING_SLOT)
        .unwrap_or(0.0)
        == 0.0
    {
        enqueue_transaction_commit_task(scope, transaction);
    }
}
