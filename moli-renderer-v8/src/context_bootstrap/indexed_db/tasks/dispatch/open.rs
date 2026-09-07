use super::*;
use crate::context_bootstrap::indexed_db::{
    INDEXED_DB_DATABASE_CLOSED_SLOT,
    schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint,
    typed_state::{
        IndexedDbWrapperKind, bind_indexed_db_upgrade_open, indexed_db_typed_wrapper_is,
        take_indexed_db_upgrade_open,
    },
};

pub(in crate::context_bootstrap::indexed_db) fn flush_open_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some((request, database, transaction, old_version, new_version)) =
        indexed_db_open_task_payload(scope, task)
    else {
        return;
    };
    bind_indexed_db_upgrade_open(scope, transaction, request, database);

    set_indexed_db_request_surface_value(
        scope,
        request,
        INDEXED_DB_REQUEST_RESULT_SLOT,
        "result",
        database.into(),
    );
    set_indexed_db_request_surface_value(
        scope,
        request,
        INDEXED_DB_REQUEST_TRANSACTION_SLOT,
        "transaction",
        transaction.into(),
    );
    let done = v8str(scope, "done").into();
    set_indexed_db_request_surface_value(
        scope,
        request,
        INDEXED_DB_REQUEST_READY_STATE_SLOT,
        "readyState",
        done,
    );

    let _ = dispatch_version_change_event(
        scope,
        request,
        "upgradeneeded",
        old_version,
        Some(new_version),
    );

    // upgradeneeded may enqueue data requests, whose handlers can enqueue more
    // requests or change schema. Use the ordinary transaction pending-request
    // gate and end-of-microtask-checkpoint deactivation; committing here would
    // publish open.success before an asynchronous migration had run.
    schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(scope, transaction);
}

pub(in crate::context_bootstrap::indexed_db) fn prepare_upgrade_open_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    successful: bool,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    let (request, database) = take_indexed_db_upgrade_open(scope, transaction)?;
    // Blink's TransactionWillFinish boundary: the database is no longer in
    // an upgrade before complete/abort handlers run. Keep request.transaction
    // attached through that event; it has a different, later lifetime.
    set_indexed_db_slot_value(
        scope,
        database,
        INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT,
        v8::null(scope).into(),
    );
    if !successful {
        close_indexed_db_database_connection(scope, database);
    }
    Some((request, database))
}

pub(in crate::context_bootstrap::indexed_db) fn finish_upgrade_open<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    database: v8::Local<'s, v8::Object>,
    successful: bool,
) {
    set_indexed_db_request_surface_value(
        scope,
        request,
        INDEXED_DB_REQUEST_TRANSACTION_SLOT,
        "transaction",
        v8::null(scope).into(),
    );
    let pending = v8str(scope, "pending").into();
    set_indexed_db_request_surface_value(
        scope,
        request,
        INDEXED_DB_REQUEST_READY_STATE_SLOT,
        "readyState",
        pending,
    );
    // Publish the open result as a separate request task after transaction
    // complete/abort, not inside that event's callback or before its microtasks.
    if successful {
        store_request_success(scope, request, database.into());
    } else {
        set_indexed_db_request_surface_value(
            scope,
            request,
            INDEXED_DB_REQUEST_RESULT_SLOT,
            "result",
            v8::undefined(scope).into(),
        );
        let error = dom_exception_value(
            scope,
            "The upgrade transaction was aborted or its connection closed.",
            "AbortError",
        );
        store_request_error(scope, request, error);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn reject_closed_open_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> bool {
    if !indexed_db_typed_wrapper_is(scope, request, IndexedDbWrapperKind::OpenRequest) {
        return false;
    }
    let Some(database) = object_hidden_value(scope, request, INDEXED_DB_PENDING_RESULT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    if !object_bool_property(scope, database, INDEXED_DB_DATABASE_CLOSED_SLOT).unwrap_or(false) {
        return false;
    }
    // A complete-event microtask can close the provisional connection after
    // success is queued. Check at delivery, as IDBOpenDBRequest does in Blink.
    set_indexed_db_request_surface_value(
        scope,
        request,
        INDEXED_DB_REQUEST_RESULT_SLOT,
        "result",
        v8::undefined(scope).into(),
    );
    let error = dom_exception_value(
        scope,
        "The connection was closed before the open result.",
        "AbortError",
    );
    store_request_error(scope, request, error);
    true
}
