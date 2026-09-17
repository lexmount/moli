use super::*;

pub(super) fn try_dispatch_object_store_write_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    operation: &operation::QueuedTransactionOperation<'s>,
) -> bool {
    match &operation.kind {
        IndexedDbTransactionOperationKindLocals::ObjectStoreWrite { prepared, add_only } => {
            execute_object_store_write_request(
                scope,
                operation.source,
                operation.request,
                operation.handle,
                &operation.store_name,
                prepared,
                *add_only,
            );
        }
        IndexedDbTransactionOperationKindLocals::ObjectStoreDelete { key } => {
            execute_object_store_delete_request(
                scope,
                operation.request,
                operation.handle,
                &operation.store_name,
                *key,
            );
        }
        IndexedDbTransactionOperationKindLocals::ObjectStoreClear => {
            execute_object_store_clear_request(
                scope,
                operation.request,
                operation.handle,
                &operation.store_name,
            );
        }
        _ => return false,
    }
    true
}
