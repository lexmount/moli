use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    SharedStorageService, StorageBucketId, StorageBucketLocator, StorageQuotaReservation,
    StorageService,
};
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use moli_crypto::sha256_hex;
use moli_indexeddb::IndexedDbManager;
use moli_storage_key::storage_key_prefix_for_origin;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod fault_injection;

const STORAGE_BUCKETS_JSON_VERSION: u32 = 5;
const STORAGE_BUCKET_CACHE_JSON_VERSION: u32 = 1;
pub const DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES: u64 = 1_073_741_824;
pub const IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME: &str = "\0moli-implicit-default";

pub type SharedStorageBucketIndexedDbManager = Arc<Mutex<IndexedDbManager>>;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageBucketDurability {
    #[default]
    Relaxed,
    Strict,
}

impl StorageBucketDurability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Relaxed => "relaxed",
            Self::Strict => "strict",
        }
    }

    fn is_relaxed(value: &Self) -> bool {
        matches!(value, Self::Relaxed)
    }
}

#[derive(Debug, Default, Clone)]
struct MemoryStorageBucketBackend {
    origins: BTreeMap<String, BTreeMap<String, StorageBucketMetadata>>,
    pending_deletions: Vec<StorageBucketIdentity>,
    next_bucket_id: u64,
}

/// Exact persistent identity of one materialized storage-bucket record.
///
/// The record is safe to carry outside the bucket metadata mutex. A web-visible
/// name can be reused after deletion, but the persistent bucket id cannot, so
/// asynchronous handles and deletion cleanup can both authorize work against
/// the exact record they captured.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageBucketIdentity {
    storage_key: String,
    name: String,
    bucket_id: StorageBucketId,
}

impl StorageBucketIdentity {
    pub fn new(storage_key: &str, name: &str, bucket_id: StorageBucketId) -> Self {
        Self {
            storage_key: storage_key.to_owned(),
            name: name.to_owned(),
            bucket_id,
        }
    }

    pub fn storage_key(&self) -> &str {
        &self.storage_key
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn bucket_id(&self) -> StorageBucketId {
        self.bucket_id
    }

    pub fn locator(&self) -> StorageBucketLocator {
        StorageBucketLocator::named(self.storage_key.clone(), self.bucket_id)
    }

    pub fn indexed_db_storage_key(&self) -> String {
        storage_bucket_indexed_db_storage_key(&self.storage_key, self.bucket_id)
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageBucketMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bucket_id: Option<StorageBucketId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires: Option<f64>,
    #[serde(default, skip_serializing_if = "StorageBucketDurability::is_relaxed")]
    durability: StorageBucketDurability,
    #[serde(skip_serializing_if = "Option::is_none")]
    quota: Option<u64>,
    #[serde(default, skip_serializing_if = "is_false")]
    persisted: bool,
    #[serde(skip)]
    cache_storage: BTreeMap<String, BTreeMap<String, StorageBucketCacheEntry>>,
    #[serde(skip)]
    cache_instance_ids: BTreeMap<String, StorageBucketCacheId>,
    #[serde(skip)]
    detached_cache_storage:
        BTreeMap<StorageBucketCacheId, BTreeMap<String, StorageBucketCacheEntry>>,
    #[serde(skip)]
    cache_instance_ref_counts: BTreeMap<StorageBucketCacheId, u64>,
    #[serde(skip)]
    next_cache_instance_id: u64,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq)]
pub struct StorageBucketCachedResponse {
    pub response_type: String,
    pub url: String,
    pub redirected: bool,
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageBucketCachedRequest {
    pub method: String,
    pub headers: Vec<(String, String)>,
}

impl Default for StorageBucketCachedRequest {
    fn default() -> Self {
        Self {
            method: "GET".to_owned(),
            headers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageBucketCacheQuery {
    pub request_url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub ignore_search: bool,
    pub ignore_method: bool,
    pub ignore_vary: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StorageBucketCacheMatch {
    pub request_url: String,
    pub request: StorageBucketCachedRequest,
    pub response: StorageBucketCachedResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StorageBucketCacheId(u64);

impl StorageBucketCacheId {
    pub const fn get(self) -> u64 {
        self.0
    }

    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone)]
struct StorageBucketCacheEntry {
    usage_bytes: u64,
    request: StorageBucketCachedRequest,
    response: StorageBucketCachedResponse,
    insertion_order: u64,
}

#[derive(Clone, Copy)]
enum StorageBucketCacheSelector<'a> {
    Named(&'a str),
    Handle {
        cache_name: &'a str,
        cache_id: StorageBucketCacheId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageBucketCachePutOutcome {
    Stored,
    Stale,
    QuotaExceeded { quota: u64, requested: u64 },
}

#[derive(Debug, Clone)]
struct JsonStorageBucketBackend {
    path: PathBuf,
    cache_storage_root: Option<PathBuf>,
    indexed_db_keys_use_bucket_ids: bool,
    cache_paths_use_bucket_ids: bool,
    memory: MemoryStorageBucketBackend,
}

#[derive(Debug)]
enum StorageBucketBackend {
    Memory(MemoryStorageBucketBackend),
    Json(JsonStorageBucketBackend),
}

pub struct StorageBucketRegistry {
    backend: StorageBucketBackend,
    storage_service: SharedStorageService,
    indexed_db_manager: Option<SharedStorageBucketIndexedDbManager>,
}

impl std::fmt::Debug for StorageBucketRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageBucketRegistry")
            .field("backend", &self.backend)
            .field("storage_service", &self.storage_service)
            .field(
                "indexed_db_manager_attached",
                &self.indexed_db_manager.is_some(),
            )
            .finish()
    }
}

pub type SharedStorageBucketRegistry = Arc<Mutex<StorageBucketRegistry>>;

/// Compatibility alias retained while renderer call sites migrate to the
/// renderer-neutral registry name.
pub type StorageBucketStore = StorageBucketRegistry;

/// Compatibility alias retained for the existing partition propagation API.
pub type SharedStorageBucketStore = SharedStorageBucketRegistry;

#[derive(Clone)]
pub struct StorageBucketQuotaOwner {
    locator: StorageBucketLocator,
    store: SharedStorageBucketStore,
    indexed_db_manager: Option<SharedStorageBucketIndexedDbManager>,
    storage_service: SharedStorageService,
}

/// Committed usage attributed to one default or named storage bucket.
///
/// This deliberately excludes transient OPFS writer reservations. Estimates
/// report committed bytes, while quota checks below use the conservative OPFS
/// quota usage which also accounts for in-flight growth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageBucketUsageSnapshot {
    pub quota: u64,
    pub indexed_db: u64,
    pub cache_storage: u64,
    pub opfs: u64,
}

impl StorageBucketUsageSnapshot {
    pub fn total(self) -> u64 {
        self.indexed_db
            .saturating_add(self.cache_storage)
            .saturating_add(self.opfs)
    }
}

impl StorageBucketQuotaOwner {
    pub fn reserve_commit(&self) -> StorageQuotaReservation {
        self.storage_service.reserve_quota_commit(&self.locator)
    }

    pub fn quota_and_non_cache_usage(&self) -> Result<(u64, u64)> {
        let (quota, _) = self.quota_and_cache_usage()?;
        let indexed_db = self.indexed_db_usage()?;
        let opfs = self
            .storage_service
            .opfs_quota_usage(&self.locator)
            .map_err(|error| anyhow!(error))?;
        Ok((quota, indexed_db.saturating_add(opfs)))
    }

    pub fn quota_and_non_indexed_db_usage(&self) -> Result<(u64, u64)> {
        let (quota, cache) = self.quota_and_cache_usage()?;
        let opfs = self
            .storage_service
            .opfs_quota_usage(&self.locator)
            .map_err(|error| anyhow!(error))?;
        Ok((quota, cache.saturating_add(opfs)))
    }

    pub fn max_opfs_usage(&self) -> Result<u64> {
        let (quota, cache) = self.quota_and_cache_usage()?;
        let indexed_db = self.indexed_db_usage()?;
        Ok(quota.saturating_sub(cache.saturating_add(indexed_db)))
    }

    pub fn usage_snapshot(&self) -> Result<StorageBucketUsageSnapshot> {
        let (quota, cache_storage) = self.quota_and_cache_usage()?;
        let indexed_db = self.indexed_db_usage()?;
        let opfs = self
            .storage_service
            .opfs_usage(&self.locator)
            .map_err(|error| anyhow!(error))?;
        Ok(StorageBucketUsageSnapshot {
            quota,
            indexed_db,
            cache_storage,
            opfs,
        })
    }

    fn quota_and_cache_usage(&self) -> Result<(u64, u64)> {
        self.store
            .lock()
            .bucket_quota_and_cache_usage(&self.locator)
            .ok_or_else(|| anyhow!("StorageBucket quota owner is no longer live"))
    }

    fn indexed_db_usage(&self) -> Result<u64> {
        let Some(indexed_db_manager) = &self.indexed_db_manager else {
            return Ok(0);
        };
        let indexed_db_storage_key = match &self.locator {
            StorageBucketLocator::Default { storage_key } => storage_key.clone(),
            StorageBucketLocator::Named {
                storage_key,
                bucket_id,
            } => storage_bucket_indexed_db_storage_key(storage_key, *bucket_id),
        };
        indexed_db_manager
            .lock()
            .origin_usage_bytes(&indexed_db_storage_key)
            .map_err(|error| anyhow!(error))
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageBucketsJson {
    version: u32,
    #[serde(default = "default_next_bucket_id")]
    next_bucket_id: u64,
    #[serde(default)]
    indexed_db_keys_use_bucket_ids: bool,
    #[serde(default)]
    cache_paths_use_bucket_ids: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pending_deletions: Vec<StorageBucketIdentity>,
    origins: BTreeMap<String, BTreeMap<String, StorageBucketMetadata>>,
}

const fn default_next_bucket_id() -> u64 {
    1
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageBucketsV1Json {
    version: u32,
    origins: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageBucketCacheJson {
    version: u32,
    entries: BTreeMap<String, StorageBucketCacheJsonEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageBucketCacheJsonEntry {
    usage_bytes: u64,
    #[serde(
        default = "default_cache_request_method",
        skip_serializing_if = "is_default_cache_request_method"
    )]
    request_method: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    request_headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "is_zero")]
    insertion_order: u64,
    #[serde(
        default = "default_cached_response_type",
        skip_serializing_if = "is_default_cached_response_type"
    )]
    response_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    url: String,
    #[serde(default, skip_serializing_if = "is_false")]
    redirected: bool,
    status: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body_base64: String,
}

fn default_cache_request_method() -> String {
    "GET".to_owned()
}

fn is_default_cache_request_method(value: &String) -> bool {
    value == "GET"
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn default_cached_response_type() -> String {
    "default".to_owned()
}

fn is_default_cached_response_type(value: &String) -> bool {
    value == "default"
}

pub fn new_shared_storage_bucket_store() -> SharedStorageBucketStore {
    Arc::new(Mutex::new(StorageBucketRegistry::default()))
}

/// Create an ephemeral registry bound to the partition's authoritative
/// IndexedDB manager.
pub fn new_shared_storage_bucket_store_with_indexed_db_manager(
    indexed_db_manager: &SharedStorageBucketIndexedDbManager,
) -> SharedStorageBucketStore {
    new_shared_storage_bucket_store_with_storage_service_and_indexed_db_manager(
        StorageService::in_memory(),
        indexed_db_manager,
    )
}

pub fn new_shared_storage_bucket_store_with_storage_service(
    storage_service: SharedStorageService,
) -> SharedStorageBucketStore {
    Arc::new(Mutex::new(StorageBucketRegistry {
        backend: StorageBucketBackend::Memory(MemoryStorageBucketBackend::default()),
        storage_service,
        indexed_db_manager: None,
    }))
}

pub fn new_shared_storage_bucket_store_with_storage_service_and_indexed_db_manager(
    storage_service: SharedStorageService,
    indexed_db_manager: &SharedStorageBucketIndexedDbManager,
) -> SharedStorageBucketStore {
    Arc::new(Mutex::new(StorageBucketRegistry {
        backend: StorageBucketBackend::Memory(MemoryStorageBucketBackend::default()),
        storage_service,
        indexed_db_manager: Some(indexed_db_manager.clone()),
    }))
}

pub fn new_shared_json_storage_bucket_store(
    path: impl AsRef<Path>,
) -> Result<SharedStorageBucketStore> {
    let storage_service = StorageService::in_memory();
    let backend = JsonStorageBucketBackend::open(path.as_ref(), None, None, None)?;
    Ok(Arc::new(Mutex::new(StorageBucketRegistry {
        backend: StorageBucketBackend::Json(backend),
        storage_service,
        indexed_db_manager: None,
    })))
}

pub fn new_shared_json_storage_bucket_store_with_cache_root(
    path: impl AsRef<Path>,
    cache_storage_root: impl AsRef<Path>,
) -> Result<SharedStorageBucketStore> {
    let storage_service = StorageService::in_memory();
    let backend = JsonStorageBucketBackend::open(
        path.as_ref(),
        Some(cache_storage_root.as_ref()),
        None,
        None,
    )?;
    Ok(Arc::new(Mutex::new(StorageBucketRegistry {
        backend: StorageBucketBackend::Json(backend),
        storage_service,
        indexed_db_manager: None,
    })))
}

pub fn new_shared_json_storage_bucket_store_with_cache_root_and_indexed_db_manager(
    path: impl AsRef<Path>,
    cache_storage_root: impl AsRef<Path>,
    indexed_db_manager: &SharedStorageBucketIndexedDbManager,
) -> Result<SharedStorageBucketStore> {
    let storage_service = StorageService::in_memory();
    let backend = JsonStorageBucketBackend::open(
        path.as_ref(),
        Some(cache_storage_root.as_ref()),
        Some(indexed_db_manager),
        None,
    )?;
    Ok(Arc::new(Mutex::new(StorageBucketRegistry {
        backend: StorageBucketBackend::Json(backend),
        storage_service,
        indexed_db_manager: Some(indexed_db_manager.clone()),
    })))
}

pub fn new_shared_json_storage_bucket_store_with_storage_service(
    path: impl AsRef<Path>,
    cache_storage_root: impl AsRef<Path>,
    indexed_db_manager: &SharedStorageBucketIndexedDbManager,
    storage_service: SharedStorageService,
) -> Result<SharedStorageBucketStore> {
    let backend = JsonStorageBucketBackend::open(
        path.as_ref(),
        Some(cache_storage_root.as_ref()),
        Some(indexed_db_manager),
        Some(&storage_service),
    )?;
    Ok(Arc::new(Mutex::new(StorageBucketRegistry {
        backend: StorageBucketBackend::Json(backend),
        storage_service,
        indexed_db_manager: Some(indexed_db_manager.clone()),
    })))
}

pub fn storage_bucket_indexed_db_storage_key(
    storage_key: &str,
    bucket_id: StorageBucketId,
) -> String {
    format!(
        "bucket:v2:{}:{}",
        sha256_hex(storage_key.as_bytes()),
        bucket_id.get()
    )
}

fn legacy_storage_bucket_indexed_db_storage_key(storage_key: &str, bucket_name: &str) -> String {
    format!(
        "bucket:v1:{}:{}",
        sha256_hex(storage_key.as_bytes()),
        sha256_hex(bucket_name.as_bytes())
    )
}

pub fn storage_bucket_origin_allows_storage(origin: &str) -> bool {
    !moli_storage_key::serialized_storage_key_has_opaque_origin(origin)
}

pub fn storage_bucket_quota_owner(
    store: &SharedStorageBucketStore,
    locator: &StorageBucketLocator,
) -> Option<StorageBucketQuotaOwner> {
    let (storage_service, indexed_db_manager) = {
        let store = store.lock();
        store.bucket_quota_and_cache_usage(locator)?;
        (store.storage_service(), store.indexed_db_manager.clone())
    };
    Some(StorageBucketQuotaOwner {
        locator: locator.clone(),
        store: store.clone(),
        indexed_db_manager,
        storage_service,
    })
}

/// Clear every backend owned by one durable named-bucket tombstone, then
/// remove that exact tombstone.
///
/// The registry mutex is never held while OPFS or IndexedDB performs IO. A
/// repeated call is idempotent: once the tombstone has been finished, the old
/// identity cannot clear a same-name replacement bucket because bucket IDs are
/// never reused.
pub fn complete_storage_bucket_deletion(
    store: &SharedStorageBucketStore,
    identity: &StorageBucketIdentity,
) -> Result<bool> {
    let (storage_service, indexed_db_manager) = {
        let store = store.lock();
        if !store
            .memory()
            .pending_deletions
            .iter()
            .any(|pending| pending == identity)
        {
            return Ok(false);
        }
        (store.storage_service(), store.indexed_db_manager.clone())
    };

    clear_storage_bucket_backends(&storage_service, indexed_db_manager.as_ref(), identity)?;
    store.lock().finish_bucket_deletion(identity)
}

fn clear_storage_bucket_backends(
    storage_service: &SharedStorageService,
    indexed_db_manager: Option<&SharedStorageBucketIndexedDbManager>,
    identity: &StorageBucketIdentity,
) -> Result<()> {
    storage_service
        .clear_opfs_bucket(&identity.locator())
        .map_err(|error| anyhow!(error))?;
    if let Some(indexed_db_manager) = indexed_db_manager {
        indexed_db_manager
            .lock()
            .clear_origin(&identity.indexed_db_storage_key())
            .map_err(|error| anyhow!(error))?;
    }
    Ok(())
}

impl Default for StorageBucketRegistry {
    fn default() -> Self {
        Self {
            backend: StorageBucketBackend::Memory(MemoryStorageBucketBackend::default()),
            storage_service: StorageService::in_memory(),
            indexed_db_manager: None,
        }
    }
}

impl StorageBucketRegistry {
    fn metadata_for_identity(
        &self,
        identity: &StorageBucketIdentity,
    ) -> Option<&StorageBucketMetadata> {
        self.memory()
            .origins
            .get(identity.storage_key())
            .and_then(|buckets| buckets.get(identity.name()))
            .filter(|metadata| metadata.bucket_id == Some(identity.bucket_id()))
    }

    fn metadata_for_identity_mut(
        &mut self,
        identity: &StorageBucketIdentity,
    ) -> Option<&mut StorageBucketMetadata> {
        self.memory_mut()
            .origins
            .get_mut(identity.storage_key())
            .and_then(|buckets| buckets.get_mut(identity.name()))
            .filter(|metadata| metadata.bucket_id == Some(identity.bucket_id()))
    }

    pub fn storage_service(&self) -> SharedStorageService {
        self.storage_service.clone()
    }

    pub fn bucket_locator_is_live(&self, locator: &StorageBucketLocator) -> bool {
        match locator {
            StorageBucketLocator::Default { .. } => true,
            StorageBucketLocator::Named {
                storage_key,
                bucket_id,
            } => self
                .memory()
                .origins
                .get(storage_key)
                .is_some_and(|buckets| {
                    buckets
                        .values()
                        .any(|metadata| metadata.bucket_id == Some(*bucket_id))
                }),
        }
    }

    pub fn opfs_quota_for_locator(&self, locator: &StorageBucketLocator) -> Option<u64> {
        match locator {
            StorageBucketLocator::Default { .. } => Some(DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES),
            StorageBucketLocator::Named {
                storage_key,
                bucket_id,
            } => self
                .memory()
                .origins
                .get(storage_key)
                .and_then(|buckets| {
                    buckets
                        .values()
                        .find(|metadata| metadata.bucket_id == Some(*bucket_id))
                })
                .map(|metadata| metadata.quota.unwrap_or(DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES)),
        }
    }

    fn bucket_quota_and_cache_usage(&self, locator: &StorageBucketLocator) -> Option<(u64, u64)> {
        match locator {
            StorageBucketLocator::Default { storage_key } => {
                let cache_usage = self
                    .memory()
                    .origins
                    .get(storage_key)
                    .and_then(|buckets| buckets.get(IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME))
                    .map(bucket_cache_storage_usage)
                    .unwrap_or(0);
                Some((DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES, cache_usage))
            }
            StorageBucketLocator::Named {
                storage_key,
                bucket_id,
            } => self
                .memory()
                .origins
                .get(storage_key)
                .and_then(|buckets| {
                    buckets
                        .values()
                        .find(|metadata| metadata.bucket_id == Some(*bucket_id))
                })
                .map(|metadata| {
                    (
                        metadata.quota.unwrap_or(DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES),
                        bucket_cache_storage_usage(metadata),
                    )
                }),
        }
    }

    pub fn open_bucket(&mut self, origin: &str, name: &str) -> Result<StorageBucketIdentity> {
        self.open_bucket_with_expires(origin, name, None)
    }

    pub fn open_bucket_with_expires(
        &mut self,
        origin: &str,
        name: &str,
        expires: Option<f64>,
    ) -> Result<StorageBucketIdentity> {
        self.open_bucket_with_options(origin, name, expires, None, None, None)
    }

    pub fn open_bucket_with_options(
        &mut self,
        origin: &str,
        name: &str,
        expires: Option<f64>,
        durability: Option<StorageBucketDurability>,
        quota: Option<u64>,
        persisted: Option<bool>,
    ) -> Result<StorageBucketIdentity> {
        if self
            .memory()
            .pending_deletions
            .iter()
            .any(|identity| identity.storage_key == origin && identity.name == name)
        {
            bail!("storage bucket `{name}` deletion is still pending");
        }
        let memory = self.memory_mut();
        let bucket_id = match memory
            .origins
            .get(origin)
            .and_then(|buckets| buckets.get(name))
            .and_then(|metadata| metadata.bucket_id)
        {
            Some(bucket_id) => bucket_id,
            None => memory.allocate_bucket_id()?,
        };
        let metadata = memory
            .origins
            .entry(origin.to_owned())
            .or_default()
            .entry(name.to_owned())
            .or_default();
        metadata.bucket_id.get_or_insert(bucket_id);
        if let Some(expires) = expires {
            metadata.expires = Some(expires);
        }
        if let Some(durability) = durability {
            metadata.durability = durability;
        }
        if let Some(quota) = quota {
            metadata.quota = Some(quota);
        }
        if let Some(persisted) = persisted {
            metadata.persisted = persisted;
        }
        let identity = StorageBucketIdentity::new(origin, name, bucket_id);
        self.flush()?;
        Ok(identity)
    }

    pub fn keys(&self, origin: &str) -> Vec<String> {
        self.memory()
            .origins
            .get(origin)
            .map(|names| {
                names
                    .keys()
                    .filter(|name| name.as_str() != IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn bucket_id(&self, origin: &str, name: &str) -> Option<StorageBucketId> {
        self.memory()
            .origins
            .get(origin)
            .and_then(|buckets| buckets.get(name))
            .and_then(|metadata| metadata.bucket_id)
    }

    pub fn bucket_identity(&self, storage_key: &str, name: &str) -> Option<StorageBucketIdentity> {
        self.bucket_id(storage_key, name)
            .map(|bucket_id| StorageBucketIdentity::new(storage_key, name, bucket_id))
    }

    pub fn bucket_locator(&self, storage_key: &str, name: &str) -> Option<StorageBucketLocator> {
        self.bucket_id(storage_key, name)
            .map(|bucket_id| StorageBucketLocator::named(storage_key, bucket_id))
    }

    pub fn bucket_locator_for_identity(
        &self,
        identity: &StorageBucketIdentity,
    ) -> Option<StorageBucketLocator> {
        self.metadata_for_identity(identity)
            .map(|_| identity.locator())
    }

    pub fn keys_for_origin_areas(&self, origin: &str) -> Vec<(String, Vec<String>)> {
        let prefix = storage_key_prefix_for_origin(origin);
        self.memory()
            .origins
            .iter()
            .filter(|(storage_key, _)| storage_key.starts_with(&prefix))
            .map(|(storage_key, buckets)| {
                (
                    storage_key.clone(),
                    buckets
                        .keys()
                        .filter(|name| name.as_str() != IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME)
                        .cloned()
                        .collect::<Vec<_>>(),
                )
            })
            .collect()
    }

    pub fn bucket_identities_for_origin_areas(&self, origin: &str) -> Vec<StorageBucketIdentity> {
        let prefix = storage_key_prefix_for_origin(origin);
        self.memory()
            .origins
            .iter()
            .filter(|(storage_key, _)| storage_key.starts_with(&prefix))
            .flat_map(|(storage_key, buckets)| {
                buckets.iter().filter_map(|(name, metadata)| {
                    (name.as_str() != IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME)
                        .then_some(metadata.bucket_id)
                        .flatten()
                        .map(|bucket_id| StorageBucketIdentity::new(storage_key, name, bucket_id))
                })
            })
            .collect()
    }

    pub fn bucket_identities(&self, storage_key: &str) -> Vec<StorageBucketIdentity> {
        self.memory()
            .origins
            .get(storage_key)
            .into_iter()
            .flat_map(|buckets| buckets.iter())
            .filter_map(|(name, metadata)| {
                (name.as_str() != IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME)
                    .then_some(metadata.bucket_id)
                    .flatten()
                    .map(|bucket_id| StorageBucketIdentity::new(storage_key, name, bucket_id))
            })
            .collect()
    }

    pub fn delete_bucket_if_expired(
        &mut self,
        origin: &str,
        name: &str,
        now_ms: f64,
    ) -> Result<Option<StorageBucketIdentity>> {
        if let Some(pending) = self.pending_deletion(origin, name) {
            return Ok(Some(pending));
        }
        let expired = self
            .memory()
            .origins
            .get(origin)
            .and_then(|buckets| buckets.get(name))
            .is_some_and(|metadata| storage_bucket_metadata_expired(metadata, now_ms));
        if expired {
            return self.delete_bucket(origin, name);
        }
        Ok(None)
    }

    pub fn delete_expired_buckets(
        &mut self,
        origin: &str,
        now_ms: f64,
    ) -> Result<Vec<StorageBucketIdentity>> {
        let expired: Vec<String> = self
            .memory()
            .origins
            .get(origin)
            .map(|buckets| {
                buckets
                    .iter()
                    .filter(|(_, metadata)| storage_bucket_metadata_expired(metadata, now_ms))
                    .map(|(name, _)| name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut cleanups: Vec<StorageBucketIdentity> = self
            .memory()
            .pending_deletions
            .iter()
            .filter(|identity| identity.storage_key == origin)
            .cloned()
            .collect();
        if expired.is_empty() {
            return Ok(cleanups);
        }
        let memory = self.memory_mut();
        let mut newly_revoked = Vec::new();
        let mut remove_origin = false;
        if let Some(buckets) = memory.origins.get_mut(origin) {
            for name in &expired {
                let Some(metadata) = buckets.remove(name) else {
                    continue;
                };
                let bucket_id = metadata
                    .bucket_id
                    .context("expired storage bucket is missing its persistent ID")?;
                newly_revoked.push(StorageBucketIdentity::new(origin, name, bucket_id));
            }
            remove_origin = buckets.is_empty();
        }
        if remove_origin {
            memory.origins.remove(origin);
        }
        memory
            .pending_deletions
            .extend(newly_revoked.iter().cloned());
        cleanups.extend(newly_revoked);
        self.flush()?;
        Ok(cleanups)
    }

    pub fn delete_bucket(
        &mut self,
        origin: &str,
        name: &str,
    ) -> Result<Option<StorageBucketIdentity>> {
        if let Some(pending) = self.pending_deletion(origin, name) {
            return Ok(Some(pending));
        }
        let memory = self.memory_mut();
        let removed = if let Some(buckets) = memory.origins.get_mut(origin) {
            let removed = buckets.remove(name);
            if buckets.is_empty() {
                memory.origins.remove(origin);
            }
            removed
        } else {
            None
        };
        let Some(metadata) = removed else {
            return Ok(None);
        };
        let bucket_id = metadata
            .bucket_id
            .context("deleted storage bucket is missing its persistent ID")?;
        let identity = StorageBucketIdentity::new(origin, name, bucket_id);
        memory.pending_deletions.push(identity.clone());
        self.flush()?;
        #[cfg(test)]
        fault_injection::crash_if_armed(fault_injection::CrashPoint::BucketTombstoneDurable);
        Ok(Some(identity))
    }

    pub fn clear_origin(&mut self, origin: &str) -> Result<Vec<StorageBucketIdentity>> {
        let mut cleanups: Vec<StorageBucketIdentity> = self
            .memory()
            .pending_deletions
            .iter()
            .filter(|identity| identity.storage_key == origin)
            .cloned()
            .collect();
        let removed = self.memory_mut().origins.remove(origin).unwrap_or_default();
        let newly_revoked = storage_bucket_identities_from_metadata(origin, removed)?;
        if newly_revoked.is_empty() {
            return Ok(cleanups);
        }
        self.memory_mut()
            .pending_deletions
            .extend(newly_revoked.iter().cloned());
        cleanups.extend(newly_revoked);
        self.flush()?;
        Ok(cleanups)
    }

    pub fn clear_origin_areas(&mut self, origin: &str) -> Result<Vec<StorageBucketIdentity>> {
        let prefix = storage_key_prefix_for_origin(origin);
        let mut cleanups: Vec<StorageBucketIdentity> = self
            .memory()
            .pending_deletions
            .iter()
            .filter(|identity| identity.storage_key.starts_with(&prefix))
            .cloned()
            .collect();
        let storage_keys: Vec<String> = self
            .memory()
            .origins
            .keys()
            .filter(|storage_key| storage_key.starts_with(&prefix))
            .cloned()
            .collect();
        let mut newly_revoked = Vec::new();
        for storage_key in storage_keys {
            let removed = self
                .memory_mut()
                .origins
                .remove(&storage_key)
                .unwrap_or_default();
            newly_revoked.extend(storage_bucket_identities_from_metadata(
                &storage_key,
                removed,
            )?);
        }
        if newly_revoked.is_empty() {
            return Ok(cleanups);
        }
        self.memory_mut()
            .pending_deletions
            .extend(newly_revoked.iter().cloned());
        cleanups.extend(newly_revoked);
        self.flush()?;
        Ok(cleanups)
    }

    pub fn finish_bucket_deletion(&mut self, identity: &StorageBucketIdentity) -> Result<bool> {
        #[cfg(test)]
        fault_injection::crash_if_armed(fault_injection::CrashPoint::BucketCleanupComplete);
        let pending_deletions = &mut self.memory_mut().pending_deletions;
        let before = pending_deletions.len();
        pending_deletions.retain(|pending| pending != identity);
        if pending_deletions.len() == before {
            return Ok(false);
        }
        if let Err(error) = self.flush() {
            self.memory_mut().pending_deletions.push(identity.clone());
            return Err(error);
        }
        #[cfg(test)]
        fault_injection::crash_if_armed(fault_injection::CrashPoint::BucketTombstoneRemovedDurable);
        Ok(true)
    }

    pub fn pending_deletions(&self) -> Vec<StorageBucketIdentity> {
        self.memory().pending_deletions.clone()
    }

    fn pending_deletion(&self, origin: &str, name: &str) -> Option<StorageBucketIdentity> {
        self.memory()
            .pending_deletions
            .iter()
            .find(|identity| identity.storage_key == origin && identity.name == name)
            .cloned()
    }

    pub fn bucket_expires(&self, origin: &str, name: &str) -> Option<f64> {
        self.memory()
            .origins
            .get(origin)
            .and_then(|buckets| buckets.get(name))
            .and_then(|metadata| metadata.expires)
    }

    pub fn bucket_durability(&self, origin: &str, name: &str) -> Option<StorageBucketDurability> {
        self.memory()
            .origins
            .get(origin)
            .and_then(|buckets| buckets.get(name))
            .map(|metadata| metadata.durability)
    }

    pub fn bucket_quota(&self, origin: &str, name: &str) -> Option<Option<u64>> {
        self.memory()
            .origins
            .get(origin)
            .and_then(|buckets| buckets.get(name))
            .map(|metadata| metadata.quota)
    }

    pub fn bucket_persisted(&self, origin: &str, name: &str) -> Option<bool> {
        self.memory()
            .origins
            .get(origin)
            .and_then(|buckets| buckets.get(name))
            .map(|metadata| metadata.persisted)
    }

    pub fn bucket_identity_is_live(&self, identity: &StorageBucketIdentity) -> bool {
        self.metadata_for_identity(identity).is_some()
    }

    pub fn bucket_expires_for_identity(
        &self,
        identity: &StorageBucketIdentity,
    ) -> Option<Option<f64>> {
        self.metadata_for_identity(identity)
            .map(|metadata| metadata.expires)
    }

    pub fn bucket_durability_for_identity(
        &self,
        identity: &StorageBucketIdentity,
    ) -> Option<StorageBucketDurability> {
        self.metadata_for_identity(identity)
            .map(|metadata| metadata.durability)
    }

    pub fn bucket_quota_for_identity(
        &self,
        identity: &StorageBucketIdentity,
    ) -> Option<Option<u64>> {
        self.metadata_for_identity(identity)
            .map(|metadata| metadata.quota)
    }

    pub fn open_cache_for_identity(
        &mut self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
    ) -> Result<bool> {
        let opened = {
            let Some(metadata) = self.metadata_for_identity_mut(identity) else {
                return Ok(false);
            };
            metadata
                .cache_storage
                .entry(cache_name.to_owned())
                .or_default();
            true
        };
        self.flush()?;
        Ok(opened)
    }

    pub fn open_cache_handle_for_identity(
        &mut self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
    ) -> Result<Option<StorageBucketCacheId>> {
        let cache_id = {
            let Some(metadata) = self.metadata_for_identity_mut(identity) else {
                return Ok(None);
            };
            metadata
                .cache_storage
                .entry(cache_name.to_owned())
                .or_default();
            let cache_id = match metadata.cache_instance_ids.get(cache_name).copied() {
                Some(cache_id) => cache_id,
                None => {
                    metadata.next_cache_instance_id =
                        metadata.next_cache_instance_id.saturating_add(1).max(1);
                    let cache_id = StorageBucketCacheId(metadata.next_cache_instance_id);
                    metadata
                        .cache_instance_ids
                        .insert(cache_name.to_owned(), cache_id);
                    cache_id
                }
            };
            let refs = metadata
                .cache_instance_ref_counts
                .entry(cache_id)
                .or_default();
            *refs = refs.saturating_add(1);
            cache_id
        };
        self.flush()?;
        Ok(Some(cache_id))
    }

    pub fn release_cache_handle_for_identity(
        &mut self,
        identity: &StorageBucketIdentity,
        cache_id: StorageBucketCacheId,
    ) {
        let Some(metadata) = self.metadata_for_identity_mut(identity) else {
            return;
        };
        let Some(refs) = metadata.cache_instance_ref_counts.get_mut(&cache_id) else {
            return;
        };
        *refs = refs.saturating_sub(1);
        if *refs != 0 {
            return;
        }
        metadata.cache_instance_ref_counts.remove(&cache_id);
        metadata.detached_cache_storage.remove(&cache_id);
    }

    pub fn cache_names_for_identity(
        &self,
        identity: &StorageBucketIdentity,
    ) -> Option<Vec<String>> {
        self.metadata_for_identity(identity)
            .map(|metadata| metadata.cache_storage.keys().cloned().collect())
    }

    pub fn delete_cache_for_identity(
        &mut self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
    ) -> Result<Option<bool>> {
        let deleted = self.metadata_for_identity_mut(identity).map(|metadata| {
            let Some(entries) = metadata.cache_storage.remove(cache_name) else {
                metadata.cache_instance_ids.remove(cache_name);
                return false;
            };
            let cache_id = metadata.cache_instance_ids.remove(cache_name);
            if let Some(cache_id) = cache_id
                && metadata
                    .cache_instance_ref_counts
                    .get(&cache_id)
                    .copied()
                    .unwrap_or(0)
                    != 0
            {
                metadata.detached_cache_storage.insert(cache_id, entries);
            }
            true
        });
        if deleted.is_some() {
            self.flush()?;
        }
        Ok(deleted)
    }

    pub fn put_cache_entry_for_identity(
        &mut self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
        request_key: &str,
        response: StorageBucketCachedResponse,
        usage_bytes: u64,
        non_cache_usage_bytes: u64,
    ) -> Result<StorageBucketCachePutOutcome> {
        self.put_cache_entry_with_request_for_identity(
            identity,
            cache_name,
            request_key,
            StorageBucketCachedRequest::default(),
            response,
            usage_bytes,
            non_cache_usage_bytes,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn put_cache_entry_with_request_for_identity(
        &mut self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
        request_key: &str,
        request: StorageBucketCachedRequest,
        response: StorageBucketCachedResponse,
        usage_bytes: u64,
        non_cache_usage_bytes: u64,
    ) -> Result<StorageBucketCachePutOutcome> {
        self.put_cache_entry_with_request_for_selector_and_identity(
            identity,
            StorageBucketCacheSelector::Named(cache_name),
            request_key,
            request,
            response,
            usage_bytes,
            non_cache_usage_bytes,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn put_cache_entry_with_request_for_handle_and_identity(
        &mut self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
        cache_id: StorageBucketCacheId,
        request_key: &str,
        request: StorageBucketCachedRequest,
        response: StorageBucketCachedResponse,
        usage_bytes: u64,
        non_cache_usage_bytes: u64,
    ) -> Result<StorageBucketCachePutOutcome> {
        self.put_cache_entry_with_request_for_selector_and_identity(
            identity,
            StorageBucketCacheSelector::Handle {
                cache_name,
                cache_id,
            },
            request_key,
            request,
            response,
            usage_bytes,
            non_cache_usage_bytes,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn put_cache_entry_with_request_for_selector_and_identity(
        &mut self,
        identity: &StorageBucketIdentity,
        cache_selector: StorageBucketCacheSelector<'_>,
        request_key: &str,
        request: StorageBucketCachedRequest,
        response: StorageBucketCachedResponse,
        usage_bytes: u64,
        non_cache_usage_bytes: u64,
    ) -> Result<StorageBucketCachePutOutcome> {
        let outcome = {
            let Some(metadata) = self.metadata_for_identity_mut(identity) else {
                return Ok(StorageBucketCachePutOutcome::Stale);
            };
            if let StorageBucketCacheSelector::Named(cache_name) = cache_selector {
                metadata
                    .cache_storage
                    .entry(cache_name.to_owned())
                    .or_default();
            }
            let current_cache_usage = bucket_cache_storage_usage(metadata);
            let normalized_request_key = cache_request_key_without_fragment(request_key);
            let Some(cache_entries) = cache_entries_for_selector(metadata, cache_selector) else {
                return Ok(StorageBucketCachePutOutcome::Stale);
            };
            let replaced_request_keys: Vec<_> = cache_entries
                .keys()
                .filter(|key| cache_request_key_without_fragment(key) == normalized_request_key)
                .cloned()
                .collect();
            let old_entry_usage = replaced_request_keys
                .iter()
                .filter_map(|key| cache_entries.get(key))
                .fold(0u64, |usage, entry| usage.saturating_add(entry.usage_bytes));
            let next_cache_usage = current_cache_usage
                .saturating_sub(old_entry_usage)
                .saturating_add(usage_bytes);
            let requested = non_cache_usage_bytes.saturating_add(next_cache_usage);
            let quota = metadata.quota.unwrap_or(DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES);
            if requested > quota {
                return Ok(StorageBucketCachePutOutcome::QuotaExceeded { quota, requested });
            }
            let insertion_order = cache_entries
                .values()
                .map(|entry| entry.insertion_order)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let entries = cache_entries_for_selector_mut(metadata, cache_selector)
                .expect("Cache selector was resolved before quota evaluation");
            for replaced_request_key in replaced_request_keys {
                entries.remove(&replaced_request_key);
            }
            entries.insert(
                request_key.to_owned(),
                StorageBucketCacheEntry {
                    usage_bytes,
                    request,
                    response,
                    insertion_order,
                },
            );
            StorageBucketCachePutOutcome::Stored
        };
        if matches!(outcome, StorageBucketCachePutOutcome::Stored) {
            self.flush()?;
        }
        Ok(outcome)
    }

    pub fn match_cache_entry_for_identity(
        &self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
        request_key: &str,
    ) -> Option<Option<StorageBucketCachedResponse>> {
        let query = StorageBucketCacheQuery {
            request_url: request_key.to_owned(),
            method: "GET".to_owned(),
            headers: Vec::new(),
            ignore_search: false,
            ignore_method: false,
            ignore_vary: false,
        };
        self.match_cache_entries_for_identity(identity, cache_name, &query)
            .map(|matches| matches.into_iter().next().map(|entry| entry.response))
    }

    pub fn match_cache_entries_for_identity(
        &self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
        query: &StorageBucketCacheQuery,
    ) -> Option<Vec<StorageBucketCacheMatch>> {
        self.match_cache_entries_for_selector_and_identity(
            identity,
            StorageBucketCacheSelector::Named(cache_name),
            query,
        )
    }

    pub fn match_cache_entries_for_handle_and_identity(
        &self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
        cache_id: StorageBucketCacheId,
        query: &StorageBucketCacheQuery,
    ) -> Option<Vec<StorageBucketCacheMatch>> {
        self.match_cache_entries_for_selector_and_identity(
            identity,
            StorageBucketCacheSelector::Handle {
                cache_name,
                cache_id,
            },
            query,
        )
    }

    fn match_cache_entries_for_selector_and_identity(
        &self,
        identity: &StorageBucketIdentity,
        cache_selector: StorageBucketCacheSelector<'_>,
        query: &StorageBucketCacheQuery,
    ) -> Option<Vec<StorageBucketCacheMatch>> {
        let metadata = self.metadata_for_identity(identity)?;
        let mut matches = cache_entries_for_selector(metadata, cache_selector)?
            .iter()
            .filter(|(request_url, entry)| cache_entry_matches_query(request_url, entry, query))
            .map(|(request_url, entry)| {
                (
                    entry.insertion_order,
                    StorageBucketCacheMatch {
                        request_url: request_url.clone(),
                        request: entry.request.clone(),
                        response: entry.response.clone(),
                    },
                )
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|(order, _)| *order);
        Some(matches.into_iter().map(|(_, entry)| entry).collect())
    }

    pub fn cache_entries_for_identity(
        &self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
    ) -> Option<Vec<StorageBucketCacheMatch>> {
        self.cache_entries_for_selector_and_identity(
            identity,
            StorageBucketCacheSelector::Named(cache_name),
        )
    }

    pub fn cache_entries_for_handle_and_identity(
        &self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
        cache_id: StorageBucketCacheId,
    ) -> Option<Vec<StorageBucketCacheMatch>> {
        self.cache_entries_for_selector_and_identity(
            identity,
            StorageBucketCacheSelector::Handle {
                cache_name,
                cache_id,
            },
        )
    }

    fn cache_entries_for_selector_and_identity(
        &self,
        identity: &StorageBucketIdentity,
        cache_selector: StorageBucketCacheSelector<'_>,
    ) -> Option<Vec<StorageBucketCacheMatch>> {
        let metadata = self.metadata_for_identity(identity)?;
        let mut entries = cache_entries_for_selector(metadata, cache_selector)?
            .iter()
            .map(|(request_url, entry)| {
                (
                    entry.insertion_order,
                    StorageBucketCacheMatch {
                        request_url: request_url.clone(),
                        request: entry.request.clone(),
                        response: entry.response.clone(),
                    },
                )
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|(order, _)| *order);
        Some(entries.into_iter().map(|(_, entry)| entry).collect())
    }

    pub fn cache_request_keys_for_identity(
        &self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
    ) -> Option<Vec<String>> {
        let metadata = self.metadata_for_identity(identity)?;
        let mut keys = metadata
            .cache_storage
            .get(cache_name)
            .into_iter()
            .flat_map(|entries| entries.iter())
            .map(|(key, entry)| (entry.insertion_order, key.clone()))
            .collect::<Vec<_>>();
        keys.sort_by_key(|(order, _)| *order);
        Some(keys.into_iter().map(|(_, key)| key).collect())
    }

    pub fn delete_cache_entry_for_identity(
        &mut self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
        request_key: &str,
    ) -> Result<Option<bool>> {
        let query = StorageBucketCacheQuery {
            request_url: request_key.to_owned(),
            method: "GET".to_owned(),
            headers: Vec::new(),
            ignore_search: false,
            ignore_method: false,
            ignore_vary: false,
        };
        self.delete_cache_entries_for_identity(identity, cache_name, &query)
    }

    pub fn delete_cache_entries_for_identity(
        &mut self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
        query: &StorageBucketCacheQuery,
    ) -> Result<Option<bool>> {
        self.delete_cache_entries_for_selector_and_identity(
            identity,
            StorageBucketCacheSelector::Named(cache_name),
            query,
        )
    }

    pub fn delete_cache_entries_for_handle_and_identity(
        &mut self,
        identity: &StorageBucketIdentity,
        cache_name: &str,
        cache_id: StorageBucketCacheId,
        query: &StorageBucketCacheQuery,
    ) -> Result<Option<bool>> {
        self.delete_cache_entries_for_selector_and_identity(
            identity,
            StorageBucketCacheSelector::Handle {
                cache_name,
                cache_id,
            },
            query,
        )
    }

    fn delete_cache_entries_for_selector_and_identity(
        &mut self,
        identity: &StorageBucketIdentity,
        cache_selector: StorageBucketCacheSelector<'_>,
        query: &StorageBucketCacheQuery,
    ) -> Result<Option<bool>> {
        let deleted = {
            let Some(metadata) = self.metadata_for_identity_mut(identity) else {
                return Ok(None);
            };
            let Some(entries) = cache_entries_for_selector_mut(metadata, cache_selector) else {
                return Ok(Some(false));
            };
            let matching_request_keys: Vec<_> = entries
                .iter()
                .filter(|(request_url, entry)| cache_entry_matches_query(request_url, entry, query))
                .map(|(request_url, _)| request_url.clone())
                .collect();
            let deleted = !matching_request_keys.is_empty();
            for matching_request_key in matching_request_keys {
                entries.remove(&matching_request_key);
            }
            deleted
        };
        if deleted {
            self.flush()?;
        }
        Ok(Some(deleted))
    }

    pub fn cache_usage_for_identity(&self, identity: &StorageBucketIdentity) -> Option<u64> {
        self.metadata_for_identity(identity)
            .map(bucket_cache_storage_usage)
    }

    pub fn cache_usage_for_origin(&self, origin: &str) -> u64 {
        self.memory()
            .origins
            .get(origin)
            .map(|buckets| {
                buckets
                    .values()
                    .map(bucket_cache_storage_usage)
                    .fold(0u64, |total, usage| total.saturating_add(usage))
            })
            .unwrap_or(0)
    }

    pub fn cache_usage_for_origin_areas(&self, origin: &str) -> u64 {
        let prefix = storage_key_prefix_for_origin(origin);
        self.memory()
            .origins
            .iter()
            .filter(|(storage_key, _)| storage_key.starts_with(&prefix))
            .flat_map(|(_, buckets)| buckets.values())
            .map(bucket_cache_storage_usage)
            .fold(0u64, |total, usage| total.saturating_add(usage))
    }

    pub fn bucket_persisted_for_identity(&self, identity: &StorageBucketIdentity) -> Option<bool> {
        self.metadata_for_identity(identity)
            .map(|metadata| metadata.persisted)
    }

    pub fn set_bucket_expires(&mut self, origin: &str, name: &str, expires: f64) -> Result<()> {
        let Some(metadata) = self
            .memory_mut()
            .origins
            .get_mut(origin)
            .and_then(|buckets| buckets.get_mut(name))
        else {
            bail!("Storage bucket `{name}` does not exist for origin `{origin}`");
        };
        metadata.expires = Some(expires);
        self.flush()
    }

    pub fn set_bucket_expires_for_identity(
        &mut self,
        identity: &StorageBucketIdentity,
        expires: Option<f64>,
    ) -> Result<bool> {
        let Some(metadata) = self.metadata_for_identity_mut(identity) else {
            return Ok(false);
        };
        metadata.expires = expires;
        self.flush()?;
        Ok(true)
    }

    fn memory(&self) -> &MemoryStorageBucketBackend {
        match &self.backend {
            StorageBucketBackend::Memory(memory) => memory,
            StorageBucketBackend::Json(json) => &json.memory,
        }
    }

    fn memory_mut(&mut self) -> &mut MemoryStorageBucketBackend {
        match &mut self.backend {
            StorageBucketBackend::Memory(memory) => memory,
            StorageBucketBackend::Json(json) => &mut json.memory,
        }
    }

    fn flush(&mut self) -> Result<()> {
        match &mut self.backend {
            StorageBucketBackend::Memory(_) => Ok(()),
            StorageBucketBackend::Json(json) => json.save(),
        }
    }
}

fn bucket_cache_storage_usage(metadata: &StorageBucketMetadata) -> u64 {
    metadata
        .cache_storage
        .values()
        .flat_map(|entries| entries.values())
        .fold(0u64, |total, entry| total.saturating_add(entry.usage_bytes))
}

fn cache_entries_for_selector<'a>(
    metadata: &'a StorageBucketMetadata,
    selector: StorageBucketCacheSelector<'_>,
) -> Option<&'a BTreeMap<String, StorageBucketCacheEntry>> {
    match selector {
        StorageBucketCacheSelector::Named(cache_name) => metadata.cache_storage.get(cache_name),
        StorageBucketCacheSelector::Handle {
            cache_name,
            cache_id,
        } => {
            if metadata.cache_instance_ids.get(cache_name) == Some(&cache_id) {
                metadata.cache_storage.get(cache_name)
            } else {
                metadata.detached_cache_storage.get(&cache_id)
            }
        }
    }
}

fn cache_entries_for_selector_mut<'a>(
    metadata: &'a mut StorageBucketMetadata,
    selector: StorageBucketCacheSelector<'_>,
) -> Option<&'a mut BTreeMap<String, StorageBucketCacheEntry>> {
    match selector {
        StorageBucketCacheSelector::Named(cache_name) => metadata.cache_storage.get_mut(cache_name),
        StorageBucketCacheSelector::Handle {
            cache_name,
            cache_id,
        } => {
            if metadata.cache_instance_ids.get(cache_name) == Some(&cache_id) {
                metadata.cache_storage.get_mut(cache_name)
            } else {
                metadata.detached_cache_storage.get_mut(&cache_id)
            }
        }
    }
}

fn cache_request_key_without_fragment(request_key: &str) -> &str {
    request_key
        .split_once('#')
        .map_or(request_key, |(without_fragment, _)| without_fragment)
}

fn cache_request_key_without_search_or_fragment(request_key: &str) -> &str {
    cache_request_key_without_fragment(request_key)
        .split_once('?')
        .map_or_else(
            || cache_request_key_without_fragment(request_key),
            |(without_search, _)| without_search,
        )
}

fn cache_entry_matches_query(
    stored_request_url: &str,
    entry: &StorageBucketCacheEntry,
    query: &StorageBucketCacheQuery,
) -> bool {
    if !query.ignore_method
        && (!query.method.eq_ignore_ascii_case("GET")
            || !entry.request.method.eq_ignore_ascii_case(&query.method))
    {
        return false;
    }
    let urls_match = if query.ignore_search {
        cache_request_key_without_search_or_fragment(stored_request_url)
            == cache_request_key_without_search_or_fragment(&query.request_url)
    } else {
        cache_request_key_without_fragment(stored_request_url)
            == cache_request_key_without_fragment(&query.request_url)
    };
    if !urls_match {
        return false;
    }
    query.ignore_vary
        || cached_response_vary_matches_request(
            &entry.response.headers,
            &entry.request.headers,
            &query.headers,
        )
}

fn cached_response_vary_matches_request(
    response_headers: &[(String, String)],
    stored_request_headers: &[(String, String)],
    query_request_headers: &[(String, String)],
) -> bool {
    let vary_names = response_headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("vary"))
        .flat_map(|(_, value)| value.split(','))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    if vary_names.contains(&"*") {
        return false;
    }
    vary_names.into_iter().all(|name| {
        cache_header_value(stored_request_headers, name)
            == cache_header_value(query_request_headers, name)
    })
}

fn cache_header_value(headers: &[(String, String)], target: &str) -> String {
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(target))
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn storage_bucket_metadata_expired(metadata: &StorageBucketMetadata, now_ms: f64) -> bool {
    metadata.expires.is_some_and(|expires| expires <= now_ms)
}

fn storage_bucket_current_unix_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1_000.0)
        .unwrap_or(0.0)
}

fn storage_bucket_cache_file_path(
    root: &Path,
    storage_key: &str,
    bucket_id: StorageBucketId,
    cache_name: &str,
) -> PathBuf {
    storage_bucket_cache_bucket_dir(root, storage_key, bucket_id).join(format!(
        "{}.json",
        encode_storage_bucket_cache_component(cache_name)
    ))
}

fn storage_bucket_cache_bucket_dir(
    root: &Path,
    storage_key: &str,
    bucket_id: StorageBucketId,
) -> PathBuf {
    root.join(encode_storage_bucket_cache_component(storage_key))
        .join(format!("bucket-{}", bucket_id.get()))
}

fn legacy_storage_bucket_cache_bucket_dir(
    root: &Path,
    storage_key: &str,
    bucket_name: &str,
) -> PathBuf {
    root.join(encode_storage_bucket_cache_component(storage_key))
        .join(encode_storage_bucket_cache_component(bucket_name))
}

fn storage_bucket_cache_replacement_paths(root: &Path) -> Result<(PathBuf, PathBuf)> {
    Ok((
        storage_bucket_cache_sibling_path(root, ".next")?,
        storage_bucket_cache_sibling_path(root, ".previous")?,
    ))
}

fn storage_bucket_cache_sibling_path(root: &Path, suffix: &str) -> Result<PathBuf> {
    let file_name = root.file_name().ok_or_else(|| {
        anyhow!(
            "StorageBucket CacheStorage root `{}` must have a file name",
            root.display()
        )
    })?;
    let mut sibling_name = OsString::from(".");
    sibling_name.push(file_name);
    sibling_name.push(suffix);
    Ok(root.with_file_name(sibling_name))
}

fn storage_bucket_cache_path_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to inspect StorageBucket CacheStorage path `{}`",
                path.display()
            )
        }),
    }
}

fn remove_storage_bucket_cache_path_if_exists(path: &Path, label: &str) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect {label} StorageBucket CacheStorage path `{}`",
                    path.display()
                )
            });
        }
    };
    if metadata.is_dir() {
        fs::remove_dir_all(path).with_context(|| {
            format!(
                "failed to remove {label} StorageBucket CacheStorage dir `{}`",
                path.display()
            )
        })?;
    } else {
        fs::remove_file(path).with_context(|| {
            format!(
                "failed to remove {label} StorageBucket CacheStorage file `{}`",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn sync_storage_bucket_cache_directory(path: &Path) -> Result<()> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .with_context(|| {
            format!(
                "failed to sync StorageBucket CacheStorage directory `{}`",
                path.display()
            )
        })
}

fn sync_storage_bucket_cache_parent(path: &Path) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        anyhow!(
            "StorageBucket CacheStorage path `{}` must have a parent",
            path.display()
        )
    })?;
    sync_storage_bucket_cache_directory(parent)
}

fn sync_storage_bucket_cache_tree(path: &Path) -> Result<()> {
    for entry in fs::read_dir(path).with_context(|| {
        format!(
            "failed to scan StorageBucket CacheStorage replacement `{}`",
            path.display()
        )
    })? {
        let entry = entry.with_context(|| {
            format!(
                "failed to read StorageBucket CacheStorage replacement entry in `{}`",
                path.display()
            )
        })?;
        let entry_path = entry.path();
        let file_type = entry.file_type().with_context(|| {
            format!(
                "failed to inspect StorageBucket CacheStorage replacement entry `{}`",
                entry_path.display()
            )
        })?;
        if file_type.is_dir() {
            sync_storage_bucket_cache_tree(&entry_path)?;
        } else if file_type.is_file() {
            fs::File::open(&entry_path)
                .and_then(|file| file.sync_all())
                .with_context(|| {
                    format!(
                        "failed to sync StorageBucket CacheStorage replacement file `{}`",
                        entry_path.display()
                    )
                })?;
        } else {
            bail!(
                "unsupported entry in StorageBucket CacheStorage replacement `{}`",
                entry_path.display()
            );
        }
    }
    sync_storage_bucket_cache_directory(path)
}

fn recover_storage_bucket_cache_root(root: &Path) -> Result<()> {
    let (next, previous) = storage_bucket_cache_replacement_paths(root)?;
    if storage_bucket_cache_path_exists(root)? {
        remove_storage_bucket_cache_path_if_exists(&next, "stale replacement")?;
        remove_storage_bucket_cache_path_if_exists(&previous, "stale previous")?;
        sync_storage_bucket_cache_parent(root)?;
        return Ok(());
    }
    if storage_bucket_cache_path_exists(&next)? {
        fs::rename(&next, root).with_context(|| {
            format!(
                "failed to promote StorageBucket CacheStorage replacement `{}` to `{}`",
                next.display(),
                root.display()
            )
        })?;
        remove_storage_bucket_cache_path_if_exists(&previous, "stale previous")?;
        sync_storage_bucket_cache_parent(root)?;
        return Ok(());
    }
    if storage_bucket_cache_path_exists(&previous)? {
        fs::rename(&previous, root).with_context(|| {
            format!(
                "failed to restore previous StorageBucket CacheStorage root `{}` to `{}`",
                previous.display(),
                root.display()
            )
        })?;
        sync_storage_bucket_cache_parent(root)?;
    }
    Ok(())
}

fn replace_storage_bucket_cache_root(root: &Path, next: &Path, previous: &Path) -> Result<()> {
    remove_storage_bucket_cache_path_if_exists(previous, "stale previous")?;
    sync_storage_bucket_cache_parent(root)?;
    if storage_bucket_cache_path_exists(root)? {
        fs::rename(root, previous).with_context(|| {
            format!(
                "failed to move current StorageBucket CacheStorage root `{}` to `{}`",
                root.display(),
                previous.display()
            )
        })?;
        sync_storage_bucket_cache_parent(root)?;
        #[cfg(test)]
        fault_injection::crash_if_armed(fault_injection::CrashPoint::CachePreviousDurable);
    }
    fs::rename(next, root).with_context(|| {
        format!(
            "failed to move replacement StorageBucket CacheStorage root `{}` to `{}`",
            next.display(),
            root.display()
        )
    })?;
    sync_storage_bucket_cache_parent(root)?;
    #[cfg(test)]
    fault_injection::crash_if_armed(fault_injection::CrashPoint::CacheCurrentDurable);
    remove_storage_bucket_cache_path_if_exists(previous, "previous")?;
    #[cfg(test)]
    fault_injection::crash_if_armed(fault_injection::CrashPoint::CachePreviousRemovedBeforeSync);
    sync_storage_bucket_cache_parent(root)?;
    Ok(())
}

fn encode_storage_bucket_cache_component(value: &str) -> String {
    percent_encoding::percent_encode(value.as_bytes(), percent_encoding::NON_ALPHANUMERIC)
        .to_string()
}

fn decode_storage_bucket_cache_component(value: &str) -> Result<String> {
    percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .map(|value| value.into_owned())
        .with_context(|| format!("failed to decode StorageBucket CacheStorage path `{value}`"))
}

fn load_storage_bucket_cache_file(
    path: &Path,
) -> Result<BTreeMap<String, StorageBucketCacheEntry>> {
    let bytes = fs::read(path).with_context(|| {
        format!(
            "failed to read StorageBucket CacheStorage file `{}`",
            path.display()
        )
    })?;
    let json: StorageBucketCacheJson = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "failed to parse StorageBucket CacheStorage file `{}`",
            path.display()
        )
    })?;
    if json.version != STORAGE_BUCKET_CACHE_JSON_VERSION {
        bail!(
            "unsupported StorageBucket CacheStorage version {} in `{}`; this Moli supports version {}",
            json.version,
            path.display(),
            STORAGE_BUCKET_CACHE_JSON_VERSION
        );
    }
    json.entries
        .into_iter()
        .enumerate()
        .map(|(index, (request_key, entry))| {
            let body = BASE64_STANDARD
                .decode(entry.body_base64.as_bytes())
                .with_context(|| {
                    format!(
                        "failed to decode StorageBucket CacheStorage body in `{}`",
                        path.display()
                    )
                })?;
            Ok((
                request_key,
                StorageBucketCacheEntry {
                    usage_bytes: entry.usage_bytes,
                    request: StorageBucketCachedRequest {
                        method: entry.request_method,
                        headers: entry.request_headers,
                    },
                    response: StorageBucketCachedResponse {
                        response_type: entry.response_type,
                        url: entry.url,
                        redirected: entry.redirected,
                        status: entry.status,
                        status_text: entry.status_text,
                        headers: entry.headers,
                        body,
                    },
                    insertion_order: if entry.insertion_order == 0 {
                        (index as u64).saturating_add(1)
                    } else {
                        entry.insertion_order
                    },
                },
            ))
        })
        .collect()
}

fn save_storage_bucket_cache_file(
    path: &Path,
    entries: &BTreeMap<String, StorageBucketCacheEntry>,
) -> Result<()> {
    let json = StorageBucketCacheJson {
        version: STORAGE_BUCKET_CACHE_JSON_VERSION,
        entries: entries
            .iter()
            .map(|(request_key, entry)| {
                (
                    request_key.clone(),
                    StorageBucketCacheJsonEntry {
                        usage_bytes: entry.usage_bytes,
                        request_method: entry.request.method.clone(),
                        request_headers: entry.request.headers.clone(),
                        insertion_order: entry.insertion_order,
                        response_type: entry.response.response_type.clone(),
                        url: entry.response.url.clone(),
                        redirected: entry.response.redirected,
                        status: entry.response.status,
                        status_text: entry.response.status_text.clone(),
                        headers: entry.response.headers.clone(),
                        body_base64: BASE64_STANDARD.encode(&entry.response.body),
                    },
                )
            })
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&json)
        .context("failed to serialize StorageBucket CacheStorage")?;
    moli_browser_profile::write_file_atomically(path, &bytes, "StorageBucket CacheStorage file")
}

impl JsonStorageBucketBackend {
    fn open(
        path: &Path,
        cache_storage_root: Option<&Path>,
        indexed_db_manager: Option<&SharedStorageBucketIndexedDbManager>,
        deletion_storage_service: Option<&SharedStorageService>,
    ) -> Result<Self> {
        let cache_storage_root = cache_storage_root.map(Path::to_path_buf);
        if !path.exists() {
            return Ok(Self {
                path: path.to_path_buf(),
                cache_storage_root,
                indexed_db_keys_use_bucket_ids: true,
                cache_paths_use_bucket_ids: true,
                memory: MemoryStorageBucketBackend::default(),
            });
        }
        let bytes = fs::read(path)
            .with_context(|| format!("failed to read storage bucket store `{}`", path.display()))?;
        let version = storage_buckets_json_version(&bytes, path)?;
        if version == 1 {
            let mut backend = Self::open_v1(path, cache_storage_root, &bytes)?;
            if let Some(manager) = indexed_db_manager {
                backend.migrate_legacy_indexed_db_storage_keys(manager)?;
                backend.indexed_db_keys_use_bucket_ids = true;
                backend.save()?;
            }
            return Ok(backend);
        }
        if !matches!(version, 2 | 3 | 4 | STORAGE_BUCKETS_JSON_VERSION) {
            bail!(
                "unsupported storage bucket store version {} in `{}`; this Moli supports version {}",
                version,
                path.display(),
                STORAGE_BUCKETS_JSON_VERSION
            );
        }
        let json: StorageBucketsJson = serde_json::from_slice(&bytes).with_context(|| {
            format!("failed to parse storage bucket store `{}`", path.display())
        })?;
        let mut memory = MemoryStorageBucketBackend {
            origins: json.origins,
            pending_deletions: json.pending_deletions,
            next_bucket_id: json.next_bucket_id,
        };
        let mut identity_migrated = memory.assign_missing_bucket_ids()?;
        let mut backend = Self {
            path: path.to_path_buf(),
            cache_storage_root,
            indexed_db_keys_use_bucket_ids: version >= 5 && json.indexed_db_keys_use_bucket_ids,
            cache_paths_use_bucket_ids: version >= 5 && json.cache_paths_use_bucket_ids,
            memory,
        };
        let cache_path_migration_pending = !backend.cache_paths_use_bucket_ids;
        backend.load_cache_storage()?;
        let implicit_default_migrated = if version <= 3 {
            backend.migrate_legacy_implicit_default_cache_storage()
        } else {
            false
        };
        if implicit_default_migrated {
            identity_migrated |= backend.memory.assign_missing_bucket_ids()?;
        }
        let mut indexed_db_identity_migrated = false;
        if !backend.indexed_db_keys_use_bucket_ids
            && let Some(manager) = indexed_db_manager
        {
            backend.migrate_legacy_indexed_db_storage_keys(manager)?;
            backend.indexed_db_keys_use_bucket_ids = true;
            indexed_db_identity_migrated = true;
        }
        let expired_buckets = backend.prune_expired_buckets(storage_bucket_current_unix_ms())?;
        let recovered_deletions =
            backend.recover_pending_deletions(indexed_db_manager, deletion_storage_service)?;
        if version != STORAGE_BUCKETS_JSON_VERSION
            || identity_migrated
            || implicit_default_migrated
            || indexed_db_identity_migrated
            || cache_path_migration_pending
            || expired_buckets > 0
            || recovered_deletions > 0
        {
            backend.save()?;
        }
        Ok(backend)
    }

    fn open_v1(path: &Path, cache_storage_root: Option<PathBuf>, bytes: &[u8]) -> Result<Self> {
        let json: StorageBucketsV1Json = serde_json::from_slice(bytes).with_context(|| {
            format!("failed to parse storage bucket store `{}`", path.display())
        })?;
        let mut memory = MemoryStorageBucketBackend::default();
        for (origin, names) in json.origins {
            memory.origins.insert(
                origin,
                names
                    .into_iter()
                    .map(|name| (name, StorageBucketMetadata::default()))
                    .collect(),
            );
        }
        memory.assign_missing_bucket_ids()?;
        let mut backend = Self {
            path: path.to_path_buf(),
            cache_storage_root,
            indexed_db_keys_use_bucket_ids: false,
            cache_paths_use_bucket_ids: false,
            memory,
        };
        backend.load_cache_storage()?;
        backend.migrate_legacy_implicit_default_cache_storage();
        backend.memory.assign_missing_bucket_ids()?;
        backend.save()?;
        Ok(backend)
    }

    fn save(&mut self) -> Result<()> {
        self.save_metadata()?;
        self.save_cache_storage()?;
        if !self.cache_paths_use_bucket_ids {
            self.cache_paths_use_bucket_ids = true;
            self.save_metadata()?;
        }
        Ok(())
    }

    fn save_metadata(&self) -> Result<()> {
        let json = StorageBucketsJson {
            version: STORAGE_BUCKETS_JSON_VERSION,
            next_bucket_id: self.memory.next_bucket_id,
            indexed_db_keys_use_bucket_ids: self.indexed_db_keys_use_bucket_ids,
            cache_paths_use_bucket_ids: self.cache_paths_use_bucket_ids,
            pending_deletions: self.memory.pending_deletions.clone(),
            origins: self.memory.origins.clone(),
        };
        let bytes =
            serde_json::to_vec_pretty(&json).context("failed to serialize storage bucket store")?;
        moli_browser_profile::write_file_atomically(&self.path, &bytes, "storage bucket store")
    }

    fn migrate_legacy_implicit_default_cache_storage(&mut self) -> bool {
        let mut migrated = false;
        for buckets in self.memory.origins.values_mut() {
            let legacy_caches = buckets
                .get_mut("default")
                .map(|metadata| std::mem::take(&mut metadata.cache_storage))
                .unwrap_or_default();
            if legacy_caches.is_empty() {
                continue;
            }
            let implicit = buckets
                .entry(IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME.to_owned())
                .or_default();
            for (cache_name, entries) in legacy_caches {
                implicit.cache_storage.entry(cache_name).or_insert(entries);
            }
            migrated = true;
        }
        migrated
    }

    fn load_cache_storage(&mut self) -> Result<()> {
        let Some(root) = self.cache_storage_root.as_ref() else {
            return Ok(());
        };
        recover_storage_bucket_cache_root(root)?;
        if !storage_bucket_cache_path_exists(root)? {
            return Ok(());
        }
        for (origin, buckets) in &mut self.memory.origins {
            for (bucket_name, metadata) in buckets {
                let bucket_id = metadata
                    .bucket_id
                    .context("StorageBucket CacheStorage bucket is missing its persistent ID")?;
                let id_bucket_dir = storage_bucket_cache_bucket_dir(root, origin, bucket_id);
                let bucket_dir = if self.cache_paths_use_bucket_ids {
                    id_bucket_dir
                } else {
                    let legacy_bucket_dir =
                        legacy_storage_bucket_cache_bucket_dir(root, origin, bucket_name);
                    if legacy_bucket_dir.exists() {
                        legacy_bucket_dir
                    } else {
                        id_bucket_dir
                    }
                };
                if !bucket_dir.exists() {
                    continue;
                }
                for entry in fs::read_dir(&bucket_dir).with_context(|| {
                    format!(
                        "failed to read StorageBucket CacheStorage dir `{}`",
                        bucket_dir.display()
                    )
                })? {
                    let entry = entry.with_context(|| {
                        format!(
                            "failed to read StorageBucket CacheStorage entry in `{}`",
                            bucket_dir.display()
                        )
                    })?;
                    let path = entry.path();
                    if !path.is_file()
                        || path.extension().and_then(|value| value.to_str()) != Some("json")
                    {
                        continue;
                    }
                    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
                        continue;
                    };
                    let cache_name = decode_storage_bucket_cache_component(stem)?;
                    let cache = load_storage_bucket_cache_file(&path)?;
                    metadata.cache_storage.insert(cache_name, cache);
                }
            }
        }
        Ok(())
    }

    fn save_cache_storage(&self) -> Result<()> {
        let Some(root) = self.cache_storage_root.as_ref() else {
            return Ok(());
        };
        recover_storage_bucket_cache_root(root)?;
        let (next, previous) = storage_bucket_cache_replacement_paths(root)?;
        remove_storage_bucket_cache_path_if_exists(&next, "stale replacement")?;
        fs::create_dir_all(&next).with_context(|| {
            format!(
                "failed to create StorageBucket CacheStorage replacement root `{}`",
                next.display()
            )
        })?;
        for (origin, buckets) in &self.memory.origins {
            for metadata in buckets.values() {
                let bucket_id = metadata
                    .bucket_id
                    .context("StorageBucket CacheStorage bucket is missing its persistent ID")?;
                for (cache_name, entries) in &metadata.cache_storage {
                    let path = storage_bucket_cache_file_path(&next, origin, bucket_id, cache_name);
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent).with_context(|| {
                            format!(
                                "failed to create StorageBucket CacheStorage dir `{}`",
                                parent.display()
                            )
                        })?;
                    }
                    save_storage_bucket_cache_file(&path, entries)?;
                }
            }
        }
        sync_storage_bucket_cache_tree(&next)?;
        sync_storage_bucket_cache_parent(&next)?;
        #[cfg(test)]
        fault_injection::crash_if_armed(fault_injection::CrashPoint::CacheNextDurable);
        replace_storage_bucket_cache_root(root, &next, &previous)?;
        Ok(())
    }

    fn migrate_legacy_indexed_db_storage_keys(
        &self,
        indexed_db_manager: &SharedStorageBucketIndexedDbManager,
    ) -> Result<()> {
        let identities: Vec<StorageBucketIdentity> = self
            .memory
            .origins
            .iter()
            .flat_map(|(storage_key, buckets)| {
                buckets.iter().filter_map(|(name, metadata)| {
                    metadata
                        .bucket_id
                        .map(|bucket_id| StorageBucketIdentity::new(storage_key, name, bucket_id))
                })
            })
            .chain(self.memory.pending_deletions.iter().cloned())
            .collect();
        let mut manager = indexed_db_manager.lock();
        for identity in identities {
            let legacy = legacy_storage_bucket_indexed_db_storage_key(
                identity.storage_key(),
                identity.name(),
            );
            manager
                .migrate_origin(&legacy, &identity.indexed_db_storage_key())
                .map_err(|error| anyhow!(error))?;
        }
        Ok(())
    }

    fn recover_pending_deletions(
        &mut self,
        indexed_db_manager: Option<&SharedStorageBucketIndexedDbManager>,
        storage_service: Option<&SharedStorageService>,
    ) -> Result<usize> {
        let (Some(indexed_db_manager), Some(storage_service)) =
            (indexed_db_manager, storage_service)
        else {
            return Ok(0);
        };
        let pending = self.memory.pending_deletions.clone();
        for identity in &pending {
            clear_storage_bucket_backends(storage_service, Some(indexed_db_manager), identity)?;
        }
        self.memory.pending_deletions.clear();
        Ok(pending.len())
    }

    fn prune_expired_buckets(&mut self, now_ms: f64) -> Result<usize> {
        let mut expired_buckets = Vec::new();
        let mut empty_origins = Vec::new();
        for (origin, buckets) in &mut self.memory.origins {
            let expired_names: Vec<String> = buckets
                .iter()
                .filter(|(_, metadata)| storage_bucket_metadata_expired(metadata, now_ms))
                .map(|(name, _)| name.clone())
                .collect();
            for name in expired_names {
                let metadata = buckets
                    .remove(&name)
                    .expect("collected expired storage bucket should still exist");
                let bucket_id = metadata
                    .bucket_id
                    .context("expired storage bucket is missing its persistent ID")?;
                expired_buckets.push(StorageBucketIdentity::new(origin, &name, bucket_id));
            }
            if buckets.is_empty() {
                empty_origins.push(origin.clone());
            }
        }
        for origin in empty_origins {
            self.memory.origins.remove(&origin);
        }
        let count = expired_buckets.len();
        self.memory.pending_deletions.extend(expired_buckets);
        Ok(count)
    }
}

fn storage_bucket_identities_from_metadata(
    storage_key: &str,
    buckets: BTreeMap<String, StorageBucketMetadata>,
) -> Result<Vec<StorageBucketIdentity>> {
    buckets
        .into_iter()
        .map(|(name, metadata)| {
            let bucket_id = metadata
                .bucket_id
                .context("revoked storage bucket is missing its persistent ID")?;
            Ok(StorageBucketIdentity::new(storage_key, &name, bucket_id))
        })
        .collect()
}

impl MemoryStorageBucketBackend {
    fn allocate_bucket_id(&mut self) -> Result<StorageBucketId> {
        let value = self.next_bucket_id.max(1);
        let next_bucket_id = value
            .checked_add(1)
            .context("storage bucket ID space is exhausted")?;
        let bucket_id = StorageBucketId::new(value)
            .context("storage bucket ID allocator produced the reserved zero ID")?;
        self.next_bucket_id = next_bucket_id;
        Ok(bucket_id)
    }

    fn assign_missing_bucket_ids(&mut self) -> Result<bool> {
        let mut next_bucket_id = self.next_bucket_id.max(1);
        let mut seen_bucket_ids = BTreeSet::new();
        let mut pending_slots = BTreeSet::new();
        for identity in &self.pending_deletions {
            if !pending_slots.insert((identity.storage_key.clone(), identity.name.clone())) {
                bail!(
                    "duplicate pending storage bucket deletion for `{}` / `{}`",
                    identity.storage_key,
                    identity.name
                );
            }
            if !seen_bucket_ids.insert(identity.bucket_id) {
                bail!(
                    "duplicate persistent storage bucket ID {}",
                    identity.bucket_id.get()
                );
            }
            let after_bucket_id = identity
                .bucket_id
                .get()
                .checked_add(1)
                .context("storage bucket ID space is exhausted")?;
            next_bucket_id = next_bucket_id.max(after_bucket_id);
        }
        for (storage_key, buckets) in &self.origins {
            for name in buckets.keys() {
                if pending_slots.contains(&(storage_key.clone(), name.clone())) {
                    bail!(
                        "storage bucket `{name}` is both live and pending deletion for `{storage_key}`"
                    );
                }
            }
        }
        for bucket_id in self
            .origins
            .values()
            .flat_map(|buckets| buckets.values())
            .filter_map(|metadata| metadata.bucket_id)
        {
            if !seen_bucket_ids.insert(bucket_id) {
                bail!("duplicate persistent storage bucket ID {}", bucket_id.get());
            }
            let after_bucket_id = bucket_id
                .get()
                .checked_add(1)
                .context("storage bucket ID space is exhausted")?;
            next_bucket_id = next_bucket_id.max(after_bucket_id);
        }

        let mut changed = self.next_bucket_id != next_bucket_id;
        for metadata in self
            .origins
            .values_mut()
            .flat_map(|buckets| buckets.values_mut())
        {
            if metadata.bucket_id.is_some() {
                continue;
            }
            let bucket_id = StorageBucketId::new(next_bucket_id)
                .context("storage bucket ID allocator produced the reserved zero ID")?;
            next_bucket_id = next_bucket_id
                .checked_add(1)
                .context("storage bucket ID space is exhausted")?;
            metadata.bucket_id = Some(bucket_id);
            changed = true;
        }
        if self.next_bucket_id != next_bucket_id {
            self.next_bucket_id = next_bucket_id;
            changed = true;
        }
        Ok(changed)
    }
}

fn storage_buckets_json_version(bytes: &[u8], path: &Path) -> Result<u32> {
    #[derive(Deserialize)]
    struct VersionOnly {
        version: u32,
    }

    let version: VersionOnly = serde_json::from_slice(bytes).with_context(|| {
        format!(
            "failed to parse storage bucket store version `{}`",
            path.display()
        )
    })?;
    Ok(version.version)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        panic::{AssertUnwindSafe, catch_unwind},
        path::PathBuf,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use anyhow::Result;
    use moli_indexeddb::{IndexedDbManager, Key, ObjectStoreOptions, OpenOptions, TransactionMode};
    use parking_lot::Mutex;

    use crate::{StorageBucketId, StorageBucketLocator, StorageService};

    use super::{
        DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES, IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME,
        SharedStorageBucketIndexedDbManager, StorageBucketCacheEntry, StorageBucketCachePutOutcome,
        StorageBucketCacheQuery, StorageBucketCachedRequest, StorageBucketCachedResponse,
        StorageBucketDurability, StorageBucketIdentity, StorageBucketRegistry,
        StorageBucketUsageSnapshot, complete_storage_bucket_deletion,
        encode_storage_bucket_cache_component,
        fault_injection::{CrashPoint, arm},
        legacy_storage_bucket_cache_bucket_dir, legacy_storage_bucket_indexed_db_storage_key,
        new_shared_json_storage_bucket_store, new_shared_json_storage_bucket_store_with_cache_root,
        new_shared_json_storage_bucket_store_with_storage_service, save_storage_bucket_cache_file,
        storage_bucket_cache_bucket_dir, storage_bucket_cache_replacement_paths,
        storage_bucket_indexed_db_storage_key, storage_bucket_origin_allows_storage,
        storage_bucket_quota_owner,
    };

    fn new_indexed_db_manager(
        root: Option<PathBuf>,
    ) -> std::result::Result<SharedStorageBucketIndexedDbManager, String> {
        let manager = match root {
            Some(path) => IndexedDbManager::new(path).map_err(|error| error.to_string())?,
            None => IndexedDbManager::new_in_memory(),
        };
        Ok(Arc::new(Mutex::new(manager)))
    }

    fn clear_indexed_db_origin(
        manager: &SharedStorageBucketIndexedDbManager,
        origin: &str,
    ) -> std::result::Result<(), String> {
        manager
            .lock()
            .clear_origin(origin)
            .map_err(|error| error.to_string())
    }

    fn indexed_db_origin_usage_bytes(
        manager: &SharedStorageBucketIndexedDbManager,
        origin: &str,
    ) -> std::result::Result<u64, String> {
        manager
            .lock()
            .origin_usage_bytes(origin)
            .map_err(|error| error.to_string())
    }

    struct TempStorePath {
        path: PathBuf,
    }

    impl TempStorePath {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "moli-storage-buckets-{name}-{}-{nonce}.json",
                std::process::id()
            ));
            Self { path }
        }

        fn cache_root(&self) -> PathBuf {
            self.path.with_extension("cache-storage")
        }

        fn indexed_db_root(&self) -> PathBuf {
            self.path.with_extension("indexed-db")
        }

        fn opfs_root(&self) -> PathBuf {
            self.path.with_extension("opfs")
        }
    }

    fn test_now_ms() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_secs_f64()
            * 1_000.0
    }

    #[test]
    fn implicit_default_bucket_is_hidden_and_distinct_from_named_default() -> Result<()> {
        let mut store = StorageBucketRegistry::default();
        let storage_key = "https://a.test";

        let implicit_identity =
            store.open_bucket(storage_key, IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME)?;
        let named_identity = store.open_bucket(storage_key, "default")?;

        assert_ne!(implicit_identity, named_identity);
        assert_eq!(store.keys(storage_key), vec!["default"]);
        assert_ne!(
            store.bucket_id(storage_key, IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME),
            store.bucket_id(storage_key, "default")
        );
        Ok(())
    }

    fn seed_indexed_db_record(
        manager: &SharedStorageBucketIndexedDbManager,
        origin: &str,
    ) -> Result<()> {
        let mut manager = manager.lock();
        let opened = manager.open(OpenOptions {
            origin: origin.to_owned(),
            name: "bucket-db".to_owned(),
            version: None,
        })?;
        let upgrade = opened
            .upgrade_transaction
            .expect("first open should create an upgrade transaction");
        manager.create_object_store(upgrade, "items", ObjectStoreOptions::default())?;
        manager.commit_transaction(upgrade)?;
        let tx = manager.begin_transaction(
            opened.database,
            &[String::from("items")],
            TransactionMode::ReadWrite,
        )?;
        manager.put(tx, "items", Some(Key::from("alpha")), b"record".to_vec())?;
        manager.commit_transaction(tx)?;
        manager.close_database(opened.database)?;
        Ok(())
    }

    fn indexed_db_usage(
        manager: &SharedStorageBucketIndexedDbManager,
        origin: &str,
    ) -> Result<u64> {
        indexed_db_origin_usage_bytes(manager, origin).map_err(anyhow::Error::msg)
    }

    impl Drop for TempStorePath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
            let cache_root = self.cache_root();
            let _ = fs::remove_dir_all(&cache_root);
            let _ = fs::remove_dir_all(self.indexed_db_root());
            let _ = fs::remove_dir_all(self.opfs_root());
            if let Ok((next, previous)) = storage_bucket_cache_replacement_paths(&cache_root) {
                let _ = fs::remove_dir_all(next);
                let _ = fs::remove_dir_all(previous);
            }
        }
    }

    #[test]
    fn storage_buckets_reject_opaque_serialized_storage_keys() {
        assert!(!storage_bucket_origin_allows_storage("null"));
        assert!(!storage_bucket_origin_allows_storage(
            "storage-key:v1;origin=null;top-level-site=https://app.example;opaque-nonce=7"
        ));
        assert!(storage_bucket_origin_allows_storage(
            "storage-key:v1;origin=https://cdn.example;top-level-site=https://app.example"
        ));
    }

    #[test]
    fn memory_storage_bucket_store_is_origin_partitioned_and_sorted() -> Result<()> {
        let mut store = StorageBucketRegistry::default();

        store.open_bucket("https://a.test", "b")?;
        store.open_bucket("https://a.test", "a")?;
        store.open_bucket("https://b.test", "c")?;

        assert_eq!(store.keys("https://a.test"), vec!["a", "b"]);
        assert_eq!(store.keys("https://b.test"), vec!["c"]);

        store.delete_bucket("https://a.test", "a")?;
        assert_eq!(store.keys("https://a.test"), vec!["b"]);
        Ok(())
    }

    #[test]
    fn renderer_neutral_quota_owner_aggregates_cache_indexed_db_and_opfs() -> Result<()> {
        let storage_service = StorageService::in_memory();
        let indexed_db_manager = new_indexed_db_manager(None).map_err(anyhow::Error::msg)?;
        let store =
            super::new_shared_storage_bucket_store_with_storage_service_and_indexed_db_manager(
                storage_service.clone(),
                &indexed_db_manager,
            );
        let storage_key = "storage-key:v1;origin=https://quota.test";
        let identity = store.lock().open_bucket_with_options(
            storage_key,
            "bucket",
            None,
            None,
            Some(DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES),
            None,
        )?;
        let locator = store
            .lock()
            .bucket_locator(storage_key, "bucket")
            .expect("opened bucket should have a locator");
        let cache_usage = 37;
        assert_eq!(
            store.lock().put_cache_entry_for_identity(
                &identity,
                "cache",
                "request",
                StorageBucketCachedResponse {
                    response_type: "default".to_owned(),
                    url: "https://quota.test/resource".to_owned(),
                    redirected: false,
                    status: 200,
                    status_text: "OK".to_owned(),
                    headers: Vec::new(),
                    body: b"cache".to_vec(),
                },
                cache_usage,
                0,
            )?,
            StorageBucketCachePutOutcome::Stored
        );

        let indexed_db_key = storage_bucket_indexed_db_storage_key(
            locator.storage_key(),
            locator.bucket_id().expect("test locator should be named"),
        );
        seed_indexed_db_record(&indexed_db_manager, &indexed_db_key)?;
        let indexed_db_usage = indexed_db_usage(&indexed_db_manager, &indexed_db_key)?;
        let opfs_key = StorageService::opfs_bucket_key(&locator)?;
        let root = storage_service.ensure_opfs_root(&locator)?;
        let file =
            storage_service.with_opfs(|opfs| opfs.get_file(&opfs_key, &root, "file", true))?;
        storage_service.with_opfs(|opfs| opfs.write_file(&opfs_key, &file, b"opfs-bytes", None))?;
        let opfs_usage = storage_service.opfs_quota_usage(&locator)?;

        let owner = storage_bucket_quota_owner(&store, &locator)
            .expect("live named bucket should have a quota owner");
        assert_eq!(
            owner.quota_and_non_cache_usage()?,
            (
                DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES,
                indexed_db_usage.saturating_add(opfs_usage),
            )
        );
        assert_eq!(
            owner.quota_and_non_indexed_db_usage()?,
            (
                DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES,
                cache_usage.saturating_add(opfs_usage),
            )
        );
        assert_eq!(
            owner.max_opfs_usage()?,
            DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES
                .saturating_sub(cache_usage.saturating_add(indexed_db_usage))
        );

        store.lock().delete_bucket(storage_key, "bucket")?;
        assert!(owner.max_opfs_usage().is_err());
        Ok(())
    }

    #[test]
    fn default_bucket_quota_owner_aggregates_cache_indexed_db_and_opfs() -> Result<()> {
        let storage_service = StorageService::in_memory();
        let indexed_db_manager = new_indexed_db_manager(None).map_err(anyhow::Error::msg)?;
        let store =
            super::new_shared_storage_bucket_store_with_storage_service_and_indexed_db_manager(
                storage_service.clone(),
                &indexed_db_manager,
            );
        let storage_key = concat!(
            "storage-key:v1;origin=https://default-quota.test;",
            "top-level-site=https://default-quota.test"
        );
        let identity = store
            .lock()
            .open_bucket(storage_key, IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME)?;
        let cache_usage = 41;
        assert_eq!(
            store.lock().put_cache_entry_for_identity(
                &identity,
                "cache",
                "request",
                StorageBucketCachedResponse {
                    response_type: "default".to_owned(),
                    url: "https://default-quota.test/resource".to_owned(),
                    redirected: false,
                    status: 200,
                    status_text: "OK".to_owned(),
                    headers: Vec::new(),
                    body: b"cache".to_vec(),
                },
                cache_usage,
                0,
            )?,
            StorageBucketCachePutOutcome::Stored
        );

        seed_indexed_db_record(&indexed_db_manager, storage_key)?;
        let indexed_db_usage = indexed_db_usage(&indexed_db_manager, storage_key)?;
        let locator = StorageBucketLocator::default_bucket(storage_key);
        let opfs_key = StorageService::opfs_bucket_key(&locator)?;
        let root = storage_service.ensure_opfs_root(&locator)?;
        let file =
            storage_service.with_opfs(|opfs| opfs.get_file(&opfs_key, &root, "file", true))?;
        storage_service
            .with_opfs(|opfs| opfs.write_file(&opfs_key, &file, b"default-opfs-bytes", None))?;
        let opfs_usage = storage_service.opfs_usage(&locator)?;

        let owner = storage_bucket_quota_owner(&store, &locator)
            .expect("default bucket should always have a quota owner");
        assert_eq!(
            owner.usage_snapshot()?,
            StorageBucketUsageSnapshot {
                quota: DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES,
                indexed_db: indexed_db_usage,
                cache_storage: cache_usage,
                opfs: opfs_usage,
            }
        );
        assert_eq!(
            owner.max_opfs_usage()?,
            DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES
                .saturating_sub(cache_usage.saturating_add(indexed_db_usage))
        );
        Ok(())
    }

    #[test]
    fn default_bucket_quota_owner_exists_before_cache_storage_is_opened() -> Result<()> {
        let storage_service = StorageService::in_memory();
        let store = super::new_shared_storage_bucket_store_with_storage_service(storage_service);
        let locator = StorageBucketLocator::default_bucket(concat!(
            "storage-key:v1;origin=https://empty-default.test;",
            "top-level-site=https://empty-default.test"
        ));

        let owner = storage_bucket_quota_owner(&store, &locator)
            .expect("default bucket quota does not depend on hidden cache metadata");
        assert_eq!(
            owner.usage_snapshot()?,
            StorageBucketUsageSnapshot {
                quota: DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES,
                indexed_db: 0,
                cache_storage: 0,
                opfs: 0,
            }
        );
        Ok(())
    }

    #[test]
    fn renderer_neutral_deletion_owner_clears_exact_backends_and_finishes_tombstone() -> Result<()>
    {
        let storage_service = StorageService::in_memory();
        let indexed_db_manager = new_indexed_db_manager(None).map_err(anyhow::Error::msg)?;
        let store =
            super::new_shared_storage_bucket_store_with_storage_service_and_indexed_db_manager(
                storage_service.clone(),
                &indexed_db_manager,
            );
        let storage_key = "storage-key:v1;origin=https://delete.test";
        let old_identity = store.lock().open_bucket(storage_key, "same-name-bucket")?;
        assert!(
            store
                .lock()
                .open_cache_for_identity(&old_identity, "old-cache")?
        );
        seed_indexed_db_record(&indexed_db_manager, &old_identity.indexed_db_storage_key())?;
        let old_key = StorageService::opfs_bucket_key(&old_identity.locator())?;
        let old_root = storage_service.ensure_opfs_root(&old_identity.locator())?;
        let old_file = storage_service
            .with_opfs(|opfs| opfs.get_file(&old_key, &old_root, "old.txt", true))?;
        storage_service
            .with_opfs(|opfs| opfs.write_file(&old_key, &old_file, b"old bytes", None))?;

        let cleanup = store
            .lock()
            .delete_bucket(storage_key, "same-name-bucket")?
            .expect("delete should publish an exact tombstone");
        assert_eq!(cleanup, old_identity);
        assert!(complete_storage_bucket_deletion(&store, &cleanup)?);
        assert!(store.lock().pending_deletions().is_empty());
        assert_eq!(
            indexed_db_usage(&indexed_db_manager, &old_identity.indexed_db_storage_key())?,
            0
        );
        assert_eq!(storage_service.opfs_usage(&old_identity.locator())?, 0);

        let replacement = store.lock().open_bucket(storage_key, "same-name-bucket")?;
        assert_ne!(replacement.bucket_id(), old_identity.bucket_id());
        seed_indexed_db_record(&indexed_db_manager, &replacement.indexed_db_storage_key())?;
        let replacement_key = StorageService::opfs_bucket_key(&replacement.locator())?;
        let replacement_root = storage_service.ensure_opfs_root(&replacement.locator())?;
        let replacement_file = storage_service.with_opfs(|opfs| {
            opfs.get_file(&replacement_key, &replacement_root, "new.txt", true)
        })?;
        storage_service.with_opfs(|opfs| {
            opfs.write_file(
                &replacement_key,
                &replacement_file,
                b"replacement bytes",
                None,
            )
        })?;

        assert!(!complete_storage_bucket_deletion(&store, &old_identity)?);
        assert!(indexed_db_usage(&indexed_db_manager, &replacement.indexed_db_storage_key())? > 0);
        assert!(storage_service.opfs_usage(&replacement.locator())? > 0);
        assert_eq!(
            store.lock().cache_names_for_identity(&replacement),
            Some(Vec::new())
        );
        Ok(())
    }

    #[test]
    fn memory_storage_bucket_store_invalidates_deleted_bucket_identity() -> Result<()> {
        let mut store = StorageBucketRegistry::default();

        let first = store.open_bucket("https://a.test", "bucket")?;
        let first_id = store.bucket_id("https://a.test", "bucket").unwrap();
        let same = store.open_bucket("https://a.test", "bucket")?;

        assert_eq!(same, first);
        assert_eq!(store.bucket_id("https://a.test", "bucket"), Some(first_id));
        assert_eq!(
            store
                .bucket_locator_for_identity(&first)
                .and_then(|locator| locator.bucket_id()),
            Some(first_id)
        );
        assert!(store.bucket_identity_is_live(&first));

        let cleanup = store
            .delete_bucket("https://a.test", "bucket")?
            .expect("deleted bucket should produce cleanup identity");
        assert!(!store.bucket_identity_is_live(&first));
        assert_eq!(store.bucket_locator("https://a.test", "bucket"), None);
        assert!(
            store
                .open_bucket("https://a.test", "bucket")
                .unwrap_err()
                .to_string()
                .contains("deletion is still pending")
        );
        assert!(store.finish_bucket_deletion(&cleanup)?);

        let recreated = store.open_bucket("https://a.test", "bucket")?;
        let recreated_id = store.bucket_id("https://a.test", "bucket").unwrap();
        assert_ne!(recreated, first);
        assert_ne!(recreated_id, first_id);
        assert!(!store.bucket_identity_is_live(&first));
        assert!(store.bucket_identity_is_live(&recreated));
        Ok(())
    }

    #[test]
    fn memory_storage_bucket_store_prunes_expired_buckets() -> Result<()> {
        let mut store = StorageBucketRegistry::default();

        let expired = store.open_bucket_with_expires("https://a.test", "expired", Some(1_000.0))?;
        let live = store.open_bucket_with_expires("https://a.test", "live", Some(3_000.0))?;
        store.open_bucket_with_expires("https://b.test", "expired", Some(1_000.0))?;

        let cleanups = store.delete_expired_buckets("https://a.test", 2_000.0)?;
        assert_eq!(
            cleanups
                .iter()
                .map(StorageBucketIdentity::name)
                .collect::<Vec<_>>(),
            vec!["expired"]
        );
        assert_eq!(store.keys("https://a.test"), vec!["live"]);
        assert_eq!(store.keys("https://b.test"), vec!["expired"]);
        assert!(!store.bucket_identity_is_live(&expired));
        assert!(store.bucket_identity_is_live(&live));

        assert!(store.finish_bucket_deletion(&cleanups[0])?);
        let recreated = store.open_bucket("https://a.test", "expired")?;
        assert_ne!(recreated, expired);
        Ok(())
    }

    #[test]
    fn memory_storage_bucket_store_deletes_one_expired_bucket() -> Result<()> {
        let mut store = StorageBucketRegistry::default();

        let expired = store.open_bucket_with_expires("https://a.test", "expired", Some(1_000.0))?;
        let live = store.open_bucket_with_expires("https://a.test", "live", Some(3_000.0))?;

        assert!(
            store
                .delete_bucket_if_expired("https://a.test", "live", 2_000.0)?
                .is_none()
        );
        assert!(
            store
                .delete_bucket_if_expired("https://a.test", "expired", 2_000.0)?
                .is_some()
        );
        assert_eq!(store.keys("https://a.test"), vec!["live"]);
        assert!(!store.bucket_identity_is_live(&expired));
        assert!(store.bucket_identity_is_live(&live));
        Ok(())
    }

    #[test]
    fn memory_storage_bucket_store_matches_runtime_cache_entries() -> Result<()> {
        let mut store = StorageBucketRegistry::default();
        let identity = store.open_bucket("https://a.test", "bucket")?;
        assert!(store.open_cache_for_identity(&identity, "cache")?);
        let response = StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: String::new(),
            redirected: false,
            status: 201,
            status_text: "Created".to_owned(),
            headers: vec![("x-cache".to_owned(), "hit".to_owned())],
            body: b"cached body".to_vec(),
        };

        assert_eq!(
            store.put_cache_entry_for_identity(
                &identity,
                "cache",
                "/receipt",
                response.clone(),
                42,
                0,
            )?,
            StorageBucketCachePutOutcome::Stored
        );

        let matched = store
            .match_cache_entry_for_identity(&identity, "cache", "/receipt")
            .flatten()
            .expect("cache entry should match");
        assert_eq!(matched.status, response.status);
        assert_eq!(matched.status_text, response.status_text);
        assert_eq!(matched.headers, response.headers);
        assert_eq!(matched.body, response.body);
        assert_eq!(store.cache_usage_for_identity(&identity), Some(42));
        assert_eq!(store.cache_usage_for_origin("https://a.test"), 42);
        assert_eq!(store.cache_usage_for_origin("https://b.test"), 0);
        assert_eq!(
            store.match_cache_entry_for_identity(&identity, "cache", "/missing"),
            Some(None)
        );
        Ok(())
    }

    #[test]
    fn memory_storage_bucket_store_normalizes_cache_request_fragments() -> Result<()> {
        let mut store = StorageBucketRegistry::default();
        let identity = store.open_bucket("https://a.test", "bucket")?;
        assert!(store.open_cache_for_identity(&identity, "cache")?);
        let first_response = StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: String::new(),
            redirected: false,
            status: 200,
            status_text: "OK".to_owned(),
            headers: Vec::new(),
            body: b"first".to_vec(),
        };

        assert_eq!(
            store.put_cache_entry_for_identity(
                &identity,
                "cache",
                "https://a.test/entry#put-fragment",
                first_response.clone(),
                42,
                0,
            )?,
            StorageBucketCachePutOutcome::Stored
        );
        assert_eq!(
            store.cache_request_keys_for_identity(&identity, "cache"),
            Some(vec!["https://a.test/entry#put-fragment".to_owned()])
        );
        assert_eq!(
            store
                .match_cache_entry_for_identity(
                    &identity,
                    "cache",
                    "https://a.test/entry#match-fragment",
                )
                .flatten(),
            Some(first_response)
        );
        assert_eq!(
            store.delete_cache_entry_for_identity(
                &identity,
                "cache",
                "https://a.test/entry#delete-fragment",
            )?,
            Some(true)
        );
        assert_eq!(
            store.cache_request_keys_for_identity(&identity, "cache"),
            Some(Vec::new())
        );
        assert_eq!(
            store.delete_cache_entry_for_identity(
                &identity,
                "cache",
                "https://a.test/entry#again",
            )?,
            Some(false)
        );

        let replacement_response = StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: String::new(),
            redirected: false,
            status: 201,
            status_text: "Created".to_owned(),
            headers: Vec::new(),
            body: b"replacement".to_vec(),
        };
        assert_eq!(
            store.put_cache_entry_for_identity(
                &identity,
                "cache",
                "https://a.test/replaced#first",
                replacement_response.clone(),
                42,
                0,
            )?,
            StorageBucketCachePutOutcome::Stored
        );
        assert_eq!(
            store.put_cache_entry_for_identity(
                &identity,
                "cache",
                "https://a.test/replaced#second",
                replacement_response.clone(),
                17,
                0,
            )?,
            StorageBucketCachePutOutcome::Stored
        );
        assert_eq!(
            store.cache_request_keys_for_identity(&identity, "cache"),
            Some(vec!["https://a.test/replaced#second".to_owned()])
        );
        assert_eq!(store.cache_usage_for_identity(&identity), Some(17));
        assert_eq!(
            store
                .match_cache_entry_for_identity(
                    &identity,
                    "cache",
                    "https://a.test/replaced#query",
                )
                .flatten(),
            Some(replacement_response)
        );
        Ok(())
    }

    #[test]
    fn memory_storage_bucket_store_applies_cache_query_options_and_vary() -> Result<()> {
        let mut store = StorageBucketRegistry::default();
        let identity = store.open_bucket("https://a.test", "bucket")?;
        assert!(store.open_cache_for_identity(&identity, "cache")?);
        let response = |body: &str, headers: Vec<(String, String)>| StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: String::new(),
            redirected: false,
            status: 200,
            status_text: "OK".to_owned(),
            headers,
            body: body.as_bytes().to_vec(),
        };
        for (request_url, request, response) in [
            (
                "https://a.test/resource?page=1",
                StorageBucketCachedRequest::default(),
                response("page-one", Vec::new()),
            ),
            (
                "https://a.test/resource?page=2",
                StorageBucketCachedRequest::default(),
                response("page-two", Vec::new()),
            ),
            (
                "https://a.test/resource?mode=vary",
                StorageBucketCachedRequest {
                    method: "GET".to_owned(),
                    headers: vec![("x-mode".to_owned(), "alpha".to_owned())],
                },
                response("vary-alpha", vec![("vary".to_owned(), "X-Mode".to_owned())]),
            ),
        ] {
            assert_eq!(
                store.put_cache_entry_with_request_for_identity(
                    &identity,
                    "cache",
                    request_url,
                    request,
                    response,
                    10,
                    0,
                )?,
                StorageBucketCachePutOutcome::Stored
            );
        }

        let ignore_search = StorageBucketCacheQuery {
            request_url: "https://a.test/resource?page=99".to_owned(),
            method: "GET".to_owned(),
            headers: Vec::new(),
            ignore_search: true,
            ignore_method: false,
            ignore_vary: false,
        };
        let matches = store
            .match_cache_entries_for_identity(&identity, "cache", &ignore_search)
            .expect("cache should remain live");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].response.body, b"page-one");
        assert_eq!(matches[1].response.body, b"page-two");

        let vary_miss = StorageBucketCacheQuery {
            request_url: "https://a.test/resource?mode=vary".to_owned(),
            method: "GET".to_owned(),
            headers: vec![("X-Mode".to_owned(), "beta".to_owned())],
            ignore_search: false,
            ignore_method: false,
            ignore_vary: false,
        };
        assert_eq!(
            store.match_cache_entries_for_identity(&identity, "cache", &vary_miss),
            Some(Vec::new())
        );
        let vary_ignored = StorageBucketCacheQuery {
            ignore_vary: true,
            ..vary_miss
        };
        assert_eq!(
            store
                .match_cache_entries_for_identity(&identity, "cache", &vary_ignored)
                .expect("cache should remain live")[0]
                .response
                .body,
            b"vary-alpha"
        );
        Ok(())
    }

    #[test]
    fn deleted_cache_mapping_keeps_live_handle_detached_until_release() -> Result<()> {
        let mut store = StorageBucketRegistry::default();
        let identity = store.open_bucket("https://a.test", "bucket")?;
        let cache_id = store
            .open_cache_handle_for_identity(&identity, "cache")?
            .expect("bucket should remain current");
        let response = StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: String::new(),
            redirected: false,
            status: 200,
            status_text: "OK".to_owned(),
            headers: Vec::new(),
            body: b"before-delete".to_vec(),
        };
        assert_eq!(
            store.put_cache_entry_with_request_for_handle_and_identity(
                &identity,
                "cache",
                cache_id,
                "https://a.test/entry",
                StorageBucketCachedRequest::default(),
                response,
                10,
                0,
            )?,
            StorageBucketCachePutOutcome::Stored
        );
        assert_eq!(
            store.delete_cache_for_identity(&identity, "cache")?,
            Some(true)
        );
        assert_eq!(store.cache_names_for_identity(&identity), Some(Vec::new()));
        assert_eq!(
            store
                .cache_entries_for_handle_and_identity(&identity, "cache", cache_id)
                .expect("detached handle should remain live")[0]
                .response
                .body,
            b"before-delete"
        );

        let reopened_id = store
            .open_cache_handle_for_identity(&identity, "cache")?
            .expect("bucket should remain current");
        assert_ne!(reopened_id, cache_id);
        assert_eq!(
            store.cache_entries_for_handle_and_identity(&identity, "cache", reopened_id),
            Some(Vec::new())
        );
        store.release_cache_handle_for_identity(&identity, cache_id);
        assert_eq!(
            store.cache_entries_for_handle_and_identity(&identity, "cache", cache_id),
            None
        );
        Ok(())
    }

    #[test]
    fn memory_storage_bucket_store_enforces_cache_quota_without_clobbering_entries() -> Result<()> {
        let mut store = StorageBucketRegistry::default();
        let identity = store.open_bucket_with_options(
            "https://a.test",
            "bucket",
            None,
            None,
            Some(100),
            None,
        )?;
        assert!(store.open_cache_for_identity(&identity, "cache")?);
        let original = StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: String::new(),
            redirected: false,
            status: 200,
            status_text: "OK".to_owned(),
            headers: Vec::new(),
            body: b"small".to_vec(),
        };
        let oversized = StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: String::new(),
            redirected: false,
            status: 200,
            status_text: "OK".to_owned(),
            headers: Vec::new(),
            body: b"oversized".to_vec(),
        };

        assert_eq!(
            store.put_cache_entry_for_identity(
                &identity,
                "cache",
                "/entry",
                original.clone(),
                40,
                50,
            )?,
            StorageBucketCachePutOutcome::Stored
        );
        assert_eq!(
            store.put_cache_entry_for_identity(
                &identity,
                "cache",
                "/too-large",
                oversized.clone(),
                60,
                50,
            )?,
            StorageBucketCachePutOutcome::QuotaExceeded {
                quota: 100,
                requested: 150,
            }
        );
        assert_eq!(
            store.put_cache_entry_for_identity(&identity, "cache", "/entry", oversized, 60, 50,)?,
            StorageBucketCachePutOutcome::QuotaExceeded {
                quota: 100,
                requested: 110,
            }
        );

        let matched = store
            .match_cache_entry_for_identity(&identity, "cache", "/entry")
            .flatten()
            .expect("original cache entry should remain after quota rejection");
        assert_eq!(matched, original);
        assert_eq!(
            store.match_cache_entry_for_identity(&identity, "cache", "/too-large"),
            Some(None)
        );
        assert_eq!(store.cache_usage_for_identity(&identity), Some(40));
        Ok(())
    }

    #[test]
    fn memory_storage_bucket_store_clear_origin_keeps_other_origins() -> Result<()> {
        let mut store = StorageBucketRegistry::default();

        store.open_bucket("https://a.test", "bucket-a")?;
        store.open_bucket("https://a.test", "bucket-b")?;
        store.open_bucket("https://b.test", "bucket-c")?;

        store.clear_origin("https://a.test")?;

        assert_eq!(store.keys("https://a.test"), Vec::<String>::new());
        assert_eq!(store.keys("https://b.test"), vec!["bucket-c"]);
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_persists_names_by_origin() -> Result<()> {
        let temp = TempStorePath::new("persist");
        let bucket_a_id;
        {
            let store = new_shared_json_storage_bucket_store(&temp.path)?;
            let mut store = store.lock();
            store.open_bucket("https://a.test", "bucket-b")?;
            store.open_bucket("https://a.test", "bucket-a")?;
            store.open_bucket("https://b.test", "bucket-c")?;
            bucket_a_id = store.bucket_id("https://a.test", "bucket-a").unwrap();
        }

        let store = new_shared_json_storage_bucket_store(&temp.path)?;
        let store = store.lock();
        assert_eq!(store.keys("https://a.test"), vec!["bucket-a", "bucket-b"]);
        assert_eq!(store.keys("https://b.test"), vec!["bucket-c"]);
        assert_eq!(
            store.bucket_id("https://a.test", "bucket-a"),
            Some(bucket_a_id)
        );
        Ok(())
    }

    #[test]
    fn json_v3_migrates_legacy_global_cache_to_hidden_implicit_default() -> Result<()> {
        let temp = TempStorePath::new("migrate-v3-implicit-default");
        let cache_root = temp.cache_root();
        {
            let store =
                new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
            let mut store = store.lock();
            let identity = store.open_bucket("https://a.test", "default")?;
            assert_eq!(
                store.put_cache_entry_for_identity(
                    &identity,
                    "global-cache",
                    "/entry",
                    StorageBucketCachedResponse {
                        response_type: "default".to_owned(),
                        url: "https://a.test/entry".to_owned(),
                        redirected: false,
                        status: 200,
                        status_text: "OK".to_owned(),
                        headers: Vec::new(),
                        body: b"legacy".to_vec(),
                    },
                    6,
                    0,
                )?,
                StorageBucketCachePutOutcome::Stored
            );
        }
        let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&temp.path)?)?;
        json["version"] = serde_json::json!(3);
        fs::write(&temp.path, serde_json::to_vec_pretty(&json)?)?;

        let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
        let store = store.lock();
        let implicit_identity = store
            .bucket_identity("https://a.test", IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME)
            .expect("migration should create the implicit default bucket");
        assert_eq!(
            store.cache_names_for_identity(&implicit_identity),
            Some(vec!["global-cache".to_owned()])
        );
        let named_identity = store
            .bucket_identity("https://a.test", "default")
            .expect("named default bucket should survive migration");
        assert_eq!(
            store.cache_names_for_identity(&named_identity),
            Some(Vec::new())
        );
        drop(store);
        let persisted: serde_json::Value = serde_json::from_slice(&fs::read(&temp.path)?)?;
        assert_eq!(persisted["version"], serde_json::json!(5));
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_does_not_reuse_deleted_bucket_id_after_restart() -> Result<()> {
        let temp = TempStorePath::new("persistent-bucket-id-recreate");
        let cache_root = temp.cache_root();
        let indexed_db_manager =
            new_indexed_db_manager(Some(temp.indexed_db_root())).map_err(anyhow::Error::msg)?;
        let storage_service = StorageService::on_disk(temp.opfs_root())?;
        let first_id;
        {
            let store = new_shared_json_storage_bucket_store_with_storage_service(
                &temp.path,
                &cache_root,
                &indexed_db_manager,
                storage_service.clone(),
            )?;
            let mut store = store.lock();
            store.open_bucket("https://a.test", "bucket")?;
            first_id = store.bucket_id("https://a.test", "bucket").unwrap();
            store.delete_bucket("https://a.test", "bucket")?;
        }

        let recreated_id;
        {
            let store = new_shared_json_storage_bucket_store_with_storage_service(
                &temp.path,
                &cache_root,
                &indexed_db_manager,
                storage_service.clone(),
            )?;
            let mut store = store.lock();
            assert_eq!(store.keys("https://a.test"), Vec::<String>::new());
            store.open_bucket("https://a.test", "bucket")?;
            recreated_id = store.bucket_id("https://a.test", "bucket").unwrap();
            assert_ne!(recreated_id, first_id);
        }

        let store = new_shared_json_storage_bucket_store_with_storage_service(
            &temp.path,
            &cache_root,
            &indexed_db_manager,
            storage_service,
        )?;
        assert_eq!(
            store.lock().bucket_id("https://a.test", "bucket"),
            Some(recreated_id)
        );
        Ok(())
    }

    #[test]
    fn json_pending_bucket_deletion_recovers_idb_and_opfs_before_same_name_recreate() -> Result<()>
    {
        let temp = TempStorePath::new("pending-delete-recovery");
        let cache_root = temp.cache_root();
        let indexed_db_manager =
            new_indexed_db_manager(Some(temp.indexed_db_root())).map_err(anyhow::Error::msg)?;
        let storage_service = StorageService::on_disk(temp.opfs_root())?;
        let storage_key = "https://a.test";
        let first_identity = {
            let store = new_shared_json_storage_bucket_store_with_storage_service(
                &temp.path,
                &cache_root,
                &indexed_db_manager,
                storage_service.clone(),
            )?;
            let mut store = store.lock();
            store.open_bucket(storage_key, "bucket")?;
            let identity = store
                .bucket_identity(storage_key, "bucket")
                .expect("opened bucket should have identity");
            seed_indexed_db_record(&indexed_db_manager, &identity.indexed_db_storage_key())?;
            let locator = identity.locator();
            let bucket_key = StorageService::opfs_bucket_key(&locator)?;
            let root = storage_service.ensure_opfs_root(&locator)?;
            let file = storage_service
                .with_opfs(|opfs| opfs.get_file(&bucket_key, &root, "old.txt", true))?;
            storage_service
                .with_opfs(|opfs| opfs.write_file(&bucket_key, &file, b"old bucket", None))?;
            assert!(indexed_db_usage(&indexed_db_manager, &identity.indexed_db_storage_key())? > 0);
            assert!(storage_service.opfs_usage(&locator)? > 0);

            let cleanup = store
                .delete_bucket(storage_key, "bucket")?
                .expect("delete should capture cleanup identity");
            assert_eq!(cleanup, identity);
            assert!(!store.bucket_locator_is_live(&locator));
            assert!(
                store
                    .open_bucket(storage_key, "bucket")
                    .unwrap_err()
                    .to_string()
                    .contains("deletion is still pending")
            );
            identity
        };

        let pending_json: serde_json::Value = serde_json::from_slice(&fs::read(&temp.path)?)?;
        assert_eq!(
            pending_json["pendingDeletions"].as_array().map(Vec::len),
            Some(1)
        );

        {
            let partial_store =
                new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
            let mut partial_store = partial_store.lock();
            assert_eq!(partial_store.pending_deletions().len(), 1);
            assert!(
                partial_store
                    .open_bucket(storage_key, "bucket")
                    .unwrap_err()
                    .to_string()
                    .contains("deletion is still pending")
            );
        }
        let still_pending_json: serde_json::Value = serde_json::from_slice(&fs::read(&temp.path)?)?;
        assert_eq!(
            still_pending_json["pendingDeletions"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );

        let store = new_shared_json_storage_bucket_store_with_storage_service(
            &temp.path,
            &cache_root,
            &indexed_db_manager,
            storage_service.clone(),
        )?;
        let mut store = store.lock();
        assert!(store.pending_deletions().is_empty());
        assert_eq!(
            indexed_db_usage(
                &indexed_db_manager,
                &first_identity.indexed_db_storage_key()
            )?,
            0
        );
        assert_eq!(storage_service.opfs_usage(&first_identity.locator())?, 0);

        store.open_bucket(storage_key, "bucket")?;
        let recreated = store
            .bucket_identity(storage_key, "bucket")
            .expect("recreated bucket should have identity");
        assert_ne!(recreated.bucket_id(), first_identity.bucket_id());
        assert_eq!(
            indexed_db_usage(&indexed_db_manager, &recreated.indexed_db_storage_key())?,
            0
        );
        assert_eq!(storage_service.opfs_usage(&recreated.locator())?, 0);
        drop(store);

        let recovered_json: serde_json::Value = serde_json::from_slice(&fs::read(&temp.path)?)?;
        assert!(recovered_json.get("pendingDeletions").is_none());
        Ok(())
    }

    #[test]
    fn json_named_bucket_tombstone_crash_points_replay_cleanup() -> Result<()> {
        for point in [
            CrashPoint::BucketTombstoneDurable,
            CrashPoint::BucketCleanupComplete,
            CrashPoint::BucketTombstoneRemovedDurable,
        ] {
            let temp = TempStorePath::new("tombstone-crash-window");
            let cache_root = temp.cache_root();
            let indexed_db_manager =
                new_indexed_db_manager(Some(temp.indexed_db_root())).map_err(anyhow::Error::msg)?;
            let storage_service = StorageService::on_disk(temp.opfs_root())?;
            let storage_key = "https://a.test";
            let old_identity;
            {
                let store = new_shared_json_storage_bucket_store_with_storage_service(
                    &temp.path,
                    &cache_root,
                    &indexed_db_manager,
                    storage_service.clone(),
                )?;
                let mut store = store.lock();
                old_identity = store.open_bucket(storage_key, "bucket")?;
                seed_indexed_db_record(
                    &indexed_db_manager,
                    &old_identity.indexed_db_storage_key(),
                )?;
                let locator = old_identity.locator();
                let bucket_key = StorageService::opfs_bucket_key(&locator)?;
                let root = storage_service.ensure_opfs_root(&locator)?;
                let file = storage_service
                    .with_opfs(|opfs| opfs.get_file(&bucket_key, &root, "old.txt", true))?;
                storage_service
                    .with_opfs(|opfs| opfs.write_file(&bucket_key, &file, b"old bucket", None))?;
                assert!(store.open_cache_for_identity(&old_identity, "cache")?);
                assert_eq!(
                    store.put_cache_entry_for_identity(
                        &old_identity,
                        "cache",
                        "/cached.txt",
                        StorageBucketCachedResponse {
                            response_type: "default".to_owned(),
                            url: String::new(),
                            redirected: false,
                            status: 200,
                            status_text: "OK".to_owned(),
                            headers: Vec::new(),
                            body: b"old cache".to_vec(),
                        },
                        32,
                        0,
                    )?,
                    StorageBucketCachePutOutcome::Stored
                );

                if point == CrashPoint::BucketTombstoneDurable {
                    let crash = catch_unwind(AssertUnwindSafe(|| {
                        let _armed = arm(point);
                        store
                            .delete_bucket(storage_key, "bucket")
                            .expect("delete should persist its tombstone");
                    }));
                    assert!(crash.is_err(), "{point:?} should interrupt bucket deletion");
                } else {
                    let cleanup = store
                        .delete_bucket(storage_key, "bucket")?
                        .expect("delete should return its cleanup identity");
                    assert_eq!(cleanup, old_identity);
                    storage_service.clear_opfs_bucket(&cleanup.locator())?;
                    clear_indexed_db_origin(&indexed_db_manager, &cleanup.indexed_db_storage_key())
                        .map_err(anyhow::Error::msg)?;
                    let crash = catch_unwind(AssertUnwindSafe(|| {
                        let _armed = arm(point);
                        store
                            .finish_bucket_deletion(&cleanup)
                            .expect("finish should reach the armed crash point");
                    }));
                    assert!(crash.is_err(), "{point:?} should interrupt bucket deletion");
                }
            }

            let interrupted_json: serde_json::Value =
                serde_json::from_slice(&fs::read(&temp.path)?)?;
            let interrupted_pending = interrupted_json
                .get("pendingDeletions")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            let expected_pending = usize::from(point != CrashPoint::BucketTombstoneRemovedDurable);
            assert_eq!(
                interrupted_pending, expected_pending,
                "unexpected durable tombstone state at {point:?}"
            );

            drop(storage_service);
            drop(indexed_db_manager);
            let indexed_db_manager =
                new_indexed_db_manager(Some(temp.indexed_db_root())).map_err(anyhow::Error::msg)?;
            let storage_service = StorageService::on_disk(temp.opfs_root())?;
            let store = new_shared_json_storage_bucket_store_with_storage_service(
                &temp.path,
                &cache_root,
                &indexed_db_manager,
                storage_service.clone(),
            )?;
            let mut store = store.lock();
            assert!(store.pending_deletions().is_empty());
            assert_eq!(
                indexed_db_usage(&indexed_db_manager, &old_identity.indexed_db_storage_key())?,
                0
            );
            assert_eq!(storage_service.opfs_usage(&old_identity.locator())?, 0);
            assert!(!store.bucket_locator_is_live(&old_identity.locator()));
            let recreated = store.open_bucket(storage_key, "bucket")?;
            assert_ne!(recreated.bucket_id(), old_identity.bucket_id());
            assert_eq!(store.cache_names_for_identity(&recreated), Some(Vec::new()));
            assert!(
                !storage_bucket_cache_bucket_dir(
                    &cache_root,
                    storage_key,
                    old_identity.bucket_id(),
                )
                .exists(),
                "{point:?} left the revoked bucket CacheStorage directory"
            );
        }
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_persists_cache_entries() -> Result<()> {
        let temp = TempStorePath::new("cache-persist");
        let cache_root = temp.cache_root();
        let response = StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: String::new(),
            redirected: false,
            status: 202,
            status_text: "Accepted".to_owned(),
            headers: vec![("x-cache".to_owned(), "persisted".to_owned())],
            body: b"profile cache body".to_vec(),
        };
        {
            let store =
                new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
            let mut store = store.lock();
            let identity = store.open_bucket("https://a.test", "bucket")?;
            assert!(store.open_cache_for_identity(&identity, "cache")?);
            assert_eq!(
                store.put_cache_entry_for_identity(
                    &identity,
                    "cache",
                    "/cached.txt",
                    response.clone(),
                    64,
                    0,
                )?,
                StorageBucketCachePutOutcome::Stored
            );
        }

        assert!(
            cache_root.exists(),
            "profile-backed CacheStorage root should be written"
        );
        let (next, previous) = storage_bucket_cache_replacement_paths(&cache_root)?;
        assert!(
            !next.exists(),
            "profile-backed CacheStorage replacement root should not be left after save"
        );
        assert!(
            !previous.exists(),
            "profile-backed CacheStorage previous root should not be left after save"
        );

        {
            let store =
                new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
            let mut store = store.lock();
            let identity = store.open_bucket("https://a.test", "bucket")?;
            assert_eq!(
                store.cache_names_for_identity(&identity),
                Some(vec!["cache".to_owned()])
            );
            let matched = store
                .match_cache_entry_for_identity(&identity, "cache", "/cached.txt")
                .flatten()
                .expect("cache entry should persist across reopen");
            assert_eq!(matched, response);
            assert_eq!(store.cache_usage_for_identity(&identity), Some(64));
            assert_eq!(
                store.delete_cache_for_identity(&identity, "cache")?,
                Some(true)
            );
        }

        let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
        let mut store = store.lock();
        let identity = store.open_bucket("https://a.test", "bucket")?;
        assert_eq!(store.cache_names_for_identity(&identity), Some(Vec::new()));
        Ok(())
    }

    #[test]
    fn json_storage_bucket_cache_crash_points_recover_one_committed_root() -> Result<()> {
        let old_response = StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: String::new(),
            redirected: false,
            status: 200,
            status_text: "OK".to_owned(),
            headers: vec![("x-cache".to_owned(), "old".to_owned())],
            body: b"old committed cache body".to_vec(),
        };
        let new_response = StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: String::new(),
            redirected: false,
            status: 201,
            status_text: "Created".to_owned(),
            headers: vec![("x-cache".to_owned(), "new".to_owned())],
            body: b"new committed cache body".to_vec(),
        };

        for point in [
            CrashPoint::CacheNextDurable,
            CrashPoint::CachePreviousDurable,
            CrashPoint::CacheCurrentDurable,
            CrashPoint::CachePreviousRemovedBeforeSync,
        ] {
            let temp = TempStorePath::new("cache-crash-window");
            let cache_root = temp.cache_root();
            {
                let store =
                    new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
                let mut store = store.lock();
                let identity = store.open_bucket("https://a.test", "bucket")?;
                assert!(store.open_cache_for_identity(&identity, "cache")?);
                assert_eq!(
                    store.put_cache_entry_for_identity(
                        &identity,
                        "cache",
                        "/cached.txt",
                        old_response.clone(),
                        64,
                        0,
                    )?,
                    StorageBucketCachePutOutcome::Stored
                );
            }

            {
                let store =
                    new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
                let mut store = store.lock();
                let identity = store.open_bucket("https://a.test", "bucket")?;
                let crash = catch_unwind(AssertUnwindSafe(|| {
                    let _armed = arm(point);
                    store
                        .put_cache_entry_for_identity(
                            &identity,
                            "cache",
                            "/cached.txt",
                            new_response.clone(),
                            96,
                            0,
                        )
                        .expect("cache mutation should reach the armed crash point");
                }));
                assert!(crash.is_err(), "{point:?} should interrupt cache commit");
            }

            let store =
                new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
            let mut store = store.lock();
            let identity = store.open_bucket("https://a.test", "bucket")?;
            let matched = store
                .match_cache_entry_for_identity(&identity, "cache", "/cached.txt")
                .flatten()
                .expect("recovery should leave one committed cache response");
            let expected = if point == CrashPoint::CacheNextDurable {
                &old_response
            } else {
                &new_response
            };
            assert_eq!(&matched, expected, "unexpected recovery at {point:?}");
            assert!(cache_root.exists());
            let (next, previous) = storage_bucket_cache_replacement_paths(&cache_root)?;
            assert!(!next.exists(), "{point:?} left a replacement root");
            assert!(!previous.exists(), "{point:?} left a previous root");
        }
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_recovers_completed_cache_storage_replacement_root() -> Result<()> {
        let temp = TempStorePath::new("cache-promote-next");
        let cache_root = temp.cache_root();
        let response = StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: String::new(),
            redirected: false,
            status: 206,
            status_text: "Partial Content".to_owned(),
            headers: vec![("x-cache".to_owned(), "next".to_owned())],
            body: b"replacement cache body".to_vec(),
        };
        {
            let store =
                new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
            let mut store = store.lock();
            let identity = store.open_bucket("https://a.test", "bucket")?;
            assert!(store.open_cache_for_identity(&identity, "cache")?);
            assert_eq!(
                store.put_cache_entry_for_identity(
                    &identity,
                    "cache",
                    "/cached.txt",
                    response.clone(),
                    80,
                    0,
                )?,
                StorageBucketCachePutOutcome::Stored
            );
        }

        let (next, previous) = storage_bucket_cache_replacement_paths(&cache_root)?;
        fs::rename(&cache_root, &next)?;
        assert!(!cache_root.exists());
        assert!(next.exists());
        assert!(!previous.exists());

        let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
        let mut store = store.lock();
        let identity = store.open_bucket("https://a.test", "bucket")?;
        let matched = store
            .match_cache_entry_for_identity(&identity, "cache", "/cached.txt")
            .flatten()
            .expect("completed replacement root should be promoted on reopen");
        assert_eq!(matched, response);
        assert!(cache_root.exists());
        assert!(!next.exists());
        assert!(!previous.exists());
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_recovers_previous_cache_storage_root() -> Result<()> {
        let temp = TempStorePath::new("cache-restore-previous");
        let cache_root = temp.cache_root();
        let response = StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: String::new(),
            redirected: false,
            status: 200,
            status_text: "OK".to_owned(),
            headers: vec![("x-cache".to_owned(), "previous".to_owned())],
            body: b"previous cache body".to_vec(),
        };
        {
            let store =
                new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
            let mut store = store.lock();
            let identity = store.open_bucket("https://a.test", "bucket")?;
            assert!(store.open_cache_for_identity(&identity, "cache")?);
            assert_eq!(
                store.put_cache_entry_for_identity(
                    &identity,
                    "cache",
                    "/cached.txt",
                    response.clone(),
                    72,
                    0,
                )?,
                StorageBucketCachePutOutcome::Stored
            );
        }

        let (next, previous) = storage_bucket_cache_replacement_paths(&cache_root)?;
        fs::rename(&cache_root, &previous)?;
        assert!(!cache_root.exists());
        assert!(!next.exists());
        assert!(previous.exists());

        let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
        let mut store = store.lock();
        let identity = store.open_bucket("https://a.test", "bucket")?;
        let matched = store
            .match_cache_entry_for_identity(&identity, "cache", "/cached.txt")
            .flatten()
            .expect("previous root should be restored on reopen");
        assert_eq!(matched, response);
        assert!(cache_root.exists());
        assert!(!next.exists());
        assert!(!previous.exists());
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_prunes_expired_buckets_on_open() -> Result<()> {
        let temp = TempStorePath::new("expires-open");
        let cache_root = temp.cache_root();
        let now_ms = test_now_ms();
        let indexed_db_manager = new_indexed_db_manager(None).map_err(anyhow::Error::msg)?;
        let storage_service = StorageService::on_disk(temp.opfs_root())?;
        let response = StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: String::new(),
            redirected: false,
            status: 200,
            status_text: "OK".to_owned(),
            headers: Vec::new(),
            body: b"expired cache body".to_vec(),
        };
        let (expired_indexed_db_key, live_indexed_db_key) = {
            let store =
                new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
            let mut store = store.lock();
            let expired_identity = store.open_bucket_with_expires(
                "https://a.test",
                "expired",
                Some(now_ms - 1_000.0),
            )?;
            store.open_bucket_with_expires("https://a.test", "live", Some(now_ms + 60_000.0))?;
            let sibling_expired_identity = store.open_bucket_with_expires(
                "https://b.test",
                "expired",
                Some(now_ms - 1_000.0),
            )?;
            assert!(store.open_cache_for_identity(&expired_identity, "cache")?);
            assert_eq!(
                store.put_cache_entry_for_identity(
                    &expired_identity,
                    "cache",
                    "/expired.txt",
                    response,
                    72,
                    0,
                )?,
                StorageBucketCachePutOutcome::Stored
            );
            assert!(store.open_cache_for_identity(&sibling_expired_identity, "cache")?);
            (
                store
                    .bucket_identity("https://a.test", "expired")
                    .expect("expired bucket should have identity")
                    .indexed_db_storage_key(),
                store
                    .bucket_identity("https://a.test", "live")
                    .expect("live bucket should have identity")
                    .indexed_db_storage_key(),
            )
        };
        seed_indexed_db_record(&indexed_db_manager, &expired_indexed_db_key)?;
        seed_indexed_db_record(&indexed_db_manager, &live_indexed_db_key)?;

        assert!(
            cache_root.exists(),
            "expired profile-backed CacheStorage should exist before reopen"
        );
        assert!(indexed_db_usage(&indexed_db_manager, &expired_indexed_db_key)? > 0);
        assert!(indexed_db_usage(&indexed_db_manager, &live_indexed_db_key)? > 0);

        let store = new_shared_json_storage_bucket_store_with_storage_service(
            &temp.path,
            &cache_root,
            &indexed_db_manager,
            storage_service,
        )?;
        let store = store.lock();
        assert_eq!(store.keys("https://a.test"), vec!["live"]);
        assert_eq!(store.keys("https://b.test"), Vec::<String>::new());
        assert_eq!(store.cache_usage_for_origin("https://a.test"), 0);
        assert_eq!(
            indexed_db_usage(&indexed_db_manager, &expired_indexed_db_key)?,
            0
        );
        assert!(indexed_db_usage(&indexed_db_manager, &live_indexed_db_key)? > 0);
        assert!(
            cache_root.exists(),
            "profile open pruning should leave an atomically replaced CacheStorage root"
        );
        assert!(
            fs::read_dir(&cache_root)?.next().is_none(),
            "profile open pruning should rewrite CacheStorage without expired bucket files"
        );
        let json = fs::read_to_string(&temp.path)?;
        assert!(json.contains(r#""live""#));
        assert!(!json.contains(r#""expired""#));
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_persists_bucket_expiration_metadata() -> Result<()> {
        let temp = TempStorePath::new("expires");
        let first_expires = test_now_ms().floor() + 60_000.0;
        let second_expires = first_expires + 60_000.0;
        {
            let store = new_shared_json_storage_bucket_store(&temp.path)?;
            let mut store = store.lock();
            store.open_bucket("https://a.test", "bucket-a")?;
            store.set_bucket_expires("https://a.test", "bucket-a", first_expires)?;
            store.open_bucket_with_expires("https://a.test", "bucket-b", Some(second_expires))?;
        }

        let json = fs::read_to_string(&temp.path)?;
        assert!(!json.contains("generation"));

        let store = new_shared_json_storage_bucket_store(&temp.path)?;
        let store = store.lock();
        assert_eq!(
            store.bucket_expires("https://a.test", "bucket-a"),
            Some(first_expires)
        );
        assert_eq!(
            store.bucket_expires("https://a.test", "bucket-b"),
            Some(second_expires)
        );
        assert_eq!(store.keys("https://a.test"), vec!["bucket-a", "bucket-b"]);
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_persists_bucket_durability_metadata() -> Result<()> {
        let temp = TempStorePath::new("durability");
        {
            let store = new_shared_json_storage_bucket_store(&temp.path)?;
            let mut store = store.lock();
            store.open_bucket("https://a.test", "default")?;
            store.open_bucket_with_options(
                "https://a.test",
                "strict",
                None,
                Some(StorageBucketDurability::Strict),
                None,
                None,
            )?;
        }

        let json = fs::read_to_string(&temp.path)?;
        assert!(json.contains(r#""durability": "strict""#));

        let store = new_shared_json_storage_bucket_store(&temp.path)?;
        let store = store.lock();
        assert_eq!(
            store.bucket_durability("https://a.test", "default"),
            Some(StorageBucketDurability::Relaxed)
        );
        assert_eq!(
            store.bucket_durability("https://a.test", "strict"),
            Some(StorageBucketDurability::Strict)
        );
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_persists_bucket_quota_metadata() -> Result<()> {
        let temp = TempStorePath::new("quota");
        {
            let store = new_shared_json_storage_bucket_store(&temp.path)?;
            let mut store = store.lock();
            store.open_bucket("https://a.test", "default")?;
            store.open_bucket_with_options(
                "https://a.test",
                "quota",
                None,
                None,
                Some(4096),
                None,
            )?;
        }

        let json = fs::read_to_string(&temp.path)?;
        assert!(json.contains(r#""quota": 4096"#));

        let store = new_shared_json_storage_bucket_store(&temp.path)?;
        let store = store.lock();
        assert_eq!(store.bucket_quota("https://a.test", "default"), Some(None));
        assert_eq!(
            store.bucket_quota("https://a.test", "quota"),
            Some(Some(4096))
        );
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_persists_bucket_persisted_metadata() -> Result<()> {
        let temp = TempStorePath::new("persisted");
        {
            let store = new_shared_json_storage_bucket_store(&temp.path)?;
            let mut store = store.lock();
            store.open_bucket("https://a.test", "default")?;
            store.open_bucket_with_options(
                "https://a.test",
                "persisted",
                None,
                None,
                None,
                Some(true),
            )?;
        }

        let json = fs::read_to_string(&temp.path)?;
        assert!(json.contains(r#""persisted": true"#));

        let store = new_shared_json_storage_bucket_store(&temp.path)?;
        let store = store.lock();
        assert_eq!(
            store.bucket_persisted("https://a.test", "default"),
            Some(false)
        );
        assert_eq!(
            store.bucket_persisted("https://a.test", "persisted"),
            Some(true)
        );
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_reads_v1_name_lists() -> Result<()> {
        let temp = TempStorePath::new("v1");
        fs::write(
            &temp.path,
            br#"{"version":1,"origins":{"https://a.test":["bucket-b","bucket-a"]}}"#,
        )?;

        {
            let store = new_shared_json_storage_bucket_store(&temp.path)?;
            let mut store = store.lock();
            assert_eq!(store.keys("https://a.test"), vec!["bucket-a", "bucket-b"]);
            assert_eq!(store.bucket_expires("https://a.test", "bucket-a"), None);
            store.set_bucket_expires("https://a.test", "bucket-a", 5678.0)?;
        }

        let json = fs::read_to_string(&temp.path)?;
        assert!(json.contains(r#""version": 5"#));
        assert!(json.contains(r#""bucketId""#));
        assert!(json.contains(r#""nextBucketId""#));
        assert!(json.contains(r#""expires": 5678.0"#));
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_migrates_v2_identity_on_open() -> Result<()> {
        let temp = TempStorePath::new("v2-bucket-id");
        fs::write(
            &temp.path,
            br#"{"version":2,"origins":{"https://a.test":{"bucket-b":{},"bucket-a":{}}}}"#,
        )?;

        let bucket_a_id;
        let bucket_b_id;
        {
            let store = new_shared_json_storage_bucket_store(&temp.path)?;
            let store = store.lock();
            bucket_a_id = store.bucket_id("https://a.test", "bucket-a").unwrap();
            bucket_b_id = store.bucket_id("https://a.test", "bucket-b").unwrap();
            assert_ne!(bucket_a_id, bucket_b_id);
        }

        let json = fs::read_to_string(&temp.path)?;
        assert!(json.contains(r#""version": 5"#));
        assert!(json.contains(r#""nextBucketId": 3"#));
        assert!(json.contains(r#""bucketId""#));

        let store = new_shared_json_storage_bucket_store(&temp.path)?;
        let store = store.lock();
        assert_eq!(
            store.bucket_id("https://a.test", "bucket-a"),
            Some(bucket_a_id)
        );
        assert_eq!(
            store.bucket_id("https://a.test", "bucket-b"),
            Some(bucket_b_id)
        );
        Ok(())
    }

    #[test]
    fn json_v4_migrates_bucket_indexed_db_from_name_to_persistent_id() -> Result<()> {
        let temp = TempStorePath::new("v4-indexed-db-identity");
        let cache_root = temp.cache_root();
        let indexed_db_manager =
            new_indexed_db_manager(Some(temp.indexed_db_root())).map_err(anyhow::Error::msg)?;
        let storage_service = StorageService::on_disk(temp.opfs_root())?;
        let storage_key = "https://a.test";
        let legacy_key = legacy_storage_bucket_indexed_db_storage_key(storage_key, "bucket");
        seed_indexed_db_record(&indexed_db_manager, &legacy_key)?;
        let legacy_usage = indexed_db_usage(&indexed_db_manager, &legacy_key)?;
        fs::write(
            &temp.path,
            br#"{"version":4,"nextBucketId":18,"origins":{"https://a.test":{"bucket":{"bucketId":17}}}}"#,
        )?;

        let store = new_shared_json_storage_bucket_store_with_storage_service(
            &temp.path,
            &cache_root,
            &indexed_db_manager,
            storage_service,
        )?;
        let store = store.lock();
        let identity = store
            .bucket_identity(storage_key, "bucket")
            .expect("migrated bucket should have identity");
        assert_eq!(identity.bucket_id().get(), 17);
        assert_eq!(indexed_db_usage(&indexed_db_manager, &legacy_key)?, 0);
        assert_eq!(
            indexed_db_usage(&indexed_db_manager, &identity.indexed_db_storage_key())?,
            legacy_usage
        );
        drop(store);

        let json: serde_json::Value = serde_json::from_slice(&fs::read(&temp.path)?)?;
        assert_eq!(json["version"], serde_json::json!(5));
        Ok(())
    }

    #[test]
    fn json_v5_pending_deletion_migrates_legacy_idb_before_recovery() -> Result<()> {
        let temp = TempStorePath::new("v5-pending-legacy-indexed-db");
        let cache_root = temp.cache_root();
        let indexed_db_manager =
            new_indexed_db_manager(Some(temp.indexed_db_root())).map_err(anyhow::Error::msg)?;
        let storage_service = StorageService::on_disk(temp.opfs_root())?;
        let storage_key = "https://a.test";
        let identity =
            StorageBucketIdentity::new(storage_key, "bucket", StorageBucketId::new(17).unwrap());
        let legacy_key = legacy_storage_bucket_indexed_db_storage_key(storage_key, "bucket");
        seed_indexed_db_record(&indexed_db_manager, &legacy_key)?;
        let locator = identity.locator();
        let bucket_key = StorageService::opfs_bucket_key(&locator)?;
        let root = storage_service.ensure_opfs_root(&locator)?;
        let file =
            storage_service.with_opfs(|opfs| opfs.get_file(&bucket_key, &root, "old.txt", true))?;
        storage_service
            .with_opfs(|opfs| opfs.write_file(&bucket_key, &file, b"old bucket", None))?;
        fs::write(
            &temp.path,
            br#"{"version":5,"nextBucketId":18,"indexedDbKeysUseBucketIds":false,"cachePathsUseBucketIds":true,"pendingDeletions":[{"storageKey":"https://a.test","name":"bucket","bucketId":17}],"origins":{}}"#,
        )?;

        let store = new_shared_json_storage_bucket_store_with_storage_service(
            &temp.path,
            &cache_root,
            &indexed_db_manager,
            storage_service.clone(),
        )?;
        assert!(store.lock().pending_deletions().is_empty());
        assert_eq!(indexed_db_usage(&indexed_db_manager, &legacy_key)?, 0);
        assert_eq!(
            indexed_db_usage(&indexed_db_manager, &identity.indexed_db_storage_key())?,
            0
        );
        assert_eq!(storage_service.opfs_usage(&locator)?, 0);
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&temp.path)?)?;
        assert_eq!(json["indexedDbKeysUseBucketIds"], serde_json::json!(true));
        assert!(json.get("pendingDeletions").is_none());
        Ok(())
    }

    #[test]
    fn json_v5_resumes_legacy_cache_path_migration() -> Result<()> {
        let temp = TempStorePath::new("v5-cache-path-recovery");
        let cache_root = temp.cache_root();
        let storage_key = "https://a.test";
        let bucket_name = "bucket";
        let bucket_id = StorageBucketId::new(17).unwrap();
        let response = StorageBucketCachedResponse {
            response_type: "default".to_owned(),
            url: "https://a.test/cached".to_owned(),
            redirected: false,
            status: 200,
            status_text: "OK".to_owned(),
            headers: vec![("x-cache".to_owned(), "legacy-path".to_owned())],
            body: b"legacy cache path".to_vec(),
        };
        let entries = BTreeMap::from([(
            "/cached".to_owned(),
            StorageBucketCacheEntry {
                usage_bytes: 17,
                request: StorageBucketCachedRequest::default(),
                response: response.clone(),
                insertion_order: 1,
            },
        )]);
        let legacy_dir =
            legacy_storage_bucket_cache_bucket_dir(&cache_root, storage_key, bucket_name);
        fs::create_dir_all(&legacy_dir)?;
        save_storage_bucket_cache_file(
            &legacy_dir.join(format!(
                "{}.json",
                encode_storage_bucket_cache_component("cache")
            )),
            &entries,
        )?;
        fs::write(
            &temp.path,
            br#"{"version":5,"nextBucketId":18,"indexedDbKeysUseBucketIds":true,"cachePathsUseBucketIds":false,"origins":{"https://a.test":{"bucket":{"bucketId":17}}}}"#,
        )?;

        let store = new_shared_json_storage_bucket_store_with_cache_root(&temp.path, &cache_root)?;
        let mut store = store.lock();
        let identity = store.open_bucket(storage_key, bucket_name)?;
        let matched = store
            .match_cache_entry_for_identity(&identity, "cache", "/cached")
            .flatten()
            .expect("legacy CacheStorage path should be recovered");
        assert_eq!(matched, response);
        drop(store);

        let json: serde_json::Value = serde_json::from_slice(&fs::read(&temp.path)?)?;
        assert_eq!(json["cachePathsUseBucketIds"], serde_json::json!(true));
        assert!(storage_bucket_cache_bucket_dir(&cache_root, storage_key, bucket_id).exists());
        assert!(!legacy_dir.exists());
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_clear_origin_persists() -> Result<()> {
        let temp = TempStorePath::new("clear-origin");
        {
            let store = new_shared_json_storage_bucket_store(&temp.path)?;
            let mut store = store.lock();
            store.open_bucket("https://a.test", "bucket-a")?;
            store.open_bucket("https://b.test", "bucket-b")?;
            store.clear_origin("https://a.test")?;
        }

        let store = new_shared_json_storage_bucket_store(&temp.path)?;
        let store = store.lock();
        assert_eq!(store.keys("https://a.test"), Vec::<String>::new());
        assert_eq!(store.keys("https://b.test"), vec!["bucket-b"]);
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_rejects_unknown_version() -> Result<()> {
        let temp = TempStorePath::new("unknown-version");
        fs::write(
            &temp.path,
            br#"{"version":999,"origins":{"https://a.test":{"bucket":{}}}}"#,
        )?;

        let error = new_shared_json_storage_bucket_store(&temp.path)
            .expect_err("unknown storage bucket store version should fail");
        assert!(
            error
                .to_string()
                .contains("unsupported storage bucket store version 999")
        );
        Ok(())
    }

    #[test]
    fn json_storage_bucket_store_rejects_duplicate_persistent_bucket_ids() -> Result<()> {
        let temp = TempStorePath::new("duplicate-bucket-id");
        fs::write(
            &temp.path,
            br#"{"version":3,"nextBucketId":2,"origins":{"https://a.test":{"one":{"bucketId":1},"two":{"bucketId":1}}}}"#,
        )?;

        let error = new_shared_json_storage_bucket_store(&temp.path)
            .expect_err("duplicate persistent bucket IDs should fail profile open");
        assert!(
            error
                .to_string()
                .contains("duplicate persistent storage bucket ID 1")
        );
        Ok(())
    }
}
