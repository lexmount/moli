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
    if index_active_transaction(scope, index).is_none() {
        return;
    }
    let query_value = parsed.query.unwrap_or_else(|| v8::undefined(scope).into());
    let query = match parse_key_or_range(scope, query_value) {
        Ok(query) => query,
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
        IndexedDbTransactionOperation::IndexCount { query },
    );
    rv.set(request.into());
}
