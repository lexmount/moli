//! Schema changes are internal entries in the common transaction operation
//! queue. Index backfill holds that entry across all bounded record round trips.

use crate::context_bootstrap::indexed_db::{
    INDEXED_DB_TRANSACTION_ABORTED_SLOT, INDEXED_DB_TRANSACTION_FINISHED_SLOT, IndexInfo,
    IndexedDbError, IndexedDbTransactionOperation, Key, KeyConversionError, KeyPath,
    TransactionHandle, deserialize_js_value, extract_index_keys_from_value,
    finish_transaction_abort, object_bool_property, request_error_object,
    schedule_internal_transaction_operation,
    typed_state::sql::{
        ensure_sql_transaction_rollback, sql_transaction_cancellation,
        submit_indexed_db_sql_task_reserved,
    },
};
use moli_indexeddb::sqlite::{
    Query, Seek,
    service::{Command, CommandResult, Completion, DatabaseClient, TaskPermit},
};

struct Backfill {
    transaction: TransactionHandle,
    store: String,
    index: IndexInfo,
}

impl Backfill {
    fn retained_bytes(&self) -> usize {
        let path = match &self.index.key_path {
            KeyPath::String(path) => path.capacity(),
            KeyPath::Sequence(paths) => paths
                .capacity()
                .saturating_mul(std::mem::size_of::<String>())
                .saturating_add(
                    paths
                        .iter()
                        .fold(0usize, |size, path| size.saturating_add(path.capacity())),
                ),
        };
        std::mem::size_of::<Self>()
            .saturating_add(self.store.capacity())
            .saturating_add(self.index.name.capacity())
            .saturating_add(path)
    }

    fn read<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        target: v8::Local<'s, v8::Object>,
        client: DatabaseClient,
        permit: TaskPermit,
        after: Option<Key>,
    ) -> bool {
        let command = Command::Query {
            transaction: self.transaction,
            store: self.store.clone(),
            query: Query {
                seek: after.map(|key| Seek {
                    key,
                    primary_key: None,
                    inclusive: false,
                }),
                ..Default::default()
            },
        };
        let retained = self.retained_bytes();
        let next_client = client.clone();
        submit(
            scope,
            target,
            &client,
            permit,
            command,
            retained,
            move |scope, target, reply| {
                if stopped(scope, target) {
                    return;
                }
                let Completion { result, permit, .. } = reply;
                let record = match result {
                    Ok(CommandResult::Records(mut records)) if records.len() <= 1 => records.pop(),
                    Err(error) => {
                        abort(scope, target, error);
                        return;
                    }
                    _ => {
                        abort(
                            scope,
                            target,
                            IndexedDbError::InvalidState(
                                "Invalid index backfill query result".into(),
                            ),
                        );
                        return;
                    }
                };
                let Some(record) = record else {
                    drop(permit);
                    finish(scope, target);
                    return;
                };
                let Some(value) = record.value else {
                    abort(
                        scope,
                        target,
                        IndexedDbError::Serialization("Index backfill has no record value".into()),
                    );
                    return;
                };
                let after = record.primary_key;
                // The projected key duplicates primary_key for an object-store
                // query. Release it before allocating derived keys or the next
                // command's copy of the seek position.
                drop(record.key);
                let cancel = sql_transaction_cancellation(scope, target);
                let keys = {
                    v8::tc_scope!(let scope, scope);
                    let keys = deserialize_js_value(scope, &value)
                        .ok_or_else(|| {
                            KeyConversionError::Backend(IndexedDbError::Serialization(
                                "Unable to decode an index backfill record".into(),
                            ))
                        })
                        .and_then(|value| {
                            extract_index_keys_from_value(
                                scope,
                                value,
                                &self.index.key_path,
                                self.index.multi_entry,
                            )
                        });
                    if matches!(keys, Err(KeyConversionError::Terminated))
                        || scope.is_execution_terminating()
                        || !scope.can_continue()
                    {
                        if scope.has_caught() {
                            scope.rethrow();
                        }
                        Err(KeyConversionError::Terminated)
                    } else {
                        keys
                    }
                };
                drop(value);
                let keys = match keys {
                    Ok(keys) => keys,
                    Err(KeyConversionError::Terminated) => {
                        cancel();
                        return;
                    }
                    Err(error) => {
                        if let Some(error) = error.into_value(scope) {
                            finish_transaction_abort(scope, target, error);
                        }
                        return;
                    }
                };
                let command = Command::BackfillIndex {
                    transaction: self.transaction,
                    store: self.store.clone(),
                    name: self.index.name.clone(),
                    primary_key: after.clone(),
                    keys,
                };
                let retained = self.retained_bytes().saturating_add(after.heap_bytes());
                let again = next_client.clone();
                submit(
                    scope,
                    target,
                    &next_client,
                    permit,
                    command,
                    retained,
                    move |scope, target, reply| {
                        if stopped(scope, target) {
                            return;
                        }
                        match reply.result {
                            Ok(CommandResult::Unit) => {
                                self.read(scope, target, again, reply.permit, Some(after));
                            }
                            Err(error) => abort(scope, target, error),
                            _ => abort(
                                scope,
                                target,
                                IndexedDbError::InvalidState(
                                    "Invalid index backfill write result".into(),
                                ),
                            ),
                        }
                    },
                );
            },
        )
    }
}

pub(in crate::context_bootstrap::indexed_db) fn schedule_schema_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    client: DatabaseClient,
    command: Command,
    backfill: Option<(String, IndexInfo)>,
) -> Result<(), IndexedDbError> {
    let handle = crate::context_bootstrap::indexed_db::transaction_handle_from_value(
        scope,
        transaction.into(),
    )
    .ok_or_else(|| {
        IndexedDbError::TransactionInactive("Schema operation lost its transaction".into())
    })?;
    let backfill = backfill.map(|(store, index)| Backfill {
        transaction: handle,
        store,
        index,
    });
    let retained = backfill.as_ref().map_or(0, Backfill::retained_bytes);
    let permit = client
        .try_reserve(
            command.kind(),
            command.input_bytes().saturating_add(retained),
        )
        .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
    let operation = IndexedDbTransactionOperation::sql(permit, move |scope, target, permit| {
        if let Err(error) = ensure_sql_transaction_rollback(scope, target) {
            abort(scope, target, error);
            return false;
        }
        let next = client.clone();
        submit(
            scope,
            target,
            &client,
            permit,
            command,
            retained,
            move |scope, target, reply| {
                if stopped(scope, target) {
                    return;
                }
                match reply.result {
                    Ok(CommandResult::Unit) => {
                        if let Some(backfill) = backfill {
                            backfill.read(scope, target, next, reply.permit, None);
                        } else {
                            drop(reply.permit);
                            finish(scope, target);
                        }
                    }
                    Err(error) => abort(scope, target, error),
                    _ => abort(
                        scope,
                        target,
                        IndexedDbError::InvalidState("Invalid SQL schema result".into()),
                    ),
                }
            },
        )
    })?;
    schedule_internal_transaction_operation(scope, transaction, operation)
}

fn submit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    client: &DatabaseClient,
    mut permit: TaskPermit,
    command: Command,
    retained: usize,
    continuation: impl for<'a, 'i> FnOnce(
        &mut v8::PinScope<'a, 'i>,
        v8::Local<'a, v8::Object>,
        Completion,
    ) + 'static,
) -> bool {
    let bytes = command.input_bytes().saturating_add(retained);
    let result_bytes = if matches!(command, Command::Query { .. }) {
        // Recycle the previous record's reservation. Repeated batches must not
        // accumulate budget for values that have already been dropped.
        permit.reserved_bytes().saturating_sub(bytes)
    } else {
        0
    };
    let result = permit
        .try_resize(permit.reserved_bytes().max(bytes))
        .map_err(|error| IndexedDbError::Resource(error.to_string()))
        .and_then(|()| {
            submit_indexed_db_sql_task_reserved(
                scope,
                target,
                client,
                permit,
                command,
                result_bytes,
                continuation,
            )
        });
    if let Err(error) = result {
        abort(scope, target, error);
        false
    } else {
        true
    }
}

fn finish<'s>(scope: &mut v8::PinScope<'s, '_>, target: v8::Local<'s, v8::Object>) {
    crate::context_bootstrap::indexed_db::finish_transaction_operation(scope, target);
}

fn stopped<'s>(scope: &mut v8::PinScope<'s, '_>, target: v8::Local<'s, v8::Object>) -> bool {
    object_bool_property(scope, target, INDEXED_DB_TRANSACTION_ABORTED_SLOT).unwrap_or(false)
        || object_bool_property(scope, target, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
            .unwrap_or(false)
}

fn abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    error: IndexedDbError,
) {
    let error = request_error_object(scope, &error);
    finish_transaction_abort(scope, target, error);
}
