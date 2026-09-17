use super::*;

pub(in crate::context_bootstrap::indexed_db) fn refresh_database_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
) -> std::result::Result<(), IndexedDbError> {
    let Some(handle) = database_handle_from_value(scope, database.into()) else {
        return Ok(());
    };
    let info = with_indexed_db_manager(scope, |manager| manager.database_info(handle))?;
    // Connection attributes are initialized at open and reverted on abort.
    // Committing only refreshes store metadata, preserving author properties.
    refresh_database_metadata(scope, database, &info)
}

pub(in crate::context_bootstrap::indexed_db) fn refresh_database_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    info: &DatabaseInfo,
) -> std::result::Result<(), IndexedDbError> {
    let Some(handle) = database_handle_from_value(scope, database.into()) else {
        return Ok(());
    };
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
    let _ = replace_indexed_db_database_metadata(scope, database, metadata);
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn object_store_info_from_database_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &IndexedDbName,
) -> Option<ObjectStoreInfo> {
    indexed_db_database_store_metadata(scope, database, store_name)
        .map(|metadata| metadata.info().clone())
}
