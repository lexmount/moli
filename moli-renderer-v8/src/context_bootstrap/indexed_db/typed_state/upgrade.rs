use super::*;

pub(super) struct IndexedDbUpgradeMetadata {
    version: u64,
    stores: BTreeMap<String, IndexedDbObjectStoreMetadata>,
}

pub(in crate::context_bootstrap::indexed_db) fn defer_indexed_db_aborted_open<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, transaction) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    if let Some(state) = table.borrow_mut().transactions.get_mut(&id) {
        state.aborted_open_request = Some(v8::Global::new(scope, request));
    }
}

pub(in crate::context_bootstrap::indexed_db) fn take_indexed_db_aborted_open<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    let id = indexed_db_typed_state_id(scope, transaction)?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    let mut table = table.borrow_mut();
    let state = table.transactions.get_mut(&id)?;
    let request = state.aborted_open_request.take()?;
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
    let (version, names, stores) = {
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
        let mut stores = Vec::new();
        let mut store_ids = BTreeSet::new();
        for (id, store) in &mut table.object_stores {
            if v8::Local::new(scope, &store.transaction) != transaction {
                continue;
            }
            store_ids.insert(*id);
            // A replacement with the same name is a different object store.
            // Only handles for pre-upgrade stores are restored from the snapshot.
            store.deleted = store.metadata.created_in_upgrade;
            let metadata = if store.deleted {
                let mut metadata = store.metadata.clone();
                metadata.info.index_names.clear();
                metadata.indexes.clear();
                metadata
            } else {
                snapshot
                    .stores
                    .get(&store.name)
                    .cloned()
                    .expect("existing store has upgrade snapshot")
            };
            store.metadata = metadata.clone();
            if let Some(wrapper) = store.wrapper.to_local(scope) {
                stores.push((wrapper, metadata));
            }
        }
        for index in table.indexes.values_mut() {
            let store =
                v8::Local::<v8::Object>::try_from(v8::Local::new(scope, &index.object_store))
                    .expect("index retains its object store");
            if indexed_db_typed_state_id(scope, store).is_some_and(|id| store_ids.contains(&id)) {
                index.deleted = index.created_in_upgrade;
            }
        }
        (snapshot.version, names, stores)
    };
    let version = v8::Number::new(scope, version as f64);
    let _ = database.define_own_property(
        scope,
        v8str(scope, "version").into(),
        version.into(),
        v8::PropertyAttribute::NONE,
    );
    let database_names = new_idb_name_list(scope, &names);
    let transaction_names = new_idb_name_list(scope, &names);
    let _ = database.define_own_property(
        scope,
        v8str(scope, "objectStoreNames").into(),
        database_names.into(),
        v8::PropertyAttribute::NONE,
    );
    let _ = transaction.define_own_property(
        scope,
        v8str(scope, "objectStoreNames").into(),
        transaction_names.into(),
        v8::PropertyAttribute::NONE,
    );
    for (store, metadata) in stores {
        let _ = sync_store_surface_from_metadata(scope, store, metadata);
    }
}

// Update every handle for this store, including separately obtained wrappers.
// V8 surface writes happen after releasing the runtime table's borrow.
pub(in crate::context_bootstrap::indexed_db) fn sync_indexed_db_store_handles<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    store_name: &str,
) {
    let metadata = indexed_db_database_store_metadata(scope, database, store_name);
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let stores = {
        let mut table = table.borrow_mut();
        let mut stores = Vec::new();
        for store in table.object_stores.values_mut() {
            if store.deleted
                || store.name != store_name
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
            store.metadata = metadata.clone();
            if let Some(wrapper) = store.wrapper.to_local(scope) {
                stores.push((wrapper, metadata));
            }
        }
        stores
    };
    for (store, metadata) in stores {
        let _ = sync_store_surface_from_metadata(scope, store, metadata);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn mark_indexed_db_index_handles_deleted(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
    store_name: &str,
    index_name: &str,
) {
    let table = indexed_db_runtime_state_table_for_object(scope, database);
    let mut table = table.borrow_mut();
    let store_ids = table
        .object_stores
        .iter()
        .filter_map(|(id, store)| {
            (!store.deleted
                && store.name == store_name
                && v8::Local::new(scope, &store.database) == database)
                .then_some(*id)
        })
        .collect::<BTreeSet<_>>();
    for index in table.indexes.values_mut() {
        let store = v8::Local::<v8::Object>::try_from(v8::Local::new(scope, &index.object_store))
            .expect("index retains its object store");
        if index.info.name == index_name
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
