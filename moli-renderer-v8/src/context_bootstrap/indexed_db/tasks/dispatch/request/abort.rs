use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbRequestReadyState, IndexedDbRequestValue, indexed_db_transaction_status,
    set_indexed_db_request_ready_state, set_request_value,
};

pub(super) fn request_aborted_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let transaction = indexed_db_request_transaction_object(scope, request)?;
    crate::context_bootstrap::indexed_db::finish_canceled_transaction(scope, transaction);
    if !indexed_db_transaction_status(scope, transaction).is_some_and(|state| state.aborted) {
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
    set_request_value(scope, request, IndexedDbRequestValue::Error, error);
    set_request_value(
        scope,
        request,
        IndexedDbRequestValue::Result,
        v8::undefined(scope).into(),
    );

    set_indexed_db_request_ready_state(scope, request, IndexedDbRequestReadyState::Done);
    let _ = dispatch_idb_named_event(scope, request, "error", |_, _| {});
    finish::finish_request_dispatch(scope, request);
}
