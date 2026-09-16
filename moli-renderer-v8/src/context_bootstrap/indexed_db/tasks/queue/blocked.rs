use super::*;
use crate::context_bootstrap::indexed_db::{
    register_indexed_db_blocked_recheck_task, register_indexed_db_version_change_task,
};

pub(in crate::context_bootstrap::indexed_db) fn enqueue_version_change_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    old_version: u64,
    new_version: Option<u64>,
) {
    let Some(context) = database.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let task = v8::Object::new(scope);
    register_indexed_db_version_change_task(scope, task, database, old_version, new_version);
    enqueue_indexed_db_task(scope, task);
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
    name: &str,
    version: Option<u64>,
    old_version: u64,
    new_version: u64,
) {
    let task = v8::Object::new(scope);
    register_indexed_db_blocked_open_task(
        scope,
        task,
        request,
        origin,
        name,
        version,
        old_version,
        new_version,
    );
    enqueue_indexed_db_task(scope, task);
}

pub(in crate::context_bootstrap::indexed_db) fn enqueue_blocked_delete_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    origin: &str,
    name: &str,
    old_version: u64,
) {
    let task = v8::Object::new(scope);
    register_indexed_db_blocked_delete_task(scope, task, request, origin, name, old_version);
    enqueue_indexed_db_task(scope, task);
}

pub(in crate::context_bootstrap::indexed_db) fn enqueue_drain_blocked_open_requests_task(
    scope: &mut v8::PinScope<'_, '_>,
) {
    let task = v8::Object::new(scope);
    register_indexed_db_task(scope, task, IndexedDbTaskKind::DrainBlockedOpens, None);
    enqueue_indexed_db_task(scope, task);
}
