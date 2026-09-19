use super::*;

mod drain;
mod event;
mod notifications;

pub(in crate::context_bootstrap::indexed_db) fn start_indexed_db_connection_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let payload = indexed_db_blocked_task_payload(scope, task).expect("connection request payload");
    let manager = match indexed_db_shared_manager(scope) {
        Ok(manager) => manager,
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, payload.request, error);
            unregister_indexed_db_task(scope, task);
            return;
        }
    };
    let owner =
        indexed_db_typed_task_execution_owner(scope, task).expect("connection request owner");
    let wake = indexed_db_connection_request_wake(scope, owner);
    let lease =
        manager
            .lock()
            .connection_request_queues()
            .enqueue(&payload.origin, &payload.name, wake);
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &*host_ptr }.register_indexed_db_connection_request(
            owner
                .execution_context()
                .expect("Page connection request owner"),
            lease.handle().clone(),
        );
    }
    set_indexed_db_connection_request(scope, payload.request, lease);
    let key = database_registry_key(&payload.origin, &payload.name);
    register_blocked_database_context(scope, key.clone(), owner);
    push_unique_object_to_indexed_db_runtime_array(
        scope,
        IndexedDbRuntimeArray::BlockedOpenQueue,
        task,
    );
    // Preserve the existing fast path for an immediately executable head. The
    // queue position was reserved first, including for unspecified-version opens.
    if drain::try_execute_unblocked_request(scope, task, false) {
        drain::remove_connection_request_task(scope, task);
    } else {
        enqueue_indexed_db_task(scope, task);
    }
}

pub(in crate::context_bootstrap::indexed_db) use self::drain::flush_drain_blocked_open_requests_task;
pub(crate) use self::notifications::complete_indexed_db_version_change_notifications;
pub(in crate::context_bootstrap::indexed_db) use self::notifications::{
    flush_blocked_recheck_task, flush_version_change_task,
};
