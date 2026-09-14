use super::*;
use crate::context_bootstrap::indexed_db::schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint;
use crate::webidl;
use moli_indexeddb::parse_regular_transaction_mode;

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
    let mode = match parse_regular_transaction_mode(parsed.mode.as_deref()) {
        Ok(mode) => mode,
        Err(_) => {
            throw_type_error(scope, "Failed to execute 'transaction': unsupported mode.");
            return;
        }
    };
    if mode == TransactionMode::ReadWrite {
        let db_key = object_string_property(scope, database, INDEXED_DB_DATABASE_KEY_SLOT)
            .unwrap_or_default();
        if !has_unfinished_readwrite_transaction_for_db(scope, &db_key) {
            match with_indexed_db_manager(scope, |manager| {
                manager.begin_transaction(handle, &store_names, mode)
            }) {
                Ok(transaction_handle) => {
                    let Some(transaction) = create_transaction_object(
                        scope,
                        database,
                        Some(transaction_handle),
                        mode,
                        &store_names,
                    ) else {
                        rv.set_undefined();
                        return;
                    };
                    register_readwrite_transaction(scope, transaction);
                    schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(
                        scope,
                        transaction,
                    );
                    rv.set(transaction.into());
                }
                Err(error) => {
                    let error = request_error_object(scope, &error);
                    scope.throw_exception(error);
                }
            }
            return;
        }
        let Some(transaction) =
            create_transaction_object(scope, database, None, mode, &store_names)
        else {
            rv.set_undefined();
            return;
        };
        register_readwrite_transaction(scope, transaction);
        schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(scope, transaction);
        rv.set(transaction.into());
        return;
    }
    match with_indexed_db_manager(scope, |manager| {
        manager.begin_transaction(handle, &store_names, mode)
    }) {
        Ok(transaction_handle) => {
            let Some(transaction) = create_transaction_object(
                scope,
                database,
                Some(transaction_handle),
                mode,
                &store_names,
            ) else {
                rv.set_undefined();
                return;
            };
            schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(
                scope,
                transaction,
            );
            rv.set(transaction.into());
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            scope.throw_exception(error);
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_database_close_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let database = args.this();
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
    let coordinated = database_handle_from_value(scope, database.into())
        .is_some_and(|handle| unregister_open_database_connection(scope, handle, database));
    if let Some(handle) = database_handle_from_value(scope, database.into()) {
        let _ = with_indexed_db_manager(scope, |manager| manager.close_database(handle));
    }
    if !coordinated
        && let Some(key) = object_string_property(scope, database, INDEXED_DB_DATABASE_KEY_SLOT)
    {
        wake_connection_requests(scope, &key);
    }
}
