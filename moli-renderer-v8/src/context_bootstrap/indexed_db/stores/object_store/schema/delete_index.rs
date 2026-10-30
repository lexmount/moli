use super::*;
use crate::context_bootstrap::indexed_db::{
    schedule_schema_failure, schedule_schema_operation,
    typed_state::sql::indexed_db_request_sql_client,
};
use crate::webidl;
use moli_indexeddb::sqlite::service::Command;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBObjectStore.deleteIndex")]
struct IdbObjectStoreDeleteIndexArgs {
    #[webidl(required)]
    index_name: String,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_delete_index_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbObjectStoreDeleteIndexArgs>(scope, &args) else {
        return;
    };
    let index_name = parsed.index_name;
    let store = args.this();
    let (transaction, database, handle, store_name) =
        match object_store_versionchange_common(scope, store) {
            Ok(values) => values,
            Err(error) => {
                let error = request_error_object(scope, &error);
                scope.throw_exception(error);
                return;
            }
        };
    let Some(client) = indexed_db_request_sql_client(scope, transaction) else {
        return;
    };
    let result = if index_info_from_store_metadata(scope, store, &index_name).is_none() {
        Err(moli_indexeddb::IndexedDbError::NotFound(
            "The index was not found".into(),
        ))
    } else {
        schedule_schema_operation(
            scope,
            transaction,
            client,
            Command::DeleteIndex {
                transaction: handle,
                store: store_name.clone(),
                name: index_name.clone(),
            },
            None,
        )
    };
    match result {
        Ok(()) => {
            if let Err(error) =
                remove_database_index_metadata(scope, database, &store_name, &index_name)
            {
                schedule_schema_failure(scope, transaction, error);
                return;
            }
            rv.set_undefined();
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            scope.throw_exception(error);
        }
    }
}
