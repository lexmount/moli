use super::*;

pub(super) struct IndexedDbUpgradeMetadata {
    version: u64,
    stores: BTreeMap<IndexedDbName, IndexedDbObjectStoreMetadata>,
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
    database.upgrade_metadata = Some(IndexedDbUpgradeMetadata {
        version: old_version,
        stores: database.metadata.clone(),
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

fn indexed_db_handle_tables(
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
        let Some(snapshot) = state.upgrade_metadata.take() else {
            return;
        };
        state.metadata = snapshot.stores.clone();
        snapshot
    };
    let version = snapshot.version;
    let names = snapshot.stores.keys().cloned().collect::<Vec<_>>();
    let tables = indexed_db_handle_tables(scope, database);
    for table in &tables {
        for store in table.borrow_mut().object_stores.values_mut() {
            if store.transaction.to_local(scope) != transaction {
                continue;
            }
            // Replacements with the same name are separate object stores.
            store.deleted = store.metadata.original_name.is_none();
            let metadata = if store.deleted {
                let mut metadata = store.metadata.clone();
                metadata.info.index_names.clear();
                metadata.indexes.clear();
                metadata
            } else {
                snapshot
                    .stores
                    .get(
                        store
                            .metadata
                            .original_name
                            .as_ref()
                            .expect("existing store name"),
                    )
                    .cloned()
                    .expect("existing store has upgrade snapshot")
            };
            store.name = metadata.info.name.clone();
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
            .map(|(id, index)| {
                let store = v8::Local::<v8::Object>::try_from(index.object_store.to_local(scope))
                    .expect("index retains its object store");
                (*id, store)
            })
            .collect::<Vec<_>>();
        for (id, store) in indexes {
            if indexed_db_object_store_transaction(scope, store) == Some(transaction) {
                let mut table = table.borrow_mut();
                let index = table
                    .indexes
                    .get_mut(&id)
                    .expect("registered index remains present");
                index.deleted = index.original_name.is_none();
                if let Some(original_name) = &index.original_name {
                    index.info.name = original_name.clone();
                }
            }
        }
    }
    set_indexed_db_database_version(scope, database, version);
    set_indexed_db_transaction_store_names(scope, transaction, &names);
}

// Update native metadata only. Author properties and cached keyPath arrays
// are independent of schema changes, including deletion and rollback.
pub(in crate::context_bootstrap::indexed_db) fn sync_indexed_db_store_handles<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &IndexedDbName,
) {
    let metadata = indexed_db_database_store_metadata(scope, database, store_name);
    for table in indexed_db_handle_tables(scope, database) {
        for store in table.borrow_mut().object_stores.values_mut() {
            if store.deleted
                || &store.name != store_name
                || store.database.to_local(scope) != database
            {
                continue;
            }
            let metadata = match &metadata {
                Some(metadata) => metadata.clone(),
                None => {
                    store.deleted = true;
                    let mut metadata = store.metadata.clone();
                    metadata.info.index_names.clear();
                    metadata.indexes.clear();
                    metadata
                }
            };
            store.name = metadata.info.name.clone();
            store.metadata = metadata;
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn mark_indexed_db_index_handles_deleted(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
    store_name: &IndexedDbName,
    index_name: &IndexedDbName,
) {
    for table in indexed_db_handle_tables(scope, database) {
        let indexes = table
            .borrow()
            .indexes
            .iter()
            .filter_map(|(id, index)| {
                if &index.info.name != index_name {
                    return None;
                }
                let store =
                    v8::Local::<v8::Object>::try_from(index.object_store.to_local(scope)).ok()?;
                Some((*id, store))
            })
            .collect::<Vec<_>>();
        for (id, store) in indexes {
            if !indexed_db_object_store_is_deleted(scope, store)
                && indexed_db_object_store_name(scope, store).as_ref() == Some(store_name)
                && indexed_db_object_store_database(scope, store) == Some(database)
            {
                table
                    .borrow_mut()
                    .indexes
                    .get_mut(&id)
                    .expect("registered index remains present")
                    .deleted = true;
            }
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_object_store_is_deleted(
    scope: &mut v8::PinScope<'_, '_>,
    store: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(id) = indexed_db_typed_state_id(scope, store) else {
        return true;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, store);
    table
        .borrow()
        .object_stores
        .get(&id)
        .is_none_or(|store| store.deleted)
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_index_is_deleted(
    scope: &mut v8::PinScope<'_, '_>,
    index: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(id) = indexed_db_typed_state_id(scope, index) else {
        return true;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, index);
    let (deleted, store) = {
        let table = table.borrow();
        let Some(index) = table.indexes.get(&id) else {
            return true;
        };
        let store = v8::Local::<v8::Object>::try_from(index.object_store.to_local(scope))
            .expect("index retains its object store");
        (index.deleted, store)
    };
    deleted || indexed_db_object_store_is_deleted(scope, store)
}

// Identity is per object store handle; retaining the wrapper weakly avoids
// rooting a cycle from index to store to transaction to database.
pub(in crate::context_bootstrap::indexed_db) fn cached_indexed_db_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    name: &IndexedDbName,
) -> Option<v8::Local<'s, v8::Object>> {
    let table = indexed_db_runtime_state_table_for_object(scope, store);
    let table = table.borrow();
    table.indexes.values().find_map(|index| {
        if !index.deleted && &index.info.name == name && index.object_store.to_local(scope) == store
        {
            index.wrapper.to_local(scope)
        } else {
            None
        }
    })
}

pub(in crate::context_bootstrap::indexed_db) fn rename_indexed_db_index_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    old_name: &IndexedDbName,
    new_name: &IndexedDbName,
) {
    let database = indexed_db_object_store_database(scope, store).expect("index database");
    let database_id = indexed_db_typed_state_id(scope, database).expect("index database id");
    let transaction = indexed_db_object_store_transaction(scope, store).expect("index transaction");
    let store_name = indexed_db_object_store_name(scope, store).expect("index store name");
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    {
        let mut table = table.borrow_mut();
        table
            .databases
            .get_mut(&database_id)
            .expect("index database metadata")
            .metadata
            .get_mut(&store_name)
            .expect("index store metadata")
            .rename_index(old_name, new_name);
    }
    for table in indexed_db_handle_tables(scope, database) {
        let indexes = table
            .borrow()
            .indexes
            .iter()
            .filter_map(|(id, index)| {
                if index.deleted || &index.info.name != old_name {
                    return None;
                }
                let store =
                    v8::Local::<v8::Object>::try_from(index.object_store.to_local(scope)).ok()?;
                Some((*id, store))
            })
            .collect::<Vec<_>>();
        for (id, store) in indexes {
            if !indexed_db_object_store_is_deleted(scope, store)
                && indexed_db_object_store_name(scope, store).as_ref() == Some(&store_name)
                && indexed_db_object_store_transaction(scope, store) == Some(transaction)
            {
                table
                    .borrow_mut()
                    .indexes
                    .get_mut(&id)
                    .expect("registered index remains present")
                    .info
                    .name = new_name.clone();
            }
        }
    }
    sync_indexed_db_store_handles(scope, database, &store_name);
}

pub(in crate::context_bootstrap::indexed_db) fn cached_indexed_db_object_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    name: &IndexedDbName,
) -> Option<v8::Local<'s, v8::Object>> {
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let table = table.borrow();
    table.object_stores.values().find_map(|store| {
        if !store.deleted && &store.name == name && store.transaction.to_local(scope) == transaction
        {
            store.wrapper.to_local(scope)
        } else {
            None
        }
    })
}

pub(in crate::context_bootstrap::indexed_db) fn rename_indexed_db_store_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    old_name: &IndexedDbName,
    new_name: &IndexedDbName,
) {
    let database = indexed_db_object_store_database(scope, store).expect("store database");
    let database_id = indexed_db_typed_state_id(scope, database).expect("store database id");
    let transaction = indexed_db_object_store_transaction(scope, store).expect("store transaction");
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let metadata = {
        let mut table = table.borrow_mut();
        let database = table
            .databases
            .get_mut(&database_id)
            .expect("store database metadata");
        let mut metadata = database
            .metadata
            .remove(old_name)
            .expect("existing store metadata");
        metadata.info.name = new_name.clone();
        database.metadata.insert(new_name.clone(), metadata.clone());
        metadata
    };
    for table in indexed_db_handle_tables(scope, database) {
        // Replacements and handles retained from finished transactions are
        // separate objects, even if their last names happen to match.
        for store in table.borrow_mut().object_stores.values_mut() {
            if !store.deleted
                && &store.name == old_name
                && store.transaction.to_local(scope) == transaction
            {
                store.name = new_name.clone();
                store.metadata = metadata.clone();
            }
        }
    }
    sync_transaction_object_store_names_from_database(scope, transaction, database);
}
