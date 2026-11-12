use super::*;
use crate::context_bootstrap::indexed_db::{
    indexed_db_transaction_status, update_indexed_db_transaction_status,
};

pub(in crate::context_bootstrap::indexed_db) fn enqueue_transaction_commit_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    if indexed_db_transaction_status(scope, transaction)
        .is_some_and(|state| state.commit_scheduled || state.finished)
    {
        return;
    }
    let _ = update_indexed_db_transaction_status(scope, transaction, |state| {
        state.commit_scheduled = true
    });
    let task = create_transaction_task(scope, "transaction-commit", transaction);
    enqueue_indexed_db_task(scope, task);
}
