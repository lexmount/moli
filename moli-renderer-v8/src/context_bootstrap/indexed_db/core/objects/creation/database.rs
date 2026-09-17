use super::*;
use crate::context_bootstrap::indexed_db::initialize_indexed_db_event_target;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::IDBDatabase,
    require_prototype,
    data_properties,
    enumerable
)]
struct IdbDatabaseObjectDeclaration {
    #[webapi(slot = INDEXED_DB_EVENT_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),

    name: String,
    version: f64,
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
    let database = IdbDatabaseObjectDeclaration::new(info.name.clone(), info.version as f64)
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
    let _ = refresh_database_metadata(scope, database, info);
    initialize_indexed_db_event_target(scope, database, None);
    register_open_database_connection(scope, owner, handle, database_key, database);
    Some(database)
}
