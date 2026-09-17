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
        state.upgrade_open_request = Some(v8::Global::new(scope, request));
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
        v8::Local::new(scope, &request),
        v8::Local::new(scope, state.database.as_ref()?),
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
        v8::Local::new(scope, database)
    };
    let Some(id) = indexed_db_typed_state_id(scope, database) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let (version, names) = {
        let mut table = table.borrow_mut();
        let Some(state) = table.databases.get_mut(&id) else {
            return;
        };
        if !state
            .upgrade_transaction
            .as_ref()
            .is_some_and(|value| v8::Local::new(scope, value) == transaction)
        {
            return;
        }
        let Some(snapshot) = state.upgrade_metadata.take() else {
            return;
        };
        state.metadata = snapshot.stores.clone();
        let names = snapshot.stores.keys().cloned().collect::<Vec<_>>();
        let mut store_ids = BTreeSet::new();
        for (id, store) in &mut table.object_stores {
            if v8::Local::new(scope, &store.transaction) != transaction {
                continue;
            }
            store_ids.insert(*id);
            // A replacement with the same name is a different object store.
            // Only handles for pre-upgrade stores are restored from the snapshot.
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
        for index in table.indexes.values_mut() {
            let store =
                v8::Local::<v8::Object>::try_from(v8::Local::new(scope, &index.object_store))
                    .expect("index retains its object store");
            if indexed_db_typed_state_id(scope, store).is_some_and(|id| store_ids.contains(&id)) {
                index.deleted = index.original_name.is_none();
                if let Some(original_name) = &index.original_name {
                    index.info.name = original_name.clone();
                }
            }
        }
        (snapshot.version, names)
    };
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
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let mut table = table.borrow_mut();
    for store in table.object_stores.values_mut() {
        if store.deleted
            || &store.name != store_name
            || v8::Local::new(scope, &store.database) != database
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

pub(in crate::context_bootstrap::indexed_db) fn mark_indexed_db_index_handles_deleted(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
    store_name: &IndexedDbName,
    index_name: &IndexedDbName,
) {
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let mut table = table.borrow_mut();
    let store_ids = table
        .object_stores
        .iter()
        .filter_map(|(id, store)| {
            (!store.deleted
                && &store.name == store_name
                && v8::Local::new(scope, &store.database) == database)
                .then_some(*id)
        })
        .collect::<BTreeSet<_>>();
    for index in table.indexes.values_mut() {
        let store = v8::Local::<v8::Object>::try_from(v8::Local::new(scope, &index.object_store))
            .expect("index retains its object store");
        if &index.info.name == index_name
            && indexed_db_typed_state_id(scope, store).is_some_and(|id| store_ids.contains(&id))
        {
            index.deleted = true;
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
    let table = table.borrow();
    let Some(index) = table.indexes.get(&id) else {
        return true;
    };
    let store = v8::Local::<v8::Object>::try_from(v8::Local::new(scope, &index.object_store))
        .expect("index retains its object store");
    index.deleted
        || indexed_db_typed_state_id(scope, store)
            .and_then(|id| table.object_stores.get(&id))
            .is_none_or(|store| store.deleted)
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
        if !index.deleted
            && &index.info.name == name
            && v8::Local::new(scope, &index.object_store) == store
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
    let table = indexed_db_runtime_state_table_for_object(scope, store);
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
        let store_ids = table
            .object_stores
            .iter()
            .filter_map(|(id, store)| {
                (!store.deleted
                    && store.name == store_name
                    && v8::Local::new(scope, &store.transaction) == transaction)
                    .then_some(*id)
            })
            .collect::<BTreeSet<_>>();
        for index in table.indexes.values_mut() {
            let store =
                v8::Local::<v8::Object>::try_from(v8::Local::new(scope, &index.object_store))
                    .expect("index retains its object store");
            if !index.deleted
                && &index.info.name == old_name
                && indexed_db_typed_state_id(scope, store).is_some_and(|id| store_ids.contains(&id))
            {
                index.info.name = new_name.clone();
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
        if !store.deleted
            && &store.name == name
            && v8::Local::new(scope, &store.transaction) == transaction
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
    let table = indexed_db_runtime_state_table_for_object(scope, store);
    {
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
        // Replacements and handles retained from finished transactions are
        // separate objects, even if their last names happen to match.
        for store in table.object_stores.values_mut() {
            if !store.deleted
                && &store.name == old_name
                && v8::Local::new(scope, &store.transaction) == transaction
            {
                store.name = new_name.clone();
                store.metadata = metadata.clone();
            }
        }
    }
    sync_transaction_object_store_names_from_database(scope, transaction, database);
}
