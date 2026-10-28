use super::*;
use crate::context_bootstrap::indexed_db::restore_indexed_db_upgrade_metadata;
use crate::context_bootstrap::indexed_db::schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint;

pub(in crate::context_bootstrap::indexed_db) fn finish_canceled_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> bool {
    use crate::context_bootstrap::indexed_db::indexed_db_transaction_start_request;
    if !indexed_db_transaction_start_request(scope, transaction).is_some_and(|request| {
        request.status() == moli_indexeddb::TransactionRequestStatus::Canceled
    }) {
        return false;
    }
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
        .unwrap_or(false)
    {
        let error = dom_exception_value(
            scope,
            "The database connection was forcibly closed.",
            "AbortError",
        );
        finish_transaction_abort(scope, transaction, error);
    }
    true
}

pub(in crate::context_bootstrap::indexed_db) fn finish_transaction_abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    restore_indexed_db_upgrade_metadata(scope, transaction);
    crate::util::set_private_value(
        scope,
        transaction,
        crate::context_bootstrap::indexed_db::TRANSACTION_ERROR_SLOT,
        error,
    );
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
    // The transaction's abort reason may be null (explicit abort), while
    // every unfinished request receives its own AbortError outcome.
    let request_error = dom_exception_value(scope, "The transaction was aborted.", "AbortError");
    abort_queued_transaction_requests(scope, transaction, request_error);
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
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_STARTED_SLOT)
        .unwrap_or(false)
    {
        finish_transaction_abort(scope, transaction, v8::null(scope).into());
        rv.set_undefined();
        return;
    }
    let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
        rv.set_undefined();
        return;
    };
    match with_indexed_db_manager(scope, |manager| manager.abort_transaction(handle)) {
        Ok(()) => {
            finish_transaction_abort(scope, transaction, v8::null(scope).into());
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            scope.throw_exception(error);
        }
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap::indexed_db) fn idb_transaction_commit_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(transaction) = idb_transaction_receiver(scope, &args) else {
        return;
    };
    schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(scope, transaction);
    enqueue_transaction_commit_task(scope, transaction);
    rv.set_undefined();
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
    if let Some(handle) = transaction_handle_from_value(scope, transaction.into()) {
        let _ = with_indexed_db_manager(scope, |manager| manager.abort_transaction(handle));
    }
    finish_transaction_abort(scope, transaction, error);
}
