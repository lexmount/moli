use super::*;

pub(in crate::context_bootstrap::indexed_db) fn register_regular_transaction(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
) {
    push_unique_object_to_indexed_db_runtime_array(
        scope,
        IndexedDbRuntimeArray::Transactions,
        transaction,
    );
}

pub(in crate::context_bootstrap::indexed_db) fn unregister_regular_transaction(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
) {
    crate::context_bootstrap::indexed_db::finish_indexed_db_transaction_start_request(
        scope,
        transaction,
    );
    let Some(queue) = indexed_db_runtime_array(scope, IndexedDbRuntimeArray::Transactions) else {
        return;
    };
    let next = v8::Array::new(scope, 0);
    for index in 0..queue.length() {
        let Some(value) = queue.get_index(scope, index) else {
            continue;
        };
        if !value.strict_equals(transaction.into()) {
            let _ = next.set_index(scope, next.length(), value);
        }
    }
    replace_indexed_db_runtime_array(scope, IndexedDbRuntimeArray::Transactions, next);
}
