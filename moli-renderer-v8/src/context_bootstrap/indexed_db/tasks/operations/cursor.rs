use super::*;

pub(in crate::context_bootstrap::indexed_db) fn submit_cursor_open_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    store_name: &moli_indexeddb::IndexedDbName,
    operation: IndexedDbCursorOpenOperation,
) {
    schedule_transaction_operation(
        scope,
        transaction,
        source,
        request,
        store_name,
        IndexedDbTransactionOperation::OpenCursor(operation),
    );
}
