use super::*;

pub(super) fn request_aborted_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let transaction = indexed_db_request_transaction_object(scope, request)?;
    crate::context_bootstrap::indexed_db::finish_canceled_transaction(scope, transaction);
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
        .unwrap_or(false)
    {
        return None;
    }
    Some(dom_exception_value(
        scope,
        "The transaction was aborted before this request completed.",
        "AbortError",
    ))
}

pub(super) fn finish_request_with_abort_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    set_indexed_db_request_surface_value(
        scope,
        request,
        INDEXED_DB_REQUEST_ERROR_SLOT,
        "error",
        error,
    );
    let done = v8str(scope, "done").into();
    set_indexed_db_request_surface_value(
        scope,
        request,
        INDEXED_DB_REQUEST_READY_STATE_SLOT,
        "readyState",
        done,
    );
    let _ = dispatch_idb_named_event(scope, request, "error", |_, _| {});
    finish::finish_request_dispatch(scope, request);
}
