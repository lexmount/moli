use super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBObjectStore.getKey")]
struct IdbObjectStoreGetKeyArgs<'s> {
    #[webidl(required, converter = "raw")]
    query: v8::Local<'s, v8::Value>,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_get_key_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbObjectStoreGetKeyArgs<'s>>(scope, &args) else {
        return;
    };
    let store = args.this();
    let Some((request, transaction, store_name)) = object_store_operation_common(scope, store)
    else {
        return;
    };
    let query = match parse_key_or_range(scope, parsed.query) {
        Ok(Some(query)) => query,
        Ok(None) => {
            let error = dom_exception_value(
                scope,
                "Failed to execute 'getKey': the query is not a valid key or key range.",
                "DataError",
            );
            scope.throw_exception(error);
            return;
        }
        Err(error) => {
            error.throw(scope);
            return;
        }
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
            IndexedDbTransactionOperationInput::ObjectStoreGetKey {
                query: parsed.query,
            },
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
    match scan_object_store_entries(scope, handle, &store_name, Some(&query)) {
        Ok(entries) => {
            let result = entries
                .first()
                .map(|(key, _)| key_to_js_value(scope, key))
                .unwrap_or_else(|| v8::undefined(scope).into());
            store_request_success(scope, request, result);
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
    rv.set(request.into());
}
