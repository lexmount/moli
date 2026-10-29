//! V8 context adapter for the renderer-neutral Storage Bucket registry.

use crate::util::context_host_ptr_from_global_bridge;

pub use moli_storage_service::{
    IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME, SharedStorageBucketStore, StorageBucketCacheId,
    StorageBucketCacheMatch, StorageBucketCachePutOutcome, StorageBucketCacheQuery,
    StorageBucketCachedRequest, StorageBucketCachedResponse, StorageBucketDurability,
    StorageBucketIdentity, StorageBucketQuotaOwner, StorageBucketStore,
    new_shared_json_storage_bucket_store, new_shared_json_storage_bucket_store_with_cache_root,
    new_shared_json_storage_bucket_store_with_cache_root_and_indexed_db_manager,
    new_shared_json_storage_bucket_store_with_storage_service, new_shared_storage_bucket_store,
    new_shared_storage_bucket_store_with_indexed_db_manager,
    new_shared_storage_bucket_store_with_storage_service,
    new_shared_storage_bucket_store_with_storage_service_and_indexed_db_manager,
    storage_bucket_indexed_db_storage_key, storage_bucket_origin_allows_storage,
};
use moli_storage_service::{StorageBucketLocator, storage_bucket_quota_owner};

#[derive(Clone, Debug)]
struct StorageBucketStoreSlot(Option<SharedStorageBucketStore>);

pub(crate) fn set_storage_bucket_store_for_context(
    context: v8::Local<'_, v8::Context>,
    store: Option<SharedStorageBucketStore>,
) {
    let _previous = context.set_slot(std::rc::Rc::new(StorageBucketStoreSlot(store)));
}

pub(in crate::context_bootstrap) fn current_storage_bucket_storage_key(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<String> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let active_child_handle = crate::native_bridge::active_child_window_handle(scope);
        let host = unsafe { &mut *host_ptr };
        return Some(
            host.active_storage_context(scope, active_child_handle)
                .storage_key()
                .serialized_storage_key(),
        );
    }
    crate::worker::worker_storage_key(scope).map(|storage_key| storage_key.serialized_storage_key())
}

pub(in crate::context_bootstrap) fn with_storage_bucket_store_entry<R>(
    scope: &mut v8::PinScope<'_, '_>,
    f: impl FnOnce(&mut StorageBucketStore) -> R,
) -> Option<R> {
    let store = current_storage_bucket_store(scope)?;
    let mut store = store.lock();
    Some(f(&mut store))
}

pub(in crate::context_bootstrap) fn current_storage_bucket_store(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<SharedStorageBucketStore> {
    scope
        .get_current_context()
        .get_slot::<StorageBucketStoreSlot>()
        .as_deref()
        .and_then(|slot| slot.0.clone())
}

pub(in crate::context_bootstrap) fn storage_bucket_quota_owner_for_locator(
    scope: &mut v8::PinScope<'_, '_>,
    locator: &StorageBucketLocator,
) -> Option<StorageBucketQuotaOwner> {
    let store = current_storage_bucket_store(scope)?;
    storage_bucket_quota_owner(&store, locator)
}

pub(in crate::context_bootstrap) fn complete_storage_bucket_deletion_for_context(
    scope: &mut v8::PinScope<'_, '_>,
    identity: &StorageBucketIdentity,
) -> anyhow::Result<bool> {
    let store = current_storage_bucket_store(scope)
        .ok_or_else(|| anyhow::anyhow!("storage bucket store is unavailable"))?;
    moli_storage_service::complete_storage_bucket_deletion(&store, identity)
}
