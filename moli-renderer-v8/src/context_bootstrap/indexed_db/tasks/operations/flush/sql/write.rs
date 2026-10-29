use super::*;
use crate::context_bootstrap::indexed_db::typed_state::sql::sql_transaction_cancellation;
use crate::context_bootstrap::indexed_db::{
    IndexedDbObjectStoreMetadata, extract_index_keys_budgeted, indexed_db_object_store_metadata,
    reserve_vec,
};
use moli_indexeddb::ReadBudget;
use moli_indexeddb::sqlite::{
    IndexKeys, PreparedWrite,
    service::{Completion, TaskPermit},
};

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
    let permit = client
        .try_reserve(
            moli_indexeddb::sqlite::service::TaskKind::Data,
            write.input_bytes(),
        )
        .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
    let client = client.clone();
    IndexedDbTransactionOperation::sql(permit, move |scope, request, permit| {
        write.start(scope, request, client, permit)
    })
}

impl Write {
    fn input_bytes(&self) -> usize {
        let key_bytes = match &self.prepared.key {
            PreparedObjectStoreKey::Explicit(key) => key.heap_bytes(),
            PreparedObjectStoreKey::Generate => 0,
            PreparedObjectStoreKey::GenerateInline { key_path } => key_path.capacity(),
        };
        std::mem::size_of::<Self>()
            .saturating_add(self.store.capacity())
            .saturating_add(key_bytes)
            .saturating_add(self.prepared.value.heap_bytes())
    }

    fn start<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        request: v8::Local<'s, v8::Object>,
        client: DatabaseClient,
        mut permit: TaskPermit,
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
            let cancel = Command::CancelWrite {
                transaction,
                request: 0,
            };
            let cleanup = match client.try_reserve(cancel.kind(), cancel.input_bytes()) {
                Ok(permit) => permit,
                Err(error) => {
                    fail_request(scope, request, &IndexedDbError::Resource(error.to_string()));
                    return false;
                }
            };
            let command = Command::PrepareWrite {
                transaction,
                store: self.store.clone(),
                add_only: self.add_only,
            };
            let completion_client = client.clone();
            let submitted = submit_indexed_db_sql_task_reserved(
                scope,
                request,
                &client,
                permit,
                command,
                0,
                move |scope, request, reply| {
                    let Completion {
                        result, mut permit, ..
                    } = reply;
                    let in_flight = match result {
                        Ok(CommandResult::Prepared(prepared)) => {
                            let id = prepared.id;
                            match self.command(scope, Some(prepared), &mut permit) {
                                Ok(command) => submit_write(
                                    scope,
                                    request,
                                    &completion_client,
                                    permit,
                                    command,
                                    Some((id, cleanup)),
                                ),
                                Err(KeyConversionError::Terminated) => {
                                    cancel_state();
                                    false
                                }
                                Err(error) => cancel_write(
                                    scope,
                                    request,
                                    &completion_client,
                                    cleanup,
                                    transaction,
                                    id,
                                    error,
                                ),
                            }
                        }
                        Err(error) => {
                            fail_request(scope, request, &error);
                            drop(permit);
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
            match self.command(scope, None, &mut permit) {
                Ok(command) => submit_write(scope, request, &client, permit, command, None),
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
        budget: &mut dyn ReadBudget,
    ) -> Result<Command, KeyConversionError> {
        v8::tc_scope!(let scope, scope);
        let result = self.materialize(scope, generated, budget);
        if matches!(result, Err(KeyConversionError::Terminated))
            || scope.is_execution_terminating()
            || !scope.can_continue()
        {
            if scope.has_caught() {
                scope.rethrow();
            }
            return Err(KeyConversionError::Terminated);
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
        budget: &mut dyn ReadBudget,
    ) -> Result<Command, KeyConversionError> {
        let mut value = self.prepared.value;
        let (key, inline) = match self.prepared.key {
            PreparedObjectStoreKey::Explicit(key) => (Some(key), None),
            PreparedObjectStoreKey::GenerateInline { key_path } => (None, Some(key_path)),
            PreparedObjectStoreKey::Generate => (None, None),
        };
        let decoded = if inline.is_some() || !self.metadata.index_names().is_empty() {
            Some(deserialize_js_value(scope, &value).ok_or_else(|| {
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
            reserve_vec(budget, &mut indexes, self.metadata.index_names().len())?;
        }
        for name in self.metadata.index_names() {
            let index = self
                .metadata
                .index(name)
                .expect("declared index has a definition");
            let keys = extract_index_keys_budgeted(
                scope,
                decoded.expect("indexed writes restore their snapshot"),
                &index.key_path,
                index.multi_entry,
                budget,
            )?;
            budget.charge(index.name.len() as u64)?;
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
    permit: TaskPermit,
    command: Command,
    pending: Option<(u64, TaskPermit)>,
) -> bool {
    let transaction = match &command {
        Command::Write { transaction, .. } | Command::FinishWrite { transaction, .. } => {
            *transaction
        }
        _ => unreachable!(),
    };
    let submitted = submit_indexed_db_sql_task_reserved(
        scope,
        request,
        client,
        permit,
        command,
        0,
        |scope, request, reply| {
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
            if let Some((id, cleanup)) = pending {
                cancel_write(
                    scope,
                    request,
                    client,
                    cleanup,
                    transaction,
                    id,
                    error.into(),
                )
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
    permit: TaskPermit,
    transaction: TransactionHandle,
    id: u64,
    error: KeyConversionError,
) -> bool {
    let submitted = submit_indexed_db_sql_task_reserved(
        scope,
        request,
        client,
        permit,
        Command::CancelWrite {
            transaction,
            request: id,
        },
        0,
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
        sql_transaction_cancellation(scope, request)();
        fail_request(scope, request, &error);
        false
    } else {
        true
    }
}
