use super::common::version_change_transaction;
use super::*;
use crate::context_bootstrap::indexed_db::{
    create_failed_object_store_object, schedule_schema_failure, schedule_schema_operation,
    typed_state::sql::indexed_db_request_sql_client,
};
use crate::webidl;
use moli_indexeddb::sqlite::service::Command;
use moli_indexeddb::{ObjectStoreOptionsValidationError, validate_object_store_options};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBDatabase.createObjectStore")]
struct IdbDatabaseCreateObjectStoreArgs {
    #[webidl(required, converter = "raw")]
    name: webidl::DomString16,
    #[webidl(index = 1, dictionary)]
    options: IdbObjectStoreParameters,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "IDBObjectStoreParameters")]
struct IdbObjectStoreParameters {
    #[webidl(with = parse_optional_idb_key_path_member)]
    key_path: Option<KeyPath>,
    #[webidl(default = false)]
    auto_increment: bool,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_database_create_object_store_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbDatabaseCreateObjectStoreArgs>(scope, &args) else {
        return;
    };
    let name = moli_indexeddb::IndexedDbName::from_utf16(parsed.name.0);
    let key_path = parsed.options.key_path;
    let auto_increment = parsed.options.auto_increment;
    let database = args.this();
    let Some(transaction) = version_change_transaction(scope, database) else {
        return;
    };
    if key_path.as_ref().is_some_and(|path| !path.is_valid()) {
        let error = dom_exception_value(scope, "The keyPath is not valid.", "SyntaxError");
        scope.throw_exception(error);
        return;
    }
    let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
        rv.set_undefined();
        return;
    };
    let info = ObjectStoreInfo {
        name: name.clone(),
        key_path: key_path.clone(),
        auto_increment,
        index_names: Vec::new(),
    };
    let Some(client) = indexed_db_request_sql_client(scope, transaction) else {
        return;
    };
    let result = if object_store_info_from_database_metadata(scope, database, &name).is_some() {
        Err(moli_indexeddb::IndexedDbError::Constraint(
            "The object store already exists".into(),
        ))
    } else if let Err(error) = validate_object_store_options(key_path.as_ref(), auto_increment) {
        Err(moli_indexeddb::IndexedDbError::InvalidAccess(
            create_object_store_options_error_message(error).into(),
        ))
    } else {
        schedule_schema_operation(
            scope,
            transaction,
            client,
            Command::CreateStore {
                transaction: handle,
                name,
                options: ObjectStoreOptions {
                    key_path,
                    auto_increment,
                },
            },
            None,
        )
    };
    match result {
        Ok(()) => {
            if let Err(error) = set_database_store_metadata(scope, database, &info) {
                schedule_schema_failure(scope, transaction, error);
                if let Some(store) =
                    create_failed_object_store_object(scope, database, transaction, &info)
                {
                    rv.set(store.into());
                }
                return;
            }
            sync_transaction_object_store_names_from_database(scope, transaction, database);
            if let Some(store) = create_object_store_object(scope, database, transaction, &info) {
                rv.set(store.into());
            } else {
                rv.set_undefined();
            }
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            scope.throw_exception(error);
        }
    }
}

fn create_object_store_options_error_message(
    error: ObjectStoreOptionsValidationError,
) -> &'static str {
    match error {
        ObjectStoreOptionsValidationError::AutoIncrementEmptyKeyPath => {
            "Failed to execute 'createObjectStore': autoIncrement cannot be used with an empty keyPath."
        }
        ObjectStoreOptionsValidationError::AutoIncrementSequenceKeyPath => {
            "Failed to execute 'createObjectStore': autoIncrement cannot be used with a sequence keyPath."
        }
    }
}
