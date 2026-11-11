use super::*;
use crate::context_bootstrap::indexed_db::{
    mark_indexed_db_index_handles_deleted, sync_indexed_db_store_handles,
};

pub(in crate::context_bootstrap::indexed_db) fn index_info_from_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    index_name: &moli_indexeddb::IndexedDbName,
) -> Option<std::rc::Rc<IndexInfo>> {
    indexed_db_object_store_metadata(scope, store)?.shared_index(index_name)
}

pub(in crate::context_bootstrap::indexed_db) fn set_database_index_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &moli_indexeddb::IndexedDbName,
    info: &IndexInfo,
    id: moli_indexeddb::sqlite::IndexId,
) -> Result<(), IndexedDbError> {
    let store = indexed_db_database_store_metadata(scope, database, store_name)
        .ok_or_else(|| IndexedDbError::InvalidState("The object store is unavailable".into()))?;
    set_indexed_db_database_index_metadata(scope, database, store_name, info, id)?;
    sync_indexed_db_store_handles(scope, database, store.id());
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn remove_database_index_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &moli_indexeddb::IndexedDbName,
    index_name: &moli_indexeddb::IndexedDbName,
) -> Result<(), IndexedDbError> {
    let store = indexed_db_database_store_metadata(scope, database, store_name)
        .ok_or_else(|| IndexedDbError::InvalidState("The object store is unavailable".into()))?;
    let index_id = store
        .index_id(index_name)
        .ok_or_else(|| IndexedDbError::InvalidState("The index is unavailable".into()))?;
    remove_indexed_db_database_index_metadata(scope, database, store_name, index_name)?;
    mark_indexed_db_index_handles_deleted(scope, database, store.id(), index_id);
    sync_indexed_db_store_handles(scope, database, store.id());
    Ok(())
}
