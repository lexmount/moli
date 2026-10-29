use super::*;
use crate::context_bootstrap::indexed_db::{
    indexed_db_transaction_mode,
    typed_state::sql::{
        ensure_sql_transaction_rollback, indexed_db_request_sql_client,
        submit_indexed_db_sql_task_reserved,
    },
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
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_START_SCHEDULED_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    if object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_STARTED_SLOT)
        .unwrap_or(false)
        || object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
            .unwrap_or(false)
        || object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
            .unwrap_or(false)
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
        // Reserve bounded cleanup memory before BEGIN. Waiting requests and
        // dormant cleanup guards acquire execution slots only when submitted.
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
            stores: request.store_names().into_iter().collect(),
            version: None,
            admission: Some(request),
        };
        let submitted = client
            .try_reserve(command.input_bytes())
            .map_err(|error| IndexedDbError::Resource(error.to_string()))
            .and_then(|permit| {
                set_indexed_db_slot_value(
                    scope,
                    transaction,
                    INDEXED_DB_TRANSACTION_START_SCHEDULED_SLOT,
                    v8::Boolean::new(scope, true).into(),
                );
                submit_indexed_db_sql_task_reserved(
                    scope,
                    transaction,
                    &client,
                    permit,
                    command,
                    0,
                    move |scope, transaction, reply| {
                        if object_bool_property(
                            scope,
                            transaction,
                            INDEXED_DB_TRANSACTION_ABORTED_SLOT,
                        )
                        .unwrap_or(false)
                        {
                            return;
                        }
                        match reply.result {
                            Ok(CommandResult::Unit) => {
                                success::start_regular_transaction(scope, transaction, handle)
                            }
                            Err(error) => {
                                fail::fail_regular_transaction_start(scope, transaction, &error)
                            }
                            _ => fail::fail_regular_transaction_start(
                                scope,
                                transaction,
                                &IndexedDbError::InvalidState("Invalid SQL BEGIN response".into()),
                            ),
                        }
                    },
                )
            });
        if let Err(error) = submitted {
            fail::fail_regular_transaction_start(scope, transaction, &error);
        }
    }
}
