use super::*;
use crate::context_bootstrap::indexed_db::{
    indexed_db_transaction_contains_store, indexed_db_transaction_database,
};
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBTransaction.objectStore")]
struct IdbTransactionObjectStoreArgs {
    #[webidl(required, with = parse_store_name_arg)]
    name: IndexedDbName,
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
        .unwrap_or(false)
    {
        let error =
            dom_exception_value(scope, "The transaction has finished.", "InvalidStateError");
        scope.throw_exception(error);
        return;
    }
    let name = parsed.name;
    let Some(database) = indexed_db_transaction_database(scope, transaction) else {
        rv.set_undefined();
        return;
    };
    let info = indexed_db_transaction_contains_store(scope, transaction, &name)
        .then(|| object_store_info_from_database_metadata(scope, database, &name))
        .flatten();
    let Some(info) = info else {
        let error = dom_exception_value(
            scope,
            "The requested object store was not found.",
            "NotFoundError",
        );
        scope.throw_exception(error);
        return;
    };
    let Some(context) = transaction.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Some(store) = create_object_store_object(scope, database, transaction, &info) {
        rv.set(store.into());
    } else {
        rv.set_undefined();
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
        webidl::Context::argument("IDBTransaction.objectStore", (index + 1) as usize),
    )
    .map(|name| IndexedDbName::from_utf16(name.0))
}
