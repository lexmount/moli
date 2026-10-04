use super::*;
use crate::context_bootstrap::indexed_db::{
    INDEXED_DB_DATABASE_CLOSED_SLOT, indexed_db_blocked_recheck_task_payload,
    indexed_db_version_change_task_payload, set_indexed_db_blocked_notifications_pending,
};

pub(in crate::context_bootstrap::indexed_db) fn flush_version_change_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some((database, old_version, new_version)) =
        indexed_db_version_change_task_payload(scope, task)
    else {
        return;
    };
    if object_bool_property(scope, database, INDEXED_DB_DATABASE_CLOSED_SLOT).unwrap_or(false) {
        return;
    }
    let _ =
        dispatch_version_change_event(scope, database, "versionchange", old_version, new_version);
}

pub(in crate::context_bootstrap::indexed_db) fn flush_blocked_recheck_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some(blocked_task) = indexed_db_blocked_recheck_task_payload(scope, task) else {
        return;
    };
    let Some(payload) = indexed_db_blocked_task_payload(scope, blocked_task) else {
        return;
    };
    // All connection notifications precede this task in the same IndexedDB
    // source, including stale tickets discarded when their realm is retired.
    set_indexed_db_blocked_notifications_pending(scope, blocked_task, false);
    let key = database_registry_key(&payload.origin, &payload.name);
    if !has_open_database_connections_for_key(scope, &key) {
        enqueue_drain_blocked_open_requests_task(scope);
        return;
    }
    event::dispatch_blocked_once(
        scope,
        payload.request,
        payload.old_version,
        payload.new_version,
    );
}
