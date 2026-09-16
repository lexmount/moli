use super::*;
use crate::context_bootstrap::indexed_db::{
    associate_indexed_db_upgrade_open,
    schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint,
};

mod abort;
mod success;
pub(in crate::context_bootstrap::indexed_db::tasks::dispatch) use abort::finish_aborted_upgrade_open;
pub(in crate::context_bootstrap::indexed_db::tasks::dispatch) use success::{
    enqueue_committed_upgrade_open, flush_open_success_task,
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

    associate_indexed_db_upgrade_open(scope, transaction, request);
    let _ = dispatch_version_change_event(
        scope,
        request,
        "upgradeneeded",
        old_version,
        Some(new_version),
    );

    // Use the same pending-request and microtask lifetime as ordinary transactions.
    // Request callbacks (including their microtasks) may enqueue more work or abort.
    schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(scope, transaction);
}
