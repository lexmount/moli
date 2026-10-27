use super::position::{next_primary_key_cursor_position, target_is_after_current_cursor};
use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBCursor.continuePrimaryKey")]
struct IdbCursorContinuePrimaryKeyArgs<'s> {
    #[webidl(required, converter = "raw")]
    key: v8::Local<'s, v8::Value>,
    #[webidl(required, converter = "raw")]
    primary_key: v8::Local<'s, v8::Value>,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_cursor_continue_primary_key_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbCursorContinuePrimaryKeyArgs<'s>>(scope, &args)
    else {
        return;
    };
    let cursor = args.this();
    if !cursor_source_is_index(scope, cursor) {
        let error = dom_exception_value(scope, "The source is not an index.", "InvalidAccessError");
        scope.throw_exception(error);
        return;
    }
    let current = cursor_position(scope, cursor);
    if current < 0 {
        let error = dom_exception_value(scope, "The cursor is exhausted.", "InvalidStateError");
        scope.throw_exception(error);
        return;
    }
    let key = match require_idb_key(scope, parsed.key) {
        Some(key) => key,
        None => return,
    };
    let primary_key = match require_idb_key(scope, parsed.primary_key) {
        Some(key) => key,
        None => return,
    };
    let direction = cursor_direction_from_cursor(scope, cursor);
    if direction.is_unique() {
        let error = dom_exception_value(
            scope,
            "continuePrimaryKey is not valid for unique cursors.",
            "InvalidAccessError",
        );
        scope.throw_exception(error);
        return;
    }
    if !target_is_after_current_cursor(
        scope,
        cursor,
        current as usize,
        direction,
        &key,
        &primary_key,
    ) {
        return;
    }
    let next = next_primary_key_cursor_position(
        scope,
        cursor,
        current as usize,
        direction,
        &key,
        &primary_key,
    );
    let _ = result::enqueue_cursor_result(scope, cursor, next);
    rv.set_undefined();
}
