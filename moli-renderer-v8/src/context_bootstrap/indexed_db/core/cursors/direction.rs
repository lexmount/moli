use super::*;

pub(in crate::context_bootstrap::indexed_db) fn cursor_direction_from_cursor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> CursorDirection {
    indexed_db_cursor_state(scope, cursor)
        .map(|state| state.direction)
        .unwrap_or_else(CursorDirection::default_next)
}
