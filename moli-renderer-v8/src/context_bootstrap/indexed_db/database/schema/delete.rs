use super::common::version_change_transaction;
use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBDatabase.deleteObjectStore")]
struct IdbDatabaseDeleteObjectStoreArgs {
    #[webidl(required, with = parse_store_name_arg)]
    name: IndexedDbName,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_database_delete_object_store_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbDatabaseDeleteObjectStoreArgs>(scope, &args) else {
        return;
    };
    let name = parsed.name;
    let database = args.this();
    let Some(transaction) = version_change_transaction(scope, database) else {
        return;
    };
    let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
        rv.set_undefined();
        return;
    };
    match with_indexed_db_manager(scope, |manager| manager.delete_object_store(handle, &name)) {
        Ok(()) => {
            let _ = remove_database_store_metadata(scope, database, &name);
            sync_transaction_object_store_names_from_database(scope, transaction, database);
            rv.set_undefined();
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            scope.throw_exception(error);
        }
    }
}

fn parse_store_name_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<IndexedDbName, webidl::WebIdlError> {
    webidl::convert::<webidl::DomString16>(
        scope,
        args.get(index),
        webidl::Context::argument("IDBDatabase.deleteObjectStore", (index + 1) as usize),
    )
    .map(|name| IndexedDbName::from_utf16(name.0))
}
