use super::*;

pub(super) fn try_dispatch_cursor_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    operation: &operation::QueuedTransactionOperation<'s>,
) -> bool {
    let IndexedDbTransactionOperation::OpenCursor(cursor) = &operation.kind else {
        return false;
    };
    execute_cursor_open_operation(
        scope,
        operation.source,
        operation.request,
        operation.handle,
        &operation.store_name,
        cursor,
    );
    true
}
