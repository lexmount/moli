use super::*;
use crate::context_bootstrap::indexed_db::defer_indexed_db_aborted_open;

mod abort;
mod commit;
mod success;
pub(in crate::context_bootstrap::indexed_db::tasks::dispatch) use abort::finish_aborted_upgrade_open;

pub(in crate::context_bootstrap::indexed_db) fn flush_open_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some((request, database, transaction, old_version, new_version)) =
        indexed_db_open_task_payload(scope, task)
    else {
        return;
    };

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

    let _ = dispatch_version_change_event(
        scope,
        request,
        "upgradeneeded",
        old_version,
        Some(new_version),
    );

    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
        .unwrap_or(false)
    {
        defer_indexed_db_aborted_open(scope, transaction, request);
        return;
    }

    if !commit::commit_upgrade_transaction(scope, request, database, transaction) {
        return;
    }
    success::finish_open_success(scope, request);
}
