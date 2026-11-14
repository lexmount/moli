use super::{
    array_contains_strict, context_host_ptr_from_global_bridge, global_constructor_prototype,
    throw_type_error, v8_string, v8str,
};
use crate::util::{new_null_prototype_object, private_key, set_private_value};

mod backend;
mod core;
mod cursor;
mod database;
mod event_target;
mod install;
mod operation_state;
mod record;
mod runtime;
mod state;
mod storage_bucket;
mod stores;
mod tasks;
mod typed_state;
#[cfg(test)]
pub(crate) use typed_state::{
    IndexedDbMetadataSharingProbe, IndexedDbTaskKind, indexed_db_has_in_flight_sql_tasks_for_test,
    indexed_db_has_queued_task_kind_for_test, indexed_db_metadata_sharing_probe_for_test,
    indexed_db_native_cursor_count_for_test, indexed_db_transaction_commit_submitted_for_test,
    indexed_db_transaction_resources_for_test,
};
#[cfg(test)]
pub(crate) fn serialize_indexed_db_value_for_test(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<IndexedDbValue> {
    serialize_js_value(scope, value)
}
#[cfg(test)]
pub(crate) fn deserialize_indexed_db_value_for_test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &IndexedDbValue,
) -> Result<Option<v8::Local<'s, v8::Value>>, IndexedDbError> {
    deserialize_js_value(scope, value)
}
#[cfg(test)]
pub(crate) fn create_sql_database_for_test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    connection: moli_indexeddb::sqlite::service::DatabaseConnection,
) -> Option<v8::Local<'s, v8::Object>> {
    let client = connection.client().clone();
    let factory = indexed_db_runtime_factory(scope)?;
    let owner = indexed_db_typed_execution_owner(scope, factory)?;
    let storage_scope = indexed_db_factory_storage_scope(scope, factory)?;
    let metadata = client.committed()?;
    let database = create_database_object(
        scope,
        storage_scope,
        owner,
        connection.handle(),
        &metadata.name,
        metadata.version,
        client,
    )
    .ok()?;
    attach_indexed_db_sql_connection(scope, database, connection)
        .expect("fixture logical connection matches its SQL database");
    Some(database)
}
#[cfg(test)]
pub(crate) fn create_sql_upgrade_for_test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    version: u64,
) -> Option<v8::Local<'s, v8::Object>> {
    let client = indexed_db_database_sql_client(scope, database)?;
    let metadata = client.committed()?;
    let names = metadata.stores.keys().cloned().collect::<Vec<_>>();
    let transaction = create_transaction_object(
        scope,
        database,
        Some(handle),
        TransactionMode::VersionChange,
        &names,
    )?;
    typed_state::sql::configure_sql_transaction(scope, transaction, client, handle).ok()?;
    typed_state::sql::ensure_sql_transaction_rollback(scope, transaction).ok()?;
    set_indexed_db_database_upgrade_transaction(scope, database, Some(transaction));
    save_indexed_db_upgrade_metadata(scope, database, metadata.version);
    set_indexed_db_database_version(scope, database, version);
    schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(scope, transaction);
    Some(transaction)
}
mod types;

#[cfg(test)]
pub(crate) fn sql_transaction_handle_for_test(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
) -> Option<TransactionHandle> {
    indexed_db_native_transaction_handle(scope, transaction)
}

#[cfg(test)]
pub(crate) fn sql_transaction_started_for_test(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
) -> bool {
    indexed_db_transaction_status(scope, transaction).is_some_and(|state| state.started)
}

use self::backend::*;
use self::core::*;
use self::cursor::*;
use self::database::*;
pub(crate) use self::event_target::dispatch_indexed_db_event_callback;
use self::event_target::*;
use self::operation_state::*;
use self::runtime::*;
use self::storage_bucket::validate_storage_bucket_scope;
use self::stores::*;
use self::tasks::*;
use self::typed_state::*;
use self::types::*;

pub(in crate::context_bootstrap) use self::event_target::idb_version_change_event_constructor_callback;

pub(crate) use self::runtime::{
    bind_indexed_db_factory_to_window_execution_context, indexed_db_has_pending_tasks,
    materialized_indexed_db_factory_for_window, scoped_indexed_db_factory,
    set_worker_indexed_db_task_wake_for_context,
};
pub(crate) use self::tasks::{
    discard_indexed_db_task_by_id, flush_indexed_db_task_by_id, flush_next_indexed_db_task,
};
pub(crate) use self::typed_state::IndexedDbTaskId;
pub(crate) use self::typed_state::deactivate_indexed_db_transaction_after_microtask_checkpoint;
pub(crate) use self::typed_state::indexed_db_has_pending_work;
pub(crate) use self::typed_state::retain_indexed_db_state_in_retired_realm;
pub(in crate::context_bootstrap::indexed_db) use self::typed_state::schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint;
#[cfg(test)]
pub(crate) use self::typed_state::{
    indexed_db_request_state_entries_for_test, indexed_db_transaction_state_entries_for_test,
};

pub(in crate::context_bootstrap) use self::core::indexed_db_usage_bytes_for_storage_key;
pub(crate) use self::core::set_indexed_db_manager_for_context;
#[cfg(test)]
pub(crate) use self::core::{
    indexed_db_manager_context_slot_present_for_test,
    indexed_db_manager_isolate_slot_present_for_test,
};
pub(crate) use self::install::install_worker_indexed_db_runtime_state;
pub(super) use self::install::{
    ensure_indexed_db_runtime_state, install_indexed_db_template_bindings, window_indexed_db_getter,
};
pub use self::state::{
    SharedIndexedDbManager, WeakIndexedDbManager, clear_indexed_db_origin,
    clear_indexed_db_origins_with_prefix, downgrade_indexed_db_manager,
    indexed_db_origin_usage_bytes, new_indexed_db_manager, new_sqlite_indexed_db_manager,
    new_sqlite_indexed_db_manager_with_runtime,
};
pub(in crate::context_bootstrap) use self::storage_bucket::scoped_storage_bucket_indexed_db_factory;
pub use moli_indexeddb::{IndexedDbName, Key, ObjectStoreOptions, TransactionMode};

fn indexed_db_request_transaction_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'_, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    request_value(scope, request, IndexedDbRequestValue::Transaction)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(crate) fn flush_indexed_db_transaction_starts(scope: &mut v8::PinScope<'_, '_>) {
    enqueue_ready_transaction_starts(scope);
}
pub(crate) use self::typed_state::retire_indexed_db_context;
