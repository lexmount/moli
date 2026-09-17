use super::*;

mod fail;
mod run_waiting;
mod success;

pub(in crate::context_bootstrap::indexed_db) fn flush_transaction_start_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(transaction) = indexed_db_transaction_task_transaction(scope, task) else {
        return;
    };
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_START_SCHEDULED_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_STARTED_SLOT)
        .unwrap_or(false)
        || object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
            .unwrap_or(false)
    {
        return;
    }
    let Some(request) = indexed_db_transaction_start_request(scope, transaction) else {
        return;
    };
    if !request.is_ready() {
        return;
    }
    match with_indexed_db_manager(scope, |manager| manager.start_queued_transaction(&request)) {
        Ok(transaction_handle) => {
            success::start_regular_transaction(scope, transaction, transaction_handle);
        }
        Err(error) => {
            fail::fail_regular_transaction_start(scope, transaction, &error);
        }
    }
}
