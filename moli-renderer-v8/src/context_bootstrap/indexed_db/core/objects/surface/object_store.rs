use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::IDBObjectStore, require_prototype, scope_lifetime = 'scope)]
struct IdbObjectStoreObjectDeclaration<'scope> {
    #[webapi(slot = crate::context_bootstrap::indexed_db::STORE_NAME_FALLBACK_SLOT)]
    name: v8::Local<'scope, v8::String>,
    #[webapi(slot = SCHEMA_KEY_PATH_SLOT)]
    key_path: v8::Local<'scope, v8::Value>,
    #[webapi(slot = SCHEMA_AUTO_INCREMENT_SLOT)]
    auto_increment: bool,
    #[webapi(slot = OBJECT_STORE_TRANSACTION)]
    transaction: v8::Local<'scope, v8::Object>,
    #[webapi(slot = OBJECT_STORE_DATABASE)]
    db: v8::Local<'scope, v8::Object>,
}

pub(in crate::context_bootstrap::indexed_db) fn create_object_store_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    db: v8::Local<'s, v8::Object>,
    tx: v8::Local<'s, v8::Object>,
    info: &ObjectStoreInfo,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(store) =
        crate::context_bootstrap::indexed_db::cached_indexed_db_object_store(scope, tx, &info.name)
    {
        return Some(store);
    }
    let metadata = indexed_db_database_store_metadata(scope, db, &info.name)?;
    let store = create_failed_object_store_object(scope, db, tx, info)?;
    let storage_scope = indexed_db_typed_storage_scope(scope, db);
    let owner = indexed_db_typed_execution_owner(scope, tx)
        .expect("IDBObjectStore should inherit typed owner from transaction");
    debug_assert_eq!(indexed_db_typed_execution_owner(scope, db), Some(owner));
    register_indexed_db_wrapper_with_owner(
        scope,
        store,
        IndexedDbWrapperKind::ObjectStore,
        owner,
        storage_scope,
    );
    register_indexed_db_object_store_lifecycle(scope, store, tx, db, metadata);
    Some(store)
}

// Schema creation can fail asynchronously after the handle has been returned.
// Its immutable values and parent references remain traced by the V8 object.
pub(in crate::context_bootstrap::indexed_db) fn create_failed_object_store_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    db: v8::Local<'s, v8::Object>,
    tx: v8::Local<'s, v8::Object>,
    info: &ObjectStoreInfo,
) -> Option<v8::Local<'s, v8::Object>> {
    let context = tx.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let key_path = match &info.key_path {
        Some(path) => key_path_to_js_value(scope, path)?,
        None => v8::null(scope).into(),
    };
    IdbObjectStoreObjectDeclaration::new(
        crate::util::v8_string_from_utf16_units(scope, info.name.as_utf16())?,
        key_path,
        info.auto_increment,
        tx,
        db,
    )
    .bind(scope)
    .ok()
}
