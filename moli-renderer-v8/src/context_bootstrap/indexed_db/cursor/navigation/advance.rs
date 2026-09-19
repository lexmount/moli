use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBCursor.advance")]
struct IdbCursorAdvanceArgs {
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    count: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBCursor.continue")]
struct IdbCursorContinueArgs<'s> {
    #[webidl(converter = "raw")]
    key: Option<v8::Local<'s, v8::Value>>,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_cursor_continue_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbCursorContinueArgs<'s>>(scope, &args) else {
        return;
    };
    let cursor = args.this();
    if cursor_active_transaction(scope, cursor).is_none()
        || cursor_effective_object_store(scope, cursor).is_none()
    {
        return;
    }
    let Some(current) = cursor_iteration_position(scope, cursor) else {
        return;
    };
    let key = parsed.key.unwrap_or_else(|| v8::undefined(scope).into());
    let target = match parse_idb_key(scope, key) {
        Ok(key) => key,
        Err(error) => {
            error.throw(scope);
            return;
        }
    };
    let direction = cursor_direction_from_cursor(scope, cursor);
    if let Some(target) = &target
        && let Some(current_key) = cursor_key_at(scope, cursor, current)
        && compare::cursor_direction_cmp(direction, target, &current_key)
            != std::cmp::Ordering::Greater
    {
        let error = dom_exception_value(
            scope,
            "Failed to execute 'continue': the key is not greater than the cursor position.",
            "DataError",
        );
        scope.throw_exception(error);
        return;
    }
    let _ = result::enqueue_cursor_result(scope, cursor, CursorIteration::Continue(target));
    rv.set_undefined();
}

pub(in crate::context_bootstrap::indexed_db) fn idb_cursor_advance_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbCursorAdvanceArgs>(scope, &args) else {
        return;
    };
    let cursor = args.this();
    let count = parsed.count;
    if count == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'advance': count must be greater than zero.",
        );
        return;
    }
    if cursor_active_transaction(scope, cursor).is_none()
        || cursor_effective_object_store(scope, cursor).is_none()
    {
        return;
    }
    if cursor_iteration_position(scope, cursor).is_none() {
        return;
    }
    let _ = result::enqueue_cursor_result(scope, cursor, CursorIteration::Advance(count));
    rv.set_undefined();
}
