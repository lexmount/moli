use super::*;
use crate::context_bootstrap::indexed_db::schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint;
use crate::webidl;
use moli_indexeddb::{DatabaseHandle, IndexedDbError, parse_regular_transaction_mode};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBDatabase.transaction")]
struct IdbDatabaseTransactionArgs<'s> {
    #[webidl(required, name = "storeNames")]
    store_names: v8::Local<'s, v8::Value>,
    mode: Option<String>,
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
    let store_names = match names::parse_transaction_store_names(scope, parsed.store_names) {
        Ok(store_names) => store_names,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if object_bool_property(scope, database, INDEXED_DB_DATABASE_CLOSED_SLOT).unwrap_or(false) {
        let error = dom_exception_value(
            scope,
            "The database connection is closing.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return;
    }
    let mode = match parse_regular_transaction_mode(parsed.mode.as_deref()) {
        Ok(mode) => mode,
        Err(_) => {
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
    store_names: &[String],
    mode: TransactionMode,
) -> Result<Option<v8::Local<'s, v8::Object>>, IndexedDbError> {
    // Objects and task queues belong to the connection's realm even when the
    // operation was borrowed from another Window. Conversion and exceptions
    // remain in the callback's realm.
    if mode == TransactionMode::ReadWrite {
        let db_key = object_string_property(scope, database, INDEXED_DB_DATABASE_KEY_SLOT)
            .unwrap_or_default();
        let transaction_handle = if has_unfinished_readwrite_transaction_for_db(scope, &db_key) {
            None
        } else {
            Some(with_indexed_db_manager(scope, |manager| {
                manager.begin_transaction(handle, store_names, mode)
            })?)
        };
        let Some(transaction) =
            create_transaction_object(scope, database, transaction_handle, mode, store_names)
        else {
            return Ok(None);
        };
        register_readwrite_transaction(scope, transaction);
        schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(scope, transaction);
        return Ok(Some(transaction));
    }
    let transaction_handle = with_indexed_db_manager(scope, |manager| {
        manager.begin_transaction(handle, store_names, mode)
    })?;
    let Some(transaction) =
        create_transaction_object(scope, database, Some(transaction_handle), mode, store_names)
    else {
        return Ok(None);
    };
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
