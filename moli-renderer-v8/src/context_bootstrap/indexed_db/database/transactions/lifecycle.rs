use super::*;
use crate::context_bootstrap::indexed_db::restore_indexed_db_upgrade_metadata;
use moli_indexeddb::IndexedDbError;

pub(in crate::context_bootstrap::indexed_db) fn finish_transaction_abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    restore_indexed_db_upgrade_metadata(scope, transaction);
    let _ = transaction.set(scope, v8str(scope, "error").into(), error);
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_ACTIVE_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_FINISHED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_ABORTED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    abort_queued_transaction_requests(scope, transaction);
    unregister_regular_transaction(scope, transaction);
    enqueue_ready_transaction_starts(scope);
    enqueue_transaction_abort_task(scope, transaction);
}

pub(in crate::context_bootstrap::indexed_db) fn idb_transaction_abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(transaction) = idb_transaction_receiver(scope, &args) else {
        return;
    };
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_COMMITTING_SLOT)
        .unwrap_or(false)
    {
        let error =
            dom_exception_value(scope, "The transaction is committing.", "InvalidStateError");
        scope.throw_exception(error);
        return;
    }
    if let Err(error) = abort_indexed_db_transaction(scope, transaction, v8::null(scope).into()) {
        let error = request_error_object(scope, &error);
        scope.throw_exception(error);
        return;
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap::indexed_db) fn abort_indexed_db_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) -> Result<(), IndexedDbError> {
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
        .unwrap_or(false)
    {
        return Err(IndexedDbError::InvalidState(
            "The transaction has finished.".to_owned(),
        ));
    }
    if let Some(handle) = transaction_handle_from_value(scope, transaction.into()) {
        with_indexed_db_manager(scope, |manager| manager.abort_transaction(handle))?;
    }
    finish_transaction_abort(scope, transaction, error);
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn abort_indexed_db_transaction_after_dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
        .unwrap_or(false)
        || !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
            .unwrap_or(false)
    {
        return;
    }
    abort_indexed_db_transaction_with_error(scope, transaction, error);
}

pub(in crate::context_bootstrap::indexed_db) fn abort_indexed_db_transaction_with_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
        .unwrap_or(false)
    {
        return;
    }
    if let Err(error) = abort_indexed_db_transaction(scope, transaction, error) {
        let error = request_error_object(scope, &error);
        finish_transaction_abort(scope, transaction, error);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_transaction_commit_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(transaction) = idb_transaction_receiver(scope, &args) else {
        return;
    };
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ACTIVE_SLOT)
        .unwrap_or(false)
    {
        let error =
            dom_exception_value(scope, "The transaction is not active.", "InvalidStateError");
        scope.throw_exception(error);
        return;
    }
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_ACTIVE_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_COMMITTING_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    enqueue_transaction_commit_task(scope, transaction);
    rv.set_undefined();
}
