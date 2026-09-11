use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::IDBDatabase,
    scope_lifetime = 'scope,
    data_properties,
    enumerable
)]
struct IdbDatabaseSurfaceDeclaration<'scope, 'value> {
    name: &'value str,
    version: f64,
    object_store_names: v8::Local<'scope, v8::Object>,
}

pub(in crate::context_bootstrap::indexed_db) fn refresh_database_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
) -> std::result::Result<(), IndexedDbError> {
    let Some(handle) = database_handle_from_value(scope, database.into()) else {
        return Ok(());
    };
    let info = with_indexed_db_manager(scope, |manager| manager.database_info(handle))?;
    let mut metadata = Vec::with_capacity(info.object_store_names.len());
    for store_name in &info.object_store_names {
        let store = with_indexed_db_manager(scope, |manager| {
            manager.object_store_info(handle, store_name)
        })?;
        let mut indexes = Vec::with_capacity(store.index_names.len());
        for index_name in &store.index_names {
            indexes.push(with_indexed_db_manager(scope, |manager| {
                manager.index_info(handle, store_name, index_name)
            })?);
        }
        metadata.push(IndexedDbObjectStoreMetadata::new(store, indexes));
    }
    let object_store_names = new_idb_dom_string_list(scope, &info.object_store_names);
    let _ = replace_indexed_db_database_metadata(scope, database, metadata);
    IdbDatabaseSurfaceDeclaration::new(&info.name, info.version as f64, object_store_names)
        .initialize(scope, database)
        .map_err(|error| IndexedDbError::InvalidState(error.to_string()))?;
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn object_store_info_from_database_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &str,
) -> Option<ObjectStoreInfo> {
    indexed_db_database_store_metadata(scope, database, store_name)
        .map(|metadata| metadata.info().clone())
}
