use super::*;
use crate::context_bootstrap::indexed_db::IndexedDbCursorIteration;

pub(super) fn enqueue_cursor_iteration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    movement: IndexedDbCursorIteration,
) -> Option<()> {
    let (request, transaction) = cursor_request_and_transaction(scope, cursor)?;
    cursor_active_transaction(scope, cursor)?;
    let store = cursor_effective_object_store(scope, cursor)?;
    let name = indexed_db_object_store_name(scope, store)?;
    begin_indexed_db_cursor_iteration(scope, cursor)?;
    queue_transaction_request(scope, transaction, request);
    prepare_cursor_request(scope, request);
    let kind = IndexedDbTransactionOperation::CursorIterate {
        cursor: v8::Global::new(scope, cursor),
        movement,
    };
    schedule_transaction_operation(scope, transaction, cursor, request, &name, kind);
    Some(())
}

pub(in crate::context_bootstrap::indexed_db) fn store_cursor_iteration_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    next_position: Option<usize>,
) -> Option<()> {
    let request = cursor_request(scope, cursor)?;
    set_indexed_db_slot_value(
        scope,
        request,
        INDEXED_DB_PENDING_CURSOR_SLOT,
        cursor.into(),
    );
    let position = next_position.map_or(-1.0, |position| position as f64);
    set_indexed_db_slot_value(
        scope,
        request,
        INDEXED_DB_PENDING_CURSOR_POSITION_SLOT,
        v8::Number::new(scope, position).into(),
    );
    let result = if next_position.is_some() {
        cursor.into()
    } else {
        v8::null(scope).into()
    };
    store_request_success(scope, request, result);
    Some(())
}
