use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::IDBObjectStore, require_prototype)]
struct IdbObjectStoreObjectDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    transaction: v8::Local<'scope, v8::Object>,

    #[webapi(data_property, enumerable)]
    db: v8::Local<'scope, v8::Object>,
}

pub(in crate::context_bootstrap::indexed_db) fn create_object_store_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    db: v8::Local<'s, v8::Object>,
    tx: v8::Local<'s, v8::Object>,
    info: &ObjectStoreInfo,
) -> Option<v8::Local<'s, v8::Object>> {
    let metadata = indexed_db_database_store_metadata(scope, db, &info.name)?;
    let store = IdbObjectStoreObjectDeclaration::new(tx, db)
        .bind(scope)
        .ok()?;
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
    register_indexed_db_object_store_lifecycle(scope, store, tx, db, metadata.clone());
    sync_store_surface_from_metadata(scope, store, metadata)?;
    Some(store)
}
