use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    num::NonZeroU64,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;

use crate::{
    EntryKind, OpfsBucketKey, OpfsError, OpfsMutationLease, OpfsPath, OpfsResult,
    catalog::{Catalog, ROOT_ENTRY_ID},
    locks::{LockMode, LockTable},
    sessions::{
        OpfsSyncAccessHandleId, OpfsWritableId, SyncAccessMode, SyncAccessSession, WritableCommand,
        WritableMode, WritableSession,
    },
    staging::{
        WritableStaging, cleanup_staging_directory, prepare_staging_directory,
        recover_staging_root, sync_directory,
    },
    sync_backing::{
        SyncBacking, cleanup_sync_directory, prepare_sync_directory, read_sync_recovery_markers,
    },
    validate_name,
};

const CATALOG_FILE_NAME: &str = "catalog.json";
const CATALOG_NEXT_FILE_NAME: &str = "catalog.json.next";
const CATALOG_PREVIOUS_FILE_NAME: &str = "catalog.json.previous";
const CONTENTS_DIR_NAME: &str = "contents";
const MAX_SYNC_FILE_OFFSET: u64 = i64::MAX as u64;
const MAX_SYNC_WRITE_SIZE: u64 = i32::MAX as u64;
const SYNC_VERSION_RESERVATION_SIZE: u64 = 4096;

/// One entry returned by directory iteration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    pub name: String,
    pub kind: EntryKind,
}

/// Immutable snapshot returned by `FileSystemFileHandle.getFile()` adapters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileSnapshot {
    pub name: String,
    pub modified_ms: u64,
    pub bytes: Vec<u8>,
    pub identity: FileSnapshotIdentity,
}

impl FileSnapshot {
    pub fn size(&self) -> u64 {
        u64::try_from(self.bytes.len()).unwrap_or(u64::MAX)
    }
}

/// Opaque identity of one observable OPFS file snapshot.
///
/// The entry identity is the file's incarnation and changes when an entry is
/// removed and recreated. The version identity changes after every observable
/// write. Together they prevent an old `File` snapshot from becoming valid
/// again merely because the same virtual path exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileSnapshotIdentity {
    entry_id: NonZeroU64,
    version_id: NonZeroU64,
}

impl FileSnapshotIdentity {
    /// Rebuild a trusted snapshot identity from renderer-owned clone metadata.
    pub fn from_raw(entry_id: u64, version_id: u64) -> Option<Self> {
        Some(Self {
            entry_id: NonZeroU64::new(entry_id)?,
            version_id: NonZeroU64::new(version_id)?,
        })
    }

    pub const fn entry_id(self) -> u64 {
        self.entry_id.get()
    }

    pub const fn version_id(self) -> u64 {
        self.version_id.get()
    }
}

/// Thread-safe memory or disk-backed OPFS namespace.
#[derive(Clone, Debug)]
pub struct Opfs {
    inner: Arc<OpfsInner>,
}

#[derive(Debug)]
pub(crate) struct OpfsInner {
    disk_root: Option<DiskRoot>,
    buckets: Mutex<BTreeMap<OpfsBucketKey, Arc<Mutex<Bucket>>>>,
    locks: Mutex<LockTable>,
    writers: Mutex<BTreeMap<OpfsWritableId, WritableSession>>,
    sync_handles: Mutex<BTreeMap<OpfsSyncAccessHandleId, SyncAccessSession>>,
    next_owner_id: AtomicU64,
}

#[derive(Debug)]
struct DiskRoot {
    path: PathBuf,
    initialization: Mutex<DiskRootInitialization>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiskRootInitialization {
    Uninitialized,
    Ready,
}

#[derive(Debug)]
struct Bucket {
    disk_dir: Option<PathBuf>,
    catalog: Catalog,
    memory_contents: BTreeMap<u64, Vec<u8>>,
    sync_backings: BTreeMap<u64, SyncBacking>,
}

impl OpfsInner {
    pub(crate) fn release_lock_owner(&self, owner_id: u64) {
        self.locks.lock().release(owner_id);
    }
}

impl Opfs {
    /// Create an ephemeral in-memory OPFS backend.
    pub fn in_memory() -> Self {
        Self::new(None)
    }

    /// Configure a disk-backed OPFS root without touching the host filesystem.
    ///
    /// The first backend operation creates and recovers the root. A failed
    /// initialization remains retryable. Web-controlled names are never
    /// appended directly to this path.
    pub fn on_disk(root: impl Into<PathBuf>) -> OpfsResult<Self> {
        Ok(Self::new(Some(root.into())))
    }

    fn new(root: Option<PathBuf>) -> Self {
        Self {
            inner: Arc::new(OpfsInner {
                disk_root: root.map(DiskRoot::new),
                buckets: Mutex::new(BTreeMap::new()),
                locks: Mutex::new(LockTable::default()),
                writers: Mutex::new(BTreeMap::new()),
                sync_handles: Mutex::new(BTreeMap::new()),
                next_owner_id: AtomicU64::new(1),
            }),
        }
    }

    /// Materialize a bucket root and return the empty virtual path.
    pub fn ensure_root(&self, bucket: &OpfsBucketKey) -> OpfsResult<OpfsPath> {
        self.bucket(bucket)?;
        Ok(OpfsPath::root())
    }

    /// Return the kind of an existing path.
    pub fn entry_kind(&self, bucket: &OpfsBucketKey, path: &OpfsPath) -> OpfsResult<EntryKind> {
        let bucket = self.bucket(bucket)?;
        let bucket = bucket.lock();
        let id = bucket.catalog.resolve(path)?;
        Ok(bucket.catalog.entry(id)?.kind)
    }

    /// Resolve or create a directory child.
    pub fn get_directory(
        &self,
        bucket_key: &OpfsBucketKey,
        parent: &OpfsPath,
        name: &str,
        create: bool,
    ) -> OpfsResult<OpfsPath> {
        self.get_child(bucket_key, parent, name, EntryKind::Directory, create, None)
    }

    /// Resolve or create a directory child under an aggregate bucket limit.
    pub fn get_directory_with_quota(
        &self,
        bucket_key: &OpfsBucketKey,
        parent: &OpfsPath,
        name: &str,
        create: bool,
        max_bucket_usage: u64,
    ) -> OpfsResult<OpfsPath> {
        self.get_child(
            bucket_key,
            parent,
            name,
            EntryKind::Directory,
            create,
            Some(max_bucket_usage),
        )
    }

    /// Resolve or create a file child.
    pub fn get_file(
        &self,
        bucket_key: &OpfsBucketKey,
        parent: &OpfsPath,
        name: &str,
        create: bool,
    ) -> OpfsResult<OpfsPath> {
        self.get_child(bucket_key, parent, name, EntryKind::File, create, None)
    }

    /// Resolve or create a file child under an aggregate bucket limit.
    pub fn get_file_with_quota(
        &self,
        bucket_key: &OpfsBucketKey,
        parent: &OpfsPath,
        name: &str,
        create: bool,
        max_bucket_usage: u64,
    ) -> OpfsResult<OpfsPath> {
        self.get_child(
            bucket_key,
            parent,
            name,
            EntryKind::File,
            create,
            Some(max_bucket_usage),
        )
    }

    fn get_child(
        &self,
        bucket_key: &OpfsBucketKey,
        parent: &OpfsPath,
        name: &str,
        expected: EntryKind,
        create: bool,
        max_bucket_usage: Option<u64>,
    ) -> OpfsResult<OpfsPath> {
        validate_name(name)?;
        let child_path = parent.child(name)?;
        let bucket = self.bucket(bucket_key)?;
        let mut bucket = bucket.lock();
        let parent_id = bucket.catalog.resolve_kind(parent, EntryKind::Directory)?;
        if let Some(child_id) = bucket.catalog.child_id(parent_id, name) {
            let actual = bucket.catalog.entry(child_id)?.kind;
            if actual != expected {
                return Err(OpfsError::TypeMismatch {
                    path: child_path.display(),
                    expected,
                    actual,
                });
            }
            return Ok(child_path);
        }
        if !create {
            return Err(OpfsError::NotFound(child_path.display()));
        }
        let _lock = self.acquire_locks(bucket_key, std::slice::from_ref(&child_path))?;

        let mut next = bucket.catalog.clone();
        let (_, backing_id) = next.create_entry(parent_id, name.to_owned(), expected, now_ms())?;
        self.check_session_adjusted_quota(bucket_key, next.usage(), max_bucket_usage)?;
        if let Some(backing_id) = backing_id
            && let Err(error) = bucket.write_new_content(backing_id, &[])
        {
            let _ = bucket.delete_content(backing_id);
            return Err(error);
        }
        if let Err(error) = bucket.persist_catalog(&next) {
            if let Some(backing_id) = backing_id {
                let _ = bucket.delete_content(backing_id);
            }
            return Err(error);
        }
        bucket.catalog = next;
        Ok(child_path)
    }

    /// Return lexicographically sorted direct children of a directory.
    pub fn read_directory(
        &self,
        bucket_key: &OpfsBucketKey,
        directory: &OpfsPath,
    ) -> OpfsResult<Vec<DirectoryEntry>> {
        let bucket = self.bucket(bucket_key)?;
        let bucket = bucket.lock();
        let directory_id = bucket
            .catalog
            .resolve_kind(directory, EntryKind::Directory)?;
        Ok(bucket
            .catalog
            .children(directory_id)
            .into_iter()
            .map(|(_, entry)| DirectoryEntry {
                name: entry.name.clone(),
                kind: entry.kind,
            })
            .collect())
    }

    /// Read an immutable file snapshot.
    pub fn read_file(
        &self,
        bucket_key: &OpfsBucketKey,
        path: &OpfsPath,
    ) -> OpfsResult<FileSnapshot> {
        let bucket = self.bucket(bucket_key)?;
        let mut bucket = bucket.lock();
        bucket.file_snapshot(path)
    }

    /// Check that a path still names the exact entry incarnation and committed
    /// content version captured by a previous file snapshot.
    pub fn validate_file_snapshot(
        &self,
        bucket_key: &OpfsBucketKey,
        path: &OpfsPath,
        identity: FileSnapshotIdentity,
    ) -> OpfsResult<()> {
        let bucket = self.bucket(bucket_key)?;
        let bucket = bucket.lock();
        let entry_id = match bucket.catalog.resolve_kind(path, EntryKind::File) {
            Ok(entry_id) => entry_id,
            Err(OpfsError::NotFound(_) | OpfsError::TypeMismatch { .. }) => {
                return Err(invalid_file_snapshot_error());
            }
            Err(error) => return Err(error),
        };
        let entry = bucket.catalog.entry(entry_id)?;
        if entry_id != identity.entry_id() || entry.version_id != Some(identity.version_id()) {
            return Err(invalid_file_snapshot_error());
        }
        Ok(())
    }

    /// Atomically replace a file's bytes.
    pub fn write_file(
        &self,
        bucket_key: &OpfsBucketKey,
        path: &OpfsPath,
        bytes: &[u8],
        max_bucket_usage: Option<u64>,
    ) -> OpfsResult<()> {
        let bucket = self.bucket(bucket_key)?;
        let mut bucket = bucket.lock();
        let _lock = self.acquire_locks(bucket_key, std::slice::from_ref(path))?;
        let entry_id = bucket.catalog.resolve_kind(path, EntryKind::File)?;
        let old_size = bucket.catalog.entry(entry_id)?.size;
        let new_size = u64::try_from(bytes.len())
            .map_err(|_| OpfsError::InvalidModification("file size exceeds u64".to_owned()))?;
        let next_usage = bucket
            .catalog
            .usage()
            .saturating_sub(old_size)
            .saturating_add(new_size);
        self.check_session_adjusted_quota(bucket_key, next_usage, max_bucket_usage)?;
        Self::replace_file_in_bucket(&mut bucket, path, bytes, max_bucket_usage)
    }

    /// Remove one file or directory subtree.
    pub fn remove_entry(
        &self,
        bucket_key: &OpfsBucketKey,
        path: &OpfsPath,
        recursive: bool,
    ) -> OpfsResult<()> {
        drop(self.remove_entry_with_mutation_lease(bucket_key, path, recursive)?);
        Ok(())
    }

    /// Remove one subtree while retaining its exclusive lock for a Promise
    /// completion owner. The caller releases the lock by dropping the lease.
    pub fn remove_entry_with_mutation_lease(
        &self,
        bucket_key: &OpfsBucketKey,
        path: &OpfsPath,
        recursive: bool,
    ) -> OpfsResult<OpfsMutationLease> {
        if path.is_root() {
            return Err(OpfsError::InvalidModification(
                "the OPFS root cannot be removed".to_owned(),
            ));
        }
        let bucket = self.bucket(bucket_key)?;
        let mut bucket = bucket.lock();
        let lease = self.acquire_locks(bucket_key, std::slice::from_ref(path))?;
        let entry_id = bucket.catalog.resolve(path)?;
        let entry = bucket.catalog.entry(entry_id)?;
        if entry.kind == EntryKind::Directory
            && !recursive
            && !bucket.catalog.children(entry_id).is_empty()
        {
            return Err(OpfsError::DirectoryNotEmpty(path.display()));
        }
        let mut next = bucket.catalog.clone();
        let backing_ids = next.remove_subtree(entry_id)?;
        bucket.persist_catalog(&next)?;
        bucket.catalog = next;
        for backing_id in backing_ids {
            let _ = bucket.delete_content(backing_id);
        }
        Ok(lease)
    }

    /// Move/rename a file or directory and return its new virtual path.
    pub fn move_entry(
        &self,
        bucket_key: &OpfsBucketKey,
        source: &OpfsPath,
        expected_kind: EntryKind,
        destination_parent: &OpfsPath,
        new_name: &str,
        max_bucket_usage: Option<u64>,
    ) -> OpfsResult<OpfsPath> {
        let (destination, lease) = self.move_entry_with_mutation_lease(
            bucket_key,
            source,
            expected_kind,
            destination_parent,
            new_name,
            max_bucket_usage,
        )?;
        drop(lease);
        Ok(destination)
    }

    /// Move or rename an entry while retaining the source/destination locks
    /// for a Promise completion owner. The caller releases both by dropping
    /// the returned lease.
    pub fn move_entry_with_mutation_lease(
        &self,
        bucket_key: &OpfsBucketKey,
        source: &OpfsPath,
        expected_kind: EntryKind,
        destination_parent: &OpfsPath,
        new_name: &str,
        max_bucket_usage: Option<u64>,
    ) -> OpfsResult<(OpfsPath, OpfsMutationLease)> {
        if source.is_root() {
            return Err(OpfsError::InvalidModification(
                "the OPFS root cannot be moved".to_owned(),
            ));
        }
        validate_name(new_name)?;
        let destination = destination_parent.child(new_name)?;
        let bucket = self.bucket(bucket_key)?;
        let mut bucket = bucket.lock();
        let lease = self.acquire_locks(bucket_key, &[source.clone(), destination.clone()])?;
        let source_id = bucket.catalog.resolve_kind(source, expected_kind)?;
        if expected_kind == EntryKind::Directory
            && source.is_ancestor_of_or_equal(destination_parent)
        {
            return Err(OpfsError::InvalidModification(
                "a directory cannot be moved into itself".to_owned(),
            ));
        }
        if source == &destination {
            return Ok((destination, lease));
        }
        let destination_parent_id = bucket
            .catalog
            .resolve_kind(destination_parent, EntryKind::Directory)?;

        let mut next = bucket.catalog.clone();
        let mut replaced_backing_ids = Vec::new();
        if let Some(destination_id) = next.child_id(destination_parent_id, new_name) {
            let destination_entry = next.entry(destination_id)?;
            if destination_entry.kind != expected_kind {
                return Err(OpfsError::TypeMismatch {
                    path: destination.display(),
                    expected: expected_kind,
                    actual: destination_entry.kind,
                });
            }
            if destination_entry.kind == EntryKind::Directory
                && !next.children(destination_id).is_empty()
            {
                return Err(OpfsError::DirectoryNotEmpty(destination.display()));
            }
            replaced_backing_ids = next.remove_subtree(destination_id)?;
        }
        let source_entry = next.entry_mut(source_id)?;
        source_entry.parent_id = destination_parent_id;
        source_entry.name = new_name.to_owned();
        source_entry.modified_ms = now_ms();
        self.check_session_adjusted_quota(bucket_key, next.usage(), max_bucket_usage)?;
        bucket.persist_catalog(&next)?;
        bucket.catalog = next;
        for backing_id in replaced_backing_ids {
            let _ = bucket.delete_content(backing_id);
        }
        Ok((destination, lease))
    }

    /// Resolve `target` relative to `base`, returning `None` when it lies
    /// outside the base subtree.
    pub fn resolve(
        &self,
        bucket_key: &OpfsBucketKey,
        base: &OpfsPath,
        target: &OpfsPath,
    ) -> OpfsResult<Option<Vec<String>>> {
        let bucket = self.bucket(bucket_key)?;
        let bucket = bucket.lock();
        bucket.catalog.resolve_kind(base, EntryKind::Directory)?;
        bucket.catalog.resolve(target)?;
        Ok(target
            .components()
            .strip_prefix(base.components())
            .map(<[String]>::to_vec))
    }

    /// Return the logical bytes used by one bucket.
    pub fn usage(&self, bucket_key: &OpfsBucketKey) -> OpfsResult<u64> {
        let bucket = self.bucket(bucket_key)?;
        let usage = bucket.lock().catalog.usage();
        Ok(usage)
    }

    /// Return live usage plus conservative growth reservations held by active
    /// atomic writable sessions.
    ///
    /// Competing siloed writers each reserve their positive growth until one
    /// commits or aborts. Shrinks are not credited before commit, so another
    /// backend cannot consume capacity which is not durable yet.
    pub fn quota_usage(&self, bucket_key: &OpfsBucketKey) -> OpfsResult<u64> {
        let bucket = self.bucket(bucket_key)?;
        let bucket = bucket.lock();
        let writers = self.inner.writers.lock();
        Ok(projected_bucket_quota_usage(
            bucket_key,
            bucket.catalog.usage(),
            &writers,
            None,
        ))
    }

    /// Open an atomic asynchronous writer.
    pub fn create_writable(
        &self,
        bucket_key: &OpfsBucketKey,
        path: &OpfsPath,
        keep_existing_data: bool,
        mode: WritableMode,
    ) -> OpfsResult<OpfsWritableId> {
        let bucket = self.bucket(bucket_key)?;
        let mut bucket = bucket.lock();
        let owner_id = self.take_owner_id()?;
        let lock_mode = match mode {
            WritableMode::Siloed => LockMode::SiloedWriter,
            WritableMode::Exclusive => LockMode::Exclusive,
        };
        self.inner
            .locks
            .lock()
            .acquire(owner_id, bucket_key, path, lock_mode)?;
        let (staging, committed_size) =
            match bucket.create_writable_staging(owner_id, path, keep_existing_data) {
                Ok(staging) => staging,
                Err(error) => {
                    self.inner.locks.lock().release(owner_id);
                    return Err(error);
                }
            };
        self.inner.writers.lock().insert(
            OpfsWritableId(owner_id),
            WritableSession {
                bucket: bucket_key.clone(),
                path: path.clone(),
                cursor: 0,
                staging,
                committed_size,
            },
        );
        Ok(OpfsWritableId(owner_id))
    }

    /// Apply a write/seek/truncate command to an active writer staging buffer.
    pub fn writable_command(
        &self,
        id: OpfsWritableId,
        command: WritableCommand,
        max_bucket_usage: Option<u64>,
    ) -> OpfsResult<()> {
        let bucket_key = self
            .inner
            .writers
            .lock()
            .get(&id)
            .map(|session| session.bucket.clone())
            .ok_or(OpfsError::InvalidState)?;
        let bucket = self.bucket(&bucket_key)?;
        let bucket = bucket.lock();
        let mut writers = self.inner.writers.lock();
        let next_len = {
            let session = writers.get(&id).ok_or(OpfsError::InvalidState)?;
            session.staging.projected_length(session.cursor, &command)?
        };
        let next_len = u64::try_from(next_len)
            .map_err(|_| OpfsError::InvalidModification("writer size exceeds u64".to_owned()))?;
        let projected_usage = projected_bucket_quota_usage(
            &bucket_key,
            bucket.catalog.usage(),
            &writers,
            Some((id, next_len)),
        );
        check_quota(0, 0, projected_usage, max_bucket_usage)?;
        let session = writers.get_mut(&id).ok_or(OpfsError::InvalidState)?;
        let mut cursor = session.cursor;
        session.staging.apply(&mut cursor, command)?;
        session.cursor = cursor;
        Ok(())
    }

    /// Commit a writer atomically and release its lock.
    pub fn close_writable(
        &self,
        id: OpfsWritableId,
        max_bucket_usage: Option<u64>,
    ) -> OpfsResult<()> {
        let bucket_key = self
            .inner
            .writers
            .lock()
            .get(&id)
            .map(|session| session.bucket.clone())
            .ok_or(OpfsError::InvalidState)?;
        let bucket = self.bucket(&bucket_key)?;
        let mut bucket = bucket.lock();
        let mut writers = self.inner.writers.lock();
        let quota_result = check_quota(
            0,
            0,
            projected_bucket_quota_usage(&bucket_key, bucket.catalog.usage(), &writers, None),
            max_bucket_usage,
        );
        let session = writers.remove(&id).ok_or(OpfsError::InvalidState)?;
        drop(writers);
        if let Err(error) = quota_result {
            let _ = session.staging.discard();
            drop(bucket);
            self.inner.locks.lock().release(id.0);
            return Err(error);
        }
        let result = Self::replace_staging_file_in_bucket(
            &mut bucket,
            &session.path,
            session.staging,
            max_bucket_usage,
        );
        drop(bucket);
        self.inner.locks.lock().release(id.0);
        result
    }

    /// Abort a writer without changing the target.
    pub fn abort_writable(&self, id: OpfsWritableId) -> OpfsResult<()> {
        let session = self
            .inner
            .writers
            .lock()
            .remove(&id)
            .ok_or(OpfsError::InvalidState)?;
        let result = session.staging.discard();
        self.inner.locks.lock().release(id.0);
        result
    }

    /// Open a synchronous access session with the mode's hierarchical lock.
    pub fn create_sync_access_handle(
        &self,
        bucket_key: &OpfsBucketKey,
        path: &OpfsPath,
        mode: SyncAccessMode,
    ) -> OpfsResult<OpfsSyncAccessHandleId> {
        let bucket = self.bucket(bucket_key)?;
        let mut bucket = bucket.lock();
        let owner_id = self.take_owner_id()?;
        let lock_mode = match mode {
            SyncAccessMode::Readwrite => LockMode::Exclusive,
            SyncAccessMode::ReadOnly => LockMode::SyncReadOnly,
            SyncAccessMode::ReadwriteUnsafe => LockMode::SyncReadwriteUnsafe,
        };
        self.inner
            .locks
            .lock()
            .acquire(owner_id, bucket_key, path, lock_mode)?;
        let entry_id = match bucket.catalog.resolve_kind(path, EntryKind::File) {
            Ok(entry_id) => entry_id,
            Err(error) => {
                self.inner.locks.lock().release(owner_id);
                return Err(error);
            }
        };
        let backing_id = match bucket.open_sync_backing(entry_id, mode != SyncAccessMode::ReadOnly)
        {
            Ok(backing_id) => backing_id,
            Err(error) => {
                self.inner.locks.lock().release(owner_id);
                return Err(error);
            }
        };
        self.inner.sync_handles.lock().insert(
            OpfsSyncAccessHandleId(owner_id),
            SyncAccessSession {
                bucket: bucket_key.clone(),
                path: path.clone(),
                entry_id,
                backing_id,
                cursor: 0,
                mode,
            },
        );
        Ok(OpfsSyncAccessHandleId(owner_id))
    }

    /// Read bytes from a sync session and advance its cursor.
    pub fn sync_read(
        &self,
        id: OpfsSyncAccessHandleId,
        length: usize,
        at: Option<u64>,
    ) -> OpfsResult<Vec<u8>> {
        let bucket_key = self.sync_bucket(id)?;
        let bucket = self.bucket(&bucket_key)?;
        let mut bucket = bucket.lock();
        let mut handles = self.inner.sync_handles.lock();
        let (backing_id, position) = handles
            .get(&id)
            .map(|session| (session.backing_id, at.unwrap_or(session.cursor)))
            .ok_or(OpfsError::InvalidState)?;
        validate_sync_offset(position, "read")?;
        let bytes = bucket.sync_backing_read(backing_id, position, length)?;
        let session = handles.get_mut(&id).ok_or(OpfsError::InvalidState)?;
        session.cursor = position.saturating_add(bytes.len() as u64);
        Ok(bytes)
    }

    /// Write bytes into a sync session and advance its cursor.
    pub fn sync_write(
        &self,
        id: OpfsSyncAccessHandleId,
        bytes: &[u8],
        at: Option<u64>,
        max_bucket_usage: Option<u64>,
    ) -> OpfsResult<usize> {
        let bucket_key = self.sync_bucket(id)?;
        let bucket = self.bucket(&bucket_key)?;
        let mut bucket = bucket.lock();
        let writers = self.inner.writers.lock();
        let mut handles = self.inner.sync_handles.lock();
        let (mode, path, entry_id, backing_id, position) = handles
            .get(&id)
            .map(|session| {
                (
                    session.mode,
                    session.path.clone(),
                    session.entry_id,
                    session.backing_id,
                    at.unwrap_or(session.cursor),
                )
            })
            .ok_or(OpfsError::InvalidState)?;
        if mode == SyncAccessMode::ReadOnly {
            return Err(OpfsError::NoModificationAllowed(path.display()));
        }
        let byte_len = u64::try_from(bytes.len()).map_err(|_| {
            OpfsError::InvalidModification("sync write length exceeds u64".to_owned())
        })?;
        validate_sync_offset(position, "write")?;
        if byte_len > MAX_SYNC_WRITE_SIZE {
            return Err(OpfsError::InvalidModification(
                "sync write exceeds the signed 32-bit host operation limit".to_owned(),
            ));
        }
        let end = position
            .checked_add(byte_len)
            .ok_or(OpfsError::QuotaExceeded {
                quota: MAX_SYNC_FILE_OFFSET,
                requested: u64::MAX,
            })?;
        if end > MAX_SYNC_FILE_OFFSET {
            return Err(OpfsError::QuotaExceeded {
                quota: MAX_SYNC_FILE_OFFSET,
                requested: end,
            });
        }
        let current_size = bucket.sync_backing_len(backing_id)?;
        let projected_size = if bytes.is_empty() {
            current_size
        } else {
            current_size.max(end)
        };
        let entry_size = bucket.catalog.entry(entry_id)?.size;
        let projected_live_usage = bucket
            .catalog
            .usage()
            .saturating_sub(entry_size)
            .saturating_add(projected_size);
        let projected_usage =
            projected_bucket_quota_usage(&bucket_key, projected_live_usage, &writers, None);
        check_quota(0, 0, projected_usage, max_bucket_usage)?;
        drop(writers);
        let version_id = if bytes.is_empty() {
            None
        } else {
            Some(bucket.prepare_sync_mutation(entry_id, backing_id)?)
        };
        let written = bucket.sync_backing_write(backing_id, position, bytes)?;
        if let Some(version_id) = version_id
            && written != 0
        {
            let actual_size = bucket.sync_backing_len(backing_id)?;
            bucket.record_sync_mutation(entry_id, backing_id, version_id, actual_size)?;
        }
        let session = handles.get_mut(&id).ok_or(OpfsError::InvalidState)?;
        session.cursor = position.saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    /// Truncate/extend a sync session with zero-filled bytes.
    pub fn sync_truncate(
        &self,
        id: OpfsSyncAccessHandleId,
        size: u64,
        max_bucket_usage: Option<u64>,
    ) -> OpfsResult<()> {
        let bucket_key = self.sync_bucket(id)?;
        let bucket = self.bucket(&bucket_key)?;
        let mut bucket = bucket.lock();
        let writers = self.inner.writers.lock();
        let mut handles = self.inner.sync_handles.lock();
        let (mode, path, entry_id, backing_id) = handles
            .get(&id)
            .map(|session| {
                (
                    session.mode,
                    session.path.clone(),
                    session.entry_id,
                    session.backing_id,
                )
            })
            .ok_or(OpfsError::InvalidState)?;
        if mode == SyncAccessMode::ReadOnly {
            return Err(OpfsError::NoModificationAllowed(path.display()));
        }
        validate_sync_offset(size, "truncate")?;
        let entry_size = bucket.catalog.entry(entry_id)?.size;
        let projected_live_usage = bucket
            .catalog
            .usage()
            .saturating_sub(entry_size)
            .saturating_add(size);
        let projected_usage =
            projected_bucket_quota_usage(&bucket_key, projected_live_usage, &writers, None);
        check_quota(0, 0, projected_usage, max_bucket_usage)?;
        drop(writers);
        let version_id = bucket.prepare_sync_mutation(entry_id, backing_id)?;
        bucket.sync_backing_truncate(backing_id, size)?;
        bucket.record_sync_mutation(entry_id, backing_id, version_id, size)?;
        let session = handles.get_mut(&id).ok_or(OpfsError::InvalidState)?;
        session.cursor = session.cursor.min(size);
        Ok(())
    }

    /// Return the staged sync file size.
    pub fn sync_size(&self, id: OpfsSyncAccessHandleId) -> OpfsResult<u64> {
        let bucket_key = self.sync_bucket(id)?;
        let bucket = self.bucket(&bucket_key)?;
        let bucket = bucket.lock();
        let backing_id = self
            .inner
            .sync_handles
            .lock()
            .get(&id)
            .map(|session| session.backing_id)
            .ok_or(OpfsError::InvalidState)?;
        bucket.sync_backing_len(backing_id)
    }

    /// Make direct sync writes durable while keeping the handle open.
    pub fn flush_sync(
        &self,
        id: OpfsSyncAccessHandleId,
        _max_bucket_usage: Option<u64>,
    ) -> OpfsResult<()> {
        let bucket_key = self.sync_bucket(id)?;
        let bucket = self.bucket(&bucket_key)?;
        let mut bucket = bucket.lock();
        let handles = self.inner.sync_handles.lock();
        let (mode, path, backing_id) = handles
            .get(&id)
            .map(|session| (session.mode, session.path.clone(), session.backing_id))
            .ok_or(OpfsError::InvalidState)?;
        if mode == SyncAccessMode::ReadOnly {
            return Err(OpfsError::NoModificationAllowed(path.display()));
        }
        bucket.flush_sync_backing(backing_id)
    }

    /// Close and release a sync handle. Direct writes are already visible.
    pub fn close_sync(
        &self,
        id: OpfsSyncAccessHandleId,
        _max_bucket_usage: Option<u64>,
    ) -> OpfsResult<()> {
        let bucket_key = self.sync_bucket(id)?;
        let bucket = self.bucket(&bucket_key)?;
        let mut bucket = bucket.lock();
        let mut handles = self.inner.sync_handles.lock();
        let session = handles.remove(&id).ok_or(OpfsError::InvalidState)?;
        drop(handles);
        let result = bucket.close_sync_backing(session.backing_id);
        drop(bucket);
        self.inner.locks.lock().release(id.0);
        result
    }

    /// Remove all data and active sessions for one exact opaque bucket.
    pub fn clear_bucket(&self, bucket_key: &OpfsBucketKey) -> OpfsResult<()> {
        let bucket = self.bucket(bucket_key)?;
        let mut bucket = bucket.lock();
        let writer_ids = self
            .inner
            .writers
            .lock()
            .iter()
            .filter_map(|(id, session)| (session.bucket == *bucket_key).then_some(*id))
            .collect::<Vec<_>>();
        let mut cleanup_error = None;
        for id in writer_ids {
            if let Some(session) = self.inner.writers.lock().remove(&id)
                && let Err(error) = session.staging.discard()
                && cleanup_error.is_none()
            {
                cleanup_error = Some(error);
            }
        }
        self.inner
            .sync_handles
            .lock()
            .retain(|_, session| session.bucket != *bucket_key);
        self.inner.locks.lock().release_bucket(bucket_key);
        let old_backing_ids = bucket.catalog.backing_ids();
        let next = bucket.catalog.cleared(now_ms());
        bucket.persist_catalog(&next)?;
        bucket.catalog = next;
        if let Err(error) = bucket.discard_sync_backings()
            && cleanup_error.is_none()
        {
            cleanup_error = Some(error);
        }
        bucket.memory_contents.clear();
        for backing_id in old_backing_ids {
            if let Err(error) = bucket.delete_content(backing_id)
                && cleanup_error.is_none()
            {
                cleanup_error = Some(error);
            }
        }
        if let Err(error) = bucket.garbage_collect_disk_contents()
            && cleanup_error.is_none()
        {
            cleanup_error = Some(error);
        }
        if let Err(error) = bucket.cleanup_staging()
            && cleanup_error.is_none()
        {
            cleanup_error = Some(error);
        }
        if let Some(error) = cleanup_error {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn replace_staging_file_in_bucket(
        bucket: &mut Bucket,
        path: &OpfsPath,
        staging: WritableStaging,
        max_bucket_usage: Option<u64>,
    ) -> OpfsResult<()> {
        let entry_id = bucket.catalog.resolve_kind(path, EntryKind::File)?;
        let old_size = bucket.catalog.entry(entry_id)?.size;
        let new_size = u64::try_from(staging.len())
            .map_err(|_| OpfsError::InvalidModification("file size exceeds u64".to_owned()))?;
        check_quota(bucket.catalog.usage(), old_size, new_size, max_bucket_usage)?;
        let mut next = bucket.catalog.clone();
        let (old_backing_id, new_backing_id) =
            next.replace_file_content(entry_id, new_size, now_ms())?;
        if let Err(error) = bucket.install_staging_content(new_backing_id, staging) {
            let _ = bucket.delete_content(new_backing_id);
            return Err(error);
        }
        #[cfg(test)]
        crate::fault_injection::crash_if_armed(
            crate::fault_injection::CrashPoint::ContentInstalledBeforeCatalog,
            || {},
        );
        if let Err(error) = bucket.persist_catalog(&next) {
            let _ = bucket.delete_content(new_backing_id);
            return Err(error);
        }
        #[cfg(test)]
        crate::fault_injection::crash_if_armed(
            crate::fault_injection::CrashPoint::CatalogCommittedBeforeMemorySwap,
            || {},
        );
        bucket.catalog = next;
        #[cfg(test)]
        crate::fault_injection::crash_if_armed(
            crate::fault_injection::CrashPoint::CatalogMemorySwappedBeforeOldContentDelete,
            || {},
        );
        let _ = bucket.delete_content(old_backing_id);
        Ok(())
    }

    fn replace_file_in_bucket(
        bucket: &mut Bucket,
        path: &OpfsPath,
        bytes: &[u8],
        max_bucket_usage: Option<u64>,
    ) -> OpfsResult<()> {
        let entry_id = bucket.catalog.resolve_kind(path, EntryKind::File)?;
        let old_size = bucket.catalog.entry(entry_id)?.size;
        check_quota(
            bucket.catalog.usage(),
            old_size,
            u64::try_from(bytes.len())
                .map_err(|_| OpfsError::InvalidModification("file size exceeds u64".to_owned()))?,
            max_bucket_usage,
        )?;
        let mut next = bucket.catalog.clone();
        let (old_backing_id, new_backing_id) = next.replace_file_content(
            entry_id,
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            now_ms(),
        )?;
        if let Err(error) = bucket.write_new_content(new_backing_id, bytes) {
            let _ = bucket.delete_content(new_backing_id);
            return Err(error);
        }
        if let Err(error) = bucket.persist_catalog(&next) {
            let _ = bucket.delete_content(new_backing_id);
            return Err(error);
        }
        bucket.catalog = next;
        let _ = bucket.delete_content(old_backing_id);
        Ok(())
    }

    fn sync_bucket(&self, id: OpfsSyncAccessHandleId) -> OpfsResult<OpfsBucketKey> {
        self.inner
            .sync_handles
            .lock()
            .get(&id)
            .map(|session| session.bucket.clone())
            .ok_or(OpfsError::InvalidState)
    }

    fn check_session_adjusted_quota(
        &self,
        bucket_key: &OpfsBucketKey,
        next_committed_usage: u64,
        max_bucket_usage: Option<u64>,
    ) -> OpfsResult<()> {
        if max_bucket_usage.is_none() {
            return Ok(());
        }
        let writers = self.inner.writers.lock();
        check_quota(
            0,
            0,
            projected_bucket_quota_usage(bucket_key, next_committed_usage, &writers, None),
            max_bucket_usage,
        )
    }

    fn acquire_locks(
        &self,
        bucket: &OpfsBucketKey,
        paths: &[OpfsPath],
    ) -> OpfsResult<OpfsMutationLease> {
        let owner_id = self.take_owner_id()?;
        self.inner
            .locks
            .lock()
            .acquire_many(owner_id, bucket, paths, LockMode::Exclusive)?;
        Ok(OpfsMutationLease::new(self.inner.clone(), owner_id))
    }

    fn take_owner_id(&self) -> OpfsResult<u64> {
        self.inner
            .next_owner_id
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| OpfsError::InvalidState)
    }

    fn bucket(&self, key: &OpfsBucketKey) -> OpfsResult<Arc<Mutex<Bucket>>> {
        let mut buckets = self.inner.buckets.lock();
        if let Some(bucket) = buckets.get(key).cloned() {
            return Ok(bucket);
        }
        let root = self
            .inner
            .disk_root
            .as_ref()
            .map(DiskRoot::ensure_initialized)
            .transpose()?;
        let bucket = Arc::new(Mutex::new(Bucket::open(root, key)?));
        buckets.insert(key.clone(), bucket.clone());
        Ok(bucket)
    }
}

impl DiskRoot {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            initialization: Mutex::new(DiskRootInitialization::Uninitialized),
        }
    }

    fn ensure_initialized(&self) -> OpfsResult<&Path> {
        let mut initialization = self.initialization.lock();
        if *initialization == DiskRootInitialization::Ready {
            return Ok(&self.path);
        }
        fs::create_dir_all(&self.path)
            .map_err(|source| OpfsError::io("create root directory", &self.path, source))?;
        recover_staging_root(&self.path)?;
        *initialization = DiskRootInitialization::Ready;
        Ok(&self.path)
    }
}

impl Bucket {
    fn open(root: Option<&Path>, key: &OpfsBucketKey) -> OpfsResult<Self> {
        let Some(root) = root else {
            return Ok(Self {
                disk_dir: None,
                catalog: Catalog::new(now_ms()),
                memory_contents: BTreeMap::new(),
                sync_backings: BTreeMap::new(),
            });
        };
        let disk_dir = disk_bucket_dir(root, key);
        let contents_dir = disk_dir.join(CONTENTS_DIR_NAME);
        fs::create_dir_all(&contents_dir).map_err(|source| {
            OpfsError::io("create bucket contents directory", &contents_dir, source)
        })?;
        prepare_staging_directory(&disk_dir)?;
        prepare_sync_directory(&disk_dir)?;
        cleanup_staging_directory(&disk_dir)?;
        recover_catalog_replacement(&disk_dir)?;
        let catalog_path = disk_dir.join(CATALOG_FILE_NAME);
        let mut catalog = if catalog_path.exists() {
            let bytes = fs::read(&catalog_path)
                .map_err(|source| OpfsError::io("read catalog", &catalog_path, source))?;
            Catalog::from_bytes(&bytes)?
        } else {
            let catalog = Catalog::new(now_ms());
            persist_catalog_to_disk(&disk_dir, &catalog)?;
            catalog
        };
        let mut recovered_sync_mutation = false;
        for marker in read_sync_recovery_markers(&disk_dir)? {
            let Ok(entry) = catalog.entry(marker.entry_id) else {
                continue;
            };
            if entry.kind != EntryKind::File || entry.backing_id != Some(marker.backing_id) {
                continue;
            }
            let path = backing_path(&disk_dir, marker.backing_id);
            let size = fs::metadata(&path)
                .map_err(|source| OpfsError::io("stat sync recovery backing", &path, source))?
                .len();
            catalog.record_in_place_mutation(marker.entry_id, size, now_ms())?;
            recovered_sync_mutation = true;
        }
        if recovered_sync_mutation {
            persist_catalog_to_disk(&disk_dir, &catalog)?;
        }
        cleanup_sync_directory(&disk_dir)?;
        let bucket = Self {
            disk_dir: Some(disk_dir),
            catalog,
            memory_contents: BTreeMap::new(),
            sync_backings: BTreeMap::new(),
        };
        bucket.validate_disk_contents()?;
        bucket.garbage_collect_disk_contents()?;
        Ok(bucket)
    }

    fn create_writable_staging(
        &mut self,
        owner_id: u64,
        path: &OpfsPath,
        keep_existing_data: bool,
    ) -> OpfsResult<(WritableStaging, u64)> {
        let entry_id = self.catalog.resolve_kind(path, EntryKind::File)?;
        let (backing_id, committed_size) = {
            let entry = self.catalog.entry(entry_id)?;
            let backing_id = entry.backing_id.ok_or_else(|| {
                OpfsError::CorruptCatalog(format!("file entry {entry_id} has no backing id"))
            })?;
            (backing_id, entry.size)
        };
        let staging = if let Some(disk_dir) = &self.disk_dir {
            let source = keep_existing_data.then(|| backing_path(disk_dir, backing_id));
            WritableStaging::disk(disk_dir, owner_id, source.as_deref())?
        } else {
            WritableStaging::memory(if keep_existing_data {
                self.read_content(backing_id)?
            } else {
                Vec::new()
            })
        };
        if keep_existing_data && u64::try_from(staging.len()).unwrap_or(u64::MAX) != committed_size
        {
            let staged_size = staging.len();
            let _ = staging.discard();
            return Err(OpfsError::CorruptCatalog(format!(
                "file entry {entry_id} size is {committed_size}, but staging copied {staged_size} bytes"
            )));
        }
        Ok((staging, committed_size))
    }

    fn file_snapshot(&mut self, path: &OpfsPath) -> OpfsResult<FileSnapshot> {
        let id = self.catalog.resolve_kind(path, EntryKind::File)?;
        let (name, modified_ms, backing_id, version_id, expected_size) = {
            let entry = self.catalog.entry(id)?;
            let backing_id = entry.backing_id.ok_or_else(|| {
                OpfsError::CorruptCatalog(format!("file entry {id} has no backing id"))
            })?;
            let version_id = entry.version_id.ok_or_else(|| {
                OpfsError::CorruptCatalog(format!("file entry {id} has no version id"))
            })?;
            (
                entry.name.clone(),
                entry.modified_ms,
                backing_id,
                version_id,
                entry.size,
            )
        };
        let bytes = self.read_content(backing_id)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != expected_size {
            return Err(OpfsError::CorruptCatalog(format!(
                "file entry {id} size does not match its content"
            )));
        }
        Ok(FileSnapshot {
            name,
            modified_ms,
            bytes,
            identity: FileSnapshotIdentity::from_raw(id, version_id).ok_or_else(|| {
                OpfsError::CorruptCatalog(format!(
                    "file entry {id} has an invalid snapshot identity"
                ))
            })?,
        })
    }

    fn persist_catalog(&self, catalog: &Catalog) -> OpfsResult<()> {
        catalog.validate()?;
        if let Some(disk_dir) = &self.disk_dir {
            persist_catalog_to_disk(disk_dir, catalog)?;
        }
        Ok(())
    }

    fn read_content(&mut self, backing_id: u64) -> OpfsResult<Vec<u8>> {
        if let Some(backing) = self.sync_backings.get_mut(&backing_id) {
            return backing.read_all();
        }
        if let Some(disk_dir) = &self.disk_dir {
            let path = backing_path(disk_dir, backing_id);
            return fs::read(&path)
                .map_err(|source| OpfsError::io("read file content", path, source));
        }
        self.memory_contents
            .get(&backing_id)
            .cloned()
            .ok_or_else(|| {
                OpfsError::CorruptCatalog(format!("missing memory backing id {backing_id}"))
            })
    }

    fn open_sync_backing(&mut self, entry_id: u64, writable: bool) -> OpfsResult<u64> {
        let (backing_id, expected_size) = {
            let entry = self.catalog.entry(entry_id)?;
            let backing_id = entry.backing_id.ok_or_else(|| {
                OpfsError::CorruptCatalog(format!("file entry {entry_id} has no backing id"))
            })?;
            (backing_id, entry.size)
        };
        if let Some(backing) = self.sync_backings.get_mut(&backing_id) {
            backing.add_handle(writable)?;
            return Ok(backing_id);
        }

        let backing = if let Some(disk_dir) = &self.disk_dir {
            SyncBacking::disk(
                disk_dir,
                backing_path(disk_dir, backing_id),
                entry_id,
                backing_id,
                writable,
            )?
        } else {
            let bytes = self.memory_contents.remove(&backing_id).ok_or_else(|| {
                OpfsError::CorruptCatalog(format!("missing memory backing id {backing_id}"))
            })?;
            SyncBacking::memory(backing_id, bytes, writable)
        };
        let actual_size = backing.len()?;
        if actual_size != expected_size {
            let (memory, cleanup) = backing.finish();
            if let Some((id, bytes)) = memory {
                self.memory_contents.insert(id, bytes);
            }
            cleanup?;
            return Err(OpfsError::CorruptCatalog(format!(
                "backing id {backing_id} has size {actual_size}, expected {expected_size}"
            )));
        }
        if self.sync_backings.insert(backing_id, backing).is_some() {
            return Err(OpfsError::CorruptCatalog(format!(
                "duplicate live sync backing id {backing_id}"
            )));
        }
        Ok(backing_id)
    }

    fn prepare_sync_mutation(&mut self, entry_id: u64, backing_id: u64) -> OpfsResult<u64> {
        if self.catalog.entry(entry_id)?.backing_id != Some(backing_id) {
            return Err(OpfsError::InvalidState);
        }
        if let Some(version_id) = self
            .sync_backings
            .get_mut(&backing_id)
            .ok_or(OpfsError::InvalidState)?
            .take_reserved_version_id()
        {
            self.sync_backings
                .get_mut(&backing_id)
                .ok_or(OpfsError::InvalidState)?
                .prepare_mutation()?;
            return Ok(version_id);
        }

        let mut next = self.catalog.clone();
        let (start, end) = next.reserve_version_ids(SYNC_VERSION_RESERVATION_SIZE)?;
        self.persist_catalog(&next)?;
        self.catalog = next;
        let backing = self
            .sync_backings
            .get_mut(&backing_id)
            .ok_or(OpfsError::InvalidState)?;
        backing.install_version_reservation(start, end)?;
        let version_id = backing
            .take_reserved_version_id()
            .ok_or(OpfsError::InvalidState)?;
        backing.prepare_mutation()?;
        Ok(version_id)
    }

    fn sync_backing_len(&self, backing_id: u64) -> OpfsResult<u64> {
        self.sync_backings
            .get(&backing_id)
            .ok_or(OpfsError::InvalidState)?
            .len()
    }

    fn sync_backing_read(
        &mut self,
        backing_id: u64,
        offset: u64,
        length: usize,
    ) -> OpfsResult<Vec<u8>> {
        self.sync_backings
            .get_mut(&backing_id)
            .ok_or(OpfsError::InvalidState)?
            .read(offset, length)
    }

    fn sync_backing_write(
        &mut self,
        backing_id: u64,
        offset: u64,
        bytes: &[u8],
    ) -> OpfsResult<usize> {
        self.sync_backings
            .get_mut(&backing_id)
            .ok_or(OpfsError::InvalidState)?
            .write(offset, bytes)
    }

    fn sync_backing_truncate(&mut self, backing_id: u64, size: u64) -> OpfsResult<()> {
        self.sync_backings
            .get_mut(&backing_id)
            .ok_or(OpfsError::InvalidState)?
            .truncate(size)
    }

    fn record_sync_mutation(
        &mut self,
        entry_id: u64,
        backing_id: u64,
        version_id: u64,
        size: u64,
    ) -> OpfsResult<()> {
        if self.catalog.entry(entry_id)?.backing_id != Some(backing_id) {
            return Err(OpfsError::InvalidState);
        }
        self.catalog
            .record_reserved_in_place_mutation(entry_id, version_id, size, now_ms())?;
        self.sync_backings
            .get_mut(&backing_id)
            .ok_or(OpfsError::InvalidState)?
            .mark_dirty();
        Ok(())
    }

    fn flush_sync_backing(&mut self, backing_id: u64) -> OpfsResult<()> {
        let dirty = {
            let backing = self
                .sync_backings
                .get_mut(&backing_id)
                .ok_or(OpfsError::InvalidState)?;
            backing.flush()?;
            backing.is_dirty()
        };
        if dirty {
            self.persist_catalog(&self.catalog)?;
        }
        self.sync_backings
            .get_mut(&backing_id)
            .ok_or(OpfsError::InvalidState)?
            .complete_checkpoint()
    }

    fn close_sync_backing(&mut self, backing_id: u64) -> OpfsResult<()> {
        let last_handle = self
            .sync_backings
            .get_mut(&backing_id)
            .ok_or(OpfsError::InvalidState)?
            .release_handle()?;
        if !last_handle {
            return Ok(());
        }
        if self
            .sync_backings
            .get(&backing_id)
            .is_some_and(SyncBacking::is_dirty)
        {
            self.flush_sync_backing(backing_id)?;
        }
        let backing = self
            .sync_backings
            .remove(&backing_id)
            .ok_or(OpfsError::InvalidState)?;
        let (memory, cleanup) = backing.finish();
        if let Some((id, bytes)) = memory
            && self.memory_contents.insert(id, bytes).is_some()
        {
            return Err(OpfsError::CorruptCatalog(format!(
                "duplicate memory backing id {id} after sync close"
            )));
        }
        cleanup
    }

    fn discard_sync_backings(&mut self) -> OpfsResult<()> {
        let mut first_error = None;
        for (_, backing) in std::mem::take(&mut self.sync_backings) {
            let (_, cleanup) = backing.finish();
            if let Err(error) = cleanup
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn write_new_content(&mut self, backing_id: u64, bytes: &[u8]) -> OpfsResult<()> {
        if let Some(disk_dir) = &self.disk_dir {
            let path = backing_path(disk_dir, backing_id);
            write_new_file(&path, bytes)?;
        } else if self
            .memory_contents
            .insert(backing_id, bytes.to_vec())
            .is_some()
        {
            return Err(OpfsError::CorruptCatalog(format!(
                "reused memory backing id {backing_id}"
            )));
        }
        Ok(())
    }

    fn install_staging_content(
        &mut self,
        backing_id: u64,
        staging: WritableStaging,
    ) -> OpfsResult<()> {
        match staging {
            WritableStaging::Memory(bytes) => self.write_new_content(backing_id, &bytes),
            WritableStaging::Disk(staging) => {
                let disk_dir = self.disk_dir.as_ref().ok_or_else(|| {
                    OpfsError::InvalidModification(
                        "disk staging cannot be committed to a memory bucket".to_owned(),
                    )
                })?;
                staging.promote(&backing_path(disk_dir, backing_id))
            }
        }
    }

    fn delete_content(&mut self, backing_id: u64) -> OpfsResult<()> {
        if let Some(disk_dir) = &self.disk_dir {
            let path = backing_path(disk_dir, backing_id);
            let mut removed = false;
            match fs::remove_file(&path) {
                Ok(()) => removed = true,
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(OpfsError::io("remove file content", path, source));
                }
            }
            if removed {
                sync_directory(&disk_dir.join(CONTENTS_DIR_NAME))?;
            }
        } else {
            self.memory_contents.remove(&backing_id);
        }
        Ok(())
    }

    fn validate_disk_contents(&self) -> OpfsResult<()> {
        let Some(disk_dir) = &self.disk_dir else {
            return Ok(());
        };
        for id in self.catalog.subtree_ids(ROOT_ENTRY_ID) {
            let entry = self.catalog.entry(id)?;
            let Some(backing_id) = entry.backing_id else {
                continue;
            };
            let path = backing_path(disk_dir, backing_id);
            let metadata = fs::metadata(&path)
                .map_err(|source| OpfsError::io("stat file content", &path, source))?;
            if metadata.len() != entry.size {
                return Err(OpfsError::CorruptCatalog(format!(
                    "content id {backing_id} has size {}, expected {}",
                    metadata.len(),
                    entry.size
                )));
            }
        }
        Ok(())
    }

    fn garbage_collect_disk_contents(&self) -> OpfsResult<()> {
        let Some(disk_dir) = &self.disk_dir else {
            return Ok(());
        };
        let contents_dir = disk_dir.join(CONTENTS_DIR_NAME);
        let referenced = self
            .catalog
            .backing_ids()
            .into_iter()
            .collect::<BTreeSet<_>>();
        let mut changed = false;
        for entry in fs::read_dir(&contents_dir)
            .map_err(|source| OpfsError::io("scan OPFS contents", &contents_dir, source))?
        {
            let entry = entry.map_err(|source| {
                OpfsError::io("read OPFS content entry", &contents_dir, source)
            })?;
            let path = entry.path();
            let Some(backing_id) = backing_id_from_path(&path) else {
                continue;
            };
            if !referenced.contains(&backing_id) {
                fs::remove_file(&path)
                    .map_err(|source| OpfsError::io("remove orphan OPFS content", &path, source))?;
                changed = true;
            }
        }
        if changed {
            sync_directory(&contents_dir)?;
        }
        Ok(())
    }

    fn cleanup_staging(&self) -> OpfsResult<()> {
        if let Some(disk_dir) = &self.disk_dir {
            cleanup_staging_directory(disk_dir)?;
        }
        Ok(())
    }
}

fn validate_sync_offset(value: u64, operation: &str) -> OpfsResult<()> {
    if value > MAX_SYNC_FILE_OFFSET {
        return Err(OpfsError::InvalidModification(format!(
            "sync {operation} offset exceeds the signed 64-bit host file limit"
        )));
    }
    Ok(())
}

fn check_quota(
    bucket_usage: u64,
    old_file_size: u64,
    new_file_size: u64,
    max_bucket_usage: Option<u64>,
) -> OpfsResult<()> {
    let requested = bucket_usage
        .saturating_sub(old_file_size)
        .saturating_add(new_file_size);
    if let Some(quota) = max_bucket_usage
        && requested > quota
    {
        return Err(OpfsError::QuotaExceeded { quota, requested });
    }
    Ok(())
}

fn invalid_file_snapshot_error() -> OpfsError {
    OpfsError::NotFound("Blob backing file is no longer available".to_owned())
}

fn projected_bucket_quota_usage(
    bucket_key: &OpfsBucketKey,
    committed_usage: u64,
    writers: &BTreeMap<OpfsWritableId, WritableSession>,
    writer_override: Option<(OpfsWritableId, u64)>,
) -> u64 {
    let writer_growth = writers
        .iter()
        .filter(|(_, session)| session.bucket == *bucket_key)
        .fold(0u64, |total, (id, session)| {
            let projected = writer_override
                .filter(|(override_id, _)| *override_id == *id)
                .map(|(_, usage)| usage)
                .unwrap_or_else(|| u64::try_from(session.staging.len()).unwrap_or(u64::MAX));
            total.saturating_add(projected.saturating_sub(session.committed_size))
        });
    committed_usage.saturating_add(writer_growth)
}

fn disk_bucket_dir(root: &Path, key: &OpfsBucketKey) -> PathBuf {
    root.join(disk_bucket_name(key))
}

fn disk_bucket_name(key: &OpfsBucketKey) -> String {
    moli_crypto::sha256_hex(key.as_str().as_bytes())
}

fn backing_path(bucket_dir: &Path, backing_id: u64) -> PathBuf {
    bucket_dir
        .join(CONTENTS_DIR_NAME)
        .join(format!("{backing_id:016x}.bin"))
}

fn backing_id_from_path(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    let encoded = name.strip_suffix(".bin")?;
    (encoded.len() == 16 && encoded.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| u64::from_str_radix(encoded, 16).ok())
        .flatten()
}

fn persist_catalog_to_disk(bucket_dir: &Path, catalog: &Catalog) -> OpfsResult<()> {
    let bytes = catalog.to_bytes()?;
    let current = bucket_dir.join(CATALOG_FILE_NAME);
    let next = bucket_dir.join(CATALOG_NEXT_FILE_NAME);
    let previous = bucket_dir.join(CATALOG_PREVIOUS_FILE_NAME);
    remove_file_if_exists(&next, "remove stale next catalog")?;
    remove_file_if_exists(&previous, "remove stale previous catalog")?;
    write_new_file(&next, &bytes)?;
    #[cfg(test)]
    crate::fault_injection::crash_if_armed(
        crate::fault_injection::CrashPoint::CatalogNextDurable,
        || {},
    );
    if current.exists() {
        fs::rename(&current, &previous)
            .map_err(|source| OpfsError::io("stage previous catalog", &current, source))?;
        sync_directory(bucket_dir)?;
        #[cfg(test)]
        crate::fault_injection::crash_if_armed(
            crate::fault_injection::CrashPoint::CatalogPreviousDurable,
            || {},
        );
    }
    if let Err(source) = fs::rename(&next, &current) {
        if previous.exists() {
            let _ = fs::rename(&previous, &current);
            let _ = sync_directory(bucket_dir);
        }
        return Err(OpfsError::io("promote next catalog", &next, source));
    }
    sync_directory(bucket_dir)?;
    #[cfg(test)]
    crate::fault_injection::crash_if_armed(
        crate::fault_injection::CrashPoint::CatalogCurrentDurable,
        || {},
    );
    remove_file_if_exists(&previous, "remove previous catalog")?;
    #[cfg(test)]
    crate::fault_injection::crash_if_armed(
        crate::fault_injection::CrashPoint::CatalogPreviousRemoved,
        || {},
    );
    sync_directory(bucket_dir)?;
    Ok(())
}

fn recover_catalog_replacement(bucket_dir: &Path) -> OpfsResult<()> {
    let current = bucket_dir.join(CATALOG_FILE_NAME);
    let next = bucket_dir.join(CATALOG_NEXT_FILE_NAME);
    let previous = bucket_dir.join(CATALOG_PREVIOUS_FILE_NAME);
    let mut changed = false;
    if current.exists() {
        changed = next.exists() || previous.exists();
        remove_file_if_exists(&next, "remove stale next catalog")?;
        remove_file_if_exists(&previous, "remove stale previous catalog")?;
    } else if next.exists() {
        fs::rename(&next, &current)
            .map_err(|source| OpfsError::io("recover next catalog", &next, source))?;
        remove_file_if_exists(&previous, "remove stale previous catalog")?;
        changed = true;
    } else if previous.exists() {
        fs::rename(&previous, &current)
            .map_err(|source| OpfsError::io("recover previous catalog", &previous, source))?;
        changed = true;
    }
    if changed {
        sync_directory(bucket_dir)?;
    }
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8]) -> OpfsResult<()> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|source| OpfsError::io("create file", path, source))?;
    file.write_all(bytes)
        .map_err(|source| OpfsError::io("write file", path, source))?;
    file.sync_all()
        .map_err(|source| OpfsError::io("sync file", path, source))?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn remove_file_if_exists(path: &Path, operation: &'static str) -> OpfsResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(OpfsError::io(operation, path, source)),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        panic::{AssertUnwindSafe, catch_unwind},
        path::PathBuf,
    };

    use super::Opfs;
    use crate::{
        EntryKind, OpfsBucketKey, OpfsError, OpfsPath, SyncAccessMode, WritableCommand,
        WritableMode,
        fault_injection::{CrashPoint, arm},
    };

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "moli-opfs-{name}-{}-{}",
                std::process::id(),
                super::now_ms()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn bucket() -> OpfsBucketKey {
        OpfsBucketKey::new("default:storage-key-for-test").unwrap()
    }

    fn directory_entries(path: &std::path::Path) -> Vec<PathBuf> {
        let mut entries = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }

    fn disk_bucket_path(root: &std::path::Path, bucket: &OpfsBucketKey) -> PathBuf {
        super::disk_bucket_dir(root, bucket)
    }

    #[test]
    fn disk_bucket_name_preserves_sha256_mapping() {
        let bucket = OpfsBucketKey::new("abc").unwrap();
        assert_eq!(
            super::disk_bucket_name(&bucket),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn assert_writable_crash_recovers(point: CrashPoint, expected: &[u8]) {
        let temp = TempRoot::new(&format!("crash-{point:?}"));
        let bucket = bucket();
        let file = OpfsPath::root().child("file").unwrap();
        {
            let opfs = Opfs::on_disk(&temp.0).unwrap();
            opfs.get_file(&bucket, &OpfsPath::root(), "file", true)
                .unwrap();
            opfs.write_file(&bucket, &file, b"old committed bytes", None)
                .unwrap();
            let writer = opfs
                .create_writable(&bucket, &file, false, WritableMode::Exclusive)
                .unwrap();
            opfs.writable_command(
                writer,
                WritableCommand::Write {
                    data: b"new committed bytes".to_vec(),
                    position: None,
                },
                None,
            )
            .unwrap();

            let armed = arm(point);
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                opfs.close_writable(writer, None).unwrap();
            }));
            drop(armed);
            assert!(outcome.is_err(), "crash point {point:?} was not reached");
        }

        let reopened = Opfs::on_disk(&temp.0).unwrap();
        assert_eq!(reopened.read_file(&bucket, &file).unwrap().bytes, expected);
        let bucket_dir = disk_bucket_path(&temp.0, &bucket);
        assert!(
            directory_entries(&bucket_dir.join(super::super::staging::STAGING_DIR_NAME)).is_empty()
        );
        assert_eq!(
            directory_entries(&bucket_dir.join(super::CONTENTS_DIR_NAME)).len(),
            1
        );
        assert!(!bucket_dir.join(super::CATALOG_NEXT_FILE_NAME).exists());
        assert!(!bucket_dir.join(super::CATALOG_PREVIOUS_FILE_NAME).exists());
    }

    #[test]
    fn memory_namespace_covers_create_iterate_resolve_move_and_remove() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let root = opfs.ensure_root(&bucket).unwrap();
        let directory = opfs.get_directory(&bucket, &root, "目录", true).unwrap();
        let file = opfs.get_file(&bucket, &directory, "a:b.txt", true).unwrap();
        opfs.write_file(&bucket, &file, b"hello", None).unwrap();

        assert_eq!(
            opfs.entry_kind(&bucket, &root).unwrap(),
            EntryKind::Directory
        );
        assert_eq!(
            opfs.read_directory(&bucket, &directory).unwrap(),
            vec![super::DirectoryEntry {
                name: "a:b.txt".to_owned(),
                kind: EntryKind::File,
            }]
        );
        assert_eq!(
            opfs.resolve(&bucket, &root, &file).unwrap(),
            Some(vec!["目录".to_owned(), "a:b.txt".to_owned()])
        );

        let moved = opfs
            .move_entry(&bucket, &file, EntryKind::File, &root, "moved.txt", None)
            .unwrap();
        assert_eq!(opfs.read_file(&bucket, &moved).unwrap().bytes, b"hello");
        assert!(matches!(
            opfs.read_file(&bucket, &file),
            Err(OpfsError::NotFound(_))
        ));
        opfs.remove_entry(&bucket, &directory, false).unwrap();
        opfs.remove_entry(&bucket, &moved, false).unwrap();
        assert_eq!(opfs.usage(&bucket).unwrap(), 0);
    }

    #[test]
    fn file_snapshot_identity_rejects_modified_and_same_path_recreated_entries() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let root = OpfsPath::root();
        let file = opfs.get_file(&bucket, &root, "snapshot", true).unwrap();
        opfs.write_file(&bucket, &file, b"first", None).unwrap();
        let first = opfs.read_file(&bucket, &file).unwrap();
        opfs.validate_file_snapshot(&bucket, &file, first.identity)
            .unwrap();

        opfs.write_file(&bucket, &file, b"second", None).unwrap();
        assert!(matches!(
            opfs.validate_file_snapshot(&bucket, &file, first.identity),
            Err(OpfsError::NotFound(_))
        ));
        let second = opfs.read_file(&bucket, &file).unwrap();
        assert_ne!(first.identity.version_id(), second.identity.version_id());
        assert_eq!(first.identity.entry_id(), second.identity.entry_id());

        opfs.remove_entry(&bucket, &file, false).unwrap();
        let recreated = opfs.get_file(&bucket, &root, "snapshot", true).unwrap();
        opfs.write_file(&bucket, &recreated, b"second", None)
            .unwrap();
        let replacement = opfs.read_file(&bucket, &recreated).unwrap();
        assert_ne!(second.identity.entry_id(), replacement.identity.entry_id());
        assert!(matches!(
            opfs.validate_file_snapshot(&bucket, &recreated, second.identity),
            Err(OpfsError::NotFound(_))
        ));
        opfs.validate_file_snapshot(&bucket, &recreated, replacement.identity)
            .unwrap();
    }

    #[test]
    fn directory_removal_requires_recursive_for_non_empty_tree() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let root = OpfsPath::root();
        let directory = opfs.get_directory(&bucket, &root, "dir", true).unwrap();
        opfs.get_file(&bucket, &directory, "file", true).unwrap();
        assert!(matches!(
            opfs.remove_entry(&bucket, &directory, false),
            Err(OpfsError::DirectoryNotEmpty(_))
        ));
        opfs.remove_entry(&bucket, &directory, true).unwrap();
        assert!(matches!(
            opfs.entry_kind(&bucket, &directory),
            Err(OpfsError::NotFound(_))
        ));
    }

    #[test]
    fn successful_move_lease_blocks_source_and_destination_until_drop() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let root = OpfsPath::root();
        let source = opfs.get_file(&bucket, &root, "source", true).unwrap();
        let sibling = opfs.get_file(&bucket, &root, "sibling", true).unwrap();

        let (destination, lease) = opfs
            .move_entry_with_mutation_lease(
                &bucket,
                &source,
                EntryKind::File,
                &root,
                "destination",
                None,
            )
            .unwrap();
        assert!(matches!(
            opfs.create_writable(&bucket, &source, false, WritableMode::Siloed),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        assert!(matches!(
            opfs.create_sync_access_handle(&bucket, &destination, SyncAccessMode::ReadOnly),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        let sibling_writer = opfs
            .create_writable(&bucket, &sibling, false, WritableMode::Exclusive)
            .unwrap();
        opfs.abort_writable(sibling_writer).unwrap();

        drop(lease);
        assert!(matches!(
            opfs.create_writable(&bucket, &source, false, WritableMode::Siloed),
            Err(OpfsError::NotFound(_))
        ));
        let destination_writer = opfs
            .create_writable(&bucket, &destination, false, WritableMode::Siloed)
            .unwrap();
        opfs.abort_writable(destination_writer).unwrap();
    }

    #[test]
    fn successful_remove_lease_outlives_namespace_commit_until_drop() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let root = OpfsPath::root();
        let file = opfs.get_file(&bucket, &root, "removed", true).unwrap();

        let lease = opfs
            .remove_entry_with_mutation_lease(&bucket, &file, false)
            .unwrap();
        assert!(matches!(
            opfs.create_sync_access_handle(&bucket, &file, SyncAccessMode::ReadwriteUnsafe),
            Err(OpfsError::NoModificationAllowed(_))
        ));

        drop(lease);
        assert!(matches!(
            opfs.create_sync_access_handle(&bucket, &file, SyncAccessMode::ReadwriteUnsafe),
            Err(OpfsError::NotFound(_))
        ));
        assert_eq!(
            opfs.get_file(&bucket, &root, "removed", true).unwrap(),
            file
        );
    }

    #[test]
    fn directory_move_reparents_one_subtree_and_rejects_invalid_replacements() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let root = OpfsPath::root();
        let source = opfs.get_directory(&bucket, &root, "source", true).unwrap();
        let nested = opfs
            .get_directory(&bucket, &source, "nested", true)
            .unwrap();
        let file = opfs.get_file(&bucket, &nested, "file", true).unwrap();
        opfs.write_file(&bucket, &file, b"subtree bytes", None)
            .unwrap();
        let destination_parent = opfs
            .get_directory(&bucket, &root, "destination", true)
            .unwrap();

        let moved = opfs
            .move_entry(
                &bucket,
                &source,
                EntryKind::Directory,
                &destination_parent,
                "renamed",
                None,
            )
            .unwrap();
        let moved_nested = moved.child("nested").unwrap();
        let moved_file = moved_nested.child("file").unwrap();
        assert_eq!(
            opfs.read_file(&bucket, &moved_file).unwrap().bytes,
            b"subtree bytes"
        );
        assert!(matches!(
            opfs.entry_kind(&bucket, &source),
            Err(OpfsError::NotFound(_))
        ));
        assert!(matches!(
            opfs.move_entry(
                &bucket,
                &moved,
                EntryKind::Directory,
                &moved_nested,
                "cycle",
                None,
            ),
            Err(OpfsError::InvalidModification(_))
        ));
        assert!(matches!(
            opfs.move_entry(
                &bucket,
                &root,
                EntryKind::Directory,
                &destination_parent,
                "root",
                None,
            ),
            Err(OpfsError::InvalidModification(_))
        ));
        assert!(matches!(
            opfs.move_entry(
                &bucket,
                &moved,
                EntryKind::File,
                &destination_parent,
                "wrong-kind",
                None,
            ),
            Err(OpfsError::TypeMismatch { .. })
        ));

        let file_target = opfs
            .get_file(&bucket, &destination_parent, "file-target", true)
            .unwrap();
        assert!(matches!(
            opfs.move_entry(
                &bucket,
                &moved,
                EntryKind::Directory,
                &destination_parent,
                "file-target",
                None,
            ),
            Err(OpfsError::TypeMismatch { .. })
        ));
        assert_eq!(
            opfs.entry_kind(&bucket, &file_target).unwrap(),
            EntryKind::File
        );

        let non_empty_target = opfs
            .get_directory(&bucket, &destination_parent, "non-empty", true)
            .unwrap();
        opfs.get_file(&bucket, &non_empty_target, "child", true)
            .unwrap();
        assert!(matches!(
            opfs.move_entry(
                &bucket,
                &moved,
                EntryKind::Directory,
                &destination_parent,
                "non-empty",
                None,
            ),
            Err(OpfsError::DirectoryNotEmpty(_))
        ));

        let empty_target = opfs
            .get_directory(&bucket, &destination_parent, "empty", true)
            .unwrap();
        let replaced = opfs
            .move_entry(
                &bucket,
                &moved,
                EntryKind::Directory,
                &destination_parent,
                "empty",
                None,
            )
            .unwrap();
        assert_eq!(replaced, empty_target);
        assert_eq!(
            opfs.read_file(
                &bucket,
                &replaced.child("nested").unwrap().child("file").unwrap()
            )
            .unwrap()
            .bytes,
            b"subtree bytes"
        );
    }

    #[test]
    fn directory_move_locks_source_and_destination_subtrees_but_not_siblings() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let root = OpfsPath::root();
        let source = opfs.get_directory(&bucket, &root, "source", true).unwrap();
        let source_file = opfs.get_file(&bucket, &source, "locked", true).unwrap();
        let sibling_file = opfs.get_file(&bucket, &root, "sibling", true).unwrap();

        let source_writer = opfs
            .create_writable(&bucket, &source_file, false, WritableMode::Siloed)
            .unwrap();
        assert!(matches!(
            opfs.move_entry(
                &bucket,
                &source,
                EntryKind::Directory,
                &root,
                "renamed",
                None,
            ),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        opfs.abort_writable(source_writer).unwrap();

        let source_sync = opfs
            .create_sync_access_handle(&bucket, &source_file, SyncAccessMode::Readwrite)
            .unwrap();
        assert!(matches!(
            opfs.move_entry(
                &bucket,
                &source,
                EntryKind::Directory,
                &root,
                "renamed",
                None,
            ),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        opfs.close_sync(source_sync, None).unwrap();

        let sibling_writer = opfs
            .create_writable(&bucket, &sibling_file, false, WritableMode::Exclusive)
            .unwrap();
        let moved = opfs
            .move_entry(
                &bucket,
                &source,
                EntryKind::Directory,
                &root,
                "renamed",
                None,
            )
            .unwrap();
        opfs.abort_writable(sibling_writer).unwrap();
        assert_eq!(moved, root.child("renamed").unwrap());

        let incoming = opfs
            .get_directory(&bucket, &root, "incoming", true)
            .unwrap();
        let destination = opfs
            .get_directory(&bucket, &root, "destination", true)
            .unwrap();
        let destination_file = opfs
            .get_file(&bucket, &destination, "locked", true)
            .unwrap();
        let destination_writer = opfs
            .create_writable(&bucket, &destination_file, false, WritableMode::Siloed)
            .unwrap();
        assert!(matches!(
            opfs.move_entry(
                &bucket,
                &incoming,
                EntryKind::Directory,
                &root,
                "destination",
                None,
            ),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        opfs.abort_writable(destination_writer).unwrap();
        assert!(matches!(
            opfs.move_entry(
                &bucket,
                &incoming,
                EntryKind::Directory,
                &root,
                "destination",
                None,
            ),
            Err(OpfsError::DirectoryNotEmpty(_))
        ));
    }

    #[test]
    fn writable_abort_is_atomic_and_locks_move_until_close() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let root = OpfsPath::root();
        let file = opfs.get_file(&bucket, &root, "file", true).unwrap();
        opfs.write_file(&bucket, &file, b"old", None).unwrap();

        let writer = opfs
            .create_writable(&bucket, &file, true, WritableMode::Siloed)
            .unwrap();
        opfs.writable_command(
            writer,
            WritableCommand::Write {
                data: b"new".to_vec(),
                position: Some(0),
            },
            None,
        )
        .unwrap();
        assert!(matches!(
            opfs.move_entry(&bucket, &file, EntryKind::File, &root, "blocked", None,),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        opfs.abort_writable(writer).unwrap();
        assert_eq!(opfs.read_file(&bucket, &file).unwrap().bytes, b"old");

        let writer = opfs
            .create_writable(&bucket, &file, false, WritableMode::Exclusive)
            .unwrap();
        opfs.writable_command(
            writer,
            WritableCommand::Write {
                data: b"committed".to_vec(),
                position: None,
            },
            None,
        )
        .unwrap();
        opfs.close_writable(writer, None).unwrap();
        assert_eq!(opfs.read_file(&bucket, &file).unwrap().bytes, b"committed");
    }

    #[test]
    fn move_same_name_checks_source_existence_and_active_locks() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let root = OpfsPath::root();
        let file = opfs.get_file(&bucket, &root, "file", true).unwrap();
        let writer = opfs
            .create_writable(&bucket, &file, false, WritableMode::Siloed)
            .unwrap();

        assert!(matches!(
            opfs.move_entry(&bucket, &file, EntryKind::File, &root, "file", None),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        opfs.abort_writable(writer).unwrap();
        assert_eq!(
            opfs.move_entry(&bucket, &file, EntryKind::File, &root, "file", None)
                .unwrap(),
            file
        );
        opfs.remove_entry(&bucket, &file, false).unwrap();
        assert!(matches!(
            opfs.move_entry(&bucket, &file, EntryKind::File, &root, "file", None),
            Err(OpfsError::NotFound(_))
        ));
    }

    #[test]
    fn move_rename_quota_failure_preserves_the_original_entry() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let root = OpfsPath::root();
        let file = opfs.get_file(&bucket, &root, "a", true).unwrap();
        opfs.write_file(&bucket, &file, b"bytes", None).unwrap();
        let quota = opfs.usage(&bucket).unwrap();

        assert!(matches!(
            opfs.move_entry(
                &bucket,
                &file,
                EntryKind::File,
                &root,
                "a-much-longer-name",
                Some(quota),
            ),
            Err(OpfsError::QuotaExceeded { .. })
        ));
        assert_eq!(opfs.read_file(&bucket, &file).unwrap().bytes, b"bytes");
        assert!(matches!(
            opfs.read_file(&bucket, &root.child("a-much-longer-name").unwrap()),
            Err(OpfsError::NotFound(_))
        ));
    }

    #[test]
    fn quota_failure_keeps_committed_file_unchanged() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let file = opfs
            .get_file(&bucket, &OpfsPath::root(), "file", true)
            .unwrap();
        let quota = opfs.usage(&bucket).unwrap() + 4;
        opfs.write_file(&bucket, &file, b"1234", Some(quota))
            .unwrap();
        assert!(matches!(
            opfs.write_file(&bucket, &file, b"12345", Some(quota)),
            Err(OpfsError::QuotaExceeded { .. })
        ));
        assert_eq!(opfs.read_file(&bucket, &file).unwrap().bytes, b"1234");
    }

    #[test]
    fn active_writer_growth_reserves_quota_until_abort() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let file = opfs
            .get_file(&bucket, &OpfsPath::root(), "file", true)
            .unwrap();
        let committed_usage = opfs.usage(&bucket).unwrap();
        let writer = opfs
            .create_writable(&bucket, &file, false, WritableMode::Siloed)
            .unwrap();
        let quota = committed_usage + 200;

        opfs.writable_command(
            writer,
            WritableCommand::Write {
                data: vec![b'x'; 200],
                position: None,
            },
            Some(quota),
        )
        .unwrap();

        assert_eq!(opfs.usage(&bucket).unwrap(), committed_usage);
        assert_eq!(opfs.quota_usage(&bucket).unwrap(), committed_usage + 200);
        assert!(matches!(
            opfs.get_file_with_quota(&bucket, &OpfsPath::root(), "other", true, quota,),
            Err(OpfsError::QuotaExceeded { .. })
        ));
        opfs.abort_writable(writer).unwrap();
        assert_eq!(opfs.quota_usage(&bucket).unwrap(), committed_usage);
        opfs.get_file_with_quota(&bucket, &OpfsPath::root(), "other", true, quota)
            .unwrap();
    }

    #[test]
    fn siloed_writer_growth_is_reserved_conservatively_across_sessions() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let file = opfs
            .get_file(&bucket, &OpfsPath::root(), "file", true)
            .unwrap();
        let committed_usage = opfs.usage(&bucket).unwrap();
        let quota = committed_usage + 6;
        let first = opfs
            .create_writable(&bucket, &file, false, WritableMode::Siloed)
            .unwrap();
        let second = opfs
            .create_writable(&bucket, &file, false, WritableMode::Siloed)
            .unwrap();

        opfs.writable_command(
            first,
            WritableCommand::Write {
                data: b"1234".to_vec(),
                position: None,
            },
            Some(quota),
        )
        .unwrap();
        assert!(matches!(
            opfs.writable_command(
                second,
                WritableCommand::Write {
                    data: b"123".to_vec(),
                    position: None,
                },
                Some(quota),
            ),
            Err(OpfsError::QuotaExceeded { .. })
        ));
        assert_eq!(opfs.quota_usage(&bucket).unwrap(), committed_usage + 4);

        opfs.abort_writable(first).unwrap();
        opfs.writable_command(
            second,
            WritableCommand::Write {
                data: b"123".to_vec(),
                position: None,
            },
            Some(quota),
        )
        .unwrap();
        assert_eq!(opfs.quota_usage(&bucket).unwrap(), committed_usage + 3);
        opfs.abort_writable(second).unwrap();
    }

    #[test]
    fn sync_close_keeps_direct_writes_instead_of_reapplying_quota() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let root = OpfsPath::root();
        let target = opfs.get_file(&bucket, &root, "target", true).unwrap();
        let sibling = opfs.get_file(&bucket, &root, "sibling", true).unwrap();
        let committed_usage = opfs.usage(&bucket).unwrap();
        let quota = committed_usage + 4;
        let handle = opfs
            .create_sync_access_handle(&bucket, &target, SyncAccessMode::Readwrite)
            .unwrap();

        opfs.sync_write(handle, b"1234", None, Some(quota)).unwrap();
        assert_eq!(opfs.quota_usage(&bucket).unwrap(), committed_usage + 4);
        assert_eq!(opfs.read_file(&bucket, &target).unwrap().bytes, b"1234");
        opfs.write_file(&bucket, &sibling, b"x", None).unwrap();

        opfs.close_sync(handle, Some(quota)).unwrap();
        assert_eq!(opfs.read_file(&bucket, &target).unwrap().bytes, b"1234");
        assert_eq!(opfs.quota_usage(&bucket).unwrap(), committed_usage + 5);
        let replacement = opfs
            .create_sync_access_handle(&bucket, &target, SyncAccessMode::Readwrite)
            .unwrap();
        opfs.close_sync(replacement, Some(quota)).unwrap();
    }

    #[test]
    fn sync_access_flushes_zero_filled_growth_and_rejects_second_handle() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let file = opfs
            .get_file(&bucket, &OpfsPath::root(), "file", true)
            .unwrap();
        let handle = opfs
            .create_sync_access_handle(&bucket, &file, SyncAccessMode::Readwrite)
            .unwrap();
        assert!(matches!(
            opfs.create_sync_access_handle(&bucket, &file, SyncAccessMode::Readwrite),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        opfs.sync_write(handle, b"xy", Some(2), None).unwrap();
        assert_eq!(opfs.sync_size(handle).unwrap(), 4);
        opfs.flush_sync(handle, None).unwrap();
        assert_eq!(opfs.read_file(&bucket, &file).unwrap().bytes, b"\0\0xy");
        opfs.close_sync(handle, None).unwrap();
        assert!(matches!(
            opfs.sync_size(handle),
            Err(OpfsError::InvalidState)
        ));
    }

    #[test]
    fn sync_shared_modes_only_coexist_with_the_same_mode() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let file = opfs
            .get_file(&bucket, &OpfsPath::root(), "file", true)
            .unwrap();

        let read_only_first = opfs
            .create_sync_access_handle(&bucket, &file, SyncAccessMode::ReadOnly)
            .unwrap();
        let read_only_second = opfs
            .create_sync_access_handle(&bucket, &file, SyncAccessMode::ReadOnly)
            .unwrap();
        assert!(matches!(
            opfs.create_sync_access_handle(&bucket, &file, SyncAccessMode::ReadwriteUnsafe),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        assert!(matches!(
            opfs.create_sync_access_handle(&bucket, &file, SyncAccessMode::Readwrite),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        assert!(matches!(
            opfs.create_writable(&bucket, &file, false, WritableMode::Siloed),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        opfs.close_sync(read_only_first, None).unwrap();
        assert!(matches!(
            opfs.create_sync_access_handle(&bucket, &file, SyncAccessMode::ReadwriteUnsafe),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        opfs.close_sync(read_only_second, None).unwrap();

        let unsafe_first = opfs
            .create_sync_access_handle(&bucket, &file, SyncAccessMode::ReadwriteUnsafe)
            .unwrap();
        let unsafe_second = opfs
            .create_sync_access_handle(&bucket, &file, SyncAccessMode::ReadwriteUnsafe)
            .unwrap();
        assert!(matches!(
            opfs.create_sync_access_handle(&bucket, &file, SyncAccessMode::ReadOnly),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        assert!(matches!(
            opfs.create_writable(&bucket, &file, false, WritableMode::Exclusive),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        opfs.close_sync(unsafe_first, None).unwrap();
        opfs.close_sync(unsafe_second, None).unwrap();
    }

    #[test]
    fn read_only_sync_handle_rejects_every_mutation_even_when_clean() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let file = opfs
            .get_file(&bucket, &OpfsPath::root(), "file", true)
            .unwrap();
        opfs.write_file(&bucket, &file, b"read-only bytes", None)
            .unwrap();
        let handle = opfs
            .create_sync_access_handle(&bucket, &file, SyncAccessMode::ReadOnly)
            .unwrap();

        assert_eq!(
            opfs.sync_read(handle, usize::MAX, Some(0)).unwrap(),
            b"read-only bytes"
        );
        assert_eq!(opfs.sync_size(handle).unwrap(), 15);
        assert!(matches!(
            opfs.sync_write(handle, b"x", Some(0), None),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        assert!(matches!(
            opfs.sync_truncate(handle, 0, None),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        assert!(matches!(
            opfs.flush_sync(handle, None),
            Err(OpfsError::NoModificationAllowed(_))
        ));
        opfs.close_sync(handle, None).unwrap();
        assert_eq!(
            opfs.read_file(&bucket, &file).unwrap().bytes,
            b"read-only bytes"
        );
    }

    #[test]
    fn unsafe_sync_handles_share_one_live_backing_and_quota_baseline() {
        let opfs = Opfs::in_memory();
        let bucket = bucket();
        let file = opfs
            .get_file(&bucket, &OpfsPath::root(), "file", true)
            .unwrap();
        opfs.write_file(&bucket, &file, b"abcd", None).unwrap();
        let committed_usage = opfs.usage(&bucket).unwrap();
        let quota = committed_usage + 2;
        let first = opfs
            .create_sync_access_handle(&bucket, &file, SyncAccessMode::ReadwriteUnsafe)
            .unwrap();
        let second = opfs
            .create_sync_access_handle(&bucket, &file, SyncAccessMode::ReadwriteUnsafe)
            .unwrap();

        opfs.sync_write(first, b"X", Some(0), Some(quota)).unwrap();
        opfs.sync_write(second, b"Y", Some(3), Some(quota)).unwrap();
        assert_eq!(
            opfs.sync_read(second, usize::MAX, Some(0)).unwrap(),
            b"XbcY"
        );
        assert_eq!(opfs.read_file(&bucket, &file).unwrap().bytes, b"XbcY");
        opfs.sync_truncate(first, 6, Some(quota)).unwrap();
        assert_eq!(opfs.sync_size(second).unwrap(), 6);
        opfs.sync_write(second, b"Z", Some(5), Some(quota)).unwrap();
        assert_eq!(opfs.read_file(&bucket, &file).unwrap().bytes, b"XbcY\0Z");
        assert_eq!(opfs.quota_usage(&bucket).unwrap(), quota);
        opfs.flush_sync(first, Some(quota)).unwrap();
        assert_eq!(
            opfs.sync_read(first, usize::MAX, Some(0)).unwrap(),
            b"XbcY\0Z"
        );

        opfs.close_sync(first, Some(quota)).unwrap();
        assert_eq!(opfs.sync_size(second).unwrap(), 6);
        opfs.close_sync(second, Some(quota)).unwrap();
        assert_eq!(opfs.read_file(&bucket, &file).unwrap().bytes, b"XbcY\0Z");
    }

    #[test]
    fn disk_backend_persists_arbitrary_virtual_names_without_host_name_mapping() {
        let temp = TempRoot::new("restart");
        let bucket = bucket();
        let file;
        {
            let opfs = Opfs::on_disk(&temp.0).unwrap();
            let directory = opfs
                .get_directory(&bucket, &OpfsPath::root(), "目录:aux", true)
                .unwrap();
            file = opfs
                .get_file(&bucket, &directory, "nul\0name", true)
                .unwrap();
            opfs.write_file(&bucket, &file, b"persistent", None)
                .unwrap();
        }

        let opfs = Opfs::on_disk(&temp.0).unwrap();
        assert_eq!(opfs.read_file(&bucket, &file).unwrap().bytes, b"persistent");
        let bucket_dirs = fs::read_dir(&temp.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(bucket_dirs.len(), 1);
        assert!(!bucket_dirs[0].contains("目录"));
        assert!(!bucket_dirs[0].contains("nul"));
    }

    #[test]
    fn disk_writable_crash_points_recover_exactly_one_committed_version() {
        for point in [
            CrashPoint::WritableStagingSynced,
            CrashPoint::WritableStagingPromoted,
            CrashPoint::WritableContentDurable,
            CrashPoint::ContentInstalledBeforeCatalog,
            CrashPoint::CatalogNextDurable,
        ] {
            assert_writable_crash_recovers(point, b"old committed bytes");
        }
        for point in [
            CrashPoint::CatalogPreviousDurable,
            CrashPoint::CatalogCurrentDurable,
            CrashPoint::CatalogPreviousRemoved,
            CrashPoint::CatalogCommittedBeforeMemorySwap,
            CrashPoint::CatalogMemorySwappedBeforeOldContentDelete,
        ] {
            assert_writable_crash_recovers(point, b"new committed bytes");
        }
    }

    #[test]
    fn disk_move_persists_the_new_virtual_path_across_restart() {
        let temp = TempRoot::new("disk-move-restart");
        let bucket = bucket();
        let root = OpfsPath::root();
        let source_directory = root.child("source").unwrap();
        let destination_directory = root.child("destination").unwrap();
        let source_file = source_directory.child("before.txt").unwrap();
        let destination_file = destination_directory.child("after.txt").unwrap();
        {
            let opfs = Opfs::on_disk(&temp.0).unwrap();
            opfs.get_directory(&bucket, &root, "source", true).unwrap();
            opfs.get_directory(&bucket, &root, "destination", true)
                .unwrap();
            opfs.get_file(&bucket, &source_directory, "before.txt", true)
                .unwrap();
            opfs.write_file(&bucket, &source_file, b"persistent move", None)
                .unwrap();
            assert_eq!(
                opfs.move_entry(
                    &bucket,
                    &source_file,
                    EntryKind::File,
                    &destination_directory,
                    "after.txt",
                    None,
                )
                .unwrap(),
                destination_file
            );
        }

        let reopened = Opfs::on_disk(&temp.0).unwrap();
        assert!(matches!(
            reopened.read_file(&bucket, &source_file),
            Err(OpfsError::NotFound(_))
        ));
        assert_eq!(
            reopened
                .read_file(&bucket, &destination_file)
                .unwrap()
                .bytes,
            b"persistent move"
        );
    }

    #[test]
    fn disk_directory_move_persists_the_entire_subtree_across_restart() {
        let temp = TempRoot::new("disk-directory-move-restart");
        let bucket = bucket();
        let root = OpfsPath::root();
        let source = root.child("source").unwrap();
        let nested = source.child("nested").unwrap();
        let source_file = nested.child("file.txt").unwrap();
        let moved = root.child("moved").unwrap();
        let moved_file = moved.child("nested").unwrap().child("file.txt").unwrap();
        {
            let opfs = Opfs::on_disk(&temp.0).unwrap();
            opfs.get_directory(&bucket, &root, "source", true).unwrap();
            opfs.get_directory(&bucket, &source, "nested", true)
                .unwrap();
            opfs.get_file(&bucket, &nested, "file.txt", true).unwrap();
            opfs.write_file(&bucket, &source_file, b"persistent subtree", None)
                .unwrap();
            assert_eq!(
                opfs.move_entry(&bucket, &source, EntryKind::Directory, &root, "moved", None,)
                    .unwrap(),
                moved
            );
        }

        let reopened = Opfs::on_disk(&temp.0).unwrap();
        assert!(matches!(
            reopened.entry_kind(&bucket, &source),
            Err(OpfsError::NotFound(_))
        ));
        assert_eq!(
            reopened.read_file(&bucket, &moved_file).unwrap().bytes,
            b"persistent subtree"
        );
    }

    #[test]
    fn disk_writers_use_unique_staging_files_and_commit_in_close_order() {
        let temp = TempRoot::new("disk-writer-close-order");
        let bucket = bucket();
        let opfs = Opfs::on_disk(&temp.0).unwrap();
        let file = opfs
            .get_file(&bucket, &OpfsPath::root(), "file", true)
            .unwrap();
        opfs.write_file(&bucket, &file, b"original", None).unwrap();

        let first = opfs
            .create_writable(&bucket, &file, true, WritableMode::Siloed)
            .unwrap();
        opfs.writable_command(
            first,
            WritableCommand::Write {
                data: b"XY".to_vec(),
                position: Some(2),
            },
            None,
        )
        .unwrap();
        let second = opfs
            .create_writable(&bucket, &file, false, WritableMode::Siloed)
            .unwrap();
        opfs.writable_command(
            second,
            WritableCommand::Write {
                data: b"second".to_vec(),
                position: None,
            },
            None,
        )
        .unwrap();

        let bucket_dir = disk_bucket_path(&temp.0, &bucket);
        let staging_dir = bucket_dir.join(super::super::staging::STAGING_DIR_NAME);
        let contents_dir = bucket_dir.join(super::CONTENTS_DIR_NAME);
        assert_eq!(directory_entries(&staging_dir).len(), 2);
        assert_eq!(directory_entries(&contents_dir).len(), 1);
        assert_eq!(opfs.read_file(&bucket, &file).unwrap().bytes, b"original");

        opfs.close_writable(second, None).unwrap();
        assert_eq!(opfs.read_file(&bucket, &file).unwrap().bytes, b"second");
        assert_eq!(directory_entries(&staging_dir).len(), 1);
        assert_eq!(directory_entries(&contents_dir).len(), 1);

        opfs.close_writable(first, None).unwrap();
        assert_eq!(opfs.read_file(&bucket, &file).unwrap().bytes, b"orXYinal");
        assert!(directory_entries(&staging_dir).is_empty());
        assert_eq!(directory_entries(&contents_dir).len(), 1);

        drop(opfs);
        let reopened = Opfs::on_disk(&temp.0).unwrap();
        assert_eq!(
            reopened.read_file(&bucket, &file).unwrap().bytes,
            b"orXYinal"
        );
    }

    #[test]
    fn disk_clear_revokes_active_sessions_and_removes_staging() {
        let temp = TempRoot::new("disk-clear-active-sessions");
        let bucket = bucket();
        let opfs = Opfs::on_disk(&temp.0).unwrap();
        let root = OpfsPath::root();
        let file = opfs.get_file(&bucket, &root, "writer", true).unwrap();
        opfs.write_file(&bucket, &file, b"committed", None).unwrap();
        let writer = opfs
            .create_writable(&bucket, &file, false, WritableMode::Exclusive)
            .unwrap();
        opfs.writable_command(
            writer,
            WritableCommand::Write {
                data: b"uncommitted".to_vec(),
                position: None,
            },
            None,
        )
        .unwrap();
        let sync_file = opfs.get_file(&bucket, &root, "sync", true).unwrap();
        let sync = opfs
            .create_sync_access_handle(&bucket, &sync_file, SyncAccessMode::Readwrite)
            .unwrap();
        opfs.sync_write(sync, b"dirty", None, None).unwrap();

        let staging_dir =
            disk_bucket_path(&temp.0, &bucket).join(super::super::staging::STAGING_DIR_NAME);
        assert_eq!(directory_entries(&staging_dir).len(), 1);
        opfs.clear_bucket(&bucket).unwrap();

        assert!(directory_entries(&staging_dir).is_empty());
        assert!(matches!(
            opfs.writable_command(writer, WritableCommand::Seek(0), None),
            Err(OpfsError::InvalidState)
        ));
        assert!(matches!(
            opfs.close_writable(writer, None),
            Err(OpfsError::InvalidState)
        ));
        assert!(matches!(opfs.sync_size(sync), Err(OpfsError::InvalidState)));
        assert!(matches!(
            opfs.read_file(&bucket, &file),
            Err(OpfsError::NotFound(_))
        ));
        assert_eq!(opfs.usage(&bucket).unwrap(), 0);

        let recreated = opfs.get_file(&bucket, &root, "writer", true).unwrap();
        let replacement = opfs
            .create_writable(&bucket, &recreated, false, WritableMode::Exclusive)
            .unwrap();
        opfs.abort_writable(replacement).unwrap();
    }

    #[test]
    fn disk_clear_and_restart_never_reuse_file_snapshot_identity() {
        let temp = TempRoot::new("disk-clear-snapshot-identity");
        let bucket = bucket();
        let root = OpfsPath::root();
        let file = root.child("same-path").unwrap();
        let original_identity;
        let replacement_identity;
        {
            let opfs = Opfs::on_disk(&temp.0).unwrap();
            opfs.get_file(&bucket, &root, "same-path", true).unwrap();
            opfs.write_file(&bucket, &file, b"same-bytes", None)
                .unwrap();
            original_identity = opfs.read_file(&bucket, &file).unwrap().identity;

            opfs.clear_bucket(&bucket).unwrap();
            opfs.get_file(&bucket, &root, "same-path", true).unwrap();
            opfs.write_file(&bucket, &file, b"same-bytes", None)
                .unwrap();
            replacement_identity = opfs.read_file(&bucket, &file).unwrap().identity;
            assert_ne!(original_identity, replacement_identity);
            assert!(matches!(
                opfs.validate_file_snapshot(&bucket, &file, original_identity),
                Err(OpfsError::NotFound(_))
            ));
        }

        let reopened = Opfs::on_disk(&temp.0).unwrap();
        reopened
            .validate_file_snapshot(&bucket, &file, replacement_identity)
            .unwrap();
        assert!(matches!(
            reopened.validate_file_snapshot(&bucket, &file, original_identity),
            Err(OpfsError::NotFound(_))
        ));
    }

    #[test]
    fn disk_close_rechecks_quota_and_rolls_back_staging() {
        let temp = TempRoot::new("disk-close-quota-rollback");
        let bucket = bucket();
        let opfs = Opfs::on_disk(&temp.0).unwrap();
        let root = OpfsPath::root();
        let file = opfs.get_file(&bucket, &root, "target", true).unwrap();
        let sibling = opfs.get_file(&bucket, &root, "sibling", true).unwrap();
        opfs.write_file(&bucket, &file, b"a", None).unwrap();
        let baseline = opfs.usage(&bucket).unwrap();
        let quota = baseline + 4;

        let writer = opfs
            .create_writable(&bucket, &file, false, WritableMode::Exclusive)
            .unwrap();
        opfs.writable_command(
            writer,
            WritableCommand::Write {
                data: b"12345".to_vec(),
                position: None,
            },
            Some(quota),
        )
        .unwrap();
        opfs.write_file(&bucket, &sibling, b"x", None).unwrap();

        assert!(matches!(
            opfs.close_writable(writer, Some(quota)),
            Err(OpfsError::QuotaExceeded { .. })
        ));
        assert_eq!(opfs.read_file(&bucket, &file).unwrap().bytes, b"a");
        let staging_dir =
            disk_bucket_path(&temp.0, &bucket).join(super::super::staging::STAGING_DIR_NAME);
        assert!(directory_entries(&staging_dir).is_empty());
        let replacement = opfs
            .create_writable(&bucket, &file, false, WritableMode::Exclusive)
            .unwrap();
        opfs.abort_writable(replacement).unwrap();
    }

    #[test]
    fn disk_restart_collects_orphan_staging_and_content() {
        let temp = TempRoot::new("disk-restart-gc");
        let bucket = bucket();
        let file;
        let bucket_dir;
        {
            let opfs = Opfs::on_disk(&temp.0).unwrap();
            file = opfs
                .get_file(&bucket, &OpfsPath::root(), "persistent", true)
                .unwrap();
            opfs.write_file(&bucket, &file, b"kept", None).unwrap();
            bucket_dir = disk_bucket_path(&temp.0, &bucket);
        }

        let staging_dir = bucket_dir.join(super::super::staging::STAGING_DIR_NAME);
        fs::write(staging_dir.join("writer-deadbeef.stage"), b"orphan").unwrap();
        let nested = staging_dir.join("orphan-directory");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("payload"), b"orphan").unwrap();
        let orphan_content = bucket_dir
            .join(super::CONTENTS_DIR_NAME)
            .join("ffffffffffffffff.bin");
        fs::write(&orphan_content, b"orphan").unwrap();

        let reopened = Opfs::on_disk(&temp.0).unwrap();
        assert_eq!(
            directory_entries(&staging_dir).len(),
            2,
            "constructing a reopened backend must defer recovery"
        );
        assert!(orphan_content.exists());
        assert_eq!(reopened.read_file(&bucket, &file).unwrap().bytes, b"kept");
        assert!(directory_entries(&staging_dir).is_empty());
        assert!(!orphan_content.exists());
        assert_eq!(
            directory_entries(&bucket_dir.join(super::CONTENTS_DIR_NAME)).len(),
            1
        );
    }
}
