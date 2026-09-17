use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBIndex.count")]
struct IdbIndexCountArgs<'s> {
    #[webidl(converter = "raw")]
    query: Option<v8::Local<'s, v8::Value>>,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_index_count_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbIndexCountArgs<'s>>(scope, &args) else {
        return;
    };
    let index = args.this();
    let Some((request, transaction, store_name, index_info)) = create_index_request(scope, index)
    else {
        return;
    };
    let query_value = parsed.query.unwrap_or_else(|| v8::undefined(scope).into());
    let query = match parse_key_or_range(scope, query_value) {
        Ok(query) => query,
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
            index,
            request,
            &store_name,
            IndexedDbTransactionOperationInput::IndexCount { query: query_value },
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
    match scan_index_entries(scope, handle, &store_name, &index_info, query.as_ref()) {
        Ok(entries) => store_request_success(
            scope,
            request,
            v8::Number::new(scope, entries.len() as f64).into(),
        ),
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
    rv.set(request.into());
}
