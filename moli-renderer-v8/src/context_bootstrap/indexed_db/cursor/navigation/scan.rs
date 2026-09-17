use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbCursorLifecycleState, IndexedDbCursorSnapshot, IndexedDbError, TransactionHandle,
};
use std::{cmp::Ordering, rc::Rc};

pub(super) fn select_cursor_result(
    scope: &mut v8::PinScope<'_, '_>,
    handle: TransactionHandle,
    store_name: &str,
    state: &IndexedDbCursorLifecycleState,
    iteration: CursorIteration,
) -> Result<(Rc<IndexedDbCursorSnapshot>, Option<usize>), IndexedDbError> {
    let revision =
        with_indexed_db_manager(scope, |manager| manager.transaction_record_revision(handle))?;
    let current = state
        .position
        .expect("validated cursor has a current record");
    let (snapshot, mut next) = if state.snapshot.record_revision == revision {
        // No writes have changed the transaction: ordinary iteration stays O(1).
        (Rc::clone(&state.snapshot), current + 1)
    } else {
        let snapshot = Rc::new(capture_cursor_snapshot(
            scope,
            handle,
            store_name,
            &state.operation,
        )?);
        let previous = &state.snapshot.entries[current];
        // A record may have disappeared or moved in an index. Seek from its
        // native key tuple, never from its old index in the cached scan.
        let next = snapshot.entries.partition_point(|entry| {
            let order = if state.direction.is_unique() {
                compare::cursor_direction_cmp(state.direction, &entry.key, &previous.key)
            } else {
                compare::cursor_tuple_cmp(
                    state.direction,
                    &entry.key,
                    &entry.primary_key,
                    &previous.key,
                    &previous.primary_key,
                )
            };
            order != Ordering::Greater
        });
        (snapshot, next)
    };
    match iteration {
        CursorIteration::Advance(count) => {
            next = next.saturating_add(count as usize - 1);
        }
        CursorIteration::Continue(None) => {}
        CursorIteration::Continue(Some(key)) => {
            next = next.max(snapshot.entries.partition_point(|entry| {
                compare::cursor_direction_cmp(state.direction, &entry.key, &key) == Ordering::Less
            }));
        }
        CursorIteration::ContinuePrimaryKey(key, primary_key) => {
            next = next.max(snapshot.entries.partition_point(|entry| {
                compare::cursor_tuple_cmp(
                    state.direction,
                    &entry.key,
                    &entry.primary_key,
                    &key,
                    &primary_key,
                ) == Ordering::Less
            }));
        }
    }
    let position = (next < snapshot.entries.len()).then_some(next);
    Ok((snapshot, position))
}
