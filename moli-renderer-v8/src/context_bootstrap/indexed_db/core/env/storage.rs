use super::*;
use crate::context_bootstrap::indexed_db::WeakIndexedDbManager;
use crate::context_bootstrap::storage_buckets::{
    storage_bucket_quota_owner_for_locator, with_storage_bucket_store_entry,
};

pub(in crate::context_bootstrap::indexed_db) struct StorageBucketIndexedDbQuotaCommit {
    pub quota_check: IndexedDbQuotaCheck,
    _reservation: moli_storage_service::StorageQuotaReservation,
}

#[derive(Clone, Debug)]
pub(crate) struct IndexedDbManagerSlot(pub(crate) Option<WeakIndexedDbManager>);

pub(crate) fn set_indexed_db_manager_for_context(
    context: v8::Local<'_, v8::Context>,
    manager: Option<WeakIndexedDbManager>,
) {
    let _previous = context.set_slot(std::rc::Rc::new(IndexedDbManagerSlot(manager)));
}

pub(in crate::context_bootstrap::indexed_db) fn with_indexed_db_manager<R>(
    scope: &mut v8::PinScope<'_, '_>,
    f: impl FnOnce(&mut IndexedDbManager) -> std::result::Result<R, IndexedDbError>,
) -> std::result::Result<R, IndexedDbError> {
    let manager = scope
        .get_current_context()
        .get_slot::<IndexedDbManagerSlot>()
        .as_deref()
        .and_then(|slot| slot.0.as_ref())
        .and_then(WeakIndexedDbManager::upgrade)
        .ok_or_else(|| {
            IndexedDbError::InvalidState("IndexedDB browser context is closed".to_owned())
        })?;
    let mut manager = manager.lock();
    f(&mut manager)
}

pub(in crate::context_bootstrap) fn indexed_db_usage_bytes_for_storage_key(
    scope: &mut v8::PinScope<'_, '_>,
    storage_key: &str,
) -> u64 {
    let usage = with_indexed_db_manager(scope, |manager| manager.origin_usage_bytes(storage_key));
    usage.unwrap_or(0)
}

pub(in crate::context_bootstrap::indexed_db) fn storage_bucket_quota_check_for_database<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
) -> Option<std::result::Result<StorageBucketIndexedDbQuotaCommit, IndexedDbError>> {
    let storage_scope = indexed_db_typed_storage_scope(scope, database)?.clone();
    let locator = if let Some(context) = storage_scope.bucket_context() {
        let Some(locator) = with_storage_bucket_store_entry(scope, |store| {
            store.bucket_locator_for_identity(&context.identity)
        }) else {
            return Some(Err(IndexedDbError::InvalidState(
                "StorageBucket IndexedDB quota store is unavailable".to_owned(),
            )));
        };
        let Some(locator) = locator else {
            return Some(Err(IndexedDbError::InvalidState(
                "StorageBucket IndexedDB bucket is no longer current".to_owned(),
            )));
        };
        locator
    } else {
        moli_storage_service::StorageBucketLocator::default_bucket(storage_scope.storage_key())
    };
    let Some(owner) = storage_bucket_quota_owner_for_locator(scope, &locator) else {
        return Some(Err(IndexedDbError::InvalidState(
            "StorageBucket IndexedDB aggregate quota owner is unavailable".to_owned(),
        )));
    };
    let reservation = owner.reserve_commit();
    let (quota, non_indexed_db_usage) = match owner.quota_and_non_indexed_db_usage() {
        Ok(usage) => usage,
        Err(error) => {
            return Some(Err(IndexedDbError::InvalidState(error.to_string())));
        }
    };
    Some(Ok(StorageBucketIndexedDbQuotaCommit {
        quota_check: IndexedDbQuotaCheck {
            quota,
            non_indexed_db_usage,
        },
        _reservation: reservation,
    }))
}

pub(in crate::context_bootstrap::indexed_db) fn validate_storage_bucket_indexed_db_context(
    scope: &mut v8::PinScope<'_, '_>,
    context: &IndexedDbStorageBucketContext,
) -> std::result::Result<(), IndexedDbError> {
    let is_current = with_storage_bucket_store_entry(scope, |store| {
        store.bucket_identity_is_live(&context.identity)
    })
    .ok_or_else(|| {
        IndexedDbError::InvalidState("StorageBucket IndexedDB store is unavailable".to_owned())
    })?;
    if is_current {
        Ok(())
    } else {
        Err(IndexedDbError::InvalidState(
            "StorageBucket IndexedDB bucket is no longer current".to_owned(),
        ))
    }
}

pub(in crate::context_bootstrap::indexed_db) fn storage_bucket_quota_check_for_object_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
) -> Option<std::result::Result<StorageBucketIndexedDbQuotaCommit, IndexedDbError>> {
    let database = indexed_db_object_store_database(scope, store)?;
    storage_bucket_quota_check_for_database(scope, database)
}

pub(in crate::context_bootstrap::indexed_db) fn storage_bucket_quota_check_for_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> Option<std::result::Result<StorageBucketIndexedDbQuotaCommit, IndexedDbError>> {
    let database = object_property_as_object(scope, transaction, "db")?;
    storage_bucket_quota_check_for_database(scope, database)
}

#[cfg(test)]
pub(crate) fn indexed_db_manager_context_slot_present_for_test(
    scope: &mut v8::PinScope<'_, '_>,
) -> bool {
    scope
        .get_current_context()
        .get_slot::<IndexedDbManagerSlot>()
        .is_some()
}

#[cfg(test)]
pub(crate) fn indexed_db_manager_isolate_slot_present_for_test(
    scope: &mut v8::PinScope<'_, '_>,
) -> bool {
    scope.get_slot::<IndexedDbManagerSlot>().is_some()
}
