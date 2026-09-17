use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    prototype = "Object",
    interface = web_api_interfaces::IDBCursor,
)]
struct IdbCursorObjectDeclaration<'scope> {
    #[webapi(slot = SOURCE)]
    source: v8::Local<'scope, v8::Object>,

    #[webapi(slot = REQUEST)]
    request_property: v8::Local<'scope, v8::Object>,
}

pub(in crate::context_bootstrap::indexed_db) fn refresh_cursor_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    position: Option<usize>,
) -> Option<()> {
    set_indexed_db_cursor_position(scope, cursor, position)?;
    // Keep cached values while iteration is pending, then invalidate them
    // when it settles, including when no record is found. The first getter
    // for a new record chooses the realm in which its value is materialized.
    for name in [KEY_CACHE, PRIMARY_KEY_CACHE, VALUE_CACHE] {
        let key = crate::util::private_key(scope, name)?;
        cursor.delete_private(scope, key)?;
    }
    Some(())
}

pub(in crate::context_bootstrap::indexed_db) fn materialize_cursor_result_in_request_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    snapshot: IndexedDbCursorSnapshot,
    operation: &IndexedDbCursorOpenOperation,
) -> Option<v8::Local<'s, v8::Object>> {
    let relevant_context = request.get_creation_context(scope)?;
    if relevant_context == scope.get_current_context() {
        return create_cursor_object_in_current_context(
            scope, source, request, snapshot, operation,
        );
    }

    let source = v8::Global::new(scope, source);
    let request = v8::Global::new(scope, request);
    let cursor = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let source = v8::Local::new(target_scope, &source);
        let request = v8::Local::new(target_scope, &request);
        create_cursor_object_in_current_context(target_scope, source, request, snapshot, operation)
            .map(|cursor| v8::Global::new(target_scope, cursor))
    }?;
    Some(v8::Local::new(scope, &cursor))
}

fn create_cursor_object_in_current_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    snapshot: IndexedDbCursorSnapshot,
    operation: &IndexedDbCursorOpenOperation,
) -> Option<v8::Local<'s, v8::Object>> {
    let cursor = IdbCursorObjectDeclaration::new(source, request)
        .bind(scope)
        .ok()?;
    let prototype = if operation.key_only {
        global_constructor_prototype(scope, "IDBCursor")?
    } else {
        global_constructor_prototype(scope, "IDBCursorWithValue")?
    };
    let _ = cursor.set_prototype(scope, prototype.into());
    let interface = if operation.key_only {
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
    register_indexed_db_cursor_lifecycle(scope, cursor, snapshot, operation);
    Some(cursor)
}
