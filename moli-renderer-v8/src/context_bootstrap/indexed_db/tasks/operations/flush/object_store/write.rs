use super::*;

pub(super) fn try_dispatch_object_store_write_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    operation: &operation::QueuedTransactionOperation<'s>,
) -> bool {
    match &operation.kind {
        IndexedDbTransactionOperation::ObjectStoreWrite {
            value,
            key,
            add_only,
        } => {
            let value = v8::Local::new(scope, value);
            let key = v8::Local::new(scope, key);
            execute_object_store_write_request(
                scope,
                operation.source,
                operation.request,
                operation.handle,
                &operation.store_name,
                value,
                key,
                *add_only,
            );
        }
        IndexedDbTransactionOperation::ObjectStoreDelete { query } => {
            execute_object_store_delete_request(
                scope,
                operation.request,
                operation.handle,
                &operation.store_name,
                query,
            );
        }
        IndexedDbTransactionOperation::ObjectStoreClear => {
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
