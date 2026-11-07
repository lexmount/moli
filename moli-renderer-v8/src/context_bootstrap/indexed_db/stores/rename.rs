use super::*;
use crate::context_bootstrap::indexed_db::{
    indexed_db_index_is_deleted, indexed_db_transaction_mode,
    object_store_info_from_database_metadata, rename_indexed_db_index,
    rename_indexed_db_object_store, schedule_schema_failure, schedule_schema_operation,
    typed_state::sql::indexed_db_request_sql_client,
};
use moli_indexeddb::{IndexedDbError, IndexedDbName, TransactionMode, sqlite::service::Command};

fn name_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    interface: &'static str,
) -> Option<IndexedDbName> {
    match webidl::convert::<webidl::DomString16>(
        scope,
        args.get(0),
        webidl::Context::member(interface, "name"),
    ) {
        Ok(value) => Some(IndexedDbName::from_utf16(value.0)),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_name_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(new_name) = name_value(scope, &args, "IDBObjectStore") else {
        return;
    };
    let result = (|| {
        let (transaction, database, handle, name) =
            object_store_versionchange_common(scope, args.this())?;
        if name == new_name {
            return Ok(());
        }
        if object_store_info_from_database_metadata(scope, database, &new_name).is_some() {
            return Err(IndexedDbError::Constraint(
                "The object store already exists".into(),
            ));
        }
        let client = indexed_db_request_sql_client(scope, transaction)
            .ok_or_else(|| IndexedDbError::InvalidState("The transaction is unavailable".into()))?;
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
        if let Err(error) =
            rename_indexed_db_object_store(scope, database, transaction, &name, &new_name)
        {
            schedule_schema_failure(scope, transaction, error);
        }
        Ok(())
    })();
    if let Err(error) = result {
        let error = request_error_object(scope, &error);
        scope.throw_exception(error);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_index_name_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(new_name) = name_value(scope, &args, "IDBIndex") else {
        return;
    };
    let result = (|| {
        let invalid = || IndexedDbError::InvalidState("The index is unavailable".into());
        let index = args.this();
        let store = indexed_db_index_object_store(scope, index).ok_or_else(invalid)?;
        let transaction = indexed_db_object_store_transaction(scope, store).ok_or_else(invalid)?;
        // Index.name checks transaction mode and activity before deletion;
        // ObjectStore.name deliberately checks deletion first.
        if indexed_db_transaction_mode(scope, transaction) != Some(TransactionMode::VersionChange) {
            return Err(invalid());
        }
        if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
            .unwrap_or(false)
        {
            return Err(IndexedDbError::TransactionInactive(
                "The upgrade is not active".into(),
            ));
        }
        if indexed_db_index_is_deleted(scope, index) {
            return Err(invalid());
        }
        let info = indexed_db_index_info(scope, index).ok_or_else(invalid)?;
        if info.name == new_name {
            return Ok(());
        }
        if index_info_from_store_metadata(scope, store, &new_name).is_some() {
            return Err(IndexedDbError::Constraint(
                "The index already exists".into(),
            ));
        }
        let database = indexed_db_object_store_database(scope, store).ok_or_else(invalid)?;
        let store_name = indexed_db_object_store_name(scope, store).ok_or_else(invalid)?;
        let handle =
            transaction_handle_from_value(scope, transaction.into()).ok_or_else(invalid)?;
        let client = indexed_db_request_sql_client(scope, transaction).ok_or_else(invalid)?;
        schedule_schema_operation(
            scope,
            transaction,
            client,
            Command::RenameIndex {
                transaction: handle,
                store: store_name.clone(),
                name: info.name.clone(),
                new_name: new_name.clone(),
            },
            None,
        )?;
        if let Err(error) = rename_indexed_db_index(
            scope,
            database,
            transaction,
            &store_name,
            &info.name,
            &new_name,
        ) {
            schedule_schema_failure(scope, transaction, error);
        }
        Ok(())
    })();
    if let Err(error) = result {
        let error = request_error_object(scope, &error);
        scope.throw_exception(error);
    }
}
