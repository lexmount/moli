use super::*;

pub(super) fn target_is_after_current_cursor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    current: usize,
    direction: CursorDirection,
    key: &Key,
    primary_key: &Key,
) -> bool {
    if let (Some(current_key), Some(current_primary_key)) = (
        cursor_key_at(scope, cursor, current),
        cursor_primary_key_at(scope, cursor, current),
    ) && compare::cursor_tuple_cmp(
        direction,
        key,
        primary_key,
        &current_key,
        &current_primary_key,
    ) != std::cmp::Ordering::Greater
    {
        let error = dom_exception_value(
            scope,
            "Failed to execute 'continuePrimaryKey': the position is not after the cursor.",
            "DataError",
        );
        scope.throw_exception(error);
        return false;
    }
    true
}
