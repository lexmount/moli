use super::*;
use crate::context_bootstrap::indexed_db::{
    indexed_db_database_is_closed, indexed_db_version_change_task_payload,
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
    if let Some(completion) =
        crate::context_bootstrap::indexed_db::take_indexed_db_version_change_completion(scope, task)
    {
        crate::context_bootstrap::microtask_checkpoint::defer_indexed_db_version_change_completion(
            scope, completion,
        );
    }
    if indexed_db_database_is_closed(scope, database) {
        return;
    }
    let _ =
        dispatch_version_change_event(scope, database, "versionchange", old_version, new_version);
}
