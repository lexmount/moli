use super::*;
use crate::context_bootstrap::indexed_db::{
    IdbTransactionDurability, indexed_db_transaction_start_wake,
    schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint,
    set_indexed_db_transaction_start_request,
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
    #[webidl(index = 2, with = parse_transaction_options_arg)]
    options: IdbTransactionOptions,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "IDBTransactionOptions")]
struct IdbTransactionOptions {
    #[webidl(converter = "enum", default = IdbTransactionDurability::Default)]
    durability: IdbTransactionDurability,
}

fn parse_transaction_options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<IdbTransactionOptions, webidl::WebIdlError> {
    let context = webidl::Context::argument("IDBDatabase.transaction", (index + 1) as usize);
    webidl::dictionary_arg(args, index, context)?
        .map(|object| webidl::parse_dictionary_object(scope, object))
        .transpose()
        .map(|options| options.unwrap_or_default())
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
    // Validate before queuing a transaction as well: its backend
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
        create_regular_transaction(
            scope,
            database,
            handle,
            &store_names,
            mode,
            parsed.options.durability,
        )
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
    store_names: &[IndexedDbName],
    mode: TransactionMode,
    durability: IdbTransactionDurability,
) -> Result<Option<v8::Local<'s, v8::Object>>, IndexedDbError> {
    // Objects and task queues belong to the connection's realm even when the
    // operation was borrowed from another Window. Conversion and exceptions
    // remain in the callback's realm.
    let owner = indexed_db_typed_execution_owner(scope, database).expect("IDB database owner");
    let wake = indexed_db_transaction_start_wake(scope, owner);
    let request = with_indexed_db_manager(scope, |manager| {
        manager.queue_transaction_start(handle, store_names, mode, wake)
    })?;
    let transaction_handle = if request.handle().is_ready() {
        Some(with_indexed_db_manager(scope, |manager| {
            manager.start_queued_transaction(request.handle())
        })?)
    } else {
        None
    };
    let Some(transaction) = create_transaction_object(
        scope,
        database,
        transaction_handle,
        mode,
        durability,
        store_names,
    ) else {
        if let Some(handle) = transaction_handle {
            let _ = with_indexed_db_manager(scope, |manager| manager.abort_transaction(handle));
        }
        return Ok(None);
    };
    set_indexed_db_transaction_start_request(scope, transaction, request);
    register_regular_transaction(scope, transaction);
    schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(scope, transaction);
    Ok(Some(transaction))
}

pub(in crate::context_bootstrap::indexed_db) fn idb_database_close_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let database = args.this();
    let Some(context) = database.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    close_indexed_db_database_connection(scope, database);
    rv.set_undefined();
}

pub(in crate::context_bootstrap::indexed_db) fn close_indexed_db_database_connection(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) {
    if object_bool_property(scope, database, INDEXED_DB_DATABASE_CLOSED_SLOT).unwrap_or(false) {
        return;
    }
    set_indexed_db_slot_value(
        scope,
        database,
        INDEXED_DB_DATABASE_CLOSED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    if let Some(handle) = database_handle_from_value(scope, database.into())
        && let Ok(manager) = crate::context_bootstrap::indexed_db::indexed_db_shared_manager(scope)
    {
        manager
            .lock()
            .connection_notifications()
            .mark_close_pending(handle);
    }
    finish_indexed_db_database_close(scope, database);
}

pub(in crate::context_bootstrap::indexed_db) fn finish_indexed_db_database_close(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) {
    if !object_bool_property(scope, database, INDEXED_DB_DATABASE_CLOSED_SLOT).unwrap_or(false) {
        return;
    }
    // Accepted transactions can still be waiting for their backend start. Keep
    // the native connection available to them while the DOM close flag rejects
    // creation of any new transactions.
    if crate::context_bootstrap::indexed_db::indexed_db_database_has_unfinished_transactions(
        scope, database,
    ) {
        return;
    }
    let Some(handle) = database_handle_from_value(scope, database.into()) else {
        return;
    };
    let Ok(manager) = crate::context_bootstrap::indexed_db::indexed_db_shared_manager(scope) else {
        return;
    };
    let _ = manager.lock().close_database(handle);
    if manager.lock().connection_notifications().contains(handle)
        || crate::context_bootstrap::indexed_db::database_connection_for_handle(scope, handle)
            .is_none()
    {
        return;
    }
    let coordinated = unregister_open_database_connection(scope, handle, database);
    if !coordinated {
        enqueue_drain_blocked_open_requests_task(scope);
    }
}
