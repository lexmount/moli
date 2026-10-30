use super::super::open::finish_aborted_upgrade_open;
use super::*;
use crate::context_bootstrap::indexed_db::take_indexed_db_upgrade_open;

pub(in crate::context_bootstrap::indexed_db) fn flush_transaction_abort_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(transaction) = indexed_db_object_task_target(scope, task) else {
        return false;
    };
    if crate::context_bootstrap::indexed_db::indexed_db_transaction_has_pending_errors(
        scope,
        transaction,
    ) {
        // An accepted write can finish preparing its request after a getter
        // calls abort(). Keep this same notification behind its request errors.
        enqueue_indexed_db_task(scope, task);
        return true;
    }
    let aborted_open = take_indexed_db_upgrade_open(scope, transaction);
    if let Some((_, database)) = aborted_open {
        // The failed upgrade connection is unusable during abort listeners,
        // while the open request must retain its transaction until afterwards.
        close_indexed_db_database_connection(scope, database);
        set_indexed_db_slot_value(
            scope,
            database,
            INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT,
            v8::null(scope).into(),
        );
    }
    crate::context_bootstrap::indexed_db::clear_indexed_db_upgrade_transaction(scope, transaction);
    let _ = dispatch_idb_named_event(scope, transaction, "abort", |_, _| {});
    release_indexed_db_transaction_dispatch_refs(scope, transaction);
    if let Some((request, database)) = aborted_open {
        finish_aborted_upgrade_open(scope, request, database);
    }
    false
}
