//! Renderer-neutral owner and identity primitives for Moli storage.
//!
//! This crate deliberately has no V8, page, worker, or protocol dependencies.
//! It owns Storage Bucket metadata/CacheStorage profile IO, aggregate quota
//! snapshots, and OPFS dispatch so backend paths never cross into renderer
//! code.

use std::{collections::HashMap, num::NonZeroU64, path::PathBuf, sync::Arc};

mod buckets;
mod io_queue;
mod opfs_leases;
mod quota;
mod web_locks;

pub use web_locks::{
    WebLockClient, WebLockEvent, WebLockEventSink, WebLockInfo, WebLockMode, WebLockQuery,
    WebLockRequest, WebLockRequestId, WebLockRequestKind, WebLockSnapshot, WebLocks,
};

pub use buckets::{
    DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES, IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME,
    SharedStorageBucketIndexedDbManager, SharedStorageBucketRegistry, SharedStorageBucketStore,
    StorageBucketCacheId, StorageBucketCacheMatch, StorageBucketCacheName, StorageBucketCachePut,
    StorageBucketCachePutOutcome, StorageBucketCacheQuery, StorageBucketCachedRequest,
    StorageBucketCachedRequestMetadata, StorageBucketCachedResponse, StorageBucketDurability,
    StorageBucketIdentity, StorageBucketQuotaOwner, StorageBucketRegistry, StorageBucketStore,
    StorageBucketUsageSnapshot, complete_storage_bucket_deletion,
    new_shared_json_storage_bucket_store, new_shared_json_storage_bucket_store_with_cache_root,
    new_shared_json_storage_bucket_store_with_cache_root_and_indexed_db_manager,
    new_shared_json_storage_bucket_store_with_storage_service, new_shared_storage_bucket_store,
    new_shared_storage_bucket_store_with_indexed_db_manager,
    new_shared_storage_bucket_store_with_storage_service,
    new_shared_storage_bucket_store_with_storage_service_and_indexed_db_manager,
    storage_bucket_indexed_db_storage_key, storage_bucket_origin_allows_storage,
    storage_bucket_quota_owner,
};
pub use io_queue::{StorageServiceDispatchError, StorageServiceTaskError};
pub use moli_opfs::{
    DirectoryEntry, EntryKind, FileSnapshot, FileSnapshotIdentity, Opfs, OpfsBucketKey, OpfsError,
    OpfsMutationLease, OpfsPath, OpfsResult, OpfsSyncAccessHandleId, OpfsWritableId,
    SyncAccessMode, WritableCommand, WritableMode,
};
pub use opfs_leases::{
    StorageOpfsMutationLease, StorageOpfsSyncAccessLease, StorageOpfsWritableLease,
};
use parking_lot::Mutex;
pub use quota::StorageQuotaReservation;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct OpfsUniqueIdKey {
    bucket: OpfsBucketKey,
    path: OpfsPath,
    kind: EntryKind,
}

/// Partition-owned storage service shared by renderer-neutral clients.
///
/// The service is the only layer that translates browser bucket identity into
/// an OPFS backend key. Backend host paths never cross this boundary.
#[derive(Clone, Debug)]
pub struct StorageService {
    opfs: Opfs,
    io_queue: io_queue::StorageIoQueue,
    quota_wait_queue: io_queue::StorageIoQueue,
    io_sequence: io_queue::StorageIoSequence,
    quota_coordinator: quota::StorageQuotaCoordinator,
    opfs_unique_ids: Arc<Mutex<HashMap<OpfsUniqueIdKey, String>>>,
    web_locks: WebLocks,
}

/// Shared partition lifetime handle for [`StorageService`].
pub type SharedStorageService = Arc<StorageService>;

impl StorageService {
    /// Create an ephemeral partition storage service.
    pub fn in_memory() -> SharedStorageService {
        Arc::new(Self {
            opfs: Opfs::in_memory(),
            io_queue: io_queue::StorageIoQueue::default(),
            quota_wait_queue: io_queue::StorageIoQueue::default(),
            io_sequence: io_queue::StorageIoSequence::default(),
            quota_coordinator: quota::StorageQuotaCoordinator::default(),
            opfs_unique_ids: Arc::new(Mutex::new(HashMap::new())),
            web_locks: WebLocks::default(),
        })
    }

    /// Create a profile-backed partition storage service.
    pub fn on_disk(opfs_root: impl Into<PathBuf>) -> OpfsResult<SharedStorageService> {
        Ok(Arc::new(Self {
            opfs: Opfs::on_disk(opfs_root)?,
            io_queue: io_queue::StorageIoQueue::default(),
            quota_wait_queue: io_queue::StorageIoQueue::default(),
            io_sequence: io_queue::StorageIoSequence::default(),
            quota_coordinator: quota::StorageQuotaCoordinator::default(),
            opfs_unique_ids: Arc::new(Mutex::new(HashMap::new())),
            web_locks: WebLocks::default(),
        }))
    }

    /// Run one synchronous OPFS operation on the partition's ordered sequence.
    ///
    /// New Promise-returning renderer APIs should prefer [`Self::dispatch_opfs`]
    /// so host filesystem IO does not block the renderer owner. This method is
    /// retained for synchronous Web APIs and migration of existing callers.
    pub fn with_opfs<T>(&self, operation: impl FnOnce(&Opfs) -> T) -> T {
        self.io_sequence.run(|| operation(&self.opfs))
    }

    /// The lock manager belongs to this storage partition, including in-memory
    /// partitions. It is never shared through a process-global origin map.
    pub fn web_locks(&self) -> &WebLocks {
        &self.web_locks
    }

    /// Run an OPFS operation on this partition's serial IO sequence.
    ///
    /// The operation and completion may not touch V8 or renderer-owned state.
    /// A renderer client should use the completion only to enqueue a typed
    /// result back onto the Promise's page or Worker owner task queue.
    pub fn dispatch_opfs<T, Operation, Completion>(
        &self,
        operation: Operation,
        completion: Completion,
    ) -> Result<(), StorageServiceDispatchError>
    where
        T: Send + 'static,
        Operation: FnOnce(&Opfs) -> T + Send + 'static,
        Completion: FnOnce(Result<T, StorageServiceTaskError>) + Send + 'static,
    {
        let opfs = self.opfs.clone();
        let reservation = self.io_sequence.reserve();
        self.io_queue.dispatch(
            move || {
                let _turn = reservation.enter();
                operation(&opfs)
            },
            completion,
        )
    }

    /// Exclusively reserve one bucket's aggregate quota commit window.
    ///
    /// Callers should take fresh usage snapshots only after this returns and
    /// retain the reservation until their backend commit succeeds or rolls
    /// back. The owned guard releases automatically on every error path.
    pub fn reserve_quota_commit(&self, locator: &StorageBucketLocator) -> StorageQuotaReservation {
        self.quota_coordinator.reserve(locator)
    }

    /// Run one synchronous OPFS commit under the bucket quota reservation and
    /// the partition's ordered storage sequence, in that lock order.
    pub fn with_opfs_quota_commit<T>(
        &self,
        locator: &StorageBucketLocator,
        operation: impl FnOnce(&Opfs) -> T,
    ) -> T {
        let _quota = self.reserve_quota_commit(locator);
        self.with_opfs(operation)
    }

    /// Dispatch one OPFS commit under aggregate quota ownership.
    ///
    /// An uncontended reservation is captured synchronously so later OPFS
    /// submissions cannot overtake this commit. A contended reservation waits
    /// on a separate storage worker and does not reserve an OPFS sequence turn;
    /// this lets the current quota holder take the fresh OPFS usage snapshot
    /// it needs before releasing the bucket.
    pub fn dispatch_opfs_quota_commit<T, Operation, Completion>(
        &self,
        locator: StorageBucketLocator,
        operation: Operation,
        completion: Completion,
    ) -> Result<(), StorageServiceDispatchError>
    where
        T: Send + 'static,
        Operation: FnOnce(&Opfs) -> T + Send + 'static,
        Completion: FnOnce(Result<T, StorageServiceTaskError>) + Send + 'static,
    {
        let opfs = self.opfs.clone();
        let quota_coordinator = self.quota_coordinator.clone();
        if let Some(quota) = quota_coordinator.try_reserve(&locator) {
            let reservation = self.io_sequence.reserve();
            return self.io_queue.dispatch(
                move || {
                    let _quota = quota;
                    let _turn = reservation.enter();
                    operation(&opfs)
                },
                completion,
            );
        }

        let io_sequence = self.io_sequence.clone();
        self.quota_wait_queue.dispatch(
            move || {
                let _quota = quota_coordinator.reserve(&locator);
                let _turn = io_sequence.reserve().enter();
                operation(&opfs)
            },
            completion,
        )
    }

    /// Derive a collision-free opaque backend key for one bucket locator.
    pub fn opfs_bucket_key(locator: &StorageBucketLocator) -> OpfsResult<OpfsBucketKey> {
        let storage_key = locator.storage_key();
        let serialized = match locator {
            StorageBucketLocator::Default { .. } => {
                format!("d:{}:{storage_key}", storage_key.len())
            }
            StorageBucketLocator::Named { bucket_id, .. } => {
                format!("n:{}:{storage_key}:{}", storage_key.len(), bucket_id.get())
            }
        };
        OpfsBucketKey::new(serialized)
    }

    /// Materialize and return the virtual root for one bucket.
    pub fn ensure_opfs_root(&self, locator: &StorageBucketLocator) -> OpfsResult<OpfsPath> {
        let key = Self::opfs_bucket_key(locator)?;
        self.with_opfs(|opfs| opfs.ensure_root(&key))
    }

    /// Return the logical OPFS bytes used by one bucket.
    pub fn opfs_usage(&self, locator: &StorageBucketLocator) -> OpfsResult<u64> {
        let key = Self::opfs_bucket_key(locator)?;
        self.with_opfs(|opfs| opfs.usage(&key))
    }

    /// Return committed OPFS usage plus active session growth reservations.
    pub fn opfs_quota_usage(&self, locator: &StorageBucketLocator) -> OpfsResult<u64> {
        let key = Self::opfs_bucket_key(locator)?;
        self.with_opfs(|opfs| opfs.quota_usage(&key))
    }

    /// Revoke active OPFS sessions and atomically clear one exact bucket.
    pub fn clear_opfs_bucket(&self, locator: &StorageBucketLocator) -> OpfsResult<()> {
        let key = Self::opfs_bucket_key(locator)?;
        self.with_opfs(|opfs| opfs.clear_bucket(&key))
    }

    /// Return the partition-session ID assigned to one virtual entry.
    ///
    /// The caller supplies a freshly generated UUID candidate. Exactly one
    /// candidate wins for a `(bucket, path, kind)` key, so concurrent handles
    /// observe the same value without exposing a backend host path.
    pub fn opfs_unique_id_or_insert(
        &self,
        bucket: OpfsBucketKey,
        path: OpfsPath,
        kind: EntryKind,
        candidate: String,
    ) -> String {
        self.opfs_unique_ids
            .lock()
            .entry(OpfsUniqueIdKey { bucket, path, kind })
            .or_insert(candidate)
            .clone()
    }
}

/// Persistent opaque identity for one named storage bucket.
///
/// IDs are scoped by the serialized storage key carried by
/// [`StorageBucketLocator::Named`]. An allocator must never reuse an ID after
/// deleting its bucket, including across profile restarts.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StorageBucketId(NonZeroU64);

impl StorageBucketId {
    /// Build an ID from its persistent integer representation.
    pub const fn new(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Return the persistent integer representation.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Browser-side identity of a default or named storage bucket.
///
/// The variants are intentionally distinct. A named bucket called `default`
/// must never alias the origin's default bucket.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum StorageBucketLocator {
    /// The implicit default bucket for one storage key.
    Default {
        /// Serialized Moli storage key.
        storage_key: String,
    },
    /// A Storage Buckets API bucket with a persistent opaque ID.
    Named {
        /// Serialized Moli storage key that owns the bucket.
        storage_key: String,
        /// Persistent bucket identity, independent of its web-visible name.
        bucket_id: StorageBucketId,
    },
}

impl StorageBucketLocator {
    /// Locate the implicit default bucket for a storage key.
    pub fn default_bucket(storage_key: impl Into<String>) -> Self {
        Self::Default {
            storage_key: storage_key.into(),
        }
    }

    /// Locate a named bucket by its persistent ID.
    pub fn named(storage_key: impl Into<String>, bucket_id: StorageBucketId) -> Self {
        Self::Named {
            storage_key: storage_key.into(),
            bucket_id,
        }
    }

    /// Return the serialized storage key that owns this bucket.
    pub fn storage_key(&self) -> &str {
        match self {
            Self::Default { storage_key } | Self::Named { storage_key, .. } => storage_key,
        }
    }

    /// Return the named bucket ID, or `None` for the default bucket.
    pub const fn bucket_id(&self) -> Option<StorageBucketId> {
        match self {
            Self::Default { .. } => None,
            Self::Named { bucket_id, .. } => Some(*bucket_id),
        }
    }

    /// Return whether this locator addresses the implicit default bucket.
    pub const fn is_default(&self) -> bool {
        matches!(self, Self::Default { .. })
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, thread, time::Duration};

    use super::{
        EntryKind, OpfsError, OpfsPath, StorageBucketId, StorageBucketLocator,
        StorageOpfsMutationLease, StorageOpfsSyncAccessLease, StorageOpfsWritableLease,
        StorageService, StorageServiceTaskError, SyncAccessMode, WritableMode,
    };

    #[test]
    fn default_and_named_default_are_distinct() {
        let storage_key =
            "storage-key:v1;origin=https://example.test;top-level-site=https://example.test";
        let default_bucket = StorageBucketLocator::default_bucket(storage_key);
        let named_default =
            StorageBucketLocator::named(storage_key, StorageBucketId::new(1).unwrap());

        assert_ne!(default_bucket, named_default);
        assert!(default_bucket.is_default());
        assert!(!named_default.is_default());
        assert_eq!(named_default.bucket_id().map(StorageBucketId::get), Some(1));
    }

    #[test]
    fn locator_json_round_trip_preserves_variant_and_id() {
        let locator = StorageBucketLocator::named(
            "storage-key:v1;origin=https://example.test;top-level-site=https://example.test",
            StorageBucketId::new(42).unwrap(),
        );

        let json = serde_json::to_string(&locator).unwrap();
        assert!(json.contains(r#""kind":"named""#));
        assert!(json.contains(r#""bucketId":42"#));
        assert_eq!(
            serde_json::from_str::<StorageBucketLocator>(&json).unwrap(),
            locator
        );
    }

    #[test]
    fn zero_is_not_a_valid_bucket_id() {
        assert_eq!(StorageBucketId::new(0), None);
    }

    #[test]
    fn service_keeps_default_and_named_default_opfs_roots_isolated() {
        let service = StorageService::in_memory();
        let storage_key = "storage-key:v1;origin=https://example.test";
        let implicit = StorageBucketLocator::default_bucket(storage_key);
        let named = StorageBucketLocator::named(storage_key, StorageBucketId::new(1).unwrap());
        let implicit_key = StorageService::opfs_bucket_key(&implicit).unwrap();
        let named_key = StorageService::opfs_bucket_key(&named).unwrap();
        let root = service.ensure_opfs_root(&implicit).unwrap();
        let file = service
            .with_opfs(|opfs| opfs.get_file(&implicit_key, &root, "implicit", true))
            .unwrap();
        service
            .with_opfs(|opfs| opfs.write_file(&implicit_key, &file, b"default", None))
            .unwrap();

        assert_ne!(implicit_key, named_key);
        assert!(
            service
                .with_opfs(|opfs| opfs.read_directory(&named_key, &root))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            service.opfs_usage(&implicit).unwrap(),
            7 + 146 + 2 * u64::try_from("implicit".len()).unwrap()
        );
        assert_eq!(service.opfs_usage(&named).unwrap(), 0);
    }

    #[test]
    fn opfs_unique_ids_are_partition_session_scoped_by_bucket_path_and_kind() {
        let service = StorageService::in_memory();
        let first_bucket = StorageService::opfs_bucket_key(&StorageBucketLocator::default_bucket(
            "storage-key:first",
        ))
        .unwrap();
        let second_bucket = StorageService::opfs_bucket_key(&StorageBucketLocator::default_bucket(
            "storage-key:second",
        ))
        .unwrap();
        let path = OpfsPath::root().child("entry").unwrap();

        assert_eq!(
            service.opfs_unique_id_or_insert(
                first_bucket.clone(),
                path.clone(),
                EntryKind::File,
                "first-file-id".to_owned(),
            ),
            "first-file-id"
        );
        assert_eq!(
            service.opfs_unique_id_or_insert(
                first_bucket.clone(),
                path.clone(),
                EntryKind::File,
                "ignored-candidate".to_owned(),
            ),
            "first-file-id"
        );
        assert_eq!(
            service.opfs_unique_id_or_insert(
                first_bucket,
                path.clone(),
                EntryKind::Directory,
                "directory-id".to_owned(),
            ),
            "directory-id"
        );
        assert_eq!(
            service.opfs_unique_id_or_insert(
                second_bucket,
                path,
                EntryKind::File,
                "second-bucket-id".to_owned(),
            ),
            "second-bucket-id"
        );
    }

    #[test]
    fn opfs_io_queue_runs_off_caller_and_preserves_submission_order() {
        let service = StorageService::in_memory();
        let caller_thread = thread::current().id();
        let (first_started_tx, first_started_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let (second_started_tx, second_started_rx) = mpsc::channel();
        let (completed_tx, completed_rx) = mpsc::channel();

        let first_completed_tx = completed_tx.clone();
        service
            .dispatch_opfs(
                move |_| {
                    first_started_tx.send(thread::current().id()).unwrap();
                    release_first_rx.recv().unwrap();
                    "first"
                },
                move |result| first_completed_tx.send(result.unwrap()).unwrap(),
            )
            .unwrap();
        let storage_thread = first_started_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert_ne!(storage_thread, caller_thread);

        service
            .dispatch_opfs(
                move |_| {
                    second_started_tx.send(thread::current().id()).unwrap();
                    "second"
                },
                move |result| completed_tx.send(result.unwrap()).unwrap(),
            )
            .unwrap();
        assert!(matches!(
            second_started_rx.recv_timeout(Duration::from_millis(25)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));

        release_first_tx.send(()).unwrap();
        assert_eq!(
            completed_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "first"
        );
        assert_eq!(
            second_started_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
            storage_thread
        );
        assert_eq!(
            completed_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "second"
        );
    }

    #[test]
    fn opfs_io_queue_reports_panics_and_keeps_accepting_work() {
        let service = StorageService::in_memory();
        let (panic_tx, panic_rx) = mpsc::channel();
        service
            .dispatch_opfs(
                |_| -> u8 { panic!("intentional storage task panic") },
                move |result| panic_tx.send(result).unwrap(),
            )
            .unwrap();
        assert_eq!(
            panic_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Err(StorageServiceTaskError::Panicked)
        );

        let (success_tx, success_rx) = mpsc::channel();
        service
            .dispatch_opfs(|_| 42, move |result| success_tx.send(result).unwrap())
            .unwrap();
        assert_eq!(
            success_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Ok(42)
        );
    }

    #[test]
    fn dropped_writable_lease_aborts_on_the_ordered_storage_sequence() {
        let service = StorageService::in_memory();
        let locator = StorageBucketLocator::default_bucket(
            "storage-key:v1;origin=https://lease.test;top-level-site=https://lease.test",
        );
        let key = StorageService::opfs_bucket_key(&locator).unwrap();
        let root = service.ensure_opfs_root(&locator).unwrap();
        let file = service
            .with_opfs(|opfs| opfs.get_file(&key, &root, "lease.txt", true))
            .unwrap();
        let writer_id = service
            .with_opfs(|opfs| opfs.create_writable(&key, &file, false, WritableMode::Exclusive))
            .unwrap();

        drop(StorageOpfsWritableLease::new(service.clone(), writer_id));

        // This synchronous reservation is ordered after the lease's abort.
        // Opening a second exclusive writer therefore proves cleanup ran and
        // released the first session's lock.
        let replacement = service
            .with_opfs(|opfs| opfs.create_writable(&key, &file, false, WritableMode::Exclusive))
            .unwrap();
        service
            .with_opfs(|opfs| opfs.abort_writable(replacement))
            .unwrap();
    }

    #[test]
    fn dropped_sync_access_lease_closes_on_the_ordered_storage_sequence() {
        let service = StorageService::in_memory();
        let locator = StorageBucketLocator::default_bucket(
            "storage-key:v1;origin=https://sync-lease.test;top-level-site=https://sync-lease.test",
        );
        let key = StorageService::opfs_bucket_key(&locator).unwrap();
        let root = service.ensure_opfs_root(&locator).unwrap();
        let file = service
            .with_opfs(|opfs| opfs.get_file(&key, &root, "sync-lease.txt", true))
            .unwrap();
        let handle_id = service
            .with_opfs(|opfs| {
                opfs.create_sync_access_handle(&key, &file, SyncAccessMode::Readwrite)
            })
            .unwrap();

        drop(StorageOpfsSyncAccessLease::new(service.clone(), handle_id));

        let replacement = service
            .with_opfs(|opfs| opfs.create_writable(&key, &file, false, WritableMode::Exclusive))
            .unwrap();
        service
            .with_opfs(|opfs| opfs.abort_writable(replacement))
            .unwrap();
    }

    #[test]
    fn dropped_mutation_lease_orders_release_after_already_reserved_work() {
        let service = StorageService::in_memory();
        let locator = StorageBucketLocator::default_bucket(
            "storage-key:v1;origin=https://mutation-lease.test;top-level-site=https://mutation-lease.test",
        );
        let key = StorageService::opfs_bucket_key(&locator).unwrap();
        let root = service.ensure_opfs_root(&locator).unwrap();
        let source = service
            .with_opfs(|opfs| opfs.get_file(&key, &root, "source", true))
            .unwrap();
        let (destination, backend_lease) = service
            .with_opfs(|opfs| {
                opfs.move_entry_with_mutation_lease(
                    &key,
                    &source,
                    EntryKind::File,
                    &root,
                    "destination",
                    None,
                )
            })
            .unwrap();
        let lease = StorageOpfsMutationLease::new(service.clone(), backend_lease);

        let (blocker_started_tx, blocker_started_rx) = mpsc::channel();
        let (release_blocker_tx, release_blocker_rx) = mpsc::channel();
        service
            .dispatch_opfs(
                move |_| {
                    blocker_started_tx.send(()).unwrap();
                    release_blocker_rx.recv().unwrap();
                },
                |_| {},
            )
            .unwrap();
        blocker_started_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();

        let (contender_tx, contender_rx) = mpsc::channel();
        let contender_key = key.clone();
        let contender_path = destination.clone();
        service
            .dispatch_opfs(
                move |opfs| {
                    opfs.create_writable(
                        &contender_key,
                        &contender_path,
                        false,
                        WritableMode::Exclusive,
                    )
                },
                move |result| contender_tx.send(result).unwrap(),
            )
            .unwrap();
        drop(lease);
        release_blocker_tx.send(()).unwrap();

        assert!(matches!(
            contender_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Ok(Err(OpfsError::NoModificationAllowed(_)))
        ));
        let replacement = service
            .with_opfs(|opfs| {
                opfs.create_writable(&key, &destination, false, WritableMode::Exclusive)
            })
            .unwrap();
        service
            .with_opfs(|opfs| opfs.abort_writable(replacement))
            .unwrap();
    }

    #[test]
    fn synchronous_opfs_call_cannot_overtake_reserved_async_work() {
        let service = StorageService::in_memory();
        let (async_started_tx, async_started_rx) = mpsc::channel();
        let (release_async_tx, release_async_rx) = mpsc::channel();
        let (async_completed_tx, async_completed_rx) = mpsc::channel();
        service
            .dispatch_opfs(
                move |_| {
                    async_started_tx.send(()).unwrap();
                    release_async_rx.recv().unwrap();
                },
                move |result| async_completed_tx.send(result).unwrap(),
            )
            .unwrap();
        async_started_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();

        let synchronous_service = service.clone();
        let (synchronous_entered_tx, synchronous_entered_rx) = mpsc::channel();
        let synchronous_thread = thread::spawn(move || {
            synchronous_service.with_opfs(|_| synchronous_entered_tx.send(()).unwrap());
        });
        assert!(matches!(
            synchronous_entered_rx.recv_timeout(Duration::from_millis(25)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));

        release_async_tx.send(()).unwrap();
        async_completed_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        synchronous_entered_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        synchronous_thread.join().unwrap();
    }

    #[test]
    fn uncontended_quota_commit_keeps_order_before_later_opfs_dispatch() {
        let service = StorageService::in_memory();
        let locator = StorageBucketLocator::default_bucket(
            "storage-key:v1;origin=https://quota-submit-order.test",
        );
        let (blocker_started_tx, blocker_started_rx) = mpsc::channel();
        let (release_blocker_tx, release_blocker_rx) = mpsc::channel();
        service
            .dispatch_opfs(
                move |_| {
                    blocker_started_tx.send(()).unwrap();
                    release_blocker_rx.recv().unwrap();
                },
                |_| {},
            )
            .unwrap();
        blocker_started_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();

        let (order_tx, order_rx) = mpsc::channel();
        let quota_order_tx = order_tx.clone();
        service
            .dispatch_opfs_quota_commit(
                locator,
                move |_| quota_order_tx.send("quota").unwrap(),
                |_| {},
            )
            .unwrap();
        service
            .dispatch_opfs(move |_| order_tx.send("barrier").unwrap(), |_| {})
            .unwrap();

        release_blocker_tx.send(()).unwrap();
        assert_eq!(
            order_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "quota"
        );
        assert_eq!(
            order_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "barrier"
        );
    }

    #[test]
    fn quota_waiting_async_commit_does_not_hold_an_opfs_sequence_ticket() {
        let service = StorageService::in_memory();
        let locator = StorageBucketLocator::named(
            "storage-key:v1;origin=https://quota-order.test",
            StorageBucketId::new(1).unwrap(),
        );
        let quota_reservation = service.reserve_quota_commit(&locator);
        let (first_started_tx, first_started_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        service
            .dispatch_opfs(
                move |_| {
                    first_started_tx.send(()).unwrap();
                    release_first_rx.recv().unwrap();
                },
                |_| {},
            )
            .unwrap();
        first_started_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();

        let (quota_commit_tx, quota_commit_rx) = mpsc::channel();
        service
            .dispatch_opfs_quota_commit(
                locator.clone(),
                |_| (),
                move |result| quota_commit_tx.send(result).unwrap(),
            )
            .unwrap();

        let synchronous_service = service.clone();
        let synchronous_locator = locator.clone();
        let (synchronous_ready_tx, synchronous_ready_rx) = mpsc::channel();
        let (synchronous_done_tx, synchronous_done_rx) = mpsc::channel();
        let synchronous_thread = thread::spawn(move || {
            synchronous_ready_tx.send(()).unwrap();
            let usage = synchronous_service.opfs_quota_usage(&synchronous_locator);
            synchronous_done_tx.send(usage).unwrap();
        });
        synchronous_ready_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        release_first_tx.send(()).unwrap();

        assert_eq!(
            synchronous_done_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("quota holder must be able to take a fresh OPFS snapshot")
                .unwrap(),
            0
        );
        drop(quota_reservation);
        quota_commit_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        synchronous_thread.join().unwrap();
    }
}
