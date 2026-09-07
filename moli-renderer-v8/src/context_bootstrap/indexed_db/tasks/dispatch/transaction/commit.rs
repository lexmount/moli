use super::*;

pub(in crate::context_bootstrap::indexed_db) fn flush_transaction_commit_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(transaction) = indexed_db_transaction_task_transaction(scope, task) else {
        return;
    };
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_COMMIT_SCHEDULED_SLOT,
        v8::Boolean::new(scope, false).into(),
    );

    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
        .unwrap_or(false)
        || object_number_property(scope, transaction, INDEXED_DB_TRANSACTION_PENDING_SLOT)
            .unwrap_or(0.0)
            > 0.0
    {
        return;
    }

    let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
        return;
    };
    let quota_commit = match storage_bucket_quota_check_for_transaction(scope, transaction) {
        Some(Ok(quota)) => Some(quota),
        Some(Err(error)) => {
            let _ = with_indexed_db_manager(scope, |manager| manager.abort_transaction(handle));
            let error_value = request_error_object(scope, &error);
            let _ = transaction.set(scope, v8str(scope, "error").into(), error_value);
            finish_failed_commit(scope, transaction);
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
            let error_value = request_error_object(scope, &error);
            let _ = transaction.set(scope, v8str(scope, "error").into(), error_value);
            finish_failed_commit(scope, transaction);
        }
    }
}

fn finish_committed_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    finish_transaction(scope, transaction);
    if let Some(db) = object_property_as_object(scope, transaction, "db") {
        let _ = refresh_database_surface(scope, db);
    }
    dispatch_transaction_terminal(scope, transaction, "complete", true);
}

fn finish_failed_commit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    finish_transaction(scope, transaction);
    dispatch_transaction_terminal(scope, transaction, "error", false);
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
    let db_key = transaction_db_key(scope, transaction);
    unregister_readwrite_transaction(scope, transaction);
    if let Some(db_key) = db_key {
        enqueue_next_readwrite_transaction_start(scope, &db_key);
    }
}
