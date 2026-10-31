use super::*;
use crate::context_bootstrap::SIMPLE_EVENT_TARGET_LISTENERS_SLOT;
use crate::web_api_interfaces;
use moli_indexeddb::IndexedDbName;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::IDBDatabase,
    require_prototype,
    data_properties,
    enumerable
)]
struct IdbDatabaseObjectDeclaration<'scope> {
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),

    #[webapi(slot = crate::context_bootstrap::indexed_db::DATABASE_NAME_SLOT)]
    name: v8::Local<'scope, v8::String>,
    #[webapi(slot = crate::context_bootstrap::indexed_db::DATABASE_VERSION_SLOT)]
    version: f64,
}

pub(in crate::context_bootstrap::indexed_db) fn create_database_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    storage_scope: IndexedDbStorageScope,
    owner: IndexedDbExecutionOwner,
    handle: DatabaseHandle,
    name: &IndexedDbName,
    version: u64,
    client: moli_indexeddb::sqlite::service::DatabaseClient,
) -> Result<v8::Local<'s, v8::Object>, IndexedDbError> {
    let name = v8::String::new_from_two_byte(scope, name.as_utf16(), v8::NewStringType::Normal)
        .ok_or_else(|| IndexedDbError::Resource("Unable to allocate IndexedDB name".into()))?;
    let database = IdbDatabaseObjectDeclaration::new(name, version as f64)
        .bind(scope)
        .map_err(|error| IndexedDbError::InvalidState(error.to_string()))?;
    register_indexed_db_wrapper_with_owner(
        scope,
        database,
        IndexedDbWrapperKind::Database,
        owner,
        Some(storage_scope.clone()),
    );
    register_indexed_db_database_lifecycle(scope, database, handle, storage_scope, client);
    if let Err(error) = refresh_database_surface(scope, database) {
        crate::context_bootstrap::indexed_db::discard_indexed_db_database_lifecycle(
            scope, database,
        );
        return Err(error);
    }
    Ok(database)
}
