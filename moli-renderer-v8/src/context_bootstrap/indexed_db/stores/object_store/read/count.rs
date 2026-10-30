use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBObjectStore.count")]
struct IdbObjectStoreCountArgs<'s> {
    #[webidl(converter = "raw")]
    query: Option<v8::Local<'s, v8::Value>>,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_count_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbObjectStoreCountArgs<'s>>(scope, &args) else {
        return;
    };
    let store = args.this();
    let Some((transaction, store_name)) = object_store_operation_state(scope, store) else {
        return;
    };
    let query_value = parsed.query.unwrap_or_else(|| v8::undefined(scope).into());
    let query = match parse_key_or_range(scope, query_value) {
        Err(error) => {
            error.throw(scope);
            return;
        }
        Ok(query) => query,
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
        IndexedDbTransactionOperation::ObjectStoreCount { query },
    );
    rv.set(request.into());
}
