use super::*;
use crate::context_bootstrap::indexed_db::indexed_db_transaction_mode;
use moli_indexeddb::TransactionMode;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBObjectStore.delete")]
struct IdbObjectStoreDeleteArgs<'s> {
    #[webidl(required, converter = "raw")]
    key: v8::Local<'s, v8::Value>,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_delete_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbObjectStoreDeleteArgs<'s>>(scope, &args) else {
        return;
    };
    let store = args.this();
    if crate::context_bootstrap::indexed_db::indexed_db_object_store_is_deleted(scope, store) {
        let error = dom_exception_value(scope, "The object store has been deleted.", "InvalidStateError");
        scope.throw_exception(error);
        return;
    }
    let Some(transaction) =
        indexed_db_object_store_transaction(scope, store).filter(|transaction| {
            object_bool_property(scope, *transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
                .unwrap_or(false)
        })
    else {
        let error = dom_exception_value(
            scope,
            "The transaction is not active.",
            "TransactionInactiveError",
        );
        scope.throw_exception(error);
        return;
    };
    let Some(store_name) = indexed_db_object_store_name(scope, store) else {
        return;
    };
    if indexed_db_transaction_mode(scope, transaction) == Some(TransactionMode::ReadOnly) {
        let error = dom_exception_value(scope, "The transaction is readonly.", "ReadOnlyError");
        scope.throw_exception(error);
        return;
    }
    let query = match parse_key_or_range(scope, parsed.key) {
        Ok(Some(query)) => query,
        Ok(None) => {
            let error = dom_exception_value(
                scope,
                "The query is not a valid key or key range.",
                "DataError",
            );
            scope.throw_exception(error);
            return;
        }
        Err(message) => {
            let error = dom_exception_value(scope, message, "DataError");
            scope.throw_exception(error);
            return;
        }
    };
    let Some(request) = create_request_object(scope, store.into(), transaction) else {
        return;
    };
    queue_transaction_request(scope, transaction, request);
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_STARTED_SLOT)
        .unwrap_or(false)
    {
        enqueue_transaction_operation(
            scope,
            transaction,
            store,
            request,
            &store_name,
            IndexedDbTransactionOperation::ObjectStoreDelete { query },
        );
        rv.set(request.into());
        return;
    }
    let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
        let error = dom_exception_value(
            scope,
            "The transaction is not active.",
            "TransactionInactiveError",
        );
        scope.throw_exception(error);
        return;
    };
    execute_object_store_delete_request(scope, request, handle, &store_name, &query);
    rv.set(request.into());
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_clear_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let store = args.this();
    let Some((request, transaction)) = create_store_request(scope, store) else {
        return;
    };
    let Some(store_name) = indexed_db_object_store_name(scope, store) else {
        rv.set(request.into());
        return;
    };
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_STARTED_SLOT)
        .unwrap_or(false)
    {
        enqueue_transaction_operation(
            scope,
            transaction,
            store,
            request,
            &store_name,
            IndexedDbTransactionOperation::ObjectStoreClear,
        );
        rv.set(request.into());
        return;
    }
    let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
        let error = dom_exception_value(
            scope,
            "The transaction is not active.",
            "TransactionInactiveError",
        );
        scope.throw_exception(error);
        return;
    };
    execute_object_store_clear_request(scope, request, handle, &store_name);
    rv.set(request.into());
}
