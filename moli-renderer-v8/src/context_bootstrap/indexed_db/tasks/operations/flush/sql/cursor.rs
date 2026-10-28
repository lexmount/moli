use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbCursorIteration, IndexedDbCursorSource, SqlCursorQuery, attach_indexed_db_sql_cursor,
    indexed_db_cursor_state, stage_indexed_db_sql_cursor_page, store_cursor_iteration_result,
    typed_state::sql::sql_transaction_cancellation,
};
use moli_indexeddb::sqlite::{Seek, service::Completion};
use std::rc::Rc;

enum Target {
    Open(v8::Global<v8::Object>),
    Iterate(v8::Global<v8::Object>),
}

pub(super) fn prepare<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    client: &DatabaseClient,
    operation: operation::TransactionOperation<'s>,
) -> Result<IndexedDbTransactionOperation, IndexedDbError> {
    let (descriptor, query, target) = match operation.kind {
        IndexedDbTransactionOperation::OpenCursor(input) => {
            let source = match input.source {
                IndexedDbCursorSource::ObjectStore => QuerySource::ObjectStore,
                IndexedDbCursorSource::Index(index) => QuerySource::Index(index.name),
            };
            let query = Query {
                source,
                range: input.query.unwrap_or_default(),
                direction: input.direction,
                keys_only: input.key_only,
                ..Default::default()
            };
            let descriptor = Rc::new(SqlCursorQuery {
                store: operation.store_name,
                query: query.clone(),
            });
            (
                descriptor,
                query,
                Target::Open(v8::Global::new(scope, operation.source)),
            )
        }
        IndexedDbTransactionOperation::CursorIterate { cursor, movement } => {
            let local = v8::Local::new(scope, &cursor);
            let state = indexed_db_cursor_state(scope, local)
                .ok_or_else(|| IndexedDbError::InvalidState("SQL cursor was released".into()))?;
            let page = state
                .sql
                .as_ref()
                .ok_or_else(|| IndexedDbError::InvalidState("SQL cursor lost its query".into()))?;
            let current = state
                .entries
                .get(state.position.ok_or_else(|| {
                    IndexedDbError::InvalidState("SQL cursor has no position".into())
                })?)
                .ok_or_else(|| {
                    IndexedDbError::InvalidState("SQL cursor has no current record".into())
                })?;
            let mut query = page.query.query.clone();
            let source_is_index = matches!(query.source, QuerySource::Index(_));
            let default_seek = || Seek {
                key: current.key.clone(),
                primary_key: (source_is_index && !query.direction.is_unique())
                    .then(|| current.primary_key.clone()),
                inclusive: false,
            };
            let (seek, skip) = match movement {
                IndexedDbCursorIteration::Continue(Some(key)) => (
                    Seek {
                        key,
                        primary_key: None,
                        inclusive: true,
                    },
                    0,
                ),
                IndexedDbCursorIteration::Continue(None) => (default_seek(), 0),
                IndexedDbCursorIteration::Advance(count) => (default_seek(), count - 1),
                IndexedDbCursorIteration::PrimaryKey { key, primary_key } => (
                    Seek {
                        key,
                        primary_key: Some(primary_key),
                        inclusive: true,
                    },
                    0,
                ),
            };
            query.seek = Some(seek);
            query.skip = skip;
            (Rc::clone(&page.query), query, Target::Iterate(cursor))
        }
        _ => unreachable!("cursor dispatcher only accepts cursor operations"),
    };
    let command = Command::Query {
        transaction: operation.handle,
        store: descriptor.store.clone(),
        query,
    };
    // The lazy page also retains the original query, including its range. The
    // worker command and current/pending pages each account for their buffers.
    let retained = descriptor_bytes(&descriptor);
    let permit = client
        .try_reserve(
            command.kind(),
            command.input_bytes().saturating_add(retained),
        )
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
            move |scope, request, reply| {
                let cancel = sql_transaction_cancellation(scope, request);
                if deliver(scope, request, target, descriptor, reply) {
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

fn descriptor_bytes(descriptor: &SqlCursorQuery) -> usize {
    let source = match &descriptor.query.source {
        QuerySource::Index(name) => name.capacity(),
        QuerySource::ObjectStore => 0,
    };
    std::mem::size_of::<SqlCursorQuery>()
        .saturating_add(descriptor.store.capacity())
        .saturating_add(source)
        .saturating_add(
            descriptor
                .query
                .range
                .lower
                .as_ref()
                .map_or(0, |key| key.heap_bytes()),
        )
        .saturating_add(
            descriptor
                .query
                .range
                .upper
                .as_ref()
                .map_or(0, |key| key.heap_bytes()),
        )
}

fn deliver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    target: Target,
    query: Rc<SqlCursorQuery>,
    reply: Completion,
) -> bool {
    let Completion { result, permit, .. } = reply;
    let records = match result {
        Ok(CommandResult::Records(records)) if records.len() <= 1 => records,
        Err(error) => {
            fail_request(scope, request, &error);
            return true;
        }
        _ => {
            fail_request(
                scope,
                request,
                &IndexedDbError::InvalidState("Invalid SQL cursor result".into()),
            );
            return true;
        }
    };
    v8::tc_scope!(let scope, scope);
    let result = match target {
        Target::Open(_) if records.is_empty() => {
            drop(permit);
            let empty = v8::null(scope).into();
            store_request_success(scope, request, empty);
            Some(())
        }
        Target::Open(source) => {
            let source = v8::Local::new(scope, &source);
            let direction = query.query.direction;
            let key_only = query.query.keys_only;
            materialize_cursor_result_in_request_realm(
                scope, source, request, records, direction, key_only,
            )
            .and_then(|cursor| {
                attach_indexed_db_sql_cursor(scope, cursor, query, permit)?;
                store_request_success(scope, request, cursor.into());
                Some(())
            })
        }
        Target::Iterate(cursor) => {
            let cursor = v8::Local::new(scope, &cursor);
            let position = (!records.is_empty()).then_some(0);
            stage_indexed_db_sql_cursor_page(scope, request, query, records, permit)
                .and_then(|()| store_cursor_iteration_result(scope, cursor, position))
        }
    };
    if scope.is_execution_terminating() || !scope.can_continue() {
        if scope.has_caught() {
            scope.rethrow();
        }
        return false;
    }
    if result.is_none() {
        let error = scope.exception().unwrap_or_else(|| {
            request_error_object(
                scope,
                &IndexedDbError::Serialization("Unable to create a SQL cursor result".into()),
            )
        });
        store_request_error(scope, request, error);
    }
    true
}
