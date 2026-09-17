use super::*;
use crate::util::serialize_v8_iter_array;

pub(in crate::context_bootstrap::indexed_db) fn idb_index_get_all_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let index = args.this();
    let Some((request, transaction, store_name, index_info)) = create_index_request(scope, index)
    else {
        return;
    };
    let parsed = match parse_collection_request_args(scope, &args, "IDBIndex.getAllKeys") {
        Ok(parsed) => parsed,
        Err(CollectionRequestArgsError::WebIdl(error)) => {
            webidl::throw_error(scope, &error);
            return;
        }
        Err(CollectionRequestArgsError::Key(error)) => {
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
            IndexedDbTransactionOperation::IndexGetAllKeys {
                query: parsed.query,
                count: parsed.count,
                direction: parsed.direction,
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
    match scan_index_entries(
        scope,
        handle,
        &store_name,
        &index_info,
        parsed.query.as_ref(),
    ) {
        Ok(entries) => {
            let entries = apply_index_collection_direction(entries, parsed.direction);
            let limit = parsed.count.unwrap_or(entries.len());
            let keys = entries
                .iter()
                .take(limit)
                .map(|entry| key_to_js_value(scope, &entry.primary_key))
                .collect::<Vec<_>>();
            let array =
                serialize_v8_iter_array(scope, keys).unwrap_or_else(|| v8::Array::new(scope, 0));
            store_request_success(scope, request, array.into());
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
    rv.set(request.into());
}
