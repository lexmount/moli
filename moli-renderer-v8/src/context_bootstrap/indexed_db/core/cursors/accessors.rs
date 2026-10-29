use super::*;
use crate::util::{get_private_value, private_key};

pub(in crate::context_bootstrap::indexed_db) fn cursor_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    v8::Local::try_from(get_private_value(scope, cursor, SOURCE)?).ok()
}

pub(in crate::context_bootstrap::indexed_db) fn cursor_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    v8::Local::try_from(get_private_value(scope, cursor, REQUEST)?).ok()
}

pub(in crate::context_bootstrap::indexed_db) fn idb_cursor_source_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(source) = cursor_source(scope, args.this()) {
        rv.set(source.into());
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_cursor_request_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(request) = cursor_request(scope, args.this()) {
        rv.set(request.into());
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_cursor_direction_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(state) = indexed_db_cursor_state(scope, args.this()) {
        rv.set(v8str(scope, state.direction.as_str()).into());
    }
}

enum CachedAttribute {
    Key,
    PrimaryKey,
    Value,
}

fn cached_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    attribute: CachedAttribute,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) -> Option<()> {
    let slot = private_key(
        scope,
        match attribute {
            CachedAttribute::Key => KEY_CACHE,
            CachedAttribute::PrimaryKey => PRIMARY_KEY_CACHE,
            CachedAttribute::Value => VALUE_CACHE,
        },
    )?;
    // A stored undefined value is cached too. The private cache belongs to
    // the cursor, so cyclic author values never become native global roots.
    if cursor.has_private(scope, slot)? {
        rv.set(cursor.get_private(scope, slot)?);
        return Some(());
    }
    let state = indexed_db_cursor_state(scope, cursor)?;
    let entry = state.entries.get(state.position?)?;
    let cancel = if state.sql.is_some() {
        let request = cursor_request(scope, cursor)?;
        Some(
            crate::context_bootstrap::indexed_db::typed_state::sql::sql_transaction_event_cancellation(
                scope, request,
            ),
        )
    } else {
        None
    };
    v8::tc_scope!(let scope, scope);
    let session = crate::context_bootstrap::indexed_db::ValueDecodeSession::new();
    let mut failure = None;
    let result = (|| {
        let value = match attribute {
            CachedAttribute::Key => {
                crate::context_bootstrap::indexed_db::try_key_to_js_value(scope, &entry.key)?
            }
            CachedAttribute::PrimaryKey => {
                crate::context_bootstrap::indexed_db::try_key_to_js_value(
                    scope,
                    &entry.primary_key,
                )?
            }
            CachedAttribute::Value => {
                let bytes = entry.value.as_ref()?;
                match session.deserialize(scope, bytes) {
                    Ok(value) => value?,
                    Err(error) => {
                        failure = Some(error);
                        return None;
                    }
                }
            }
        };
        cursor.set_private(scope, slot, value)?;
        rv.set(value);
        Some(())
    })();
    if scope.is_execution_terminating() || !scope.can_continue() {
        if scope.has_caught() {
            scope.rethrow();
        }
        if let Some(cancel) = cancel {
            cancel();
        }
        return None;
    }
    if let Some(error) = failure {
        scope.reset();
        let error = request_error_object(scope, &error);
        scope.throw_exception(error);
    }
    if result.is_none() && !scope.has_caught() {
        let error = dom_exception_value(
            scope,
            "Unable to decode the current IndexedDB cursor attribute.",
            "UnknownError",
        );
        scope.throw_exception(error);
    }
    if scope.has_caught() {
        scope.rethrow();
    } else if result.is_some() {
        session.publish();
    }
    result
}

pub(in crate::context_bootstrap::indexed_db) fn idb_cursor_key_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    let _ = cached_attribute_getter(scope, args.this(), CachedAttribute::Key, rv);
}

pub(in crate::context_bootstrap::indexed_db) fn idb_cursor_primary_key_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    let _ = cached_attribute_getter(scope, args.this(), CachedAttribute::PrimaryKey, rv);
}

pub(in crate::context_bootstrap::indexed_db) fn idb_cursor_value_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    let _ = cached_attribute_getter(scope, args.this(), CachedAttribute::Value, rv);
}
