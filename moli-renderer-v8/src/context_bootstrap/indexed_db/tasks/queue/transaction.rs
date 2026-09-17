use super::*;

mod abort;
mod commit;
mod start;
mod successor;

pub(in crate::context_bootstrap::indexed_db) use self::abort::enqueue_transaction_abort_task;
pub(in crate::context_bootstrap::indexed_db) use self::commit::enqueue_transaction_commit_task;
pub(in crate::context_bootstrap::indexed_db) use self::successor::enqueue_ready_transaction_starts;

pub(in crate::context_bootstrap::indexed_db) fn enqueue_transaction_operation_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    error: IndexedDbError,
) {
    let task = v8::Object::new(scope);
    crate::context_bootstrap::indexed_db::register_indexed_db_transaction_error_task(
        scope,
        task,
        transaction,
        error,
    );
    enqueue_indexed_db_task(scope, task);
}

fn create_transaction_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: &'static str,
    transaction: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let typed_kind = match kind {
        "transaction-start" => IndexedDbTaskKind::TransactionStart,
        "transaction-commit" => IndexedDbTaskKind::TransactionCommit,
        "transaction-abort" => IndexedDbTaskKind::TransactionAbort,
        _ => unreachable!("unknown IndexedDB transaction task kind: {kind}"),
    };
    let task = v8::Object::new(scope);
    register_indexed_db_transaction_task(scope, task, typed_kind, transaction);
    task
}
