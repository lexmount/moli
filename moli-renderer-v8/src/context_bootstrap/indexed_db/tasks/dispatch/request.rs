use super::*;
use crate::context_bootstrap::indexed_db::{
    indexed_db_transaction_status, update_indexed_db_transaction_status,
};

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
    if indexed_db_transaction_status(scope, transaction)
        .is_some_and(|state| state.finished || state.committing || state.aborted)
    {
        return;
    }
    let _ = update_indexed_db_transaction_status(scope, transaction, |state| state.active = true);
}
