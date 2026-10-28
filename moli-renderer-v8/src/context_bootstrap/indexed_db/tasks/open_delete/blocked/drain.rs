use super::*;

pub(in crate::context_bootstrap::indexed_db) fn flush_drain_blocked_open_requests_task(
    scope: &mut v8::PinScope<'_, '_>,
) {
    let Some(queue) = indexed_db_runtime_array(scope, IndexedDbRuntimeArray::BlockedOpenQueue)
    else {
        return;
    };
    let next = v8::Array::new(scope, 0);
    for index in 0..queue.length() {
        let Some(task) = queue
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        if !try_execute_unblocked_request(scope, task, true) {
            let _ = next.set_index(scope, next.length(), task.into());
        } else {
            unregister_connection_request_task(scope, task);
        }
    }
    replace_indexed_db_runtime_array(scope, IndexedDbRuntimeArray::BlockedOpenQueue, next);
}

fn unregister_connection_request_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    if let Some(payload) = indexed_db_blocked_task_payload(scope, task) {
        let owner =
            indexed_db_typed_task_execution_owner(scope, task).expect("connection request owner");
        unregister_blocked_database_context(
            scope,
            &database_registry_key(&payload.origin, &payload.name),
            owner,
        );
    }
    unregister_indexed_db_task(scope, task);
}

pub(super) fn remove_connection_request_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    if let Some(queue) = indexed_db_runtime_array(scope, IndexedDbRuntimeArray::BlockedOpenQueue) {
        let next = v8::Array::new(scope, 0);
        for index in 0..queue.length() {
            if let Some(value) = queue.get_index(scope, index)
                && !value.strict_equals(task.into())
            {
                let _ = next.set_index(scope, next.length(), value);
            }
        }
        replace_indexed_db_runtime_array(scope, IndexedDbRuntimeArray::BlockedOpenQueue, next);
    }
    unregister_connection_request_task(scope, task);
}

pub(super) fn try_execute_unblocked_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    allow_notifications: bool,
) -> bool {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(owner) = indexed_db_typed_task_execution_owner(scope, task)
            .and_then(|owner| owner.execution_context())
        && !unsafe { &*host_ptr }.window_execution_context_identity_is_current(owner)
    {
        if let Some(payload) = indexed_db_blocked_task_payload(scope, task) {
            finish_indexed_db_connection_request(scope, payload.request);
        }
        return true;
    }
    let owner = indexed_db_typed_task_owner_scope(scope, task).expect("connection request owner");
    let restore = owner.enter(scope);
    let executed = try_execute_in_owner_scope(scope, task, allow_notifications);
    // Advancing a head only enqueues callback tasks. This path can also run
    // synchronously inside IDBFactory, so restore before returning to its caller.
    owner.restore(scope, restore);
    executed
}

fn try_execute_in_owner_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    allow_notifications: bool,
) -> bool {
    let Some(payload) = indexed_db_blocked_task_payload(scope, task) else {
        return true;
    };
    if !indexed_db_connection_request_is_head(scope, payload.request)
        || payload.notifications_pending
    {
        return false;
    }
    let Some(storage_scope) = indexed_db_typed_task_storage_scope(scope, task) else {
        return true;
    };
    let kind = indexed_db_typed_task_kind(scope, task).expect("connection request kind");
    // Resolve both the existing and default requested version when this request
    // reaches the head, after every earlier upgrade or deletion has finished.
    let version = validate_storage_bucket_scope(scope, &storage_scope).and_then(|()| {
        with_indexed_db_manager(scope, |manager| {
            manager.database_version(&payload.origin, &payload.name)
        })
    });
    let old_version = match version {
        Ok(version) => version.unwrap_or(0),
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, payload.request, error);
            return true;
        }
    };
    let new_version = match kind {
        IndexedDbTaskKind::OpenBlocked => Some(payload.version.unwrap_or(old_version.max(1))),
        IndexedDbTaskKind::DeleteBlocked => None,
        _ => unreachable!("connection queue only contains open and delete requests"),
    };
    let needs_exclusive_access = new_version.is_none_or(|version| version > old_version);
    let key = database_registry_key(&payload.origin, &payload.name);
    if needs_exclusive_access && has_open_database_connections_for_key(scope, &key) {
        if allow_notifications && !payload.notifications_started {
            start_indexed_db_connection_notifications(scope, task, old_version, new_version);
            enqueue_version_change_to_open_connections(scope, &key, old_version, new_version, task);
        }
        return false;
    }
    match kind {
        IndexedDbTaskKind::OpenBlocked => open::execute_open_request(
            scope,
            payload.request,
            storage_scope,
            payload.name,
            payload.version,
        ),
        IndexedDbTaskKind::DeleteBlocked => delete::execute_delete_database_request(
            scope,
            payload.request,
            storage_scope,
            payload.name,
        ),
        _ => unreachable!(),
    }
    true
}
