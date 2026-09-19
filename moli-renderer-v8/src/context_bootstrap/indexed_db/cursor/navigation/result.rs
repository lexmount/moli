use super::*;

pub(super) fn enqueue_cursor_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    iteration: CursorIteration,
) -> Option<()> {
    let (request, transaction) = cursor_request_and_transaction(scope, cursor)?;
    cursor_active_transaction(scope, cursor)?;
    let state = indexed_db_cursor_state(scope, cursor)?;
    let handle = transaction_handle_from_value(scope, transaction.into())?;
    let source = cursor_source(scope, cursor)?;
    let store = indexed_db_index_object_store(scope, source).unwrap_or(source);
    let store_name = indexed_db_object_store_name(scope, store)?;
    begin_indexed_db_cursor_iteration(scope, cursor)?;
    queue_transaction_request(scope, transaction, request);
    prepare_cursor_request(scope, request);
    let (snapshot, next_position) =
        match scan::select_cursor_result(scope, handle, &store_name, &state, iteration) {
            Ok(result) => result,
            Err(error) => {
                let error = request_error_object(scope, &error);
                store_request_error(scope, request, error);
                return Some(());
            }
        };
    set_indexed_db_cursor_pending_snapshot(scope, cursor, snapshot)?;
    set_indexed_db_slot_value(
        scope,
        request,
        INDEXED_DB_PENDING_CURSOR_SLOT,
        cursor.into(),
    );
    let position = next_position
        .map(|position| position as f64)
        .unwrap_or(-1.0);
    set_indexed_db_slot_value(
        scope,
        request,
        INDEXED_DB_PENDING_CURSOR_POSITION_SLOT,
        v8::Number::new(scope, position).into(),
    );
    if next_position.is_some() {
        store_request_success(scope, request, cursor.into());
    } else {
        store_request_success(scope, request, v8::null(scope).into());
    }
    Some(())
}
