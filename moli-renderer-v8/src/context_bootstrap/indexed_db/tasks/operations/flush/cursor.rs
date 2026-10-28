use super::*;

pub(super) fn try_dispatch_cursor_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    operation: &operation::TransactionOperation<'s>,
) -> bool {
    match &operation.kind {
        IndexedDbTransactionOperation::OpenCursor(cursor) => execute_cursor_open_operation(
            scope,
            operation.source,
            operation.request,
            operation.handle,
            &operation.store_name,
            cursor,
        ),
        IndexedDbTransactionOperation::CursorDelete { key } => {
            match with_indexed_db_manager(scope, |manager| {
                manager.delete(operation.handle, &operation.store_name, key)
            }) {
                Ok(()) => {
                    store_request_success(scope, operation.request, v8::undefined(scope).into())
                }
                Err(error) => {
                    let error = request_error_object(scope, &error);
                    store_request_error(scope, operation.request, error);
                }
            }
        }
        IndexedDbTransactionOperation::CursorIterate { cursor, movement } => {
            let cursor = v8::Local::new(scope, cursor);
            crate::context_bootstrap::indexed_db::execute_cursor_iteration(scope, cursor, movement);
        }
        _ => return false,
    }
    true
}
