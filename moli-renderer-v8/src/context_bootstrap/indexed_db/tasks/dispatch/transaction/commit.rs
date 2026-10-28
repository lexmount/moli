use super::super::open::enqueue_committed_upgrade_open;
use super::*;
use crate::context_bootstrap::indexed_db::{
    finish_transaction_abort, indexed_db_transaction_database, take_indexed_db_upgrade_open,
};
use crate::context_bootstrap::indexed_db::{
    indexed_db_transaction_can_commit, indexed_db_transaction_has_pending_errors,
    indexed_db_transaction_is_committing,
};

pub(in crate::context_bootstrap::indexed_db) fn flush_transaction_commit_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(transaction) = indexed_db_object_task_target(scope, task) else {
        return;
    };
    if crate::context_bootstrap::indexed_db::finish_canceled_transaction(scope, transaction) {
        return;
    }
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_COMMIT_SCHEDULED_SLOT,
        v8::Boolean::new(scope, false).into(),
    );

    if !indexed_db_transaction_can_commit(scope, transaction) {
        return;
    }

    let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
        return;
    };
    crate::context_bootstrap::indexed_db::mark_indexed_db_transaction_committing(
        scope,
        transaction,
    );
    if indexed_db_transaction_is_committing(scope, transaction)
        && indexed_db_transaction_has_pending_errors(scope, transaction)
    {
        let error = dom_exception_value(
            scope,
            "A request failed while the transaction was committing.",
            "AbortError",
        );
        finish_transaction_abort(scope, transaction, error);
        return;
    }
    if let Some(client) =
        crate::context_bootstrap::indexed_db::typed_state::sql::indexed_db_request_sql_client(
            scope,
            transaction,
        )
    {
        use crate::context_bootstrap::indexed_db::typed_state::sql::{
            disarm_sql_transaction_rollback, submit_indexed_db_sql_task_reserved,
            take_sql_transaction_finish_permit,
        };
        let Some(permit) = take_sql_transaction_finish_permit(scope, transaction) else {
            return;
        };
        set_indexed_db_slot_value(
            scope,
            transaction,
            INDEXED_DB_TRANSACTION_COMMIT_SCHEDULED_SLOT,
            v8::Boolean::new(scope, true).into(),
        );
        let result = submit_indexed_db_sql_task_reserved(
            scope,
            transaction,
            &client,
            permit,
            moli_indexeddb::sqlite::service::Command::Commit {
                transaction: handle,
            },
            0,
            |scope, transaction, reply| {
                // The native receipt is sent only after COMMIT (or rollback on
                // failure), committed metadata publication, and admission release.
                disarm_sql_transaction_rollback(scope, transaction);
                match reply.result {
                    Ok(moli_indexeddb::sqlite::service::CommandResult::Metadata(_)) => {
                        finish_committed_transaction(scope, transaction)
                    }
                    Err(error) => {
                        let error = request_error_object(scope, &error);
                        crate::context_bootstrap::indexed_db::finish_transaction_abort_after_rollback(scope, transaction, error);
                    }
                    _ => {
                        let error = dom_exception_value(
                            scope,
                            "Invalid SQL COMMIT response",
                            "UnknownError",
                        );
                        crate::context_bootstrap::indexed_db::finish_transaction_abort_after_rollback(scope, transaction, error);
                    }
                }
            },
        );
        if let Err(error) = result {
            let error = request_error_object(scope, &error);
            finish_transaction_abort(scope, transaction, error);
        }
        return;
    }
    let quota_commit = match storage_bucket_quota_check_for_transaction(scope, transaction) {
        Some(Ok(quota)) => Some(quota),
        Some(Err(error)) => {
            let error_value = request_error_object(scope, &error);
            finish_transaction_abort(scope, transaction, error_value);
            return;
        }
        None => None,
    };
    match with_indexed_db_manager(scope, |manager| {
        if let Some(quota) = quota_commit {
            manager.commit_transaction_with_quota(handle, quota.quota_check)
        } else {
            manager.commit_transaction(handle)
        }
    }) {
        Ok(()) => {
            finish_committed_transaction(scope, transaction);
        }
        Err(error) => {
            // Forced closure can win the manager lock after the entry check.
            if crate::context_bootstrap::indexed_db::finish_canceled_transaction(scope, transaction)
            {
                return;
            }
            let error_value = request_error_object(scope, &error);
            finish_transaction_abort(scope, transaction, error_value);
        }
    }
}

fn finish_committed_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    finish_transaction(scope, transaction);
    if let Some(db) = indexed_db_transaction_database(scope, transaction) {
        let _ = refresh_database_surface(scope, db);
    }
    let upgrade_open = take_indexed_db_upgrade_open(scope, transaction);
    if let Some((_, database)) = upgrade_open {
        set_indexed_db_slot_value(
            scope,
            database,
            INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT,
            v8::null(scope).into(),
        );
    }
    let _ = dispatch_idb_named_event(scope, transaction, "complete", |_, _| {});
    release_indexed_db_transaction_dispatch_refs(scope, transaction);
    if let Some((request, database)) = upgrade_open {
        enqueue_committed_upgrade_open(scope, request, database);
    }
}

fn finish_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
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
    unregister_regular_transaction(scope, transaction);
    enqueue_ready_transaction_starts(scope);
}
