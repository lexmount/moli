use super::*;
use crate::context_bootstrap::indexed_db::restore_indexed_db_upgrade_metadata;
use crate::context_bootstrap::indexed_db::{
    indexed_db_transaction_is_committing, request_indexed_db_transaction_commit,
};

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
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
        .unwrap_or(false)
        || object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
            .unwrap_or(false)
    {
        return;
    }
    if let Some(client) =
        crate::context_bootstrap::indexed_db::typed_state::sql::indexed_db_request_sql_client(
            scope,
            transaction,
        )
    {
        use crate::context_bootstrap::indexed_db::typed_state::sql::{
            sql_transaction_cancellation, sql_transaction_rollback_is_armed,
            submit_indexed_db_sql_task_reserved, take_sql_transaction_finish_permit,
        };
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
            INDEXED_DB_TRANSACTION_ABORTED_SLOT,
            v8::Boolean::new(scope, true).into(),
        );
        abort_queued_transaction_requests(scope, transaction, error);
        let error = v8::Global::new(scope, error);
        let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
            return;
        };
        // Keep the independent emergency rollback armed until the reply. The
        // same reserved completion slot serves COMMIT or explicit ROLLBACK.
        if let Some(permit) = take_sql_transaction_finish_permit(scope, transaction) {
            let submitted = submit_indexed_db_sql_task_reserved(
                scope,
                transaction,
                &client,
                permit,
                moli_indexeddb::sqlite::service::Command::Abort {
                    transaction: handle,
                },
                0,
                move |scope, transaction, reply| {
                    if reply.result.is_ok()
                        || matches!(
                            reply.result,
                            Err(moli_indexeddb::IndexedDbError::TransactionInactive(_))
                        )
                    {
                        crate::context_bootstrap::indexed_db::typed_state::sql::disarm_sql_transaction_rollback(scope, transaction);
                    }
                    let error = v8::Local::new(scope, &error);
                    finish_transaction_abort_after_rollback(scope, transaction, error);
                },
            );
            if submitted.is_ok() {
                return;
            }
            // A realm that cannot retain its completion cannot receive an
            // abort event. Release the native emergency guard immediately.
            sql_transaction_cancellation(scope, transaction)();
            return;
        }
        if sql_transaction_rollback_is_armed(scope, transaction) {
            sql_transaction_cancellation(scope, transaction)();
            return;
        }
        // An unstarted transaction has no SQL work to roll back. Dropping its
        // admission below cancels the already-enqueued frontend start task.
        let error = crate::util::get_private_value(
            scope,
            transaction,
            crate::context_bootstrap::indexed_db::TRANSACTION_ERROR_SLOT,
        )
        .unwrap_or_else(|| v8::null(scope).into());
        finish_transaction_abort_after_rollback(scope, transaction, error);
        return;
    }
    if let Some(handle) = transaction_handle_from_value(scope, transaction.into()) {
        let _ = with_indexed_db_manager(scope, |manager| manager.abort_transaction(handle));
    }
    finish_transaction_abort_after_rollback(scope, transaction, error);
}

pub(in crate::context_bootstrap::indexed_db) fn finish_transaction_abort_after_rollback<'s>(
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
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
        .unwrap_or(false)
        || object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
            .unwrap_or(false)
        || indexed_db_transaction_is_committing(scope, transaction)
    {
        let error = dom_exception_value(
            scope,
            "The transaction is finished or committing.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return;
    }
    let error = v8::null(scope).into();
    finish_transaction_abort(scope, transaction, error);
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
    if let Err(error) = request_indexed_db_transaction_commit(scope, transaction) {
        let error = request_error_object(scope, &error);
        scope.throw_exception(error);
        return;
    }
    enqueue_transaction_commit_task(scope, transaction);
    rv.set_undefined();
}

pub(in crate::context_bootstrap::indexed_db) fn abort_indexed_db_transaction_with_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    finish_transaction_abort(scope, transaction, error);
}
