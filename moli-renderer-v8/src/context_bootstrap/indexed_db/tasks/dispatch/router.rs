use super::*;
use crate::context_bootstrap::indexed_db::typed_state::IndexedDbTaskId;

pub(in crate::context_bootstrap::indexed_db) fn flush_indexed_db_task_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let _ = flush_next_indexed_db_task(scope);
}

pub(crate) fn flush_next_indexed_db_task(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let Some(task) = pop_first_indexed_db_task(scope) else {
        return false;
    };
    flush_indexed_db_task(scope, task)
}

pub(crate) fn flush_indexed_db_task_by_id(
    scope: &mut v8::PinScope<'_, '_>,
    task_id: IndexedDbTaskId,
) -> bool {
    let Some(task) = take_indexed_db_task_by_id(scope, task_id) else {
        return false;
    };
    flush_indexed_db_task(scope, task)
}

pub(crate) fn discard_indexed_db_task_by_id(
    scope: &mut v8::PinScope<'_, '_>,
    task_id: IndexedDbTaskId,
) -> bool {
    let Some(task) = take_indexed_db_task_by_id(scope, task_id) else {
        return false;
    };
    unregister_indexed_db_task(scope, task);
    true
}

fn flush_indexed_db_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(kind) = indexed_db_typed_task_kind(scope, task) else {
        return false;
    };
    let owner = indexed_db_typed_task_owner_scope(scope, task)
        .expect("IDB task should have typed owner state");
    let owner_restore = owner.enter(scope);
    match kind {
        IndexedDbTaskKind::RequestSuccess => flush_request_success_task(scope, task),
        IndexedDbTaskKind::RequestError => flush_request_error_task(scope, task),
        IndexedDbTaskKind::Open => flush_open_task(scope, task),
        IndexedDbTaskKind::DrainConnectionRequests => {
            crate::context_bootstrap::indexed_db::flush_indexed_db_connection_requests(scope, None)
        }
        IndexedDbTaskKind::DatabasesSettle => flush_databases_settle_task(scope, task),
        IndexedDbTaskKind::TransactionStart => flush_transaction_start_task(scope, task),
        IndexedDbTaskKind::TransactionCommit => flush_transaction_commit_task(scope, task),
        IndexedDbTaskKind::TransactionAbort => flush_transaction_abort_task(scope, task),
    }
    unregister_indexed_db_task(scope, task);
    owner.defer_restore(scope, owner_restore);
    true
}
