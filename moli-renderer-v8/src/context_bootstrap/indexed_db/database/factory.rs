use super::*;
use moli_indexeddb::IndexedDbError;

mod cmp;
mod databases;
mod delete_database;
mod open;

pub(in crate::context_bootstrap::indexed_db) use self::cmp::idb_factory_cmp_callback;
pub(in crate::context_bootstrap::indexed_db) use self::databases::{
    flush_databases_settle_task, idb_factory_databases_callback,
};
pub(in crate::context_bootstrap::indexed_db) use self::delete_database::idb_factory_delete_database_callback;
pub(in crate::context_bootstrap::indexed_db) use self::open::idb_factory_open_callback;

fn idb_factory_effective_storage_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    factory: v8::Local<'s, v8::Object>,
    owner: IndexedDbExecutionOwner,
) -> Option<IndexedDbStorageScope> {
    let storage_scope = indexed_db_factory_storage_scope(scope, factory)
        .filter(|_| {
            indexed_db_typed_execution_owner(scope, factory)
                .is_some_and(|stored_owner| stored_owner == owner)
        })
        .or_else(|| {
            owner.execution_context().and_then(|execution_context| {
                storage_scope_for_window_execution_context(scope, execution_context)
            })
        })
        .or_else(|| {
            context_host_ptr_from_global_bridge(scope)
                .is_none()
                .then(|| current_storage_scope(scope))
                .flatten()
        })?;
    origin_allows_indexed_db(storage_scope.storage_key()).then_some(storage_scope)
}

use crate::context_bootstrap::indexed_db::typed_state::factory::FactoryOperation;

fn factory_sql_service(
    scope: &mut v8::PinScope<'_, '_>,
) -> Result<moli_indexeddb::sqlite::service::ServiceClient, IndexedDbError> {
    with_indexed_db_manager(scope, |manager| Ok(manager.sqlite_service()))
}

fn dispatch_factory_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    storage_scope: IndexedDbStorageScope,
    name: IndexedDbName,
    operation: FactoryOperation,
) {
    let result = factory_sql_service(scope).and_then(|service| {
        crate::context_bootstrap::indexed_db::typed_state::factory::enqueue_sql_factory_request(
            scope,
            request,
            service,
            storage_scope,
            name,
            operation,
        )
    });
    if let Err(error) = result {
        let error = request_error_object(scope, &error);
        store_request_error(scope, request, error);
    }
}
