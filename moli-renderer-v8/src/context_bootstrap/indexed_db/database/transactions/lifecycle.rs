use super::*;
use crate::context_bootstrap::indexed_db::typed_state::sql::{
    SQL_ABORT_REQUEST, disarm_sql_transaction_rollback, indexed_db_request_sql_client,
};
use crate::context_bootstrap::indexed_db::{
    TRANSACTION_ERROR_SLOT, complete_indexed_db_transaction_operation,
    indexed_db_database_sql_connection, indexed_db_transaction_database,
    indexed_db_transaction_is_committing, indexed_db_transaction_mode,
    indexed_db_transaction_start_request, mark_indexed_db_transaction_cancellation_scheduled,
    request_indexed_db_transaction_commit, restore_indexed_db_upgrade_metadata,
};
use crate::util::{get_private_value, set_private_value};
use moli_indexeddb::TransactionMode;

pub(in crate::context_bootstrap::indexed_db) fn finish_canceled_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> bool {
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
        if indexed_db_request_sql_client(scope, transaction).is_some() {
            // Started SQL admissions are canceled only after worker rollback.
            // This also completes an abort whose final submission raced close.
            let error =
                if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
                    .unwrap_or(false)
                {
                    get_private_value(scope, transaction, TRANSACTION_ERROR_SLOT).unwrap_or(error)
                } else {
                    error
                };
            disarm_sql_transaction_rollback(scope, transaction);
            finish_transaction_abort_after_rollback(scope, transaction, error);
        } else {
            finish_transaction_abort(scope, transaction, error);
        }
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
    if let Some(client) = indexed_db_request_sql_client(scope, transaction) {
        use crate::context_bootstrap::indexed_db::typed_state::sql::{
            sql_transaction_cancellation, sql_transaction_rollback_is_armed,
            submit_sql_transaction_abort,
        };
        set_private_value(scope, transaction, TRANSACTION_ERROR_SLOT, error);
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
        let error = v8::Global::new(scope, error);
        let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
            return;
        };
        if client.is_retired() {
            disarm_sql_transaction_rollback(scope, transaction);
            let error = v8::Local::new(scope, &error);
            finish_transaction_abort_after_rollback(scope, transaction, error);
            return;
        }
        if sql_transaction_rollback_is_armed(scope, transaction) {
            let submitted = submit_sql_transaction_abort(
                scope,
                transaction,
                &client,
                handle,
                move |scope, transaction, reply| {
                    if reply.result.is_ok()
                        || matches!(
                            reply.result,
                            Err(moli_indexeddb::IndexedDbError::TransactionInactive(_))
                        )
                    {
                        disarm_sql_transaction_rollback(scope, transaction);
                    }
                    let error = v8::Local::new(scope, &error);
                    finish_transaction_abort_after_rollback(scope, transaction, error);
                },
            );
            if submitted.is_ok() {
                return;
            }
            if !scope.is_execution_terminating()
                && let Some(database) = indexed_db_transaction_database(scope, transaction)
                && let Some(connection) = indexed_db_database_sql_connection(scope, database)
            {
                // Rejection may race native shutdown or deletion. Retire this
                // logical connection and let its rollback receipt finish abort.
                if indexed_db_transaction_mode(scope, transaction)
                    == Some(TransactionMode::VersionChange)
                {
                    mark_indexed_db_transaction_cancellation_scheduled(scope, transaction);
                }
                let _ = connection.retire();
                if connection.is_forced_closed() {
                    disarm_sql_transaction_rollback(scope, transaction);
                    let error = get_private_value(scope, transaction, TRANSACTION_ERROR_SLOT)
                        .unwrap_or_else(|| v8::null(scope).into());
                    finish_transaction_abort_after_rollback(scope, transaction, error);
                }
                return;
            }
            // An interrupted realm cannot retain an abort completion. The
            // emergency guard still rolls back if submission did not occur.
            sql_transaction_cancellation(scope, transaction)();
            return;
        }
        // An unstarted transaction has no SQL work to roll back. Dropping its
        // admission below cancels the already-enqueued frontend start task.
        let error = get_private_value(scope, transaction, TRANSACTION_ERROR_SLOT)
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
    set_private_value(scope, transaction, TRANSACTION_ERROR_SLOT, error);
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
    let pending_request = get_private_value(scope, transaction, SQL_ABORT_REQUEST)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    if let Some(request) = pending_request {
        set_private_value(
            scope,
            transaction,
            SQL_ABORT_REQUEST,
            v8::undefined(scope).into(),
        );
        store_request_error(scope, request, request_error);
        complete_indexed_db_transaction_operation(scope, transaction);
    }
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
