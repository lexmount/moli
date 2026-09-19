use super::*;

pub(super) fn request_aborted_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let transaction = indexed_db_request_transaction_object(scope, request)?;
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
        .unwrap_or(false)
    {
        return None;
    }
    Some(dom_exception_value(
        scope,
        "The transaction was aborted.",
        "AbortError",
    ))
}

pub(super) fn finish_request_with_abort_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    set_indexed_db_slot_value(
        scope,
        request,
        INDEXED_DB_REQUEST_RESULT_SLOT,
        v8::undefined(scope).into(),
    );
    set_indexed_db_slot_value(scope, request, INDEXED_DB_REQUEST_ERROR_SLOT, error);
    let done = v8str(scope, "done").into();
    set_indexed_db_slot_value(scope, request, INDEXED_DB_REQUEST_READY_STATE_SLOT, done);
    let _ = dispatch_idb_named_event(scope, request, "error", |_, _| {});
    finish::finish_request_dispatch(scope, request);
}
