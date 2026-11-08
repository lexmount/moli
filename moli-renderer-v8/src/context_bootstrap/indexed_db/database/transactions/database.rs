use super::*;
use crate::context_bootstrap::indexed_db::schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint;
use crate::context_bootstrap::indexed_db::{
    INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT, INDEXED_DB_TRANSACTION_FINISHED_SLOT,
    object_store_info_from_database_metadata,
};
use crate::context_bootstrap::indexed_db::{
    indexed_db_database_sql_client, set_indexed_db_transaction_start_request,
};
use crate::webidl;
use moli_indexeddb::{DatabaseHandle, IndexedDbError};

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "IDBTransactionMode")]
enum TransactionModeWebIdl {
    Readonly,
    Readwrite,
    Versionchange,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBDatabase.transaction")]
struct IdbDatabaseTransactionArgs {
    #[webidl(required, name = "storeNames", converter = "raw")]
    store_names: names::TransactionStoreNames,
    #[webidl(converter = "enum", default = TransactionModeWebIdl::Readonly)]
    mode: TransactionModeWebIdl,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_database_transaction_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbDatabaseTransactionArgs>(scope, &args) else {
        return;
    };
    let database = args.this();
    let Some(handle) = database_handle_from_value(scope, database.into()) else {
        rv.set_undefined();
        return;
    };
    // A live upgrade includes its inactive and committing states. Checking the
    // connection's own transaction also works when this method is borrowed
    // from another realm.
    if let Some(upgrade) = object_property_as_object(
        scope,
        database,
        INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT,
    ) && !object_bool_property(scope, upgrade, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
        .unwrap_or(false)
    {
        let error = dom_exception_value(
            scope,
            "The database is running a version change transaction.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return;
    }
    if object_bool_property(scope, database, INDEXED_DB_DATABASE_CLOSED_SLOT).unwrap_or(false) {
        let error = dom_exception_value(
            scope,
            "The database connection is closing.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return;
    }
    let mut store_names = parsed.store_names.0;
    store_names.sort_unstable();
    store_names.dedup();
    // Validate before queuing a readwrite transaction as well: its backend
    // handle may only be allocated after an earlier transaction finishes.
    if store_names
        .iter()
        .any(|name| object_store_info_from_database_metadata(scope, database, name).is_none())
    {
        let error = dom_exception_value(
            scope,
            "An object store in the transaction scope was not found.",
            "NotFoundError",
        );
        scope.throw_exception(error);
        return;
    }
    if store_names.is_empty() {
        let error = dom_exception_value(
            scope,
            "The transaction scope is empty.",
            "InvalidAccessError",
        );
        scope.throw_exception(error);
        return;
    }
    let mode = match parsed.mode {
        TransactionModeWebIdl::Readonly => TransactionMode::ReadOnly,
        TransactionModeWebIdl::Readwrite => TransactionMode::ReadWrite,
        // This is a valid IDL enum value, so its rejection comes after the
        // connection and scope checks, unlike an unrecognized enum string.
        TransactionModeWebIdl::Versionchange => {
            throw_type_error(scope, "Failed to execute 'transaction': unsupported mode.");
            return;
        }
    };
    let Some(context) = database.get_creation_context(scope) else {
        return;
    };
    let result = {
        let scope = &mut v8::ContextScope::new(scope, context);
        create_regular_transaction(scope, database, handle, &store_names, mode)
    };
    match result {
        Ok(Some(transaction)) => rv.set(transaction.into()),
        Ok(None) => rv.set_undefined(),
        Err(error) => {
            let error = request_error_object(scope, &error);
            scope.throw_exception(error);
        }
    }
}

fn create_regular_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    handle: DatabaseHandle,
    store_names: &[moli_indexeddb::IndexedDbName],
    mode: TransactionMode,
) -> Result<Option<v8::Local<'s, v8::Object>>, IndexedDbError> {
    // Objects and task queues belong to the connection's realm even when the
    // operation was borrowed from another Window. Conversion and exceptions
    // remain in the callback's realm.
    let owner =
        crate::context_bootstrap::indexed_db::indexed_db_typed_execution_owner(scope, database)
            .expect("IDB database owner");
    let wake =
        crate::context_bootstrap::indexed_db::indexed_db_transaction_start_wake(scope, owner);
    let client = indexed_db_database_sql_client(scope, database).ok_or_else(|| {
        IndexedDbError::InvalidState("IndexedDB connection is unavailable".into())
    })?;
    let transaction_handle = client.allocate_transaction_handle()?;
    let request = if let Some(connection) =
        crate::context_bootstrap::indexed_db::indexed_db_database_sql_connection(scope, database)
    {
        connection.queue_transaction_start(store_names, mode, wake)?
    } else {
        client.queue_transaction_start(handle, store_names, mode, wake)?
    };
    let Some(transaction) = create_transaction_object(scope, database, None, mode, store_names)
    else {
        return Ok(None);
    };
    crate::context_bootstrap::indexed_db::typed_state::sql::configure_sql_transaction(
        scope,
        transaction,
        client,
        transaction_handle,
    )?;
    set_indexed_db_transaction_start_request(scope, transaction, request);
    register_regular_transaction(scope, transaction);
    enqueue_ready_transaction_starts(scope);
    schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(scope, transaction);
    Ok(Some(transaction))
}

pub(in crate::context_bootstrap::indexed_db) fn idb_database_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let database = args.this();
    let Some(context) = database.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    close_indexed_db_database_connection(scope, database);
    rv.set_undefined();
}

pub(in crate::context_bootstrap::indexed_db) fn close_indexed_db_database_connection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
) {
    if let Some(connection) =
        crate::context_bootstrap::indexed_db::indexed_db_database_sql_connection(scope, database)
    {
        connection.mark_close_pending();
    }
    set_indexed_db_slot_value(
        scope,
        database,
        INDEXED_DB_DATABASE_CLOSED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    finish_indexed_db_database_close(scope, database);
}

pub(in crate::context_bootstrap::indexed_db) fn finish_indexed_db_database_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
) {
    if crate::context_bootstrap::indexed_db::indexed_db_database_is_forced_closed(scope, database) {
        crate::context_bootstrap::indexed_db::enqueue_database_forced_close_task(scope, database);
        return;
    }
    if !object_bool_property(scope, database, INDEXED_DB_DATABASE_CLOSED_SLOT).unwrap_or(false)
        || crate::context_bootstrap::indexed_db::indexed_db_database_has_unfinished_transactions(
            scope, database,
        )
    {
        return;
    }
    let Some(handle) = database_handle_from_value(scope, database.into()) else {
        return;
    };
    if let Some(connection) =
        crate::context_bootstrap::indexed_db::indexed_db_database_sql_connection(scope, database)
    {
        // Native close keeps the connection as a blocker until its worker has
        // verified that every accepted transaction has drained.
        let _ = connection.close(|_| {});
    }
    if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &*host }.unregister_indexed_db_open_connection(handle);
    }
}
