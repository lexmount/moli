use super::*;
use crate::context_bootstrap::indexed_db::typed_state::IndexedDbTaskId;
use crate::context_bootstrap::indexed_db::typed_state::sql::flush_indexed_db_sql_task;
use crate::util::context_host_ptr_from_global_bridge;

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
    let Some(task) = take_indexed_db_task_by_id(scope, task_id, true) else {
        return false;
    };
    flush_indexed_db_task(scope, task)
}

pub(crate) fn discard_indexed_db_task_by_id(
    scope: &mut v8::PinScope<'_, '_>,
    task_id: IndexedDbTaskId,
) -> bool {
    let Some(task) = take_indexed_db_task_by_id(scope, task_id, false) else {
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
    let retire =
        crate::context_bootstrap::indexed_db::capture_indexed_db_task_retirement(scope, task);
    let owner_restore = owner.enter(scope);
    match kind {
        IndexedDbTaskKind::SqlCompletion => flush_indexed_db_sql_task(scope, task),
        IndexedDbTaskKind::FactoryRequest => {
            crate::context_bootstrap::indexed_db::typed_state::factory::flush_factory_task(
                scope, task,
            )
        }
        IndexedDbTaskKind::RequestSuccess => flush_request_success_task(scope, task),
        IndexedDbTaskKind::RequestError => flush_request_error_task(scope, task),
        IndexedDbTaskKind::Open => flush_open_task(scope, task),
        IndexedDbTaskKind::OpenSuccess => flush_open_success_task(scope, task),
        IndexedDbTaskKind::VersionChange => flush_version_change_task(scope, task),
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
                    && let Some(host) = context_host_ptr_from_global_bridge(scope)
                {
                    unsafe { &*host }.unregister_indexed_db_open_connection(handle);
                }
                let _ = dispatch_idb_named_event(scope, database, "close", |_, _| {});
            }
        }
    }
    if scope.is_execution_terminating() {
        // The task may have lost V8 execution while invoking script. Cancel its
        // native state without another private-slot read or V8 allocation.
        retire();
        owner.defer_restore(scope, owner_restore);
        return true;
    }
    let factory_pending = kind == IndexedDbTaskKind::FactoryRequest
        && crate::context_bootstrap::indexed_db::typed_state::factory::retain_pending_factory_task(
            scope, task,
        );
    if !factory_pending {
        unregister_indexed_db_task(scope, task);
    }
    owner.defer_restore(scope, owner_restore);
    true
}
