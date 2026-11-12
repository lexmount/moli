use super::*;
use crate::context_bootstrap::indexed_db::indexed_db_transaction_status;

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
    if indexed_db_transaction_status(scope, transaction).map_or(0, |state| state.pending) == 0 {
        enqueue_transaction_commit_task(scope, transaction);
    }
}
