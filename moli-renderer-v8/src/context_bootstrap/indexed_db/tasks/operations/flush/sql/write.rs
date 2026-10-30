use super::*;
use crate::context_bootstrap::indexed_db::typed_state::sql::sql_transaction_cancellation;
use crate::context_bootstrap::indexed_db::typed_state::sql::{
    abort_sql_dispatch, submit_indexed_db_sql_quota_task,
};
use crate::context_bootstrap::indexed_db::{
    IndexedDbObjectStoreMetadata, extract_index_keys_from_value, indexed_db_object_store_metadata,
    reserve_vec,
};
use moli_indexeddb::sqlite::{IndexKeys, PreparedWrite, service::Completion};

struct Write {
    transaction: TransactionHandle,
    store: String,
    prepared: PreparedObjectStoreWrite,
    metadata: IndexedDbObjectStoreMetadata,
    add_only: bool,
}

pub(super) fn prepare<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    client: &DatabaseClient,
    operation: operation::TransactionOperation<'s>,
) -> Result<IndexedDbTransactionOperation, IndexedDbError> {
    let IndexedDbTransactionOperation::ObjectStoreWrite { prepared, add_only } = operation.kind
    else {
        unreachable!("SQL write dispatcher accepts prepared writes");
    };
    let metadata = indexed_db_object_store_metadata(scope, operation.source)
        .ok_or_else(|| IndexedDbError::InvalidState("IndexedDB store lost its metadata".into()))?;
    let write = Write {
        transaction: operation.handle,
        store: operation.store_name,
        prepared,
        metadata,
        add_only,
    };
    let client = client.clone();
    Ok(IndexedDbTransactionOperation::sql(move |scope, request| {
        write.start(scope, request, client)
    }))
}

impl Write {
    fn start<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        request: v8::Local<'s, v8::Object>,
        client: DatabaseClient,
    ) -> bool {
        if let Err(error) = ensure_sql_transaction_rollback(scope, request) {
            fail_request(scope, request, &error);
            return false;
        }
        let transaction = self.transaction;
        let cancel_state = sql_transaction_cancellation(scope, request);
        if matches!(
            self.prepared.key,
            PreparedObjectStoreKey::GenerateInline { .. }
        ) {
            let command = Command::PrepareWrite {
                transaction,
                store: self.store.clone(),
                add_only: self.add_only,
            };
            let completion_client = client.clone();
            let submitted = submit_indexed_db_sql_task(
                scope,
                request,
                &client,
                command,
                move |scope, request, reply| {
                    let Completion { result, .. } = reply;
                    let in_flight = match result {
                        Ok(CommandResult::Prepared(prepared)) => {
                            let id = prepared.id;
                            match self.command(scope, Some(prepared)) {
                                Ok(command) => submit_write(
                                    scope,
                                    request,
                                    &completion_client,
                                    command,
                                    Some(id),
                                ),
                                Err(KeyConversionError::Terminated) => {
                                    cancel_state();
                                    false
                                }
                                Err(error) => cancel_write(
                                    scope,
                                    request,
                                    &completion_client,
                                    transaction,
                                    id,
                                    error,
                                ),
                            }
                        }
                        Err(error) => {
                            fail_request(scope, request, &error);
                            false
                        }
                        _ => {
                            cancel_state();
                            fail_request(
                                scope,
                                request,
                                &IndexedDbError::InvalidState("Invalid SQL prepare result".into()),
                            );
                            false
                        }
                    };
                    if !in_flight {
                        operation::finish_transaction_operation(scope, request);
                    }
                },
            );
            if let Err(error) = submitted {
                fail_request(scope, request, &error);
                false
            } else {
                true
            }
        } else {
            match self.command(scope, None) {
                Ok(command) => submit_write(scope, request, &client, command, None),
                Err(KeyConversionError::Terminated) => {
                    cancel_state();
                    false
                }
                Err(error) => {
                    if let Some(error) = error.into_value(scope) {
                        store_request_error(scope, request, error);
                    }
                    false
                }
            }
        }
    }

    fn command(
        self,
        scope: &mut v8::PinScope<'_, '_>,
        generated: Option<PreparedWrite>,
    ) -> Result<Command, KeyConversionError> {
        v8::tc_scope!(let scope, scope);
        let result = self.materialize(scope, generated);
        if matches!(result, Err(KeyConversionError::Terminated))
            || scope.is_execution_terminating()
            || !scope.can_continue()
        {
            if scope.has_caught() {
                scope.rethrow();
            }
            return Err(KeyConversionError::Terminated);
        }
        if matches!(
            &result,
            Err(KeyConversionError::Backend(IndexedDbError::Resource(_)))
        ) {
            scope.reset();
            return result;
        }
        if let Some(error) = scope.exception() {
            return Err(KeyConversionError::Exception(v8::Global::new(scope, error)));
        }
        result
    }

    fn materialize(
        self,
        scope: &mut v8::PinScope<'_, '_>,
        generated: Option<PreparedWrite>,
    ) -> Result<Command, KeyConversionError> {
        let session = crate::context_bootstrap::indexed_db::ValueDecodeSession::new();
        let mut value = self.prepared.value;
        let (key, inline) = match self.prepared.key {
            PreparedObjectStoreKey::Explicit(key) => (Some(key), None),
            PreparedObjectStoreKey::GenerateInline { key_path } => (None, Some(key_path)),
            PreparedObjectStoreKey::Generate => (None, None),
        };
        let decoded = if inline.is_some() || !self.metadata.index_names().is_empty() {
            Some(session.deserialize(scope, &value)?.ok_or_else(|| {
                IndexedDbError::Serialization("Failed to read a prepared IndexedDB value".into())
            })?)
        } else {
            None
        };
        if let Some(path) = inline {
            let generated = generated.as_ref().ok_or_else(|| {
                IndexedDbError::InvalidState("Inline write has no generated key".into())
            })?;
            let decoded = decoded.expect("inline writes restore their snapshot");
            inject_key_path_into_value(scope, decoded, &path, &generated.key).ok_or_else(|| {
                IndexedDbError::Serialization("Failed to inject a prepared inline key".into())
            })?;
            value = serialize_js_value(scope, decoded).ok_or_else(|| {
                IndexedDbError::Serialization("Failed to store a prepared IndexedDB value".into())
            })?;
        }
        let mut indexes = Vec::new();
        if !self.metadata.index_names().is_empty() {
            reserve_vec(&mut indexes, self.metadata.index_names().len())?;
        }
        for name in self.metadata.index_names() {
            let index = self
                .metadata
                .index(name)
                .expect("declared index has a definition");
            let keys = extract_index_keys_from_value(
                scope,
                decoded.expect("indexed writes restore their snapshot"),
                &index.key_path,
                index.multi_entry,
            )?;
            indexes.push(IndexKeys {
                name: index.name.clone(),
                keys,
            });
        }
        Ok(match generated {
            Some(prepared) => Command::FinishWrite {
                transaction: self.transaction,
                request: prepared.id,
                value,
                indexes,
                quota: None,
            },
            None => Command::Write {
                transaction: self.transaction,
                store: self.store,
                key,
                value,
                indexes,
                add_only: self.add_only,
                quota: None,
            },
        })
    }
}

fn submit_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    client: &DatabaseClient,
    command: Command,
    pending: Option<u64>,
) -> bool {
    let transaction = match &command {
        Command::Write { transaction, .. } | Command::FinishWrite { transaction, .. } => {
            *transaction
        }
        _ => unreachable!(),
    };
    let submitted = submit_indexed_db_sql_quota_task(
        scope,
        request,
        client,
        command,
        |scope, request, reply| {
            let reply = match reply {
                Ok(reply) => reply,
                Err(error) => {
                    abort_sql_dispatch(scope, request, error);
                    return;
                }
            };
            let cancel = sql_transaction_cancellation(scope, request);
            let can_continue = match &reply.result {
                Ok(result) => deliver_result(scope, request, Output::Key, result),
                Err(error) => {
                    fail_request(scope, request, error);
                    true
                }
            };
            drop(reply);
            if can_continue {
                operation::finish_transaction_operation(scope, request);
            } else {
                cancel();
            }
        },
    );
    match submitted {
        Ok(()) => true,
        Err(error) => {
            if let Some(id) = pending {
                cancel_write(scope, request, client, transaction, id, error.into())
            } else {
                fail_request(scope, request, &error);
                false
            }
        }
    }
}

fn cancel_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    client: &DatabaseClient,
    transaction: TransactionHandle,
    id: u64,
    error: KeyConversionError,
) -> bool {
    let cancel = sql_transaction_cancellation(scope, request);
    let submitted =
        crate::context_bootstrap::indexed_db::typed_state::sql::submit_indexed_db_sql_task(
            scope,
            request,
            client,
            Command::CancelWrite {
                transaction,
                request: id,
            },
            move |scope, request, reply| {
                if let Err(error) = &reply.result {
                    fail_request(scope, request, error);
                } else if let Some(error) = error.into_value(scope) {
                    store_request_error(scope, request, error);
                }
                drop(reply);
                operation::finish_transaction_operation(scope, request);
            },
        );
    if let Err(error) = submitted {
        if scope.is_execution_terminating() {
            cancel();
        } else {
            // A live owner still owes request/error/abort events. Retirement
            // cleanup discards them, so use the normal rollback receipt path.
            abort_sql_dispatch(scope, request, error);
        }
        false
    } else {
        true
    }
}
