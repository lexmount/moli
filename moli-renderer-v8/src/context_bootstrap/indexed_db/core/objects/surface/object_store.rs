use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::IDBObjectStore, require_prototype)]
struct IdbObjectStoreObjectDeclaration {}

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
    let key_path = match &info.key_path {
        Some(path) => key_path_to_js_value(scope, path)?,
        None => v8::null(scope).into(),
    };
    let store = IdbObjectStoreObjectDeclaration::new().bind(scope).ok()?;
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
    register_indexed_db_object_store_lifecycle(scope, store, tx, db, metadata, key_path);
    Some(store)
}
