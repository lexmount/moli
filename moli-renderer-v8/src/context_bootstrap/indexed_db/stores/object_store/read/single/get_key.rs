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
    if object_store_active_transaction(scope, store).is_none() {
        return;
    }
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
    let Some((transaction, store_name)) = object_store_operation_state(scope, store) else {
        return;
    };
    let Some(request) = create_operation_request(scope, store, transaction) else {
        return;
    };
    schedule_transaction_operation(
        scope,
        transaction,
        store,
        request,
        &store_name,
        IndexedDbTransactionOperation::ObjectStoreGetKey { query },
    );
    rv.set(request.into());
}
