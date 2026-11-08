use super::*;

pub(super) struct IndexedDbUpgradeMetadata {
    version: u64,
    pub(super) stores: Rc<StoreMap>,
}

pub(in crate::context_bootstrap::indexed_db) fn associate_indexed_db_upgrade_open<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    if let Some(state) = table.borrow_mut().transactions.get_mut(&id) {
        state.upgrade_open_request = Some(RealmValueHandle::new(scope, request.into()));
    }
}

pub(in crate::context_bootstrap::indexed_db) fn take_indexed_db_upgrade_open<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    let id = indexed_db_typed_state_id(scope, transaction)?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let mut table = table.borrow_mut();
    let state = table.transactions.get_mut(&id)?;
    let request = state.upgrade_open_request.take()?;
    Some((
        v8::Local::<v8::Object>::try_from(request.to_local(scope)).ok()?,
        v8::Local::<v8::Object>::try_from(state.database.as_ref()?.to_local(scope)).ok()?,
    ))
}

pub(in crate::context_bootstrap::indexed_db) fn save_indexed_db_upgrade_metadata(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
    old_version: u64,
) {
    let Some(id) = indexed_db_typed_state_id(scope, database) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let mut table = table.borrow_mut();
    let Some(database) = table.databases.get_mut(&id) else {
        return;
    };
    let Some(mirror) = database.mirror.upgrade() else {
        return;
    };
    let mut mirror = mirror.borrow_mut();
    mirror.upgrade_metadata = Some(IndexedDbUpgradeMetadata {
        version: old_version,
        stores: mirror
            .metadata
            .as_ref()
            .expect("open database has schema")
            .clone(),
    });
}

// Methods borrowed from another realm may create wrappers in a different
// runtime table. Track those actual registration tables without rooting realms.
pub(super) fn track_indexed_db_handle_table(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
    handles: &Rc<RefCell<IndexedDbRuntimeStateTable>>,
) {
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    if Rc::ptr_eq(&table, handles) {
        return;
    }
    let Some(id) = indexed_db_typed_state_id(scope, database) else {
        return;
    };
    let mut table = table.borrow_mut();
    let Some(database) = table.databases.get_mut(&id) else {
        return;
    };
    database
        .handle_tables
        .retain(|table| table.strong_count() != 0);
    let handles = Rc::downgrade(handles);
    if !database
        .handle_tables
        .iter()
        .any(|table| table.ptr_eq(&handles))
    {
        database.handle_tables.push(handles);
    }
}

pub(super) fn indexed_db_handle_tables(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) -> Vec<Rc<RefCell<IndexedDbRuntimeStateTable>>> {
    let owner = indexed_db_runtime_state_table_for_object(scope, database);
    let mut tables = vec![Rc::clone(&owner)];
    if let Some(id) = indexed_db_typed_state_id(scope, database)
        && let Some(database) = owner.borrow_mut().databases.get_mut(&id)
    {
        database.handle_tables.retain(|table| {
            if let Some(table) = table.upgrade() {
                tables.push(table);
                true
            } else {
                false
            }
        });
    }
    tables
}

pub(in crate::context_bootstrap::indexed_db) fn restore_indexed_db_upgrade_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    let Some(transaction_id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let database = {
        let table = table.borrow();
        let Some(state) = table.transactions.get(&transaction_id) else {
            return;
        };
        let Some(database) = state.database.as_ref() else {
            return;
        };
        let Ok(database) = v8::Local::<v8::Object>::try_from(database.to_local(scope)) else {
            return;
        };
        database
    };
    let Some(id) = indexed_db_typed_state_id(scope, database) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let snapshot = {
        let mut table = table.borrow_mut();
        let Some(state) = table.databases.get_mut(&id) else {
            return;
        };
        if !state
            .upgrade_transaction
            .as_ref()
            .is_some_and(|value| value.to_local(scope) == transaction)
        {
            return;
        }
        let Some(mirror) = state.mirror.upgrade() else {
            return;
        };
        let mut mirror = mirror.borrow_mut();
        let Some(snapshot) = mirror.upgrade_metadata.take() else {
            return;
        };
        mirror.metadata = Some(snapshot.stores.clone());
        snapshot
    };
    let version = snapshot.version;
    let names = snapshot.stores.keys().cloned().collect::<Vec<_>>();
    let tables = indexed_db_handle_tables(scope, database);
    for table in &tables {
        for store in table
            .borrow()
            .object_stores
            .values()
            .filter_map(std::rc::Weak::upgrade)
        {
            let mut store = store.borrow_mut();
            if store.transaction.to_local(scope) != Some(transaction) {
                continue;
            }
            // Replacements with the same name are separate object stores.
            store.deleted = store.metadata.created_in_upgrade.get();
            let metadata = if store.deleted {
                store.metadata.without_indexes()
            } else {
                snapshot
                    .stores
                    .get(store.name())
                    .cloned()
                    .expect("existing store has upgrade snapshot")
            };
            store.metadata = metadata;
        }
    }
    for table in &tables {
        // An index and its object store may themselves belong to different
        // tables. Resolve the store only after releasing the index-table borrow.
        let indexes = table
            .borrow()
            .indexes
            .iter()
            .filter_map(|(id, index)| {
                let index = index.upgrade()?;
                let index = index.borrow();
                let store = index.object_store.to_local(scope)?;
                Some((*id, store, index.created_in_upgrade))
            })
            .collect::<Vec<_>>();
        for (id, store, created_in_upgrade) in indexes {
            // A collected alias may have been swept by the store lookup.
            if indexed_db_object_store_transaction(scope, store) == Some(transaction)
                && let Some(index) = table
                    .borrow()
                    .indexes
                    .get(&id)
                    .and_then(std::rc::Weak::upgrade)
            {
                index.borrow_mut().deleted = created_in_upgrade;
            }
        }
    }
    set_indexed_db_database_version(scope, database, version);
    let transaction_names = new_idb_name_list(scope, &names);
    let _ = transaction.define_own_property(
        scope,
        v8str(scope, "objectStoreNames").into(),
        transaction_names.into(),
        v8::PropertyAttribute::NONE,
    );
}

// Update native metadata for every alias. Author shadows and cached keyPath
// arrays belong to the handle and survive schema edits and rollback.
pub(in crate::context_bootstrap::indexed_db) fn sync_indexed_db_store_handles<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &moli_indexeddb::IndexedDbName,
) {
    let metadata = indexed_db_database_store_metadata(scope, database, store_name);
    for table in indexed_db_handle_tables(scope, database) {
        for store in table
            .borrow()
            .object_stores
            .values()
            .filter_map(std::rc::Weak::upgrade)
        {
            let mut store = store.borrow_mut();
            if store.deleted
                || store.name() != store_name
                || store.database.to_local(scope) != Some(database)
            {
                continue;
            }
            let metadata = match &metadata {
                Some(metadata) => metadata.clone(),
                None => {
                    store.deleted = true;
                    store.metadata.without_indexes()
                }
            };
            store.metadata = metadata;
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn mark_indexed_db_index_handles_deleted(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
    store_name: &moli_indexeddb::IndexedDbName,
    index_name: &moli_indexeddb::IndexedDbName,
) {
    for table in indexed_db_handle_tables(scope, database) {
        let indexes = table
            .borrow()
            .indexes
            .iter()
            .filter_map(|(id, index)| {
                let index = index.upgrade()?;
                let index = index.borrow();
                if &index.info.name != index_name {
                    return None;
                }
                let store = index.object_store.to_local(scope)?;
                Some((*id, store))
            })
            .collect::<Vec<_>>();
        for (id, store) in indexes {
            if !indexed_db_object_store_is_deleted(scope, store)
                && indexed_db_object_store_name(scope, store).as_ref() == Some(store_name)
                && indexed_db_object_store_database(scope, store) == Some(database)
                && let Some(index) = table
                    .borrow()
                    .indexes
                    .get(&id)
                    .and_then(std::rc::Weak::upgrade)
            {
                index.borrow_mut().deleted = true;
            }
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_object_store_is_deleted(
    scope: &mut v8::PinScope<'_, '_>,
    store: v8::Local<'_, v8::Object>,
) -> bool {
    object_store_native_state(scope, store).is_none_or(|state| state.borrow().deleted)
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_index_is_deleted(
    scope: &mut v8::PinScope<'_, '_>,
    index: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(state) = index_native_state(scope, index) else {
        return true;
    };
    let (deleted, store) = {
        let index = state.borrow();
        let Some(store) = index.object_store.to_local(scope) else {
            return true;
        };
        (index.deleted, store)
    };
    deleted || indexed_db_object_store_is_deleted(scope, store)
}

pub(in crate::context_bootstrap::indexed_db) fn clear_indexed_db_upgrade_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    let Some(database) = indexed_db_transaction_database(scope, transaction) else {
        return;
    };
    let Some(id) = indexed_db_typed_state_id(scope, database) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let mut table = table.borrow_mut();
    if let Some(state) = table.databases.get_mut(&id)
        && state
            .upgrade_transaction
            .as_ref()
            .is_some_and(|owner| owner.to_local(scope) == transaction)
    {
        state.upgrade_transaction = None;
        if let Some(mirror) = state.mirror.upgrade() {
            mirror.borrow_mut().upgrade_metadata = None;
        }
    }
}
