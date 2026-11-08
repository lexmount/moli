//! Bucket identities must reach disk before a database can reference them.
//! Candidates contain metadata only; CacheStorage bodies are never cloned.

use super::*;

#[cfg(test)]
mod tests;

impl MemoryStorageBucketBackend {
    pub(super) fn clone_metadata(&self) -> Self {
        Self {
            origins: self
                .origins
                .iter()
                .map(|(origin, buckets)| {
                    (
                        origin.clone(),
                        buckets
                            .iter()
                            .map(|(name, metadata)| {
                                (
                                    name.clone(),
                                    StorageBucketMetadata {
                                        bucket_id: metadata.bucket_id,
                                        expires: metadata.expires,
                                        durability: metadata.durability,
                                        quota: metadata.quota,
                                        persisted: metadata.persisted,
                                        ..Default::default()
                                    },
                                )
                            })
                            .collect(),
                    )
                })
                .collect(),
            pending_deletions: self.pending_deletions.clone(),
            next_bucket_id: self.next_bucket_id,
        }
    }

    fn retain_caches_from(&mut self, mut previous: Self) {
        for (origin, buckets) in &mut self.origins {
            let Some(old_buckets) = previous.origins.get_mut(origin) else {
                continue;
            };
            for (name, metadata) in buckets {
                let Some(mut old) = old_buckets.remove(name) else {
                    continue;
                };
                if old.bucket_id != metadata.bucket_id {
                    continue;
                }
                old.expires = metadata.expires;
                old.durability = metadata.durability;
                old.quota = metadata.quota;
                old.persisted = metadata.persisted;
                *metadata = old;
            }
        }
    }
}

impl StorageBucketRegistry {
    pub(super) fn metadata_publication_uncertain(&self) -> bool {
        matches!(&self.backend, StorageBucketBackend::Json(json) if json.metadata_publication_uncertain)
    }

    pub(super) fn edit_metadata<T>(
        &mut self,
        edit: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        if matches!(self.backend, StorageBucketBackend::Memory(_)) {
            return edit(self);
        }
        if self.metadata_publication_uncertain() {
            bail!("storage bucket metadata publication is uncertain; reopen the profile");
        }
        let mut candidate = Self {
            backend: StorageBucketBackend::Memory(self.memory().clone_metadata()),
            storage_service: self.storage_service.clone(),
            indexed_db_manager: None,
        };
        let result = edit(&mut candidate)?;
        let StorageBucketBackend::Memory(mut metadata) = candidate.backend else {
            unreachable!("metadata candidates are memory-only");
        };
        let StorageBucketBackend::Json(json) = &mut self.backend else {
            unreachable!("persistent metadata has a JSON backend");
        };
        // Until this succeeds neither the candidate ID nor its high water mark
        // is visible to another backend. The registry serializes publishers.
        json.publish_metadata(&metadata)?;
        let previous = std::mem::take(&mut json.memory);
        metadata.retain_caches_from(previous);
        json.memory = metadata;
        // Removing buckets also reclaims their CacheStorage files. A failure
        // leaves the durable tombstone available for recovery and retry.
        json.save_cache_storage()?;
        Ok(result)
    }
}

impl JsonStorageBucketBackend {
    pub(super) fn publish_metadata(&mut self, metadata: &MemoryStorageBucketBackend) -> Result<()> {
        if self.metadata_publication_uncertain {
            bail!("storage bucket metadata publication is uncertain; reopen the profile");
        }
        let metadata = metadata.clone_metadata();
        let json = StorageBucketsJson {
            version: STORAGE_BUCKETS_JSON_VERSION,
            next_bucket_id: metadata.next_bucket_id,
            cache_paths_use_bucket_ids: self.cache_paths_use_bucket_ids,
            pending_deletions: metadata.pending_deletions,
            origins: metadata.origins,
        };
        let bytes =
            serde_json::to_vec_pretty(&json).context("failed to serialize storage bucket store")?;
        if let Err(error) = moli_browser_profile::write_profile_file_durable(
            &self.path,
            &bytes,
            "storage bucket store",
        ) {
            self.metadata_publication_uncertain |= error.publication_uncertain();
            return Err(error.into());
        }
        Ok(())
    }
}
