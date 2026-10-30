use super::*;
use crate::context_bootstrap::indexed_db::{
    complete_indexed_db_transaction_operation, typed_state::take_indexed_db_transaction_task_error,
};

mod abort;
mod commit;

pub(in crate::context_bootstrap::indexed_db) use self::abort::flush_transaction_abort_task;
pub(in crate::context_bootstrap::indexed_db) use self::commit::flush_transaction_commit_task;

pub(in crate::context_bootstrap::indexed_db) fn flush_transaction_operation_error_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(transaction) = indexed_db_object_task_target(scope, task) else {
        return;
    };
    let Some(error) = take_indexed_db_transaction_task_error(scope, task) else {
        return;
    };
    if !crate::context_bootstrap::indexed_db::finish_canceled_transaction(scope, transaction) {
        let error = request_error_object(scope, &error);
        crate::context_bootstrap::indexed_db::abort_indexed_db_transaction_with_error(
            scope,
            transaction,
            error,
        );
    }
    complete_indexed_db_transaction_operation(scope, transaction);
}
