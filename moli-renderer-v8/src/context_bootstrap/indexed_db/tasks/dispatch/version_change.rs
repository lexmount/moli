use super::*;
use crate::context_bootstrap::indexed_db::{
    INDEXED_DB_DATABASE_CLOSED_SLOT, indexed_db_version_change_task_payload,
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
