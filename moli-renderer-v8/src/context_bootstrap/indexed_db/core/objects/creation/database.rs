use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::IDBDatabase,
    require_prototype,
    scope_lifetime = 'scope,
    data_properties,
    enumerable
)]
struct IdbDatabaseObjectDeclaration<'scope, 'value> {
    #[webapi(slot = INDEXED_DB_EVENT_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),

    name: &'value str,
    version: f64,
    object_store_names: v8::Local<'scope, v8::Object>,
    #[webapi(init = "null")]
    onabort: (),
    #[webapi(init = "null")]
    onclose: (),
    #[webapi(init = "null")]
    onerror: (),
    #[webapi(init = "null")]
    onversionchange: (),
}

pub(in crate::context_bootstrap::indexed_db) fn create_database_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    storage_scope: IndexedDbStorageScope,
    owner: IndexedDbExecutionOwner,
    handle: DatabaseHandle,
    info: &DatabaseInfo,
) -> Option<v8::Local<'s, v8::Object>> {
    let storage_key = storage_scope.storage_key().to_owned();
    let database_key = database_registry_key(&storage_key, &info.name);
    let object_store_names = new_idb_dom_string_list(scope, &info.object_store_names);
    let database =
        IdbDatabaseObjectDeclaration::new(&info.name, info.version as f64, object_store_names)
            .bind(scope)
            .ok()?;
    register_indexed_db_wrapper_with_owner(
        scope,
        database,
        IndexedDbWrapperKind::Database,
        owner,
        Some(storage_scope.clone()),
    );
    register_indexed_db_database_lifecycle(
        scope,
        database,
        handle,
        database_key.clone(),
        storage_scope,
    );
    let _ = refresh_database_surface(scope, database);
    register_open_database_connection(scope, owner, handle, database_key, info.version, database);
    Some(database)
}
