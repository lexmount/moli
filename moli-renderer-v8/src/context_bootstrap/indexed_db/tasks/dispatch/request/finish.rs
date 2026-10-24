use super::*;

pub(in crate::context_bootstrap::indexed_db) fn release_request_dispatch_refs<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) {
    if object_string_property(scope, request, INDEXED_DB_REQUEST_READY_STATE_SLOT).as_deref()
        == Some("pending")
    {
        return;
    }
    release_indexed_db_request_dispatch_refs(scope, request);
}

pub(super) fn finish_request_dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) {
    request_finished(scope, request);
    release_request_dispatch_refs(scope, request);
}

fn request_finished<'s>(scope: &mut v8::PinScope<'s, '_>, request: v8::Local<'s, v8::Object>) {
    let Some(transaction) = indexed_db_request_transaction_object(scope, request) else {
        return;
    };
    let pending = object_number_property(scope, transaction, INDEXED_DB_TRANSACTION_PENDING_SLOT)
        .unwrap_or(0.0);
    let next_pending = (pending - 1.0).max(0.0);
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_PENDING_SLOT,
        v8::Number::new(scope, next_pending).into(),
    );
    if next_pending == 0.0
        && object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
            .unwrap_or(false)
        && !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
            .unwrap_or(false)
    {
        enqueue_transaction_commit_task(scope, transaction);
    }
}
