use super::*;
use crate::context_bootstrap::indexed_db::restore_indexed_db_upgrade_metadata;

pub(super) fn commit_upgrade_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    database: v8::Local<'s, v8::Object>,
    transaction: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
        return true;
    };
    let quota_commit = match storage_bucket_quota_check_for_transaction(scope, transaction) {
        Some(Ok(quota)) => Some(quota),
        Some(Err(error)) => {
            let _ = with_indexed_db_manager(scope, |manager| manager.abort_transaction(handle));
            return finish_failed_upgrade_commit(scope, request, transaction, error);
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
            let _ = refresh_database_surface(scope, database);
            set_indexed_db_slot_value(
                scope,
                database,
                INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT,
                v8::null(scope).into(),
            );
            let _ = dispatch_idb_named_event(scope, transaction, "complete", |_, _| {});
            release_indexed_db_transaction_dispatch_refs(scope, transaction);
            true
        }
        Err(error) => finish_failed_upgrade_commit(scope, request, transaction, error),
    }
}

fn finish_failed_upgrade_commit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    transaction: v8::Local<'s, v8::Object>,
    error: IndexedDbError,
) -> bool {
    restore_indexed_db_upgrade_metadata(scope, transaction);
    let error_value = request_error_object(scope, &error);
    let _ = transaction.set(scope, v8str(scope, "error").into(), error_value);
    set_indexed_db_request_surface_value(
        scope,
        request,
        INDEXED_DB_REQUEST_ERROR_SLOT,
        "error",
        error_value,
    );
    let done = v8str(scope, "done").into();
    set_indexed_db_request_surface_value(
        scope,
        request,
        INDEXED_DB_REQUEST_READY_STATE_SLOT,
        "readyState",
        done,
    );
    let _ = dispatch_idb_named_event(scope, transaction, "error", |_, _| {});
    release_indexed_db_transaction_dispatch_refs(scope, transaction);
    let _ = dispatch_idb_named_event(scope, request, "error", |_, _| {});
    release_request_dispatch_refs(scope, request);
    false
}
