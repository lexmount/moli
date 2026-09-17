use super::*;
use crate::context_bootstrap::indexed_db::{
    register_indexed_db_blocked_recheck_task, register_indexed_db_version_change_task,
};

pub(in crate::context_bootstrap::indexed_db) fn enqueue_version_change_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    old_version: u64,
    new_version: Option<u64>,
) -> Option<v8::Global<v8::Object>> {
    let context = database.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let task = v8::Object::new(scope);
    register_indexed_db_version_change_task(scope, task, database, old_version, new_version);
    enqueue_indexed_db_task(scope, task);
    Some(v8::Global::new(scope, task))
}

pub(in crate::context_bootstrap::indexed_db) fn enqueue_blocked_recheck_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    blocked_task: v8::Local<'s, v8::Object>,
) {
    let Some(context) = blocked_task.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let task = v8::Object::new(scope);
    register_indexed_db_blocked_recheck_task(scope, task, blocked_task);
    enqueue_indexed_db_task(scope, task);
}

pub(in crate::context_bootstrap::indexed_db) fn enqueue_blocked_open_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    origin: &str,
    name: &IndexedDbName,
    version: Option<u64>,
) {
    let context = request
        .get_creation_context(scope)
        .expect("connection request realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let task = v8::Object::new(scope);
    register_indexed_db_blocked_open_task(
        scope,
        task,
        request,
        origin,
        name,
        version,
        0,
        version.unwrap_or(1),
    );
    start_indexed_db_connection_request(scope, task);
}

pub(in crate::context_bootstrap::indexed_db) fn enqueue_blocked_delete_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    origin: &str,
    name: &IndexedDbName,
) {
    let context = request
        .get_creation_context(scope)
        .expect("connection request realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let task = v8::Object::new(scope);
    register_indexed_db_blocked_delete_task(scope, task, request, origin, name, 0);
    start_indexed_db_connection_request(scope, task);
}

pub(in crate::context_bootstrap::indexed_db) fn enqueue_drain_blocked_open_requests_task(
    scope: &mut v8::PinScope<'_, '_>,
) {
    let task = v8::Object::new(scope);
    register_indexed_db_task(scope, task, IndexedDbTaskKind::DrainBlockedOpens, None);
    enqueue_indexed_db_task(scope, task);
}
