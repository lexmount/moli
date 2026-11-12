use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbCursorIteration, IndexedDbRequestValue, set_indexed_db_request_cursor_position,
    set_request_value,
};

pub(super) fn enqueue_cursor_iteration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    movement: IndexedDbCursorIteration,
) -> Option<()> {
    let (request, transaction) = cursor_request_and_transaction(scope, cursor)?;
    cursor_active_transaction(scope, cursor)?;
    cursor_effective_object_store(scope, cursor)?;
    begin_indexed_db_cursor_iteration(scope, cursor)?;
    queue_transaction_request(scope, transaction, request);
    prepare_cursor_request(scope, request);
    let kind = IndexedDbTransactionOperation::CursorIterate {
        cursor: v8::Global::new(scope, cursor),
        movement,
    };
    schedule_transaction_operation(scope, transaction, cursor, request, kind);
    Some(())
}

pub(in crate::context_bootstrap::indexed_db) fn store_cursor_iteration_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    next_position: Option<usize>,
) -> Option<()> {
    let request = cursor_request(scope, cursor)?;
    set_request_value(
        scope,
        request,
        IndexedDbRequestValue::PendingCursor,
        cursor.into(),
    );
    set_indexed_db_request_cursor_position(scope, request, next_position);
    let result = if next_position.is_some() {
        cursor.into()
    } else {
        v8::null(scope).into()
    };
    store_request_success(scope, request, result);
    Some(())
}
