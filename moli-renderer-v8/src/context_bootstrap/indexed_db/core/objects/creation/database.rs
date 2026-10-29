use super::*;
use crate::context_bootstrap::{
    SIMPLE_EVENT_TARGET_LISTENERS_SLOT, simple_event_target_handler_getter_callback,
    simple_event_target_handler_setter_callback,
};
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
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),

    #[webapi(slot = crate::context_bootstrap::indexed_db::DATABASE_NAME_SLOT)]
    name: String,
    #[webapi(slot = crate::context_bootstrap::indexed_db::DATABASE_VERSION_SLOT)]
    version: f64,
    #[webapi(
        accessor_property = "onabort",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onabort"),
        setter_data = v8str(scope, "onabort"),
        enumerable,
        receiver = web_api_interfaces::IDBDatabase::is_instance
    )]
    onabort: (),
    #[webapi(
        accessor_property = "onclose",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onclose"),
        setter_data = v8str(scope, "onclose"),
        enumerable,
        receiver = web_api_interfaces::IDBDatabase::is_instance
    )]
    onclose: (),
    #[webapi(
        accessor_property = "onerror",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onerror"),
        setter_data = v8str(scope, "onerror"),
        enumerable,
        receiver = web_api_interfaces::IDBDatabase::is_instance
    )]
    onerror: (),
    #[webapi(
        accessor_property = "onversionchange",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onversionchange"),
        setter_data = v8str(scope, "onversionchange"),
        enumerable,
        receiver = web_api_interfaces::IDBDatabase::is_instance
    )]
    onversionchange: (),
}

pub(in crate::context_bootstrap::indexed_db) fn create_database_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    storage_scope: IndexedDbStorageScope,
    owner: IndexedDbExecutionOwner,
    handle: DatabaseHandle,
    info: &DatabaseInfo,
    client: moli_indexeddb::sqlite::service::DatabaseClient,
) -> Result<v8::Local<'s, v8::Object>, IndexedDbError> {
    let storage_key = storage_scope.storage_key().to_owned();
    let database_key = database_registry_key(&storage_key, &info.name);
    let database = IdbDatabaseObjectDeclaration::new(info.name.clone(), info.version as f64)
        .bind(scope)
        .map_err(|error| IndexedDbError::InvalidState(error.to_string()))?;
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
        client,
    );
    if let Err(error) = refresh_database_surface(scope, database) {
        crate::context_bootstrap::indexed_db::discard_indexed_db_database_lifecycle(
            scope, database,
        );
        return Err(error);
    }
    Ok(database)
}
