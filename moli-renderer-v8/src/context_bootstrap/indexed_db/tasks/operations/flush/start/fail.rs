use super::*;
use moli_indexeddb::IndexedDbError;

pub(super) fn fail_regular_transaction_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    error: &IndexedDbError,
) {
    let error = request_error_object(scope, error);
    crate::context_bootstrap::indexed_db::finish_transaction_abort(scope, transaction, error);
}
