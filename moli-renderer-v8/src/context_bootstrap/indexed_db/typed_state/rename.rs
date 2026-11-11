use super::*;

// Runtime metadata and SQL share the same stable schema identities.
pub(in crate::context_bootstrap::indexed_db) fn rename_indexed_db_object_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    transaction: v8::Local<'s, v8::Object>,
    store_id: StoreId,
    new_name: &moli_indexeddb::IndexedDbName,
) -> Result<(), IndexedDbError> {
    let metadata = {
        let mirror = indexed_db_database_mirror(scope, database).ok_or_else(missing_mirror)?;
        let mut mirror = mirror.borrow_mut();
        let current = mirror.metadata.as_ref().ok_or_else(missing_mirror)?;
        let (name, source) = current
            .iter()
            .find(|(_, store)| store.id == store_id)
            .ok_or_else(missing_mirror)?;
        let metadata = source.with_name(new_name);
        let candidate = edit_shared(current, |map| {
            map.remove(name);
            map.insert(new_name.clone(), metadata.clone());
        });
        mirror.metadata = Some(candidate);
        metadata
    };
    for table in indexed_db_handle_tables(scope, database) {
        for store in table
            .borrow()
            .object_stores
            .values()
            .filter_map(std::rc::Weak::upgrade)
        {
            let mut store = store.borrow_mut();
            if !store.deleted
                && store.metadata.id == metadata.id
                && store.transaction.to_local(scope) == Some(transaction)
            {
                store.metadata = metadata.clone();
                store.name_override = None;
            }
        }
    }
    sync_transaction_object_store_names_from_database(scope, transaction, database);
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn rename_indexed_db_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    transaction: v8::Local<'s, v8::Object>,
    store_id: StoreId,
    index_id: IndexId,
    new_name: &moli_indexeddb::IndexedDbName,
) -> Result<(), IndexedDbError> {
    let metadata = {
        let mirror = indexed_db_database_mirror(scope, database).ok_or_else(missing_mirror)?;
        let mut mirror = mirror.borrow_mut();
        let current = mirror.metadata.as_ref().ok_or_else(missing_mirror)?;
        let (store_name, store) = current
            .iter()
            .find(|(_, store)| store.id == store_id)
            .ok_or_else(missing_mirror)?;
        let (name, _) = store
            .indexes
            .iter()
            .find(|(_, index)| index.id == index_id)
            .ok_or_else(missing_mirror)?;
        let store = store.with_renamed_index(name, new_name);
        let candidate = edit_shared(current, |map| {
            map.insert(store_name.clone(), store.clone());
        });
        mirror.metadata = Some(candidate);
        store
    };
    let index = metadata
        .indexes
        .get(new_name)
        .expect("renamed index exists");
    sync_indexed_db_store_handles(scope, database, store_id);
    for table in indexed_db_handle_tables(scope, database) {
        let indexes = table
            .borrow()
            .indexes
            .values()
            .filter_map(std::rc::Weak::upgrade)
            .collect::<Vec<_>>();
        for state in indexes {
            let store = {
                let state = state.borrow();
                if state.deleted || state.schema_id != index.id {
                    continue;
                }
                state.object_store.to_local(scope)
            };
            if store.and_then(|store| indexed_db_object_store_transaction(scope, store))
                == Some(transaction)
            {
                let mut state = state.borrow_mut();
                state.info = Rc::clone(&index.info);
            }
        }
    }
    Ok(())
}
