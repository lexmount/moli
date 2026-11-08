use super::*;
use crate::context_bootstrap::indexed_db::set_indexed_db_request_error_pending;

pub(in crate::context_bootstrap::indexed_db) fn store_request_success<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    result: v8::Local<'s, v8::Value>,
) {
    define_non_enumerable_value_property(scope, request, INDEXED_DB_PENDING_RESULT_SLOT, result);
    enqueue_request_task(scope, "request-success", request);
}

pub(in crate::context_bootstrap::indexed_db) fn store_request_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    mut error: v8::Local<'s, v8::Value>,
) {
    if let Some(transaction) = indexed_db_request_transaction_object(scope, request)
        && crate::context_bootstrap::indexed_db::indexed_db_transaction_is_committing(
            scope,
            transaction,
        )
        && !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
            .unwrap_or(false)
    {
        // A failed operation while committing aborts immediately, before its
        // event can be canceled. All still-pending requests receive AbortError;
        // the transaction retains the original failure as its error.
        crate::context_bootstrap::indexed_db::finish_transaction_abort(scope, transaction, error);
        error = dom_exception_value(scope, "The transaction was aborted.", "AbortError");
    }
    set_indexed_db_request_error_pending(scope, request, true);
    define_non_enumerable_value_property(scope, request, INDEXED_DB_PENDING_ERROR_SLOT, error);
    enqueue_request_task(scope, "request-error", request);
}

pub(in crate::context_bootstrap::indexed_db) fn enqueue_request_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: &str,
    request: v8::Local<'s, v8::Object>,
) {
    let Some(relevant_context) = request.get_creation_context(scope) else {
        return;
    };
    if relevant_context != scope.get_current_context() {
        let request = v8::Global::new(scope, request);
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let request = v8::Local::new(target_scope, &request);
        enqueue_request_task_in_current_context(target_scope, kind, request);
        return;
    }
    enqueue_request_task_in_current_context(scope, kind, request);
}

fn enqueue_request_task_in_current_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: &str,
    request: v8::Local<'s, v8::Object>,
) {
    let typed_kind = match kind {
        "request-success" => IndexedDbTaskKind::RequestSuccess,
        "request-error" => IndexedDbTaskKind::RequestError,
        "open-success" => IndexedDbTaskKind::OpenSuccess,
        _ => return,
    };
    let task = v8::Object::new(scope);
    register_indexed_db_request_dispatch_task(scope, task, typed_kind, request);
    enqueue_indexed_db_task(scope, task);
}
