use super::*;

pub(in crate::context_bootstrap::indexed_db) fn enqueue_database_forced_close_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
) {
    use crate::context_bootstrap::indexed_db::{
        indexed_db_database_has_pending_transaction_events,
        mark_indexed_db_database_forced_close_scheduled,
    };
    if indexed_db_database_has_pending_transaction_events(scope, database)
        || !mark_indexed_db_database_forced_close_scheduled(scope, database)
    {
        return;
    }
    let task = v8::Object::new(scope);
    register_indexed_db_object_task(
        scope,
        task,
        IndexedDbTaskKind::DatabaseForcedClose,
        database,
    );
    enqueue_indexed_db_task(scope, task);
}
