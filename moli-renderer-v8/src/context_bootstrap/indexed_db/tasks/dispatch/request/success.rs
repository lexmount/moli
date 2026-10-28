use super::*;
use crate::context_bootstrap::indexed_db::schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint;
use crate::context_bootstrap::indexed_db::{
    dispatch_idb_named_event_with_result, finish_transaction_abort,
};

pub(in crate::context_bootstrap::indexed_db) fn flush_request_success_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(request) = indexed_db_request_dispatch_task_request(scope, task) else {
        return;
    };
    if let Some(error) = abort::request_aborted_error(scope, request) {
        abort::finish_request_with_abort_error(scope, request, error);
        return;
    }
    let transaction = indexed_db_request_transaction_object(scope, request);
    if let Some(transaction) = transaction {
        set_transaction_active_for_request_event(scope, transaction);
    }
    refresh_pending_cursor_surface(scope, request);
    if let Some(result) = object_hidden_value(scope, request, INDEXED_DB_PENDING_RESULT_SLOT) {
        set_indexed_db_request_surface_value(
            scope,
            request,
            INDEXED_DB_REQUEST_RESULT_SLOT,
            "result",
            result,
        );
    }
    let null = v8::null(scope).into();
    set_indexed_db_request_surface_value(
        scope,
        request,
        INDEXED_DB_REQUEST_ERROR_SLOT,
        "error",
        null,
    );
    let done = v8str(scope, "done").into();
    set_indexed_db_request_surface_value(
        scope,
        request,
        INDEXED_DB_REQUEST_READY_STATE_SLOT,
        "readyState",
        done,
    );
    let cancel =
        crate::context_bootstrap::indexed_db::typed_state::sql::indexed_db_request_sql_client(
            scope, request,
        )
        .map(|_| {
            crate::context_bootstrap::indexed_db::typed_state::sql::sql_transaction_event_cancellation(
                scope, request,
            )
        });
    let outcome = dispatch_idb_named_event_with_result(scope, request, "success", |_, _| {});
    if scope.is_execution_terminating()
        && let Some(cancel) = cancel
    {
        // Termination cleanup must not inspect or allocate JS values, including
        // transaction.error, after the event listener was interrupted.
        cancel();
        return;
    }
    if let Some(transaction) = transaction
        && outcome.did_listeners_throw
        && object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
            .unwrap_or(false)
    {
        let error = dom_exception_value(
            scope,
            "An IndexedDB success listener threw an exception.",
            "AbortError",
        );
        finish_transaction_abort(scope, transaction, error);
    }
    finish::finish_request_dispatch(scope, request);
    if let Some(transaction) = transaction {
        schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(scope, transaction);
    }
}

fn refresh_pending_cursor_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) {
    let Some(cursor) = object_hidden_value(scope, request, INDEXED_DB_PENDING_CURSOR_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let position = object_number_property(scope, request, INDEXED_DB_PENDING_CURSOR_POSITION_SLOT)
        .unwrap_or(-1.0);
    let position = (position >= 0.0).then_some(position as usize);
    crate::context_bootstrap::indexed_db::publish_indexed_db_sql_cursor_page(
        scope, request, cursor,
    );
    let _ = refresh_cursor_surface(scope, cursor, position);
}
