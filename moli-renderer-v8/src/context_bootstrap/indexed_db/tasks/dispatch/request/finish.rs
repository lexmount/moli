use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbRequestReadyState, indexed_db_request_ready_state,
    set_indexed_db_request_error_pending, update_indexed_db_transaction_status,
};

fn release_request_dispatch_refs<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) {
    if indexed_db_request_ready_state(scope, request) == Some(IndexedDbRequestReadyState::Pending) {
        return;
    }
    release_indexed_db_request_dispatch_refs(scope, request);
}

pub(super) fn finish_request_dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) {
    set_indexed_db_request_error_pending(scope, request, false);
    request_finished(scope, request);
    release_request_dispatch_refs(scope, request);
}

fn request_finished<'s>(scope: &mut v8::PinScope<'s, '_>, request: v8::Local<'s, v8::Object>) {
    let Some(transaction) = indexed_db_request_transaction_object(scope, request) else {
        return;
    };
    let should_commit = update_indexed_db_transaction_status(scope, transaction, |state| {
        state.pending = state.pending.saturating_sub(1);
        state.pending == 0 && !state.finished
    })
    .unwrap_or(false);
    if should_commit {
        enqueue_transaction_commit_task(scope, transaction);
    }
}
