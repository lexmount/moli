use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbRequestValue, indexed_db_database_is_closed, request_value, set_request_value,
};

pub(in crate::context_bootstrap::indexed_db::tasks::dispatch) fn enqueue_committed_upgrade_open<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    database: v8::Local<'s, v8::Object>,
) {
    set_request_value(
        scope,
        request,
        IndexedDbRequestValue::PendingResult,
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
    let Some(database) = request_value(scope, request, IndexedDbRequestValue::PendingResult)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    if indexed_db_database_is_closed(scope, database) {
        let error = dom_exception_value(scope, "The database open was aborted.", "AbortError");
        set_request_value(scope, request, IndexedDbRequestValue::PendingError, error);
        flush_request_error_task(scope, task);
        return;
    }
    flush_request_success_task(scope, task);
}
