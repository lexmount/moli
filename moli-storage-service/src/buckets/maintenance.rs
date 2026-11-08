//! Bucket metadata and cleanup run on the existing maintenance sequence.

use super::*;

#[cfg(test)]
mod tests;
use moli_indexeddb::sqlite::service::ServiceClient;

#[derive(Debug)]
pub enum SqlBucketOperation {
    Open {
        storage_key: String,
        name: String,
        expires: Option<f64>,
        durability: Option<StorageBucketDurability>,
        quota: Option<u64>,
        persisted: Option<bool>,
    },
    Keys {
        storage_key: String,
    },
    Delete {
        storage_key: String,
        name: String,
    },
    SetExpires {
        identity: StorageBucketIdentity,
        expires: Option<f64>,
    },
    Expire {
        identity: StorageBucketIdentity,
    },
}

#[derive(Debug)]
pub enum SqlBucketOperationValue {
    Opened(StorageBucketIdentity),
    Keys { names: Vec<String> },
    Complete,
    Stale,
}

#[derive(Debug)]
pub struct SqlBucketOperationResult {
    pub result: std::result::Result<SqlBucketOperationValue, String>,
}

pub struct SqlBucketMaintenance {
    store: SharedStorageBucketStore,
    sql: ServiceClient,
    operation: SqlBucketOperation,
}

pub fn prepare_sql_bucket_maintenance(
    store: &SharedStorageBucketStore,
    operation: SqlBucketOperation,
) -> Result<Option<SqlBucketMaintenance>> {
    let manager = store.lock().indexed_db_manager.clone();
    let Some(sql) = manager.map(|manager| manager.lock().sqlite_service()) else {
        return Ok(None);
    };
    if sql.is_closed() {
        bail!("IndexedDB storage service is closed");
    }
    Ok(Some(SqlBucketMaintenance {
        store: store.clone(),
        sql,
        operation,
    }))
}

impl SqlBucketOperation {
    fn storage_key(&self) -> &str {
        match self {
            Self::Open { storage_key, .. }
            | Self::Keys { storage_key }
            | Self::Delete { storage_key, .. } => storage_key,
            Self::SetExpires { identity, .. } | Self::Expire { identity } => identity.storage_key(),
        }
    }

    fn execute(self, store: &SharedStorageBucketStore) -> Result<SqlBucketOperationValue> {
        match self {
            Self::Open {
                storage_key,
                name,
                expires,
                durability,
                quota,
                persisted,
            } => {
                let expired = store.lock().delete_bucket_if_expired(
                    &storage_key,
                    &name,
                    storage_bucket_current_unix_ms(),
                )?;
                if let Some(identity) = expired {
                    complete_storage_bucket_deletion_with_quota(store, &identity)?;
                }
                let identity = store.lock().open_bucket_with_options(
                    &storage_key,
                    &name,
                    expires,
                    durability,
                    quota,
                    persisted,
                )?;
                Ok(SqlBucketOperationValue::Opened(identity))
            }
            Self::Keys { storage_key } => {
                let expired = store
                    .lock()
                    .delete_expired_buckets(&storage_key, storage_bucket_current_unix_ms())?;
                for identity in expired {
                    complete_storage_bucket_deletion_with_quota(store, &identity)?;
                }
                let store = store.lock();
                Ok(SqlBucketOperationValue::Keys {
                    names: store.keys(&storage_key),
                })
            }
            Self::Delete { storage_key, name } => {
                let identity = store.lock().delete_bucket(&storage_key, &name)?;
                if let Some(identity) = identity {
                    complete_storage_bucket_deletion_with_quota(store, &identity)?;
                }
                Ok(SqlBucketOperationValue::Complete)
            }
            Self::SetExpires { identity, expires } => {
                let mut store = store.lock();
                if !store.bucket_identity_is_live(&identity) {
                    return Ok(SqlBucketOperationValue::Stale);
                }
                Ok(
                    if store.set_bucket_expires_for_identity(&identity, expires)? {
                        SqlBucketOperationValue::Complete
                    } else {
                        SqlBucketOperationValue::Stale
                    },
                )
            }
            Self::Expire { identity } => {
                let expired = {
                    let mut store = store.lock();
                    // An expired old handle must never revoke a replacement.
                    if store
                        .bucket_identity(identity.storage_key(), identity.name())
                        .as_ref()
                        != Some(&identity)
                        && !store.memory().pending_deletions.contains(&identity)
                    {
                        return Ok(SqlBucketOperationValue::Complete);
                    }
                    store.delete_bucket_if_expired(
                        identity.storage_key(),
                        identity.name(),
                        storage_bucket_current_unix_ms(),
                    )?
                };
                if let Some(identity) = expired {
                    complete_storage_bucket_deletion_with_quota(store, &identity)?;
                }
                Ok(SqlBucketOperationValue::Complete)
            }
        }
    }
}

fn complete_storage_bucket_deletion_with_quota(
    store: &SharedStorageBucketStore,
    identity: &StorageBucketIdentity,
) -> Result<()> {
    let service = store.lock().storage_service();
    let _quota = service.reserve_quota_commit(&identity.locator());
    complete_storage_bucket_deletion(store, identity)?;
    Ok(())
}

impl SqlBucketMaintenance {
    pub fn locator(&self) -> StorageBucketLocator {
        StorageBucketLocator::default_bucket(self.operation.storage_key().to_owned())
    }

    pub fn dispatch(
        self,
        completion: impl FnOnce(SqlBucketOperationResult) + Send + 'static,
    ) -> std::result::Result<(), crate::StorageServiceDispatchError> {
        let Self {
            store,
            sql,
            operation,
        } = self;
        let service = store.lock().storage_service();
        service.maintenance_queue.dispatch(
            move || -> std::result::Result<SqlBucketOperationValue, String> {
                if sql.is_closed() {
                    return Err("IndexedDB storage service is closed".into());
                }
                operation.execute(&store).map_err(|error| error.to_string())
            },
            move |result| {
                completion(SqlBucketOperationResult {
                    result: result.unwrap_or_else(|error| Err(error.to_string())),
                });
            },
        )
    }
}
