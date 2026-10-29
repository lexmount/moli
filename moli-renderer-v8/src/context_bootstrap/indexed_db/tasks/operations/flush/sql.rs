use super::*;
use crate::context_bootstrap::indexed_db::{
    stores::collection::CollectionKind,
    typed_state::sql::{ensure_sql_transaction_rollback, submit_indexed_db_sql_task_reserved},
};
use moli_indexeddb::sqlite::{
    Query, QuerySource,
    service::{Command, CommandResult, DatabaseClient},
};

mod cursor;
mod write;

#[derive(Clone, Copy)]
enum Output {
    Single(CollectionKind),
    Many(CollectionKind),
    Count,
    Unit,
    Key,
}

pub(super) fn prepare_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    client: &DatabaseClient,
    operation: operation::TransactionOperation<'s>,
) -> Result<IndexedDbTransactionOperation, IndexedDbError> {
    if matches!(
        operation.kind,
        IndexedDbTransactionOperation::OpenCursor(_)
            | IndexedDbTransactionOperation::CursorIterate { .. }
    ) {
        return cursor::prepare(scope, client, operation);
    }
    if matches!(
        operation.kind,
        IndexedDbTransactionOperation::ObjectStoreWrite { .. }
    ) {
        return write::prepare(scope, client, operation);
    }
    let (command, output) = command(scope, operation)?;
    let permit = client
        .try_reserve(command.kind(), command.input_bytes())
        .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
    let client = client.clone();
    IndexedDbTransactionOperation::sql(permit, move |scope, request, permit| {
        if let Err(error) = ensure_sql_transaction_rollback(scope, request) {
            fail_request(scope, request, &error);
            return false;
        }
        let submitted = submit_indexed_db_sql_task_reserved(
            scope,
            request,
            &client,
            permit,
            command,
            0,
            move |scope, request, mut completion| {
                let cancel = crate::context_bootstrap::indexed_db::typed_state::sql::sql_transaction_cancellation(scope, request);
                let can_continue = match &completion.result {
                    Ok(result) => {
                        deliver_result(scope, request, output, result, &mut completion.permit)
                    }
                    Err(error) => {
                        fail_request(scope, request, error);
                        true
                    }
                };
                drop(completion);
                if can_continue {
                    operation::finish_transaction_operation(scope, request);
                } else {
                    cancel();
                }
            },
        );
        if let Err(error) = submitted {
            fail_request(scope, request, &error);
            false
        } else {
            true
        }
    })
}

fn command(
    scope: &mut v8::PinScope<'_, '_>,
    operation: operation::TransactionOperation<'_>,
) -> Result<(Command, Output), IndexedDbError> {
    let source = match &operation.kind {
        IndexedDbTransactionOperation::IndexGet { .. }
        | IndexedDbTransactionOperation::IndexGetKey { .. }
        | IndexedDbTransactionOperation::IndexGetAll(_)
        | IndexedDbTransactionOperation::IndexCount { .. } => {
            let index = indexed_db_index_info(scope, operation.source).ok_or_else(|| {
                IndexedDbError::InvalidState("IndexedDB index lost its metadata".into())
            })?;
            QuerySource::Index(index.name)
        }
        _ => QuerySource::ObjectStore,
    };
    let transaction = operation.handle;
    let store = operation.store_name;
    let (mut query, output) = match operation.kind {
        IndexedDbTransactionOperation::ObjectStoreGet { query }
        | IndexedDbTransactionOperation::IndexGet { query } => (
            Query {
                range: query,
                ..Default::default()
            },
            Output::Single(CollectionKind::Value),
        ),
        IndexedDbTransactionOperation::ObjectStoreGetKey { query }
        | IndexedDbTransactionOperation::IndexGetKey { query } => (
            Query {
                range: query,
                ..Default::default()
            },
            Output::Single(CollectionKind::Key),
        ),
        IndexedDbTransactionOperation::ObjectStoreGetAll(collection)
        | IndexedDbTransactionOperation::IndexGetAll(collection) => (
            Query {
                range: collection.query.unwrap_or_default(),
                direction: collection.direction,
                limit: collection
                    .count
                    .filter(|count| *count != 0)
                    .map(|count| count as u32)
                    .unwrap_or(u32::MAX),
                ..Default::default()
            },
            Output::Many(collection.kind),
        ),
        IndexedDbTransactionOperation::ObjectStoreCount { query }
        | IndexedDbTransactionOperation::IndexCount { query } => {
            return Ok((
                Command::Count {
                    transaction,
                    store,
                    source,
                    range: query.unwrap_or_default(),
                },
                Output::Count,
            ));
        }
        IndexedDbTransactionOperation::ObjectStoreDelete { query } => {
            return Ok((
                Command::Delete {
                    transaction,
                    store,
                    range: query,
                },
                Output::Unit,
            ));
        }
        IndexedDbTransactionOperation::ObjectStoreClear => {
            return Ok((
                Command::Delete {
                    transaction,
                    store,
                    range: Default::default(),
                },
                Output::Unit,
            ));
        }
        IndexedDbTransactionOperation::CursorDelete { key } => {
            return Ok((
                Command::Delete {
                    transaction,
                    store,
                    range: IdbKeyRangeQuery {
                        lower: Some(key.clone()),
                        upper: Some(key),
                        ..Default::default()
                    },
                },
                Output::Unit,
            ));
        }
        // A SQL transaction must never fall back to the legacy database.
        _ => {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB SQL operation is not yet connected".into(),
            ));
        }
    };
    query.source = source;
    query.keys_only = matches!(
        output,
        Output::Single(CollectionKind::Key) | Output::Many(CollectionKind::Key)
    );
    Ok((
        Command::Query {
            transaction,
            store,
            query,
        },
        output,
    ))
}

fn fail_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    error: &IndexedDbError,
) {
    let error = request_error_object(scope, error);
    store_request_error(scope, request, error);
}

fn deliver_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    output: Output,
    result: &CommandResult,
    budget: &mut moli_indexeddb::sqlite::service::TaskPermit,
) -> bool {
    v8::tc_scope!(let scope, scope);
    let mut session = crate::context_bootstrap::indexed_db::ValueDecodeSession::new(budget);
    let value = match (output, result) {
        (Output::Count, CommandResult::Count(count)) => {
            Ok(Some(v8::Number::new(scope, *count as f64).into()))
        }
        (Output::Unit, CommandResult::Unit) => Ok(Some(v8::undefined(scope).into())),
        (Output::Key, CommandResult::Key(key)) => Ok(
            crate::context_bootstrap::indexed_db::try_key_to_js_value(scope, key),
        ),
        (Output::Single(kind), CommandResult::Records(records)) => match records.first() {
            Some(record) => record_result(scope, kind, record, &mut session),
            None => Ok(Some(v8::undefined(scope).into())),
        },
        (Output::Many(kind), CommandResult::Records(records)) => {
            collection_result(scope, kind, records, &mut session)
        }
        _ => Ok(None),
    };
    if scope.is_execution_terminating() || !scope.can_continue() {
        if scope.has_caught() {
            scope.rethrow();
        }
        return false;
    }
    match value {
        Ok(Some(value)) => {
            session.publish();
            store_request_success(scope, request, value);
        }
        Err(error) => {
            // V8 may add a generic clone exception after the host delegate
            // refuses allocation. Keep the actual capacity/storage error.
            scope.reset();
            fail_request(scope, request, &error);
        }
        Ok(None) => {
            let error = scope.exception().unwrap_or_else(|| {
                request_error_object(
                    scope,
                    &IndexedDbError::Serialization(
                        "Unable to decode IndexedDB query result".into(),
                    ),
                )
            });
            store_request_error(scope, request, error);
        }
    }
    true
}

fn record_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: CollectionKind,
    record: &CursorSnapshotEntry,
    budget: &mut crate::context_bootstrap::indexed_db::ValueDecodeSession<'_>,
) -> Result<Option<v8::Local<'s, v8::Value>>, IndexedDbError> {
    if matches!(kind, CollectionKind::Key) {
        return Ok(crate::context_bootstrap::indexed_db::try_key_to_js_value(
            scope,
            &record.primary_key,
        ));
    }
    let Some(bytes) = &record.value else {
        return Ok(None);
    };
    let Some(value) = budget.deserialize(scope, bytes)? else {
        return Ok(None);
    };
    Ok(if matches!(kind, CollectionKind::Record) {
        crate::context_bootstrap::indexed_db::record::create_record(
            scope,
            &record.key,
            &record.primary_key,
            value,
        )
        .map(Into::into)
    } else {
        Some(value)
    })
}

fn collection_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: CollectionKind,
    records: &[CursorSnapshotEntry],
    budget: &mut crate::context_bootstrap::indexed_db::ValueDecodeSession<'_>,
) -> Result<Option<v8::Local<'s, v8::Value>>, IndexedDbError> {
    let array = v8::Array::new(scope, 0);
    for (index, record) in records.iter().enumerate() {
        let Some(value) = record_result(scope, kind, record, budget)? else {
            return Ok(None);
        };
        let Some(index) = u32::try_from(index).ok() else {
            return Ok(None);
        };
        if moli_webapi_declare::define_array_data_property(scope, array, index, value).is_none() {
            return Ok(None);
        }
    }
    Ok(Some(array.into()))
}
