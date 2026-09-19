use super::super::open::finish_aborted_upgrade_open;
use super::*;
use crate::context_bootstrap::indexed_db::take_indexed_db_upgrade_open;

pub(in crate::context_bootstrap::indexed_db) fn flush_transaction_abort_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(transaction) = indexed_db_transaction_task_transaction(scope, task) else {
        return;
    };
    let aborted_open = take_indexed_db_upgrade_open(scope, transaction);
    if let Some(database) =
        crate::context_bootstrap::indexed_db::indexed_db_transaction_database(scope, transaction)
    {
        crate::context_bootstrap::indexed_db::finish_indexed_db_database_close(scope, database);
    }
    if let Some((_, database)) = aborted_open {
        set_indexed_db_slot_value(
            scope,
            database,
            INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT,
            v8::null(scope).into(),
        );
    }
    let _ = dispatch_idb_named_event(scope, transaction, "abort", |_, _| {});
    release_indexed_db_transaction_dispatch_refs(scope, transaction);
    if let Some((request, database)) = aborted_open {
        finish_aborted_upgrade_open(scope, request, database);
    }
}
