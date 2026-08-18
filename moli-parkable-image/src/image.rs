use std::{
    fmt, io,
    ops::Deref,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use moli_disk_pool::{DiskData, DiskPool, ReservedChunk};
use parking_lot::Mutex;

use crate::manager::ParkableImageManager;

/// Encoded image bytes that can move between memory and one disk extent.
#[derive(Clone)]
pub struct ParkableImage {
    inner: Arc<ParkableImageInner>,
}

pub(crate) struct WeakParkableImage {
    inner: Weak<ParkableImageInner>,
}

struct ParkableImageInner {
    id: u64,
    manager: ParkableImageManager,
    state: Mutex<FrozenImageState>,
}

struct FrozenImageState {
    storage: FrozenImageStorage,
    frozen_at: Instant,
    used: bool,
    len: usize,
    reader_count: usize,
}

enum FrozenImageStorage {
    /// Encoded bytes are readable in memory. A disk backup, when present,
    /// makes the next park a memory-only state transition.
    Resident {
        bytes: Arc<Vec<u8>>,
        disk_backup: Option<Arc<DiskData>>,
    },
    /// The first disk write is running without holding the image mutex.
    Parking { bytes: Arc<Vec<u8>> },
    /// Resident bytes have been discarded; reads synchronously restore them.
    Parked { disk: Arc<DiskData> },
}

/// A read-only snapshot. While a snapshot exists, its image cannot discard the
/// corresponding in-memory bytes.
pub struct ParkableImageSnapshot {
    // Field order is intentional: Rust drops struct fields in declaration
    // order, so the encoded-byte reference is released before the lease can
    // make the image eligible for parking and wake the scheduler.
    data: Arc<Vec<u8>>,
    lease: SnapshotLease,
}

struct SnapshotLease {
    owner: WeakParkableImage,
}

enum ParkPreparation {
    Done(ParkOutcome),
    Write {
        pool: DiskPool,
        chunk: ReservedChunk,
        bytes: Arc<Vec<u8>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParkOutcome {
    Parked,
    AlreadyParked,
    BelowMinimum,
    Delayed { remaining: Duration },
    InUse,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParkableImageStorageState {
    Resident,
    Parking,
    ResidentWithDiskBackup,
    Parked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParkableImageDiagnostics {
    pub len: usize,
    pub storage: ParkableImageStorageState,
    pub used: bool,
    pub snapshot_count: usize,
    pub retained_memory_bytes: usize,
    pub retained_disk_bytes: usize,
}

impl Drop for ParkableImageInner {
    fn drop(&mut self) {
        self.manager.unregister(self.id);
    }
}

impl ParkableImage {
    pub(crate) fn new(manager: ParkableImageManager, id: u64, bytes: Vec<u8>) -> Self {
        Self {
            inner: Arc::new(ParkableImageInner {
                id,
                manager,
                state: Mutex::new(FrozenImageState {
                    len: bytes.len(),
                    storage: FrozenImageStorage::Resident {
                        bytes: Arc::new(bytes),
                        disk_backup: None,
                    },
                    frozen_at: Instant::now(),
                    used: false,
                    reader_count: 0,
                }),
            }),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.state.lock().len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Memory retained by the encoded bytes, excluding the image handle.
    pub fn retained_memory_bytes(&self) -> usize {
        self.diagnostics().retained_memory_bytes
    }

    /// Returns whether two handles refer to the same encoded image backing.
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Returns a zero-copy read-only snapshot of resident bytes. A parked image
    /// is synchronously read back into memory first.
    pub fn snapshot(&self) -> io::Result<ParkableImageSnapshot> {
        let snapshot = {
            let mut frozen = self.inner.state.lock();

            frozen.used = true;
            if let FrozenImageStorage::Parked { disk } = &frozen.storage {
                let disk = Arc::clone(disk);
                let data = disk.to_vec()?;
                debug_assert_eq!(data.len(), frozen.len);
                let bytes = Arc::new(data);
                frozen.storage = FrozenImageStorage::Resident {
                    bytes: Arc::clone(&bytes),
                    disk_backup: Some(disk),
                };
                self.inner.manager.move_to_resident_while_image_locked(self);
            }
            let bytes = match &frozen.storage {
                FrozenImageStorage::Resident { bytes, .. }
                | FrozenImageStorage::Parking { bytes, .. } => Arc::clone(bytes),
                FrozenImageStorage::Parked { .. } => {
                    unreachable!("parked storage should have been restored")
                }
            };
            frozen.reader_count = frozen
                .reader_count
                .checked_add(1)
                .expect("parkable image reader count should fit usize");
            ParkableImageSnapshot {
                data: bytes,
                lease: SnapshotLease {
                    owner: self.downgrade(),
                },
            }
        };
        self.inner.manager.notify_schedule_changed();
        Ok(snapshot)
    }

    pub fn read_range(&self, offset: usize, max_len: usize) -> io::Result<Vec<u8>> {
        let snapshot = self.snapshot()?;
        let Some(remaining) = snapshot.get(offset..) else {
            return Ok(Vec::new());
        };
        Ok(remaining[..remaining.len().min(max_len)].to_vec())
    }

    pub(crate) fn id(&self) -> u64 {
        self.inner.id
    }

    fn retain_snapshot_reader(&self) {
        let mut frozen = self.inner.state.lock();
        frozen.reader_count = frozen
            .reader_count
            .checked_add(1)
            .expect("parkable image reader count should fit usize");
    }

    fn release_snapshot_reader(&self) {
        let became_parkable = {
            let mut frozen = self.inner.state.lock();
            frozen.reader_count = frozen
                .reader_count
                .checked_sub(1)
                .expect("parkable image reader leases must remain balanced");
            frozen.reader_count == 0
        };
        if became_parkable {
            self.inner.manager.notify_schedule_changed();
        }
    }

    pub(crate) fn parking_deadline(&self, now: Instant) -> Option<Instant> {
        let frozen = self.inner.state.lock();
        let policy = self.inner.manager.policy();
        if frozen.len < policy.min_size_to_park {
            return None;
        }
        let FrozenImageStorage::Resident { disk_backup, .. } = &frozen.storage else {
            return None;
        };
        if frozen.reader_count != 0 {
            return None;
        }
        if disk_backup.is_none()
            && !self
                .inner
                .manager
                .disk_pool()
                .is_some_and(|pool| pool.may_write())
        {
            return None;
        }
        if frozen.used || disk_backup.is_some() {
            return Some(now);
        }
        frozen.frozen_at.checked_add(policy.parking_delay)
    }

    /// Attempts to discard resident bytes according to the Blink parking
    /// policy. A successful first park writes exactly one disk extent.
    pub(crate) fn maybe_park(&self) -> io::Result<ParkOutcome> {
        match self.prepare_parking() {
            ParkPreparation::Done(outcome) => {
                if outcome == ParkOutcome::Parked {
                    self.inner.manager.notify_schedule_changed();
                }
                Ok(outcome)
            }
            ParkPreparation::Write { pool, chunk, bytes } => {
                let write_result = pool.write(chunk, bytes.as_slice());
                let outcome = self.complete_parking_write(write_result, bytes);
                self.inner.manager.notify_schedule_changed();
                outcome
            }
        }
    }

    fn prepare_parking(&self) -> ParkPreparation {
        let mut frozen = self.inner.state.lock();
        let policy = self.inner.manager.policy();
        if frozen.len < policy.min_size_to_park {
            return ParkPreparation::Done(ParkOutcome::BelowMinimum);
        }
        match &frozen.storage {
            FrozenImageStorage::Resident { .. } => {}
            FrozenImageStorage::Parking { .. } => {
                return ParkPreparation::Done(ParkOutcome::InUse);
            }
            FrozenImageStorage::Parked { .. } => {
                return ParkPreparation::Done(ParkOutcome::AlreadyParked);
            }
        }
        if frozen.reader_count != 0 {
            return ParkPreparation::Done(ParkOutcome::InUse);
        }
        if !frozen.used {
            let elapsed = frozen.frozen_at.elapsed();
            if elapsed < policy.parking_delay {
                return ParkPreparation::Done(ParkOutcome::Delayed {
                    remaining: policy.parking_delay - elapsed,
                });
            }
        }

        let disk_backup = match &frozen.storage {
            FrozenImageStorage::Resident { disk_backup, .. } => disk_backup.clone(),
            FrozenImageStorage::Parking { .. } | FrozenImageStorage::Parked { .. } => {
                unreachable!("non-resident storage returned before parking eligibility")
            }
        };
        if let Some(disk) = disk_backup {
            frozen.storage = FrozenImageStorage::Parked { disk };
            self.inner.manager.move_to_parked_while_image_locked(self);
            return ParkPreparation::Done(ParkOutcome::Parked);
        }

        let Some(pool) = self.inner.manager.disk_pool() else {
            return ParkPreparation::Done(ParkOutcome::Unavailable);
        };
        let Some(chunk) = pool.try_reserve_chunk(frozen.len) else {
            return ParkPreparation::Done(ParkOutcome::Unavailable);
        };
        let bytes = match &frozen.storage {
            FrozenImageStorage::Resident { bytes, .. } => Arc::clone(bytes),
            FrozenImageStorage::Parking { .. } | FrozenImageStorage::Parked { .. } => {
                unreachable!("non-resident storage returned before starting a parking job")
            }
        };
        frozen.storage = FrozenImageStorage::Parking {
            bytes: Arc::clone(&bytes),
        };
        ParkPreparation::Write { pool, chunk, bytes }
    }

    fn complete_parking_write(
        &self,
        write_result: io::Result<DiskData>,
        write_bytes: Arc<Vec<u8>>,
    ) -> io::Result<ParkOutcome> {
        let mut frozen = self.inner.state.lock();
        if !matches!(frozen.storage, FrozenImageStorage::Parking { .. }) {
            return Err(io::Error::other(
                "parking write completed after its state was replaced",
            ));
        }

        match write_result {
            Ok(disk) if frozen.reader_count == 0 => {
                frozen.storage = FrozenImageStorage::Parked {
                    disk: Arc::new(disk),
                };
                self.inner.manager.move_to_parked_while_image_locked(self);
                Ok(ParkOutcome::Parked)
            }
            Ok(disk) => {
                frozen.storage = FrozenImageStorage::Resident {
                    bytes: write_bytes,
                    disk_backup: Some(Arc::new(disk)),
                };
                Ok(ParkOutcome::InUse)
            }
            Err(error) => {
                frozen.storage = FrozenImageStorage::Resident {
                    bytes: write_bytes,
                    disk_backup: None,
                };
                Err(error)
            }
        }
    }

    pub(crate) fn diagnostics(&self) -> ParkableImageDiagnostics {
        let frozen = self.inner.state.lock();
        let (storage, snapshot_count, retained_memory_bytes, retained_disk_bytes) =
            match &frozen.storage {
                FrozenImageStorage::Resident {
                    bytes,
                    disk_backup: None,
                } => (
                    ParkableImageStorageState::Resident,
                    frozen.reader_count,
                    bytes.capacity(),
                    0,
                ),
                FrozenImageStorage::Resident {
                    bytes,
                    disk_backup: Some(disk),
                } => (
                    ParkableImageStorageState::ResidentWithDiskBackup,
                    frozen.reader_count,
                    bytes.capacity(),
                    disk.len(),
                ),
                FrozenImageStorage::Parking { bytes, .. } => (
                    ParkableImageStorageState::Parking,
                    frozen.reader_count,
                    bytes.capacity(),
                    0,
                ),
                FrozenImageStorage::Parked { disk } => {
                    (ParkableImageStorageState::Parked, 0, 0, disk.len())
                }
            };
        ParkableImageDiagnostics {
            len: frozen.len,
            storage,
            used: frozen.used,
            snapshot_count,
            retained_memory_bytes,
            retained_disk_bytes,
        }
    }

    pub(crate) fn downgrade(&self) -> WeakParkableImage {
        WeakParkableImage {
            inner: Arc::downgrade(&self.inner),
        }
    }
}

impl fmt::Debug for ParkableImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParkableImage")
            .field("diagnostics", &self.diagnostics())
            .finish_non_exhaustive()
    }
}

impl WeakParkableImage {
    pub(crate) fn upgrade(&self) -> Option<ParkableImage> {
        self.inner.upgrade().map(|inner| ParkableImage { inner })
    }
}

impl Clone for WeakParkableImage {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl fmt::Debug for WeakParkableImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WeakParkableImage")
            .field("live", &(self.inner.strong_count() != 0))
            .finish()
    }
}

impl Deref for ParkableImageSnapshot {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.data.as_slice()
    }
}

impl Clone for ParkableImageSnapshot {
    fn clone(&self) -> Self {
        let data = Arc::clone(&self.data);
        let lease = self.lease.clone();
        Self { data, lease }
    }
}

impl Clone for SnapshotLease {
    fn clone(&self) -> Self {
        if let Some(image) = self.owner.upgrade() {
            image.retain_snapshot_reader();
        }
        Self {
            owner: self.owner.clone(),
        }
    }
}

impl Drop for SnapshotLease {
    fn drop(&mut self) {
        if let Some(image) = self.owner.upgrade() {
            image.release_snapshot_reader();
        }
    }
}

impl AsRef<[u8]> for ParkableImageSnapshot {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl fmt::Debug for ParkableImageSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParkableImageSnapshot")
            .field("len", &self.data.len())
            .finish_non_exhaustive()
    }
}
