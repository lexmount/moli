use super::*;

pub(in crate::context_bootstrap::indexed_db) fn current_storage_scope(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<IndexedDbStorageScope> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let active_child_handle = crate::native_bridge::active_child_window_handle(scope);
        let host = unsafe { &mut *host_ptr };
        let storage_key = host
            .active_storage_context(scope, active_child_handle)
            .storage_key()
            .serialized_storage_key();
        let identity = host.browser_context_runtime().storage_partition_identity();
        return Some(IndexedDbStorageScope::new(
            storage_key,
            identity.browser_context_id(),
            identity.profile_partition_id(),
        ));
    }
    let storage_key = crate::worker::worker_storage_key(scope)?.serialized_storage_key();
    let identity = crate::worker::worker_storage_partition_identity(scope)?;
    Some(IndexedDbStorageScope::new(
        storage_key,
        identity.browser_context_id(),
        identity.profile_partition_id(),
    ))
}

pub(in crate::context_bootstrap::indexed_db) fn storage_scope_for_window_execution_context(
    scope: &mut v8::PinScope<'_, '_>,
    execution_context: crate::native_bridge::WindowExecutionContextIdentity,
) -> Option<IndexedDbStorageScope> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let host = unsafe { &mut *host_ptr };
    if !host.window_execution_context_identity_is_current(execution_context) {
        return None;
    }
    let storage_context = match execution_context.dispatch_scope() {
        crate::native_bridge::OwnerDispatchScope::Top => host.top_document_storage_context(),
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            host.storage_context_for_child_browsing_context(handle)?
        }
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
            host.storage_context_for_lightweight_popup(popup_id)?
        }
    };
    let identity = host.browser_context_runtime().storage_partition_identity();
    Some(IndexedDbStorageScope::new(
        storage_context.storage_key().serialized_storage_key(),
        identity.browser_context_id(),
        identity.profile_partition_id(),
    ))
}

pub(in crate::context_bootstrap::indexed_db) fn storage_scope_for_current_partition(
    scope: &mut v8::PinScope<'_, '_>,
    storage_key: impl Into<String>,
) -> Option<IndexedDbStorageScope> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &mut *host_ptr };
        let identity = host.browser_context_runtime().storage_partition_identity();
        return Some(IndexedDbStorageScope::new(
            storage_key,
            identity.browser_context_id(),
            identity.profile_partition_id(),
        ));
    }
    let identity = crate::worker::worker_storage_partition_identity(scope)?;
    Some(IndexedDbStorageScope::new(
        storage_key,
        identity.browser_context_id(),
        identity.profile_partition_id(),
    ))
}

pub(in crate::context_bootstrap::indexed_db) fn origin_allows_indexed_db(origin: &str) -> bool {
    !moli_storage_key::serialized_storage_key_has_opaque_origin(origin)
}

#[cfg(test)]
mod tests {
    #[test]
    fn indexed_db_rejects_opaque_serialized_storage_keys() {
        assert!(!super::origin_allows_indexed_db("null"));
        assert!(!super::origin_allows_indexed_db(
            "storage-key:v1;origin=null;top-level-site=https://app.example;opaque-nonce=7"
        ));
        assert!(super::origin_allows_indexed_db(
            "storage-key:v1;origin=https://cdn.example;top-level-site=https://app.example"
        ));
    }
}
