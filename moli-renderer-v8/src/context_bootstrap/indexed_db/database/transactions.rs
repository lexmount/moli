use super::*;

mod database;
mod lifecycle;
mod names;
mod object_store;

fn idb_transaction_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let transaction = args.this();
    if indexed_db_typed_wrapper_is(scope, transaction, IndexedDbWrapperKind::Transaction) {
        return Some(transaction);
    }
    throw_type_error(scope, "Illegal invocation");
    None
}

pub(in crate::context_bootstrap::indexed_db) use self::database::{
    close_indexed_db_database_connection, idb_database_close_callback,
    idb_database_transaction_callback,
};
pub(in crate::context_bootstrap::indexed_db) use self::lifecycle::{
    finish_transaction_abort, idb_transaction_abort_callback, idb_transaction_commit_callback,
};
pub(in crate::context_bootstrap::indexed_db) use self::object_store::idb_transaction_object_store_callback;
