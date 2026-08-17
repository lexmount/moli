use super::*;

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

fn idb_factory_effective_execution_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    factory: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbExecutionOwner> {
    let stored_owner = indexed_db_typed_execution_owner(scope, factory)?;
    let Some(popup_id) = crate::native_bridge::active_lightweight_popup_id(scope) else {
        return Some(stored_owner);
    };
    let runtime_factory = indexed_db_runtime_factory(scope)?;
    if !factory.strict_equals(runtime_factory.into()) {
        return Some(stored_owner);
    }
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let execution_context = unsafe { &*host_ptr }
        .current_runtime_window_execution_context_identity_for_dispatch_scope(
            scope,
            crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id),
        )?;
    Some(IndexedDbExecutionOwner::for_window_execution_context(
        execution_context,
    ))
}

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
