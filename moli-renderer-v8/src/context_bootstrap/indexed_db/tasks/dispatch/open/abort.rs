use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbRequestReadyState, IndexedDbRequestValue, set_indexed_db_request_ready_state,
    set_request_value,
};

pub(in crate::context_bootstrap::indexed_db::tasks::dispatch) fn finish_aborted_upgrade_open<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    database: v8::Local<'s, v8::Object>,
) {
    close_indexed_db_database_connection(scope, database);
    // The upgrade's abort event has finished. Reset the open request until its
    // own error task publishes the final result and done flag.
    set_request_value(
        scope,
        request,
        IndexedDbRequestValue::Error,
        v8::null(scope).into(),
    );

    set_indexed_db_request_ready_state(scope, request, IndexedDbRequestReadyState::Pending);
    let undefined = v8::undefined(scope).into();
    set_request_value(scope, request, IndexedDbRequestValue::Result, undefined);
    set_request_value(
        scope,
        request,
        IndexedDbRequestValue::Transaction,
        v8::null(scope).into(),
    );
    let error = dom_exception_value(scope, "The database open was aborted.", "AbortError");
    store_request_error(scope, request, error);
}
