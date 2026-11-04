use super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBIndex.getKey")]
struct IdbIndexGetKeyArgs<'s> {
    #[webidl(required, converter = "raw")]
    query: v8::Local<'s, v8::Value>,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_index_get_key_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbIndexGetKeyArgs<'s>>(scope, &args) else {
        return;
    };
    let index = args.this();
    if index_active_transaction(scope, index).is_none() {
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
    let Some((transaction, store_name, _)) = index_operation_state(scope, index) else {
        return;
    };
    let Some(request) = create_operation_request(scope, index, transaction) else {
        return;
    };
    schedule_transaction_operation(
        scope,
        transaction,
        index,
        request,
        &store_name,
        IndexedDbTransactionOperation::IndexGetKey { query },
    );
    rv.set(request.into());
}
