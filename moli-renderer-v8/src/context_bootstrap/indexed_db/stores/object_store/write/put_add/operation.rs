use super::*;
use crate::context_bootstrap::indexed_db::{
    INDEXED_DB_TRANSACTION_ABORTED_SLOT, KeyConversionError, indexed_db_transaction_mode,
    store_request_error,
};
use moli_indexeddb::TransactionMode;

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
    if indexed_db_transaction_mode(scope, transaction) == Some(TransactionMode::ReadOnly) {
        let error = dom_exception_value(scope, "The transaction is readonly.", "ReadOnlyError");
        scope.throw_exception(error);
        return;
    }
    let explicit_key = match parse_idb_key(scope, key) {
        Ok(Some(key)) => ObjectStoreKey::Key(key),
        Ok(None) => ObjectStoreKey::Missing,
        Err(KeyConversionError::Invalid(_)) => ObjectStoreKey::Invalid,
        Err(
            error @ (KeyConversionError::Exception(_)
            | KeyConversionError::Terminated
            | KeyConversionError::Backend(_)),
        ) => {
            error.throw(scope);
            return;
        }
    };
    // Key conversion can invoke getters too, including one that aborts the transaction.
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
        .unwrap_or(false)
    {
        throw_inactive(scope);
        return;
    }
    let Some(prepared) = prepare_object_store_write(scope, store, transaction, value, explicit_key)
    else {
        return;
    };
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
        .unwrap_or(false)
    {
        // Admission preceded serialization. A getter may abort the transaction
        // while cloning; the accepted call still returns its pending request,
        // whose error must precede the transaction's abort notification.
        let Some(request) = create_request_object(scope, store.into(), transaction) else {
            return;
        };
        queue_transaction_request(scope, transaction, request);
        let error = dom_exception_value(scope, "The transaction was aborted.", "AbortError");
        store_request_error(scope, request, error);
        rv.set(request.into());
        return;
    }
    let Some(store_name) = indexed_db_object_store_name(scope, store) else {
        return;
    };
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_STARTED_SLOT)
        .unwrap_or(false)
        && transaction_handle_from_value(scope, transaction.into()).is_none()
    {
        throw_inactive(scope);
        return;
    }
    let Some((request, _)) = create_store_request(scope, store) else {
        throw_inactive(scope);
        return;
    };
    schedule_transaction_operation(
        scope,
        transaction,
        store,
        request,
        &store_name,
        IndexedDbTransactionOperation::ObjectStoreWrite { prepared, add_only },
    );
    rv.set(request.into());
}

fn throw_inactive(scope: &mut v8::PinScope<'_, '_>) {
    let error = dom_exception_value(
        scope,
        "The transaction is not active.",
        "TransactionInactiveError",
    );
    scope.throw_exception(error);
}
