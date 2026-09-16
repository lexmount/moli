use super::*;
use moli_indexeddb::IndexedDbError;

pub(super) fn fail_readwrite_transaction_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    error: &IndexedDbError,
) {
    let error = request_error_object(scope, error);
    let _ = transaction.set(scope, v8str(scope, "error").into(), error);
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
    unregister_readwrite_transaction(scope, transaction);
    if let Some(database) =
        crate::context_bootstrap::indexed_db::indexed_db_transaction_database(scope, transaction)
    {
        crate::context_bootstrap::indexed_db::finish_indexed_db_database_close(scope, database);
    }
    let _ = dispatch_idb_named_event(scope, transaction, "error", |_, _| {});
}
