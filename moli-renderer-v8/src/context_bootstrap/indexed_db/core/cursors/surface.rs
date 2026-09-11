use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    prototype = "Object",
    interface = web_api_interfaces::IDBCursor,
    data_properties,
    enumerable
)]
struct IdbCursorObjectDeclaration<'scope> {
    source: v8::Local<'scope, v8::Object>,

    #[webapi(data_property = "request", enumerable)]
    request_property: v8::Local<'scope, v8::Object>,

    direction: &'static str,
}

fn set_cursor_position(
    scope: &mut v8::PinScope<'_, '_>,
    cursor: v8::Local<'_, v8::Object>,
    position: i32,
) {
    set_indexed_db_slot_value(
        scope,
        cursor,
        INDEXED_DB_CURSOR_POSITION_SLOT,
        v8::Number::new(scope, position as f64).into(),
    );
}

pub(in crate::context_bootstrap::indexed_db) fn refresh_cursor_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    position: Option<usize>,
) -> Option<()> {
    match position.and_then(|index| cursor_entry_object(scope, cursor, index)) {
        Some(entry) => {
            set_cursor_position(scope, cursor, position? as i32);
            let key = entry.get(scope, v8str(scope, "key").into())?;
            let primary_key = entry.get(scope, v8str(scope, "primaryKey").into())?;
            let value = entry.get(scope, v8str(scope, "value").into())?;
            let _ = cursor.set(scope, v8str(scope, "key").into(), key);
            let _ = cursor.set(scope, v8str(scope, "primaryKey").into(), primary_key);
            if !object_bool_property(scope, cursor, INDEXED_DB_CURSOR_KEY_ONLY_SLOT)
                .unwrap_or(false)
            {
                let _ = cursor.set(scope, v8str(scope, "value").into(), value);
            }
        }
        None => {
            set_cursor_position(scope, cursor, -1);
            let undefined = v8::undefined(scope);
            let _ = cursor.set(scope, v8str(scope, "key").into(), undefined.into());
            let _ = cursor.set(scope, v8str(scope, "primaryKey").into(), undefined.into());
            if !object_bool_property(scope, cursor, INDEXED_DB_CURSOR_KEY_ONLY_SLOT)
                .unwrap_or(false)
            {
                let _ = cursor.set(scope, v8str(scope, "value").into(), undefined.into());
            }
        }
    }
    Some(())
}

pub(in crate::context_bootstrap::indexed_db) fn materialize_cursor_result_in_request_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    entries: &[CursorSnapshotEntry],
    direction: CursorDirection,
    key_only: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let relevant_context = request.get_creation_context(scope)?;
    if relevant_context == scope.get_current_context() {
        return create_cursor_object_in_current_context(
            scope, source, request, entries, direction, key_only,
        );
    }

    let source = v8::Global::new(scope, source);
    let request = v8::Global::new(scope, request);
    let cursor = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let source = v8::Local::new(target_scope, &source);
        let request = v8::Local::new(target_scope, &request);
        create_cursor_object_in_current_context(
            target_scope,
            source,
            request,
            entries,
            direction,
            key_only,
        )
        .map(|cursor| v8::Global::new(target_scope, cursor))
    }?;
    Some(v8::Local::new(scope, &cursor))
}

fn create_cursor_object_in_current_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    entries: &[CursorSnapshotEntry],
    direction: CursorDirection,
    key_only: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let entries_array = cursor_entries_to_js_array(scope, entries)?;
    let cursor = IdbCursorObjectDeclaration::new(source, request, direction.as_str())
        .bind(scope)
        .ok()?;
    let prototype = if key_only {
        global_constructor_prototype(scope, "IDBCursor")?
    } else {
        global_constructor_prototype(scope, "IDBCursorWithValue")?
    };
    let _ = cursor.set_prototype(scope, prototype.into());
    let interface = if key_only {
        "IDBCursor"
    } else {
        "IDBCursorWithValue"
    };
    web_api_interfaces::initialize(scope, cursor, interface).ok()?;
    let storage_scope = indexed_db_typed_storage_scope(scope, request);
    let owner = indexed_db_typed_execution_owner(scope, request)
        .expect("IDBCursor should inherit typed owner from request");
    debug_assert_eq!(indexed_db_typed_execution_owner(scope, source), Some(owner));
    register_indexed_db_wrapper_with_owner(
        scope,
        cursor,
        IndexedDbWrapperKind::Cursor,
        owner,
        storage_scope,
    );
    register_indexed_db_cursor_lifecycle(scope, cursor, request, entries_array, key_only, 0.0);
    let _ = refresh_cursor_surface(
        scope,
        cursor,
        if entries.is_empty() { None } else { Some(0) },
    );
    Some(cursor)
}
