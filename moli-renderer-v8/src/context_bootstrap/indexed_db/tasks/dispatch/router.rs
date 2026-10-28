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
    crate::context_bootstrap::indexed_db::flush_indexed_db_transaction_start_wake(scope);
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
        IndexedDbTaskKind::OpenSuccess => flush_open_success_task(scope, task),
        IndexedDbTaskKind::OpenBlocked => flush_open_blocked_task(scope, task),
        IndexedDbTaskKind::DeleteBlocked => flush_delete_blocked_task(scope, task),
        IndexedDbTaskKind::VersionChange => flush_version_change_task(scope, task),
        IndexedDbTaskKind::BlockedRecheck => flush_blocked_recheck_task(scope, task),
        IndexedDbTaskKind::DrainBlockedOpens => flush_drain_blocked_open_requests_task(scope),
        IndexedDbTaskKind::DatabasesSettle => flush_databases_settle_task(scope, task),
        IndexedDbTaskKind::TransactionStart => flush_transaction_start_task(scope, task),
        IndexedDbTaskKind::TransactionCommit => flush_transaction_commit_task(scope, task),
        IndexedDbTaskKind::TransactionAbort => flush_transaction_abort_task(scope, task),
        IndexedDbTaskKind::TransactionOperationError => {
            flush_transaction_operation_error_task(scope, task)
        }
        IndexedDbTaskKind::TransactionCancel => {
            if let Some(transaction) = indexed_db_object_task_target(scope, task) {
                crate::context_bootstrap::indexed_db::finish_canceled_transaction(
                    scope,
                    transaction,
                );
            }
        }
        IndexedDbTaskKind::DatabaseForcedClose => {
            if let Some(database) = indexed_db_object_task_target(scope, task) {
                if let Some(handle) =
                    crate::context_bootstrap::indexed_db::database_handle_from_value(
                        scope,
                        database.into(),
                    )
                    && !crate::context_bootstrap::indexed_db::unregister_open_database_connection(
                        scope, handle, database,
                    )
                {
                    enqueue_drain_blocked_open_requests_task(scope);
                }
                let _ = dispatch_idb_named_event(scope, database, "close", |_, _| {});
            }
        }
    }
    if !indexed_db_runtime_array_contains_object(
        scope,
        IndexedDbRuntimeArray::BlockedOpenQueue,
        task,
    ) {
        unregister_indexed_db_task(scope, task);
    }
    owner.defer_restore(scope, owner_restore);
    true
}
