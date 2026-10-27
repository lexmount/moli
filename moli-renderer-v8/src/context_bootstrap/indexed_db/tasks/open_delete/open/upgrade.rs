use super::*;
use crate::context_bootstrap::indexed_db::save_indexed_db_upgrade_metadata;

pub(super) fn enqueue_upgrade_needed_open_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    database: v8::Local<'s, v8::Object>,
    upgrade_handle: Option<TransactionHandle>,
    info: &DatabaseInfo,
    old_version: u64,
    new_version: u64,
) {
    let Some(upgrade_handle) = upgrade_handle else {
        let error = dom_exception_value(
            scope,
            "Missing upgrade transaction for version change.",
            "InvalidStateError",
        );
        store_request_error(scope, request, error);
        return;
    };
    let store_names = info.object_store_names.clone();
    let Some(transaction) = create_transaction_object(
        scope,
        database,
        Some(upgrade_handle),
        TransactionMode::VersionChange,
        &store_names,
    ) else {
        return;
    };
    set_indexed_db_slot_value(
        scope,
        database,
        INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT,
        transaction.into(),
    );
    save_indexed_db_upgrade_metadata(scope, database, old_version);
    enqueue_open_task(
        scope,
        request,
        database,
        transaction,
        old_version,
        new_version,
    );
}
