use super::*;

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
    let Some((request, transaction)) = create_store_request(scope, store) else {
        return;
    };
    let Some(store_name) = indexed_db_object_store_name(scope, store) else {
        rv.set(request.into());
        return;
    };
    match parse_idb_key(scope, parsed.key) {
        Ok(Some(_)) => {}
        _ => {
            throw_type_error(scope, "Failed to execute 'delete': invalid key.");
            return;
        }
    }
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_STARTED_SLOT)
        .unwrap_or(false)
    {
        enqueue_transaction_operation(
            scope,
            transaction,
            store,
            request,
            &store_name,
            IndexedDbTransactionOperationInput::ObjectStoreDelete { key: parsed.key },
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
    execute_object_store_delete_request(scope, request, handle, &store_name, parsed.key);
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
            IndexedDbTransactionOperationInput::ObjectStoreClear,
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
