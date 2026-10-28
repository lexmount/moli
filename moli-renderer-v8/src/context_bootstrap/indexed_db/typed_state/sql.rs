//! SQL results use the accepting realm's existing IndexedDB task queue.

use super::*;
use moli_indexeddb::sqlite::service::{Command, Completion, DatabaseClient, DatabaseId};
use parking_lot::Mutex;
use std::sync::Arc;

type Continuation =
    Box<dyn for<'s, 'i> FnOnce(&mut v8::PinScope<'s, 'i>, v8::Local<'s, v8::Object>, Completion)>;

pub(super) struct SqlTaskPayload {
    source: RealmValueHandle,
    completion: Arc<Mutex<Option<Completion>>>,
    database: DatabaseId,
    generation: [u8; 16],
    continuation: Continuation,
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_request_sql_client<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> Option<DatabaseClient> {
    let transaction = indexed_db_request_transaction_object(scope, request)?;
    let id = indexed_db_typed_state_id(scope, transaction)?;
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table.borrow().transactions.get(&id)?.sql_client.clone()
}

pub(in crate::context_bootstrap::indexed_db) fn submit_indexed_db_sql_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    client: &DatabaseClient,
    command: Command,
    result_bytes: usize,
    continuation: impl for<'a, 'i> FnOnce(
        &mut v8::PinScope<'a, 'i>,
        v8::Local<'a, v8::Object>,
        Completion,
    ) + 'static,
) -> Result<(), IndexedDbError> {
    let context = source.get_creation_context(scope).ok_or_else(|| {
        IndexedDbError::InvalidState("IndexedDB SQL task lost its accepting realm".into())
    })?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let owner = indexed_db_typed_execution_owner(scope, source)
        .ok_or_else(|| IndexedDbError::InvalidState("IndexedDB SQL task lost its owner".into()))?;
    let storage = indexed_db_typed_storage_scope(scope, source);
    let task = v8::Object::new(scope);
    let id = register_indexed_db_task_with_owner(
        scope,
        task,
        IndexedDbTaskKind::SqlCompletion,
        owner,
        storage,
    );
    let completion = Arc::new(Mutex::new(None));
    let receiver = Arc::downgrade(&completion);
    let wake = super::super::runtime::indexed_db_sql_completion_wake(scope, owner, id);
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let payload = SqlTaskPayload {
        source: RealmValueHandle::new(scope, source.into()),
        completion,
        database: client.id(),
        generation: client.identity().generation,
        continuation: Box::new(continuation),
    };
    table.borrow_mut().sql_tasks.insert(id, payload);
    let queued =
        indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue).is_some_and(|queue| {
            moli_webapi_declare::define_array_data_property(
                scope,
                queue,
                queue.length(),
                task.into(),
            )
            .is_some()
        });
    if !queued {
        unregister_indexed_db_task(scope, task);
        return Err(IndexedDbError::InvalidState(
            "Unable to retain IndexedDB SQL task in its accepting realm".into(),
        ));
    }
    let submitted = client.submit(command, result_bytes, move |result| {
        let Some(receiver) = receiver.upgrade() else {
            return;
        };
        // Publish the ticket under the readiness lock. An explicit owner pump
        // cannot consume the result before its producer has published the wake.
        let mut slot = receiver.lock();
        *slot = Some(result);
        let discarded = if wake() { None } else { slot.take() };
        drop(slot);
        drop(discarded);
    });
    if let Err(error) = submitted {
        discard_indexed_db_task_by_id(scope, id);
        return Err(IndexedDbError::Resource(error.to_string()));
    }
    Ok(())
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_task_is_ready<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> bool {
    if indexed_db_typed_task_kind(scope, task) != Some(IndexedDbTaskKind::SqlCompletion) {
        return true;
    }
    let Some(id) = indexed_db_typed_task_id(scope, task) else {
        return false;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    table
        .borrow()
        .sql_tasks
        .get(&id)
        .is_some_and(|task| task.completion.lock().is_some())
}

pub(in crate::context_bootstrap::indexed_db) fn flush_indexed_db_sql_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_task_id(scope, task) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, task);
    let Some(payload) = table.borrow_mut().sql_tasks.remove(&id) else {
        return;
    };
    let Some(mut completion) = payload.completion.lock().take() else {
        return;
    };
    if completion.database != payload.database || completion.generation != payload.generation {
        completion.result = Err(IndexedDbError::InvalidState(
            "IndexedDB completion belongs to another database generation".into(),
        ));
    }
    let Ok(source) = v8::Local::<v8::Object>::try_from(payload.source.to_local(scope)) else {
        return;
    };
    (payload.continuation)(scope, source, completion);
}

#[cfg(test)]
mod tests;
