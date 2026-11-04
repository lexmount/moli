use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBTransaction.objectStore")]
struct IdbTransactionObjectStoreArgs {
    #[webidl(required, converter = "raw")]
    name: webidl::DomString16,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_transaction_object_store_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(transaction) = idb_transaction_receiver(scope, &args) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<IdbTransactionObjectStoreArgs>(scope, &args) else {
        return;
    };
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
        .unwrap_or(true)
        || object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
            .unwrap_or(true)
    {
        let error =
            dom_exception_value(scope, "The transaction has finished.", "InvalidStateError");
        scope.throw_exception(error);
        return;
    }
    let name = moli_indexeddb::IndexedDbName::from_utf16(parsed.name.0);
    let Some(database) =
        crate::context_bootstrap::indexed_db::indexed_db_transaction_database(scope, transaction)
    else {
        rv.set_undefined();
        return;
    };
    let names = crate::context_bootstrap::indexed_db::indexed_db_transaction_store_names(
        scope,
        transaction,
    )
    .unwrap_or_default();
    if !names.contains(&name) {
        let error = dom_exception_value(
            scope,
            "The object store is outside the transaction scope.",
            "NotFoundError",
        );
        scope.throw_exception(error);
        return;
    }
    let Some(info) = object_store_info_from_database_metadata(scope, database, &name) else {
        let error = dom_exception_value(
            scope,
            "The requested object store was not found.",
            "NotFoundError",
        );
        scope.throw_exception(error);
        return;
    };
    if let Some(store) = create_object_store_object(scope, database, transaction, &info) {
        rv.set(store.into());
    } else {
        rv.set_undefined();
    }
}
