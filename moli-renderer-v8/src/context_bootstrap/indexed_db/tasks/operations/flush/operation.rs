use super::*;

pub(super) struct QueuedTransactionOperation<'s> {
    pub(super) source: v8::Local<'s, v8::Object>,
    pub(super) request: v8::Local<'s, v8::Object>,
    pub(super) handle: TransactionHandle,
    pub(super) store_name: String,
    pub(super) kind: IndexedDbTransactionOperation,
}

impl<'s> QueuedTransactionOperation<'s> {
    fn from_pending(
        scope: &mut v8::PinScope<'s, '_>,
        transaction: v8::Local<'s, v8::Object>,
        pending: IndexedDbPendingTransactionOperation,
    ) -> Self {
        let operation = pending.into_locals(scope);
        Self {
            source: operation.source,
            request: operation.request,
            handle: transaction_handle_from_value(scope, transaction.into())
                .expect("started IDB transactions should retain their backend handle"),
            store_name: operation.store_name,
            kind: operation.kind,
        }
    }
}

pub(super) fn flush_queued_transaction_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    pending: IndexedDbPendingTransactionOperation,
) {
    let owner = pending.owner();
    let owner_restore = owner.enter(scope);
    let operation = QueuedTransactionOperation::from_pending(scope, transaction, pending);
    let dispatched = cursor::try_dispatch_cursor_operation(scope, &operation)
        || object_store::try_dispatch_object_store_operation(scope, &operation)
        || index::try_dispatch_index_operation(scope, &operation);
    assert!(dispatched, "typed IDB operation should have a dispatcher");
    owner.defer_restore(scope, owner_restore);
}
