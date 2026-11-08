use super::start::enqueue_transaction_start_task;
use super::*;

pub(in crate::context_bootstrap::indexed_db) fn enqueue_ready_transaction_starts(
    scope: &mut v8::PinScope<'_, '_>,
) {
    crate::context_bootstrap::indexed_db::typed_state::factory::enqueue_native_connection_notifications(scope);
    let Some(queue) = indexed_db_runtime_array(scope, IndexedDbRuntimeArray::Transactions) else {
        return;
    };
    for index in 0..queue.length() {
        let Some(value) = queue.get_index(scope, index) else {
            continue;
        };
        let Ok(transaction) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
            .unwrap_or(false)
        {
            continue;
        }
        let Some(request) =
            crate::context_bootstrap::indexed_db::indexed_db_transaction_start_request(
                scope,
                transaction,
            )
        else {
            continue;
        };
        match request.status() {
            moli_indexeddb::TransactionRequestStatus::Canceled => {
                super::enqueue_transaction_cancel_task(scope, transaction);
            }
            moli_indexeddb::TransactionRequestStatus::Ready => {
                enqueue_transaction_start_task(scope, transaction);
            }
            _ => {}
        }
    }
    for database in crate::context_bootstrap::indexed_db::indexed_db_forced_closed_databases(scope)
    {
        crate::context_bootstrap::indexed_db::typed_state::factory::finish_forced_upgrade(
            scope, database,
        );
        crate::context_bootstrap::indexed_db::finish_indexed_db_database_close(scope, database);
    }
}
