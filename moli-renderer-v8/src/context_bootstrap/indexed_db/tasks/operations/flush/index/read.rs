use super::*;

pub(super) fn try_dispatch_index_read_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    operation: &operation::QueuedTransactionOperation<'s>,
) -> bool {
    match &operation.kind {
        IndexedDbTransactionOperation::IndexGet { query } => {
            execute_index_get_request(
                scope,
                operation.source,
                operation.request,
                operation.handle,
                &operation.store_name,
                query,
            );
        }
        IndexedDbTransactionOperation::IndexGetKey { query } => {
            execute_index_get_key_request(
                scope,
                operation.source,
                operation.request,
                operation.handle,
                &operation.store_name,
                query,
            );
        }
        IndexedDbTransactionOperation::IndexGetAll(collection) => {
            execute_index_get_all_request(
                scope,
                operation.source,
                operation.request,
                operation.handle,
                &operation.store_name,
                collection,
            );
        }

        IndexedDbTransactionOperation::IndexCount { query } => {
            execute_index_count_request(
                scope,
                operation.source,
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
