//! SQL submissions retain the shared bucket reservation through publication.
//! Quota contention occupies no worker; usage sampling is off V8 and SQL.

use super::*;
use moli_indexeddb::{
    IndexedDbError,
    sqlite::{
        QuotaCheck,
        service::{Command, Completion, DatabaseClient},
    },
};

#[cfg(test)]
mod tests;

impl StorageBucketQuotaOwner {
    pub fn dispatch_indexed_db(
        &self,
        client: DatabaseClient,
        mut command: Command,
        completion: impl FnOnce(Result<Completion, IndexedDbError>) + Send + 'static,
    ) -> Result<(), crate::StorageServiceDispatchError> {
        let locator = self.locator.clone();
        let owner = self.clone();
        let sampling_client = client.clone();
        self.storage_service.dispatch_quota_commit(
            locator,
            move |quota| {
                let check = owner.indexed_db_quota_check(&sampling_client);
                (quota, check)
            },
            move |sample| {
                let (reservation, check) = match sample {
                    Ok((reservation, check)) => (Some(reservation), check),
                    Err(error) => (None, Err(IndexedDbError::Io(error.to_string()))),
                };
                let is_write =
                    matches!(command, Command::Write { .. } | Command::FinishWrite { .. });
                if let Ok(check) = &check {
                    match &mut command {
                        Command::Write { quota, .. } | Command::FinishWrite { quota, .. } => {
                            *quota = Some(check.database_limit())
                        }
                        Command::Commit { quota, .. } => *quota = Some(*check),
                        _ => {}
                    }
                }
                let preflight = check.as_ref().map(|_| ()).map_err(Clone::clone);
                // Late dispatch errors have no SQL rollback receipt. Report
                // them separately so the owner keeps its emergency rollback.
                let completion = Arc::new(Mutex::new(Some(completion)));
                let delivered = completion.clone();
                let submitted = client.submit_checked(command, preflight, move |mut reply| {
                    if is_write
                        && let Ok(check) = check
                        && let Err(IndexedDbError::QuotaExceeded { requested, .. }) = &reply.result
                    {
                        reply.result = Err(IndexedDbError::QuotaExceeded {
                            quota: check.quota,
                            requested: check.other_usage.saturating_add(*requested),
                        });
                    }
                    // Native committed metadata and admission are published
                    // before this callback. Release before waking the owner.
                    drop(reservation);
                    if let Some(completion) = delivered.lock().take() {
                        completion(Ok(reply));
                    }
                });
                if let Err(error) = submitted
                    && let Some(completion) = completion.lock().take()
                {
                    completion(Err(IndexedDbError::Resource(error.to_string())));
                }
            },
        )
    }

    fn indexed_db_quota_check(
        &self,
        client: &DatabaseClient,
    ) -> Result<QuotaCheck, IndexedDbError> {
        let storage_key = match &self.locator {
            StorageBucketLocator::Default { storage_key } => storage_key.clone(),
            StorageBucketLocator::Named {
                storage_key,
                bucket_id,
            } => storage_bucket_indexed_db_storage_key(storage_key, *bucket_id),
        };
        if client.identity().owner_key != storage_key.as_bytes() {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB quota owner belongs to another bucket".into(),
            ));
        }
        let (quota, other_backends) = self
            .quota_and_non_indexed_db_usage()
            .map_err(|error| IndexedDbError::InvalidState(error.to_string()))?;
        let all_databases = self
            .indexed_db_usage()
            .map_err(|error| IndexedDbError::InvalidState(error.to_string()))?;
        let own = client
            .committed()
            .ok_or_else(|| {
                IndexedDbError::InvalidState("IndexedDB committed usage is unavailable".into())
            })?
            .usage;
        let other_databases = all_databases.checked_sub(own).ok_or_else(|| {
            IndexedDbError::Corruption(
                "IndexedDB aggregate usage is smaller than this database".into(),
            )
        })?;
        Ok(QuotaCheck {
            quota,
            other_usage: other_backends.saturating_add(other_databases),
        })
    }
}
