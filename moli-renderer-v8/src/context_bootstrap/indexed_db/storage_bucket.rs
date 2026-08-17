//! Storage Bucket identity, liveness, and quota integration for IndexedDB.

use super::{
    IndexedDbError, IndexedDbQuotaCheck, IndexedDbStorageScope, build_scoped_indexed_db_factory,
    indexed_db_object_store_database, indexed_db_typed_storage_scope, object_property_as_object,
    storage_scope_for_current_partition,
};
use crate::context_bootstrap::storage_buckets::{
    storage_bucket_quota_owner_for_locator, with_storage_bucket_store_entry,
};
use moli_storage_service::{StorageBucketIdentity, StorageBucketLocator};

pub(in crate::context_bootstrap) fn scoped_storage_bucket_indexed_db_factory<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    identity: &StorageBucketIdentity,
) -> Option<v8::Local<'s, v8::Object>> {
    let storage_scope =
        storage_scope_for_current_partition(scope, identity.indexed_db_storage_key())?
            .with_bucket_identity(identity.clone());
    build_scoped_indexed_db_factory(scope, storage_scope)
}

pub(in crate::context_bootstrap::indexed_db) struct IndexedDbBucketQuotaCommit {
    pub quota_check: IndexedDbQuotaCheck,
    _reservation: moli_storage_service::StorageQuotaReservation,
}

fn live_bucket_locator(
    scope: &mut v8::PinScope<'_, '_>,
    identity: &StorageBucketIdentity,
) -> std::result::Result<StorageBucketLocator, IndexedDbError> {
    with_storage_bucket_store_entry(scope, |store| store.bucket_locator_for_identity(identity))
        .ok_or_else(|| {
            IndexedDbError::InvalidState("StorageBucket IndexedDB store is unavailable".to_owned())
        })?
        .ok_or_else(|| {
            IndexedDbError::InvalidState(
                "StorageBucket IndexedDB bucket is no longer current".to_owned(),
            )
        })
}

pub(in crate::context_bootstrap::indexed_db) fn validate_storage_bucket_scope(
    scope: &mut v8::PinScope<'_, '_>,
    storage_scope: &IndexedDbStorageScope,
) -> std::result::Result<(), IndexedDbError> {
    let Some(identity) = storage_scope.bucket_identity() else {
        return Ok(());
    };
    live_bucket_locator(scope, identity).map(|_| ())
}

fn storage_bucket_quota_check_for_database<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
) -> Option<std::result::Result<IndexedDbBucketQuotaCommit, IndexedDbError>> {
    let storage_scope = indexed_db_typed_storage_scope(scope, database)?;
    let locator = match storage_scope.bucket_identity() {
        Some(identity) => match live_bucket_locator(scope, identity) {
            Ok(locator) => locator,
            Err(error) => return Some(Err(error)),
        },
        None => StorageBucketLocator::default_bucket(storage_scope.storage_key()),
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
    Some(Ok(IndexedDbBucketQuotaCommit {
        quota_check: IndexedDbQuotaCheck {
            quota,
            non_indexed_db_usage,
        },
        _reservation: reservation,
    }))
}

pub(in crate::context_bootstrap::indexed_db) fn storage_bucket_quota_check_for_object_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
) -> Option<std::result::Result<IndexedDbBucketQuotaCommit, IndexedDbError>> {
    let database = indexed_db_object_store_database(scope, store)?;
    storage_bucket_quota_check_for_database(scope, database)
}

pub(in crate::context_bootstrap::indexed_db) fn storage_bucket_quota_check_for_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
) -> Option<std::result::Result<IndexedDbBucketQuotaCommit, IndexedDbError>> {
    let database = object_property_as_object(scope, transaction, "db")?;
    storage_bucket_quota_check_for_database(scope, database)
}
