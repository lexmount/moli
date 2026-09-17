use super::*;

pub(super) fn object_store_write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    value: v8::Local<'s, v8::Value>,
    key: v8::Local<'s, v8::Value>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    add_only: bool,
) {
    let store = args.this();
    let Some(transaction) = object_store_active_transaction(scope, store) else {
        return;
    };
    let Some(prepared) = prepare_object_store_write(scope, store, transaction, value, key) else {
        return;
    };
    let Some(store_name) = indexed_db_object_store_name(scope, store) else {
        return;
    };
    // Only accepted operations allocate a request. Both immediate and queued
    // writes use the same immutable snapshot and native converted key.
    let Some(request) = create_request_object(scope, store.into(), transaction) else {
        return;
    };
    rv.set(request.into());
    // A getter can abort while the value is being serialized. Admission was
    // already checked; return the request without reviving or writing the
    // finished transaction.
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
        .unwrap_or(false)
    {
        return;
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
            IndexedDbTransactionOperation::ObjectStoreWrite { prepared, add_only },
        );
    } else if let Some(handle) = transaction_handle_from_value(scope, transaction.into()) {
        execute_object_store_write_request(
            scope,
            store,
            request,
            handle,
            &store_name,
            &prepared,
            add_only,
        );
    }
}
