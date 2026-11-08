use super::*;
use crate::context_bootstrap::indexed_db::indexed_db_database_has_sql_schema;

pub(in crate::context_bootstrap::indexed_db) fn refresh_database_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
) -> std::result::Result<(), IndexedDbError> {
    let client =
        crate::context_bootstrap::indexed_db::indexed_db_database_sql_client(scope, database)
            .ok_or_else(|| {
                IndexedDbError::InvalidState("SQL database client is unavailable".into())
            })?;
    let metadata = client.committed().ok_or_else(|| {
        IndexedDbError::InvalidState("SQL database metadata is unavailable".into())
    })?;
    if indexed_db_database_has_sql_schema(scope, database, &metadata.stores) {
        return Ok(());
    }
    let stores = metadata
        .stores
        .iter()
        .map(|(name, store)| IndexedDbObjectStoreMetadata::from_sql(name, store));
    replace_indexed_db_database_metadata(scope, database, stores, metadata.stores.clone())?;
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn object_store_info_from_database_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &moli_indexeddb::IndexedDbName,
) -> Option<ObjectStoreInfo> {
    indexed_db_database_store_metadata(scope, database, store_name)
        .map(|metadata| metadata.info().clone())
}
