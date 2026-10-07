use super::*;
use crate::context_bootstrap::indexed_db::register_indexed_db_version_change_task;

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
