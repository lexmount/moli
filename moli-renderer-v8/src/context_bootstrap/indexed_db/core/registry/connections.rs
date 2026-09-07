use super::*;

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
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        return !unsafe { &*host_ptr }
            .indexed_db_open_connection_snapshots(scope, key)
            .is_empty();
    }
    !local_open_database_connections_for_key(scope, key).is_empty()
}

pub(in crate::context_bootstrap::indexed_db) fn dispatch_version_change_to_open_connections(
    scope: &mut v8::PinScope<'_, '_>,
    key: &str,
    old_version: u64,
    new_version: Option<u64>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        for database in local_open_database_connections_for_key(scope, key) {
            let _ = dispatch_version_change_event(
                scope,
                database,
                "versionchange",
                old_version,
                new_version,
            );
        }
        return;
    };

    // Connections share backend coordination but retain the V8 realm that
    // created each IDBDatabase wrapper. Snapshot roots before invoking script
    // so close()/navigation can mutate the coordinator during dispatch.
    let connections = unsafe { &*host_ptr }.indexed_db_open_connection_snapshots(scope, key);
    for connection in connections {
        if !unsafe { &*host_ptr }
            .window_execution_context_identity_is_current(connection.execution_context)
        {
            continue;
        }
        let context = v8::Local::new(scope, &connection.context);
        let target_scope = &mut v8::ContextScope::new(scope, context);
        if crate::native_bridge::current_runtime_observable_context_token(target_scope)
            != Some(connection.execution_context.realm_token())
        {
            continue;
        }
        let database = v8::Local::new(target_scope, &connection.database);
        let _ = dispatch_version_change_event(
            target_scope,
            database,
            "versionchange",
            old_version,
            new_version,
        );
    }
}

pub(in crate::context_bootstrap::indexed_db) fn register_open_database_connection(
    scope: &mut v8::PinScope<'_, '_>,
    owner: IndexedDbExecutionOwner,
    handle: DatabaseHandle,
    database_key: String,
    database: v8::Local<'_, v8::Object>,
) {
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

/// Removes a connection and wakes only the head connection request for its key.
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
