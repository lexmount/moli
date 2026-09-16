use super::*;

mod enqueue;
mod execute;

pub(super) fn object_store_write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    value: v8::Local<'s, v8::Value>,
    key: v8::Local<'s, v8::Value>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    add_only: bool,
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
        enqueue::enqueue_deferred_object_store_write(
            scope,
            transaction,
            store,
            request,
            &store_name,
            value,
            key,
            add_only,
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
    if execute::execute_started_object_store_write(
        scope,
        store,
        request,
        handle,
        &store_name,
        value,
        key,
        add_only,
    ) {
        rv.set(request.into());
    }
}
