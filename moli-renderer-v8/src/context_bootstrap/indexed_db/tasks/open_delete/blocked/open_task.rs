use super::*;

pub(in crate::context_bootstrap::indexed_db) fn flush_open_blocked_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(payload) = indexed_db_blocked_task_payload(scope, task) else {
        return;
    };
    let Some(new_version) = payload.new_version else {
        return;
    };
    let Some(storage_scope) = blocked_task_storage_scope(scope, task) else {
        return;
    };
    let key = database_registry_key(&payload.origin, &payload.name);
    if queue_behind_earlier_blocked_request(scope, task, &key) {
        return;
    }
    if !has_open_database_connections_for_key(scope, &key) {
        open::execute_open_request(
            scope,
            payload.request,
            storage_scope,
            payload.name,
            payload.version,
        );
        return;
    }
    dispatch_version_change_to_open_connections(
        scope,
        &key,
        payload.old_version,
        Some(new_version),
    );
    if !has_open_database_connections_for_key(scope, &key) {
        open::execute_open_request(
            scope,
            payload.request,
            storage_scope,
            payload.name,
            payload.version,
        );
        return;
    }
    push_unique_object_to_indexed_db_runtime_array(
        scope,
        IndexedDbRuntimeArray::BlockedOpenQueue,
        task,
    );
    let owner = indexed_db_typed_task_execution_owner(scope, task)
        .expect("blocked open task must retain its IndexedDB execution owner");
    register_blocked_database_context(scope, key, owner);
    event::dispatch_blocked_once(
        scope,
        payload.request,
        payload.old_version,
        Some(new_version),
    );
}
