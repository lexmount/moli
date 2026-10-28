use super::*;
pub(in crate::context_bootstrap::indexed_db) fn refresh_database_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
) -> std::result::Result<(), IndexedDbError> {
    let Some(handle) = database_handle_from_value(scope, database.into()) else {
        return Ok(());
    };
    let info = if let Some(client) =
        crate::context_bootstrap::indexed_db::indexed_db_database_sql_client(scope, database)
    {
        let metadata = client.committed().ok_or_else(|| {
            IndexedDbError::InvalidState("SQL database metadata is unavailable".into())
        })?;
        let mut names: Vec<_> = metadata.stores.keys().cloned().collect();
        names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        DatabaseInfo {
            name: metadata.name.clone(),
            version: metadata.version,
            object_store_names: names,
        }
    } else {
        with_indexed_db_manager(scope, |manager| manager.database_info(handle))?
    };
    refresh_database_metadata(scope, database, &info)
}

pub(in crate::context_bootstrap::indexed_db) fn refresh_database_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    info: &DatabaseInfo,
) -> std::result::Result<(), IndexedDbError> {
    if let Some(client) =
        crate::context_bootstrap::indexed_db::indexed_db_database_sql_client(scope, database)
    {
        let metadata = client.committed().ok_or_else(|| {
            IndexedDbError::InvalidState("SQL database metadata is unavailable".into())
        })?;
        let stores = metadata.stores.iter().map(|(name, store)| {
            let mut names: Vec<_> = store.indexes.keys().cloned().collect();
            names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            IndexedDbObjectStoreMetadata::new(
                ObjectStoreInfo {
                    name: name.clone(),
                    key_path: store.options.key_path.clone(),
                    auto_increment: store.options.auto_increment,
                    index_names: names,
                },
                store.indexes.iter().map(|(name, index)| IndexInfo {
                    name: name.clone(),
                    key_path: index.options.key_path.clone(),
                    unique: index.options.unique,
                    multi_entry: index.options.multi_entry,
                }),
            )
        });
        replace_indexed_db_database_metadata(scope, database, stores);
        return Ok(());
    }
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
    store_name: &str,
) -> Option<ObjectStoreInfo> {
    indexed_db_database_store_metadata(scope, database, store_name)
        .map(|metadata| metadata.info().clone())
}
