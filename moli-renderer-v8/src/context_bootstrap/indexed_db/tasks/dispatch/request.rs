use super::*;

mod abort;
mod error;
mod finish;
mod success;

pub(in crate::context_bootstrap::indexed_db) use self::error::flush_request_error_task;
pub(in crate::context_bootstrap::indexed_db) use self::success::flush_request_success_task;

fn set_transaction_active_for_request_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) {
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
        .unwrap_or(false)
        || object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_COMMITTING_SLOT)
            .unwrap_or(false)
    {
        return;
    }
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_ACTIVE_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
}
