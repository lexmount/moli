use super::*;

// Names select schema at admission time; identities keep live handles and
// rollback snapshots associated with the same object when a name is reused.
pub(in crate::context_bootstrap::indexed_db) fn rename_indexed_db_object_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    transaction: v8::Local<'s, v8::Object>,
    name: &moli_indexeddb::IndexedDbName,
    new_name: &moli_indexeddb::IndexedDbName,
) -> Result<(), IndexedDbError> {
    let metadata = {
        let mirror = indexed_db_database_mirror(scope, database).ok_or_else(missing_mirror)?;
        let mut mirror = mirror.borrow_mut();
        let current = mirror.metadata.as_ref().ok_or_else(missing_mirror)?;
        let metadata = current
            .get(name)
            .ok_or_else(missing_mirror)?
            .with_name(new_name);
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
                && Rc::ptr_eq(&store.metadata.identity, &metadata.identity)
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
    store_name: &moli_indexeddb::IndexedDbName,
    name: &moli_indexeddb::IndexedDbName,
    new_name: &moli_indexeddb::IndexedDbName,
) -> Result<(), IndexedDbError> {
    let metadata = {
        let mirror = indexed_db_database_mirror(scope, database).ok_or_else(missing_mirror)?;
        let mut mirror = mirror.borrow_mut();
        let current = mirror.metadata.as_ref().ok_or_else(missing_mirror)?;
        let store = current.get(store_name).ok_or_else(missing_mirror)?;
        if !store.indexes.contains_key(name) {
            return Err(missing_mirror());
        }
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
    sync_indexed_db_store_handles(scope, database, store_name);
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
                if state.deleted || !Rc::ptr_eq(&state.identity, &index.identity) {
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
