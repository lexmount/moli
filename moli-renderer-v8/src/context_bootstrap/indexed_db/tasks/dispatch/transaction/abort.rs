use super::*;

pub(in crate::context_bootstrap::indexed_db) fn flush_transaction_abort_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(transaction) = indexed_db_transaction_task_transaction(scope, task) else {
        return;
    };
    dispatch_transaction_terminal(scope, transaction, "abort", false);
}
