use std::{collections::HashMap, path::PathBuf};

use moli_cookie_jar::SharedBrowserCookieStore;
use moli_core::{
    network::SharedWebStorageStore,
    storage::{SharedIndexedDbManager, SharedStorageBucketStore, StorageBucketIdentity},
};

use super::BrowserContextStoragePartitionHandles;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct OriginStorageUsage {
    pub(crate) local_storage_usage: u64,
    pub(crate) indexed_db_usage: u64,
    pub(crate) storage_buckets_usage: u64,
    pub(crate) total_usage: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SiteDataClearOptions {
    pub cookies: bool,
    pub local_storage: bool,
    pub indexed_db: bool,
    pub storage_buckets: bool,
    pub http_cache: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StoragePartitionKind {
    ProfileBacked,
    Ephemeral,
}

/// The stores and their policy share one owner; only capability handles clone.
pub(super) struct StoragePartition {
    pub(super) kind: StoragePartitionKind,
    pub(super) handles: BrowserContextStoragePartitionHandles,
    storage_quota_overrides: HashMap<String, f64>,
    http_cache_root: Option<PathBuf>,
    http_cache_max_bytes: Option<u64>,
}

impl StoragePartition {
    pub(super) fn new(
        handles: BrowserContextStoragePartitionHandles,
        kind: StoragePartitionKind,
        http_cache_root: Option<PathBuf>,
        http_cache_max_bytes: Option<u64>,
    ) -> Self {
        Self {
            kind,
            handles,
            storage_quota_overrides: HashMap::new(),
            http_cache_root,
            http_cache_max_bytes,
        }
    }

    pub(super) fn usage_for_origin(
        &self,
        serialized_origin: &str,
    ) -> Result<OriginStorageUsage, String> {
        let local_storage_usage = {
            let store = self.web_storage_store().lock();
            usize_to_u64_saturating(store.usage_bytes_for_origin_areas(serialized_origin))
        };
        let indexed_db_usage = moli_core::storage::indexed_db_origins_with_prefix_usage_bytes(
            self.indexed_db_manager(),
            &moli_storage_key::storage_key_prefix_for_origin(serialized_origin),
        )?;
        let storage_buckets_usage = self.storage_bucket_usage_for_origin(serialized_origin)?;
        Ok(OriginStorageUsage {
            local_storage_usage,
            indexed_db_usage,
            storage_buckets_usage,
            total_usage: local_storage_usage
                .saturating_add(indexed_db_usage)
                .saturating_add(storage_buckets_usage),
        })
    }

    fn storage_bucket_usage_for_origin(&self, serialized_origin: &str) -> Result<u64, String> {
        let (bucket_identities, cache_usage, storage_service) = {
            let store = self.storage_bucket_store().lock();
            (
                store.bucket_identities_for_origin_areas(serialized_origin),
                store.cache_usage_for_origin_areas(serialized_origin),
                store.storage_service(),
            )
        };
        let mut usage = cache_usage;
        for identity in bucket_identities {
            let indexed_db_usage = moli_core::storage::indexed_db_origin_usage_bytes(
                self.indexed_db_manager(),
                &identity.indexed_db_storage_key(),
            )?;
            let opfs_usage = storage_service
                .opfs_usage(&identity.locator())
                .map_err(|error| format!("FailedToReadStorageBucketOpfsUsage: {error}"))?;
            usage = usage
                .saturating_add(indexed_db_usage)
                .saturating_add(opfs_usage);
        }
        Ok(usage)
    }

    fn complete_storage_bucket_deletions(
        &self,
        cleanups: Vec<StorageBucketIdentity>,
    ) -> Result<(), String> {
        let bucket_store = self.storage_bucket_store();
        for cleanup in cleanups {
            moli_core::storage::complete_storage_bucket_deletion(bucket_store, &cleanup)
                .map_err(|error| format!("FailedToCompleteStorageBucketDeletion: {error}"))?;
        }
        Ok(())
    }

    pub(super) fn clear_site_data_for_origin(
        &mut self,
        origin: &url::Url,
        options: SiteDataClearOptions,
    ) -> Result<(), String> {
        if options.cookies
            && let Some(host) = origin.host_str().map(str::to_ascii_lowercase)
        {
            let mut cookie_store = self.cookie_store().lock();
            cookie_store.delete_cookies(None, None, None, Some(host.as_str()));
        }

        let serialized_origin = origin.origin().ascii_serialization();
        if options.local_storage {
            let mut store = self.web_storage_store().lock();
            store
                .try_clear_origin_areas(&serialized_origin)
                .map_err(|error| format!("FailedToClearLocalStorage: {error}"))?;
        }

        if options.indexed_db {
            moli_core::storage::clear_indexed_db_origins_with_prefix(
                self.indexed_db_manager(),
                &moli_storage_key::storage_key_prefix_for_origin(&serialized_origin),
            )?;
        }

        if options.storage_buckets {
            let cleanups = self
                .storage_bucket_store()
                .lock()
                .clear_origin_areas(&serialized_origin)
                .map_err(|error| format!("FailedToClearStorageBuckets: {error}"))?;
            self.complete_storage_bucket_deletions(cleanups)?;
        }

        if options.http_cache {
            self.clear_http_cache_for_origin(origin)?;
        }

        Ok(())
    }

    pub(super) fn clear_site_data_for_storage_key(
        &mut self,
        storage_key: &moli_storage_key::MoliStorageKey,
        options: SiteDataClearOptions,
    ) -> Result<(), String> {
        let origin = url::Url::parse(storage_key.origin())
            .map_err(|error| format!("UnableToDeserializeStorageKeyOrigin: {error}"))?;
        if origin.origin().ascii_serialization() != storage_key.origin() {
            return Err("UnableToDeserializeStorageKeyOrigin".to_owned());
        }

        if options.cookies
            && let Some(host) = origin.host_str().map(str::to_ascii_lowercase)
        {
            let mut cookie_store = self.cookie_store().lock();
            cookie_store.delete_cookies(None, None, None, Some(host.as_str()));
        }

        let serialized_storage_key = storage_key.serialized_storage_key();
        if options.local_storage {
            let mut store = self.web_storage_store().lock();
            store
                .try_clear_origin(&serialized_storage_key)
                .map_err(|error| format!("FailedToClearLocalStorage: {error}"))?;
        }

        if options.indexed_db {
            moli_core::storage::clear_indexed_db_origin(
                self.indexed_db_manager(),
                &serialized_storage_key,
            )?;
        }

        if options.storage_buckets {
            let cleanups = self
                .storage_bucket_store()
                .lock()
                .clear_origin(&serialized_storage_key)
                .map_err(|error| format!("FailedToClearStorageBuckets: {error}"))?;
            self.complete_storage_bucket_deletions(cleanups)?;
        }

        if options.http_cache {
            self.clear_http_cache_for_origin(&origin)?;
        }

        Ok(())
    }

    pub(super) fn kind_label(&self) -> &'static str {
        match self.kind {
            StoragePartitionKind::ProfileBacked => "profile-backed",
            StoragePartitionKind::Ephemeral => "ephemeral",
        }
    }

    pub(super) fn cookie_store(&self) -> &SharedBrowserCookieStore {
        &self.handles.cookie_store
    }

    pub(super) fn web_storage_store(&self) -> &SharedWebStorageStore {
        &self.handles.web_storage_store
    }

    pub(super) fn indexed_db_manager(&self) -> &SharedIndexedDbManager {
        &self.handles.indexed_db_manager
    }

    pub(super) fn storage_bucket_store(&self) -> &SharedStorageBucketStore {
        &self.handles.storage_bucket_store
    }

    #[cfg(test)]
    pub(super) fn replace_storage_bucket_store(
        &mut self,
        storage_bucket_store: SharedStorageBucketStore,
    ) {
        self.handles.storage_bucket_store = storage_bucket_store;
    }

    pub(super) fn storage_quota_for_origin(&self, origin: &str) -> (f64, bool) {
        self.storage_quota_overrides
            .get(origin)
            .copied()
            .map(|quota| (quota, true))
            .unwrap_or((
                moli_core::storage::DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES as f64,
                false,
            ))
    }

    pub(super) fn set_storage_quota_override(&mut self, origin: String, quota: f64) {
        self.storage_quota_overrides.insert(origin, quota);
    }

    pub(super) fn clear_storage_quota_override(&mut self, origin: &str) {
        self.storage_quota_overrides.remove(origin);
    }

    pub(super) fn clear_http_cache(&self) -> Result<(), String> {
        let Some(cache_root) = self.http_cache_root.as_ref() else {
            return Ok(());
        };
        moli_fetch::clear_http_cache_root(cache_root, self.http_cache_max_bytes)
            .map_err(|error| format!("FailedToClearHttpCache: {error}"))
    }

    fn clear_http_cache_for_origin(&self, origin: &url::Url) -> Result<usize, String> {
        let Some(cache_root) = self.http_cache_root.as_ref() else {
            return Ok(0);
        };
        moli_fetch::clear_http_cache_root_for_origin(cache_root, self.http_cache_max_bytes, origin)
            .map_err(|error| format!("FailedToClearHttpCache: {error}"))
    }

    #[cfg(test)]
    pub(super) fn http_cache_configuration(&self) -> (Option<&std::path::Path>, Option<u64>) {
        (self.http_cache_root.as_deref(), self.http_cache_max_bytes)
    }
}

impl std::fmt::Debug for StoragePartition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoragePartition")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
