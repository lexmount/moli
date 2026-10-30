use super::*;
use moli_indexeddb::sqlite::service::Command;

pub(in crate::context_bootstrap::indexed_db) fn rename_indexed_db_object_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    new_name: IndexedDbName,
) -> Result<(), IndexedDbError> {
    let (transaction, database, handle, name) = object_store_versionchange_common(scope, store)?;
    if name == new_name {
        return Ok(());
    }
    let client =
        sql::indexed_db_request_sql_client(scope, transaction).ok_or_else(missing_mirror)?;
    let mirror = indexed_db_database_mirror(scope, database).ok_or_else(missing_mirror)?;
    let (candidate, metadata) = {
        let mirror = mirror.borrow();
        let current = mirror.metadata.as_ref().ok_or_else(missing_mirror)?;
        if current.contains_key(&new_name) {
            return Err(IndexedDbError::Constraint(
                "The object store already exists".into(),
            ));
        }
        let metadata = current
            .get(&name)
            .ok_or_else(missing_mirror)?
            .with_name(&new_name);
        let candidate = edit_shared(current, |stores| {
            stores.remove(&name);
            stores.insert(new_name.clone(), metadata.clone());
        });
        (candidate, metadata)
    };
    // Allocate the immutable candidate first. Only publish it after the native
    // schema command has entered the same FIFO as preceding data requests.
    schedule_schema_operation(
        scope,
        transaction,
        client,
        Command::RenameStore {
            transaction: handle,
            name: name.clone(),
            new_name: new_name.clone(),
        },
        None,
    )?;
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
        .unwrap_or(false)
    {
        return Ok(());
    }
    mirror.borrow_mut().metadata = Some(candidate);
    for table in indexed_db_handle_tables(scope, database) {
        for store in table
            .borrow()
            .object_stores
            .values()
            .filter_map(std::rc::Weak::upgrade)
        {
            let mut store = store.borrow_mut();
            if !store.deleted
                && store.transaction.to_local(scope) == Some(transaction)
                && store.name() == &name
            {
                // Canonical handles exist before the first rename. Preserve
                // their original label once, without copying every readonly
                // handle's potentially large UTF-16 name at registration.
                store.original_name.get_or_insert_with(|| name.clone());
                store.metadata = metadata.clone();
            }
        }
    }
    core::rekey_object_store_handle_cache(scope, transaction, &name, &new_name);
    sync_transaction_scope_from_database(scope, transaction, database);
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn rename_indexed_db_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: v8::Local<'s, v8::Object>,
    new_name: IndexedDbName,
) -> Result<(), IndexedDbError> {
    if indexed_db_index_is_deleted(scope, index) {
        return Err(IndexedDbError::InvalidState(
            "The index has been deleted".into(),
        ));
    }
    let store = indexed_db_index_object_store(scope, index).ok_or_else(missing_mirror)?;
    let (transaction, database, handle, store_name) =
        object_store_versionchange_common(scope, store)?;
    let name = indexed_db_index_name(scope, index).ok_or_else(missing_mirror)?;
    if name == new_name {
        return Ok(());
    }
    let client =
        sql::indexed_db_request_sql_client(scope, transaction).ok_or_else(missing_mirror)?;
    let mirror = indexed_db_database_mirror(scope, database).ok_or_else(missing_mirror)?;
    let (candidate, info) = {
        let mirror = mirror.borrow();
        let current = mirror.metadata.as_ref().ok_or_else(missing_mirror)?;
        let metadata = current.get(&store_name).ok_or_else(missing_mirror)?;
        if metadata.index(&new_name).is_some() {
            return Err(IndexedDbError::Constraint(
                "The index already exists".into(),
            ));
        }
        let metadata = metadata.with_renamed_index(&name, &new_name);
        let info = metadata.shared_index(&new_name).expect("renamed index");
        let candidate = edit_shared(current, |stores| {
            *stores.get_mut(&store_name).expect("existing store") = metadata;
        });
        (candidate, info)
    };
    schedule_schema_operation(
        scope,
        transaction,
        client,
        Command::RenameIndex {
            transaction: handle,
            store: store_name.clone(),
            name: name.clone(),
            new_name: new_name.clone(),
        },
        None,
    )?;
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
        .unwrap_or(false)
    {
        return Ok(());
    }
    mirror.borrow_mut().metadata = Some(candidate);
    for table in indexed_db_handle_tables(scope, database) {
        // The parent store can live in another registration table. Release the
        // table borrow before resolving that store's native identity.
        let indexes = table
            .borrow()
            .indexes
            .values()
            .filter_map(std::rc::Weak::upgrade)
            .filter_map(|state| {
                let index = state.borrow();
                if index.deleted || index.info.name != name {
                    return None;
                }
                let store = index.object_store.to_local(scope)?;
                drop(index);
                Some((state, store))
            })
            .collect::<Vec<_>>();
        for (index, store) in indexes {
            if indexed_db_object_store_transaction(scope, store) == Some(transaction)
                && indexed_db_object_store_name(scope, store).as_ref() == Some(&store_name)
            {
                index.borrow_mut().info = Rc::clone(&info);
                core::rekey_index_handle_cache(scope, store, &name, &new_name);
            }
        }
    }
    sync_indexed_db_store_handles(scope, database, &store_name);
    Ok(())
}
