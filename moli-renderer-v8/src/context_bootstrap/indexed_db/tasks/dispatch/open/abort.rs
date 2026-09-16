use super::*;

pub(in crate::context_bootstrap::indexed_db::tasks::dispatch) fn finish_aborted_upgrade_open<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    database: v8::Local<'s, v8::Object>,
) {
    close_indexed_db_database_connection(scope, database);
    // The upgrade's abort event has finished. Reset the open request until its
    // own error task publishes the final result and done flag.
    set_indexed_db_slot_value(
        scope,
        request,
        INDEXED_DB_REQUEST_ERROR_SLOT,
        v8::null(scope).into(),
    );
    let pending = v8str(scope, "pending").into();
    set_indexed_db_slot_value(scope, request, INDEXED_DB_REQUEST_READY_STATE_SLOT, pending);
    let undefined = v8::undefined(scope).into();
    set_indexed_db_slot_value(scope, request, INDEXED_DB_REQUEST_RESULT_SLOT, undefined);
    set_indexed_db_slot_value(
        scope,
        request,
        INDEXED_DB_REQUEST_TRANSACTION_SLOT,
        v8::null(scope).into(),
    );
    let error = dom_exception_value(scope, "The database open was aborted.", "AbortError");
    store_request_error(scope, request, error);
}
