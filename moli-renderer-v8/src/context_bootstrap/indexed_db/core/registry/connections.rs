use super::*;
use crate::context_bootstrap::indexed_db::{
    enqueue_blocked_recheck_task, enqueue_version_change_task,
    indexed_db_connection_notification_wake, indexed_db_connection_request_wake,
    indexed_db_has_external_task_source, indexed_db_shared_manager,
    indexed_db_typed_task_execution_owner, set_indexed_db_blocked_notifications_pending,
    set_indexed_db_version_change_batch,
};

pub(in crate::context_bootstrap::indexed_db) fn database_registry_key(
    origin: &str,
    name: &str,
) -> String {
    format!("{origin}\u{0}{name}")
}

pub(in crate::context_bootstrap::indexed_db) fn has_open_database_connections_for_key(
    scope: &mut v8::PinScope<'_, '_>,
    key: &str,
) -> bool {
    indexed_db_shared_manager(scope).is_ok_and(|manager| {
        manager
            .lock()
            .connection_notifications()
            .has_connections(key)
    })
}

pub(in crate::context_bootstrap::indexed_db) fn enqueue_version_change_to_open_connections<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: &str,
    old_version: u64,
    new_version: Option<u64>,
    blocked_task: v8::Local<'s, v8::Object>,
) {
    set_indexed_db_blocked_notifications_pending(scope, blocked_task, true);
    if indexed_db_has_external_task_source(scope) {
        let owner = indexed_db_typed_task_execution_owner(scope, blocked_task)
            .expect("notification request owner");
        let wake = indexed_db_connection_request_wake(scope, owner);
        let manager = indexed_db_shared_manager(scope).expect("connection notification manager");
        let coordinator = manager.lock().connection_notifications();
        let batch = coordinator.begin_version_change(key, old_version, new_version, wake);
        set_indexed_db_version_change_batch(scope, blocked_task, batch);
        return;
    }
    // Bare isolates have no external task transport. Retain their local
    // microtask fallback; Window and worker event loops use the shared batch.
    let mut notifications = Vec::new();
    for database in local_open_database_connections_for_key(scope, key) {
        notifications.extend(enqueue_version_change_task(
            scope,
            database,
            old_version,
            new_version,
        ));
    }
    crate::context_bootstrap::microtask_checkpoint::enqueue_indexed_db_version_change_completion(
        scope,
        blocked_task,
        notifications,
    );
    enqueue_blocked_recheck_task(scope, blocked_task);
}

pub(in crate::context_bootstrap::indexed_db) fn register_open_database_connection(
    scope: &mut v8::PinScope<'_, '_>,
    owner: IndexedDbExecutionOwner,
    handle: DatabaseHandle,
    database_key: String,
    database: v8::Local<'_, v8::Object>,
) {
    let wake = indexed_db_connection_notification_wake(scope, owner, handle);
    if let Ok(manager) = indexed_db_shared_manager(scope) {
        manager
            .lock()
            .connection_notifications()
            .register(handle, database_key.clone(), wake);
    }
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let execution_context = owner.execution_context().unwrap_or_else(|| {
            panic!(
                "Page IDBDatabase must retain the exact Window realm inherited from its factory; owner was {owner:?}"
            )
        });
        unsafe { &*host_ptr }.register_indexed_db_open_connection(
            scope,
            execution_context,
            handle,
            database_key,
            database,
        );
        return;
    }
    push_unique_object_to_indexed_db_runtime_array(
        scope,
        IndexedDbRuntimeArray::OpenDatabases,
        database,
    );
}

pub(in crate::context_bootstrap::indexed_db) fn database_connection_for_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: DatabaseHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        return unsafe { &*host_ptr }.indexed_db_open_connection_for_handle(scope, handle);
    }
    let registry = indexed_db_runtime_array(scope, IndexedDbRuntimeArray::OpenDatabases)?;
    for index in 0..registry.length() {
        let database = registry
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
        if let Some(database) = database
            && crate::context_bootstrap::indexed_db::database_handle_from_value(
                scope,
                database.into(),
            ) == Some(handle)
        {
            return Some(database);
        }
    }
    None
}

/// Removes a connection and schedules blocked-request rechecks in every page
/// realm that was waiting on the same database key.
///
/// Returns true when the page-owned coordinator handled the connection. Worker
/// and standalone contexts retain their realm-local fallback queue.
pub(in crate::context_bootstrap::indexed_db) fn unregister_open_database_connection(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DatabaseHandle,
    database: v8::Local<'_, v8::Object>,
) -> bool {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && unsafe { &*host_ptr }.unregister_indexed_db_open_connection(handle)
    {
        return true;
    }

    let Some(registry) = indexed_db_runtime_array(scope, IndexedDbRuntimeArray::OpenDatabases)
    else {
        return false;
    };
    let next = v8::Array::new(scope, 0);
    for index in 0..registry.length() {
        let Some(value) = registry.get_index(scope, index) else {
            continue;
        };
        if value.strict_equals(database.into()) {
            continue;
        }
        let _ = next.set_index(scope, next.length(), value);
    }
    replace_indexed_db_runtime_array(scope, IndexedDbRuntimeArray::OpenDatabases, next);
    false
}

pub(in crate::context_bootstrap::indexed_db) fn register_blocked_database_context(
    scope: &mut v8::PinScope<'_, '_>,
    database_key: String,
    owner: IndexedDbExecutionOwner,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let execution_context = owner
        .execution_context()
        .expect("Page blocked IndexedDB request must retain its exact accepting Window realm");
    unsafe { &*host_ptr }.register_indexed_db_blocked_context(database_key, execution_context);
}

pub(in crate::context_bootstrap::indexed_db) fn unregister_blocked_database_context(
    scope: &mut v8::PinScope<'_, '_>,
    database_key: &str,
    owner: IndexedDbExecutionOwner,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let execution_context = owner
        .execution_context()
        .expect("Page blocked IndexedDB request must retain its exact accepting Window realm");
    unsafe { &*host_ptr }.unregister_indexed_db_blocked_context(database_key, execution_context);
}

fn local_open_database_connections_for_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: &str,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(registry) = indexed_db_runtime_array(scope, IndexedDbRuntimeArray::OpenDatabases)
    else {
        return Vec::new();
    };
    let mut connections = Vec::new();
    for index in 0..registry.length() {
        let Some(value) = registry.get_index(scope, index) else {
            continue;
        };
        let Ok(database) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        if object_bool_property(scope, database, INDEXED_DB_DATABASE_CLOSED_SLOT).unwrap_or(false) {
            continue;
        }
        if object_string_property(scope, database, INDEXED_DB_DATABASE_KEY_SLOT).as_deref()
            != Some(key)
        {
            continue;
        }
        connections.push(database);
    }
    connections
}
