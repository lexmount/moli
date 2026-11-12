use super::*;
use crate::context_bootstrap::indexed_db::{
    indexed_db_transaction_status, update_indexed_db_transaction_status,
};

pub(in crate::context_bootstrap::indexed_db) fn enqueue_transaction_abort_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    if indexed_db_transaction_status(scope, transaction).is_some_and(|state| state.abort_dispatched)
    {
        return;
    }
    let _ = update_indexed_db_transaction_status(scope, transaction, |state| {
        state.abort_dispatched = true
    });
    let task = create_transaction_task(scope, "transaction-abort", transaction);
    enqueue_indexed_db_task(scope, task);
}
