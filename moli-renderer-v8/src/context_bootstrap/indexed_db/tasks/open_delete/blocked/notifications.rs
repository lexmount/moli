use super::*;
use crate::context_bootstrap::indexed_db::{
    INDEXED_DB_DATABASE_CLOSED_SLOT, defer_indexed_db_blocked_recheck_to_checkpoint,
    indexed_db_blocked_recheck_task_payload, indexed_db_version_change_task_payload,
    set_indexed_db_blocked_event_required, set_indexed_db_blocked_notifications_pending,
};

pub(crate) fn complete_indexed_db_version_change_notifications<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    blocked_task: v8::Local<'s, v8::Object>,
    notifications: &[v8::Global<v8::Object>],
) -> bool {
    let Some(payload) = indexed_db_blocked_task_payload(scope, blocked_task) else {
        return true;
    };
    let host_ptr = context_host_ptr_from_global_bridge(scope);
    let is_retired =
        |owner: crate::context_bootstrap::indexed_db::typed_state::IndexedDbExecutionOwner| {
            host_ptr
                .zip(owner.execution_context())
                .is_some_and(|(host, owner)| {
                    !unsafe { &*host }.window_execution_context_identity_is_current(owner)
                })
        };
    if indexed_db_typed_task_execution_owner(scope, blocked_task).is_none_or(is_retired) {
        return true;
    }
    for notification in notifications {
        let notification = v8::Local::new(scope, notification);
        // A dispatched or discarded ticket has no typed task state. A retired
        // recipient must not keep a live request waiting for its callback.
        if indexed_db_typed_task_execution_owner(scope, notification)
            .is_some_and(|owner| !is_retired(owner))
        {
            return false;
        }
    }
    // This runs after every notification's microtasks, before another task
    // source can close the connections and erase an already-required event.
    let key = database_registry_key(&payload.origin, &payload.name);
    let required = has_open_database_connections_for_key(scope, &key);
    if set_indexed_db_blocked_event_required(scope, blocked_task, required) {
        enqueue_blocked_recheck_task(scope, blocked_task);
    }
    true
}

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
    let Some(required) = payload.blocked_event_required else {
        // A retired recipient may be discarded without running a checkpoint.
        // Let the checkpoint schedule the retry: the standalone fallback runs
        // IDB work as microtasks, so immediately retrying could starve it.
        defer_indexed_db_blocked_recheck_to_checkpoint(scope, blocked_task);
        return;
    };
    set_indexed_db_blocked_notifications_pending(scope, blocked_task, false);
    if required {
        event::dispatch_blocked_once(
            scope,
            payload.request,
            payload.old_version,
            payload.new_version,
        );
    }
    // A timer may have closed the last connection while this event was queued.
    // Advance only after dispatch, even if its close-triggered drain ran earlier.
    enqueue_drain_blocked_open_requests_task(scope);
}
