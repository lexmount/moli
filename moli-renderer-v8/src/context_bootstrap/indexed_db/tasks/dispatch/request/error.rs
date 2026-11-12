use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbRequestReadyState, IndexedDbRequestValue, dispatch_idb_named_event_with_result,
    finish_transaction_abort, indexed_db_transaction_status, request_value,
    schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint,
    set_indexed_db_request_ready_state, set_request_value,
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
        && indexed_db_transaction_status(scope, transaction).is_some_and(|state| state.committing)
        && let Some(error) = request_value(scope, request, IndexedDbRequestValue::PendingError)
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
    if let Some(error) = request_value(scope, request, IndexedDbRequestValue::PendingError) {
        set_request_value(scope, request, IndexedDbRequestValue::Error, error);
    }
    set_request_value(
        scope,
        request,
        IndexedDbRequestValue::Result,
        v8::undefined(scope).into(),
    );

    set_indexed_db_request_ready_state(scope, request, IndexedDbRequestReadyState::Done);
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
        && indexed_db_transaction_status(scope, transaction)
            .is_some_and(|state| !state.finished && state.active)
        && (outcome.did_listeners_throw || outcome.uncanceled)
    {
        let error = if outcome.did_listeners_throw {
            dom_exception_value(
                scope,
                "An IndexedDB error listener threw an exception.",
                "AbortError",
            )
        } else {
            request_value(scope, request, IndexedDbRequestValue::Error)
                .expect("a failed request retains its native error")
        };
        finish_transaction_abort(scope, transaction, error);
    }
    finish::finish_request_dispatch(scope, request);
    if let Some(transaction) = transaction {
        schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(scope, transaction);
    }
}
