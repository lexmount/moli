//! SQLite bucket cleanup uses the existing storage queue and shared quota lock.
//! Ownership travels with queued work and results; no V8 values cross it.

use super::*;
use moli_indexeddb::sqlite::service::OwnerClearRequest;

#[cfg(test)]
mod tests;

pub struct SqlBucketDeletion {
    store: SharedStorageBucketStore,
    service: SharedStorageService,
    identity: StorageBucketIdentity,
    clear: OwnerClearRequest,
}

#[derive(Debug)]
pub struct SqlBucketDeletionResult {
    pub result: std::result::Result<(), String>,
}

/// Prepare native cleanup; duplicate identities are checked on the worker.
pub fn prepare_sql_bucket_deletion(
    store: &SharedStorageBucketStore,
    identity: &StorageBucketIdentity,
) -> Result<Option<SqlBucketDeletion>> {
    let (service, manager) = {
        let store = store.lock();
        (store.storage_service(), store.indexed_db_manager.clone())
    };
    let Some(manager) = manager else {
        return Ok(None);
    };
    let sql = manager.lock().sqlite_service();
    let clear = sql.reserve_clear_owner(identity.indexed_db_storage_key().into_bytes())?;
    Ok(Some(SqlBucketDeletion {
        store: store.clone(),
        service,
        identity: identity.clone(),
        clear,
    }))
}

impl SqlBucketDeletion {
    pub fn locator(&self) -> StorageBucketLocator {
        self.identity.locator()
    }

    pub fn dispatch(
        self,
        completion: impl FnOnce(SqlBucketDeletionResult) + Send + 'static,
    ) -> std::result::Result<(), crate::StorageServiceDispatchError> {
        let Self {
            store,
            service,
            identity,
            clear,
        } = self;
        let worker_service = service.clone();
        service.quota_wait_queue.dispatch(
            move || -> std::result::Result<(), String> {
                let _quota = worker_service.reserve_quota_commit(&identity.locator());
                // Another queued delete may already have completed this exact
                // tombstone. A replacement with the same name has another ID.
                let pending = store
                    .lock()
                    .memory()
                    .pending_deletions
                    .iter()
                    .any(|old| old == &identity);
                if !pending {
                    return Ok(());
                }
                clear.execute().map_err(|error| error.to_string())?;
                worker_service
                    .clear_opfs_bucket(&identity.locator())
                    .map_err(|error| error.to_string())?;
                store
                    .lock()
                    .finish_bucket_deletion(&identity)
                    .map_err(|error| error.to_string())?;
                Ok(())
            },
            move |result| {
                completion(SqlBucketDeletionResult {
                    result: result.unwrap_or_else(|error| Err(error.to_string())),
                });
            },
        )
    }
}
