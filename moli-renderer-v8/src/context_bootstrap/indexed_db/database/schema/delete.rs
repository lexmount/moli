use super::common::version_change_transaction;
use super::*;
use crate::context_bootstrap::indexed_db::{
    schedule_schema_failure, schedule_schema_operation,
    typed_state::sql::indexed_db_request_sql_client,
};
use crate::webidl;
use moli_indexeddb::sqlite::service::Command;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBDatabase.deleteObjectStore")]
struct IdbDatabaseDeleteObjectStoreArgs {
    #[webidl(required, converter = "raw")]
    name: webidl::DomString16,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_database_delete_object_store_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbDatabaseDeleteObjectStoreArgs>(scope, &args) else {
        return;
    };
    let name = moli_indexeddb::IndexedDbName::from_utf16(parsed.name.0);
    let database = args.this();
    let Some(transaction) = version_change_transaction(scope, database) else {
        return;
    };
    let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
        rv.set_undefined();
        return;
    };
    let Some(client) = indexed_db_request_sql_client(scope, transaction) else {
        return;
    };
    let metadata = crate::context_bootstrap::indexed_db::indexed_db_database_store_metadata(
        scope, database, &name,
    );
    let result = if let Some(metadata) = metadata {
        schedule_schema_operation(
            scope,
            transaction,
            client,
            Command::DeleteStore {
                transaction: handle,
                store: metadata.id(),
            },
            None,
        )
    } else {
        Err(moli_indexeddb::IndexedDbError::NotFound(
            "The object store was not found".into(),
        ))
    };
    match result {
        Ok(()) => {
            if let Err(error) = remove_database_store_metadata(scope, database, &name) {
                schedule_schema_failure(scope, transaction, error);
                return;
            }
            sync_transaction_object_store_names_from_database(scope, transaction, database);
            rv.set_undefined();
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            scope.throw_exception(error);
        }
    }
}
