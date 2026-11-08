use super::*;
use crate::context_bootstrap::indexed_db::schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint;
use crate::context_bootstrap::indexed_db::{
    dispatch_idb_named_event_with_result, finish_transaction_abort,
};

pub(in crate::context_bootstrap::indexed_db) fn flush_request_error_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(request) = indexed_db_request_dispatch_task_request(scope, task) else {
        return;
    };
    if let Some(lease) =
        crate::context_bootstrap::indexed_db::take_indexed_db_connection_request(scope, request)
    {
        crate::context_bootstrap::microtask_checkpoint::defer_indexed_db_connection_request(
            scope, lease,
        );
    }
    let transaction = indexed_db_request_transaction_object(scope, request);
    if let Some(transaction) = transaction
        && object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_COMMITTING_SLOT)
            .unwrap_or(false)
        && let Some(error) = object_hidden_value(scope, request, INDEXED_DB_PENDING_ERROR_SLOT)
    {
        // A failed operation cannot be rescued by canceling its error event
        // after the transaction has already entered explicit commit.
        crate::context_bootstrap::indexed_db::abort_indexed_db_transaction_with_error(
            scope,
            transaction,
            error,
        );
    }
    if let Some(error) = abort::request_aborted_error(scope, request) {
        abort::finish_request_with_abort_error(scope, request, error);
        return;
    }

    if let Some(transaction) = transaction {
        set_transaction_active_for_request_event(scope, transaction);
    }
    if let Some(error) = object_hidden_value(scope, request, INDEXED_DB_PENDING_ERROR_SLOT) {
        set_indexed_db_slot_value(scope, request, INDEXED_DB_REQUEST_ERROR_SLOT, error);
    }
    set_indexed_db_slot_value(
        scope,
        request,
        INDEXED_DB_REQUEST_RESULT_SLOT,
        v8::undefined(scope).into(),
    );
    let done = v8str(scope, "done").into();
    set_indexed_db_slot_value(scope, request, INDEXED_DB_REQUEST_READY_STATE_SLOT, done);
    let cancel =
        crate::context_bootstrap::indexed_db::typed_state::sql::indexed_db_request_sql_client(
            scope, request,
        )
        .map(|_| {
            crate::context_bootstrap::indexed_db::typed_state::sql::sql_transaction_event_cancellation(
                scope, request,
            )
        });
    let outcome = dispatch_idb_named_event_with_result(scope, request, "error", |_, _| {});
    if scope.is_execution_terminating() {
        if let Some(cancel) = cancel {
            cancel();
        }
        return;
    }
    if let Some(transaction) = transaction
        && !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
            .unwrap_or(false)
        && object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
            .unwrap_or(false)
        && (outcome.did_listeners_throw || outcome.uncanceled)
    {
        let error = if outcome.did_listeners_throw {
            dom_exception_value(
                scope,
                "An IndexedDB error listener threw an exception.",
                "AbortError",
            )
        } else {
            object_hidden_value(scope, request, INDEXED_DB_REQUEST_ERROR_SLOT)
                .expect("a failed request retains its native error")
        };
        finish_transaction_abort(scope, transaction, error);
    }
    finish::finish_request_dispatch(scope, request);
    if let Some(transaction) = transaction {
        schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(scope, transaction);
    }
}
