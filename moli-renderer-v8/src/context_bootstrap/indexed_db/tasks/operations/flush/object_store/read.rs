use super::*;

pub(super) fn try_dispatch_object_store_read_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    operation: &operation::QueuedTransactionOperation<'s>,
) -> bool {
    match &operation.kind {
        IndexedDbTransactionOperation::ObjectStoreGet { query } => {
            execute_object_store_get_request(
                scope,
                operation.request,
                operation.handle,
                &operation.store_name,
                query,
            );
        }
        IndexedDbTransactionOperation::ObjectStoreGetAll {
            query,
            count,
            direction,
        } => {
            execute_object_store_get_all_request(
                scope,
                operation.request,
                operation.handle,
                &operation.store_name,
                query.as_ref(),
                *count,
                *direction,
            );
        }
        IndexedDbTransactionOperation::ObjectStoreGetKey { query } => {
            execute_object_store_get_key_request(
                scope,
                operation.request,
                operation.handle,
                &operation.store_name,
                query,
            );
        }
        IndexedDbTransactionOperation::ObjectStoreGetAllKeys {
            query,
            count,
            direction,
        } => {
            execute_object_store_get_all_keys_request(
                scope,
                operation.request,
                operation.handle,
                &operation.store_name,
                query.as_ref(),
                *count,
                *direction,
            );
        }
        IndexedDbTransactionOperation::ObjectStoreCount { query } => {
            execute_object_store_count_request(
                scope,
                operation.request,
                operation.handle,
                &operation.store_name,
                query.as_ref(),
            );
        }
        _ => return false,
    }
    true
}
