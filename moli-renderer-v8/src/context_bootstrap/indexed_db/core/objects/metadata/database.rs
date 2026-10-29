use super::*;
use crate::context_bootstrap::indexed_db::{
    indexed_db_database_store_names, sync_indexed_db_store_handles,
};

pub(in crate::context_bootstrap::indexed_db) fn sync_transaction_object_store_names_from_database<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    database: v8::Local<'s, v8::Object>,
) {
    let store_names = indexed_db_database_store_names(scope, database);
    let object_store_names = new_idb_name_list(scope, &store_names);
    let _ = transaction.set(
        scope,
        v8str(scope, "objectStoreNames").into(),
        object_store_names.into(),
    );
}

pub(in crate::context_bootstrap::indexed_db) fn set_database_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    info: &ObjectStoreInfo,
    indexes: &[IndexInfo],
) -> Result<(), IndexedDbError> {
    let typed_metadata = if let Some(client) = indexed_db_database_sql_client(scope, database) {
        IndexedDbObjectStoreMetadata::try_from_info(&client, info)?
    } else {
        IndexedDbObjectStoreMetadata::new(info.clone(), indexes.iter().cloned())
    };
    set_indexed_db_database_store_metadata(scope, database, typed_metadata)?;
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn remove_database_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &str,
) -> Result<(), IndexedDbError> {
    remove_indexed_db_database_store_metadata(scope, database, store_name)?;
    sync_indexed_db_store_handles(scope, database, store_name);
    Ok(())
}
