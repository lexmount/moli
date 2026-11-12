use super::*;
use crate::context_bootstrap::indexed_db::{
    indexed_db_transaction_mode, indexed_db_transaction_status,
    typed_state::sql::{
        ensure_sql_transaction_rollback, indexed_db_request_sql_client, submit_indexed_db_sql_task,
    },
    update_indexed_db_transaction_status,
};

use moli_indexeddb::sqlite::service::{Command, CommandResult};

mod fail;
mod success;

pub(in crate::context_bootstrap::indexed_db) fn flush_transaction_start_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(transaction) = indexed_db_object_task_target(scope, task) else {
        return;
    };
    if crate::context_bootstrap::indexed_db::finish_canceled_transaction(scope, transaction) {
        return;
    }
    let _ = update_indexed_db_transaction_status(scope, transaction, |state| {
        state.start_scheduled = false
    });
    if indexed_db_transaction_status(scope, transaction)
        .is_some_and(|state| state.started || state.aborted || state.finished)
    {
        return;
    }
    let Some(request) = crate::context_bootstrap::indexed_db::indexed_db_transaction_start_request(
        scope,
        transaction,
    ) else {
        return;
    };
    if !request.is_ready() {
        return;
    }
    if let Some(client) = indexed_db_request_sql_client(scope, transaction) {
        // Arm rollback before BEGIN so owner retirement can clean up accepted work.
        if let Err(error) = ensure_sql_transaction_rollback(scope, transaction) {
            fail::fail_regular_transaction_start(scope, transaction, &error);
            return;
        }
        let Some(handle) = transaction_handle_from_value(scope, transaction.into()) else {
            return;
        };
        let Some(mode) = indexed_db_transaction_mode(scope, transaction) else {
            return;
        };
        let command = Command::Begin {
            transaction: handle,
            mode,
            stores: request.stores().into_iter().collect(),
            version: None,
            admission: Some(request),
        };
        let _ = update_indexed_db_transaction_status(scope, transaction, |state| {
            state.start_scheduled = true
        });
        let submitted = submit_indexed_db_sql_task(
            scope,
            transaction,
            &client,
            command,
            move |scope, transaction, reply| {
                if indexed_db_transaction_status(scope, transaction)
                    .is_some_and(|state| state.aborted)
                {
                    return;
                }
                match reply.result {
                    Ok(CommandResult::Unit) => {
                        success::start_regular_transaction(scope, transaction, handle)
                    }
                    Err(error) => fail::fail_regular_transaction_start(scope, transaction, &error),
                    _ => fail::fail_regular_transaction_start(
                        scope,
                        transaction,
                        &IndexedDbError::InvalidState("Invalid SQL BEGIN response".into()),
                    ),
                }
            },
        );
        if let Err(error) = submitted {
            fail::fail_regular_transaction_start(scope, transaction, &error);
        }
    }
}
