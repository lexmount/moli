use super::*;
use crate::context_bootstrap::indexed_db::INDEXED_DB_DATABASE_CLOSED_SLOT;

pub(in crate::context_bootstrap::indexed_db::tasks::dispatch) fn enqueue_committed_upgrade_open<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    database: v8::Local<'s, v8::Object>,
) {
    set_indexed_db_request_surface_value(
        scope,
        request,
        INDEXED_DB_REQUEST_TRANSACTION_SLOT,
        "transaction",
        v8::null(scope).into(),
    );
    define_non_enumerable_value_property(
        scope,
        request,
        INDEXED_DB_PENDING_RESULT_SLOT,
        database.into(),
    );
    enqueue_request_task(scope, "open-success", request);
}

pub(in crate::context_bootstrap::indexed_db::tasks::dispatch) fn flush_open_success_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(request) = indexed_db_request_dispatch_task_request(scope, task) else {
        return;
    };
    let Some(database) = object_hidden_value(scope, request, INDEXED_DB_PENDING_RESULT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    if object_bool_property(scope, database, INDEXED_DB_DATABASE_CLOSED_SLOT).unwrap_or(false) {
        finish_aborted_upgrade_open(scope, request, database);
        return;
    }
    // The complete event's microtasks may close the upgrade connection. Keep
    // its queue position until that final success-versus-AbortError decision.
    finish_indexed_db_connection_request(scope, request);
    flush_request_success_task(scope, task);
}
