use super::*;
use crate::context_bootstrap::indexed_db::abort_indexed_db_transaction_after_dispatch;
use crate::context_bootstrap::indexed_db::abort_indexed_db_transaction_with_error;
use crate::context_bootstrap::indexed_db::schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint;

pub(in crate::context_bootstrap::indexed_db) fn flush_request_error_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(request) = indexed_db_request_dispatch_task_request(scope, task) else {
        return;
    };
    let transaction = indexed_db_request_transaction_object(scope, request);
    if let Some(transaction) = transaction
        && object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_COMMITTING_SLOT)
            .unwrap_or(false)
        && let Some(error) = object_hidden_value(scope, request, INDEXED_DB_PENDING_ERROR_SLOT)
    {
        // Failed operations abort an explicit commit before error delivery;
        // error handlers cannot cancel that backend failure. This request and
        // the other pending requests then receive the abort notifications.
        abort_indexed_db_transaction_with_error(scope, transaction, error);
    }
    if let Some(error) = abort::request_aborted_error(scope, request) {
        abort::finish_request_with_abort_error(scope, request, error);
        return;
    }
    if let Some(transaction) = transaction {
        set_transaction_active_for_request_event(scope, transaction);
    }
    set_indexed_db_slot_value(
        scope,
        request,
        INDEXED_DB_REQUEST_RESULT_SLOT,
        v8::undefined(scope).into(),
    );
    if let Some(error) = object_hidden_value(scope, request, INDEXED_DB_PENDING_ERROR_SLOT) {
        set_indexed_db_slot_value(scope, request, INDEXED_DB_REQUEST_ERROR_SLOT, error);
    }
    let done = v8str(scope, "done").into();
    set_indexed_db_slot_value(scope, request, INDEXED_DB_REQUEST_READY_STATE_SLOT, done);
    let result = dispatch_idb_named_event(scope, request, "error", |_, _| {});
    if let Some(transaction) = transaction {
        let error = if result.did_throw {
            Some(dom_exception_value(
                scope,
                "An IndexedDB event listener threw.",
                "AbortError",
            ))
        } else if result.uncanceled {
            object_hidden_value(scope, request, INDEXED_DB_REQUEST_ERROR_SLOT)
        } else {
            None
        };
        if let Some(error) = error {
            abort_indexed_db_transaction_after_dispatch(scope, transaction, error);
        }
    }
    finish::finish_request_dispatch(scope, request);
    if let Some(transaction) = transaction {
        schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(scope, transaction);
    }
}
