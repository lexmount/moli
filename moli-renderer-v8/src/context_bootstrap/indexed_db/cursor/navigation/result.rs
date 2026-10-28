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

pub(in crate::context_bootstrap::indexed_db) fn execute_cursor_iteration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    movement: &IndexedDbCursorIteration,
) {
    let next = legacy_next_position(scope, cursor, movement);
    let _ = store_cursor_iteration_result(scope, cursor, next);
}

fn legacy_next_position(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
    movement: &IndexedDbCursorIteration,
) -> Option<usize> {
    let state = indexed_db_cursor_state(scope, cursor)?;
    let current = state.position?;
    if let IndexedDbCursorIteration::Advance(count) = movement {
        return current
            .checked_add(*count as usize)
            .filter(|position| *position < state.entries.len());
    }
    state
        .entries
        .iter()
        .enumerate()
        .skip(current + 1)
        .find_map(|(position, entry)| {
            let cmp = match movement {
                IndexedDbCursorIteration::Continue(None) => std::cmp::Ordering::Greater,
                IndexedDbCursorIteration::Continue(Some(key)) => {
                    compare::cursor_direction_cmp(state.direction, &entry.key, key)
                }
                IndexedDbCursorIteration::PrimaryKey { key, primary_key } => {
                    compare::cursor_tuple_cmp(
                        state.direction,
                        &entry.key,
                        &entry.primary_key,
                        key,
                        primary_key,
                    )
                }
                IndexedDbCursorIteration::Advance(_) => unreachable!(),
            };
            (cmp != std::cmp::Ordering::Less).then_some(position)
        })
}
