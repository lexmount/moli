use super::*;

mod abort;
mod commit;

pub(in crate::context_bootstrap::indexed_db) use self::abort::flush_transaction_abort_task;
pub(in crate::context_bootstrap::indexed_db) use self::commit::flush_transaction_commit_task;

fn dispatch_transaction_terminal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    event: &str,
    successful: bool,
) {
    let upgrade = prepare_upgrade_open_result(scope, transaction, successful);
    let _ = dispatch_idb_named_event(scope, transaction, event, |_, _| {});
    if let Some((request, database)) = upgrade {
        finish_upgrade_open(scope, request, database, successful);
    }
    release_indexed_db_transaction_dispatch_refs(scope, transaction);
}
