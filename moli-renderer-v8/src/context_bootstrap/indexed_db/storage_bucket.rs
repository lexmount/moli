//! Storage Bucket identity, liveness, and quota integration for IndexedDB.

use super::{
    IndexedDbError, IndexedDbStorageScope, build_scoped_indexed_db_factory,
    indexed_db_transaction_database, indexed_db_typed_storage_scope,
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

pub(in crate::context_bootstrap::indexed_db) fn sql_quota_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Result<Option<moli_storage_service::StorageBucketQuotaOwner>, IndexedDbError> {
    let transaction = if super::indexed_db_typed_wrapper_is(
        scope,
        source,
        super::IndexedDbWrapperKind::Transaction,
    ) {
        source
    } else {
        super::indexed_db_request_transaction_object(scope, source).ok_or_else(|| {
            IndexedDbError::InvalidState("SQL request lost its transaction".into())
        })?
    };
    if super::indexed_db_transaction_mode(scope, transaction)
        == Some(moli_indexeddb::TransactionMode::ReadOnly)
    {
        return Ok(None);
    }
    let database = indexed_db_transaction_database(scope, transaction)
        .ok_or_else(|| IndexedDbError::InvalidState("SQL transaction lost its database".into()))?;
    super::indexed_db_database_sql_connection(scope, database).ok_or_else(|| {
        IndexedDbError::InvalidState("SQL database lost its logical connection".into())
    })?;
    let storage_scope = indexed_db_typed_storage_scope(scope, database).ok_or_else(|| {
        IndexedDbError::InvalidState("SQL database lost its storage owner".into())
    })?;
    let locator = match storage_scope.bucket_identity() {
        Some(identity) => live_bucket_locator(scope, identity)?,
        None => StorageBucketLocator::default_bucket(storage_scope.storage_key()),
    };
    storage_bucket_quota_owner_for_locator(scope, &locator)
        .map(Some)
        .ok_or_else(|| {
            IndexedDbError::InvalidState("SQL database quota owner is unavailable".into())
        })
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
