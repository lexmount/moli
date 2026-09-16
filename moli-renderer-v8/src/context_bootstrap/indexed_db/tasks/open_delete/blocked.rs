use super::*;

mod delete_task;
mod drain;
mod event;
mod open_task;

fn queue_behind_earlier_blocked_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    key: &str,
) -> bool {
    let Some(queue) = indexed_db_runtime_array(scope, IndexedDbRuntimeArray::BlockedOpenQueue)
    else {
        return false;
    };
    let has_earlier_request = (0..queue.length()).any(|index| {
        queue
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .filter(|queued| !queued.strict_equals(task.into()))
            .and_then(|queued| indexed_db_blocked_task_payload(scope, queued))
            .is_some_and(|payload| database_registry_key(&payload.origin, &payload.name) == key)
    });
    if !has_earlier_request {
        return false;
    }
    // A newly dispatched request must not overtake a request already waiting for
    // this connection to close, even if its drain task has not run yet.
    push_unique_object_to_indexed_db_runtime_array(
        scope,
        IndexedDbRuntimeArray::BlockedOpenQueue,
        task,
    );
    let owner = indexed_db_typed_task_execution_owner(scope, task)
        .expect("queued connection request must retain its IndexedDB execution owner");
    register_blocked_database_context(scope, key.to_owned(), owner);
    if !has_open_database_connections_for_key(scope, key) {
        enqueue_drain_blocked_open_requests_task(scope);
    }
    true
}

pub(super) fn blocked_task_storage_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbStorageScope> {
    indexed_db_typed_task_storage_scope(scope, task)
}

pub(in crate::context_bootstrap::indexed_db) use self::delete_task::flush_delete_blocked_task;
pub(in crate::context_bootstrap::indexed_db) use self::drain::flush_drain_blocked_open_requests_task;
pub(in crate::context_bootstrap::indexed_db) use self::open_task::flush_open_blocked_task;
