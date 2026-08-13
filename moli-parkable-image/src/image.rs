use std::{
    fmt, io,
    ops::Deref,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use moli_disk_pool::DiskData;
use parking_lot::Mutex;

use crate::manager::ParkableImageManager;

/// Encoded image bytes that can move between memory and one disk extent.
#[derive(Clone)]
pub struct ParkableImage {
    inner: Arc<ParkableImageInner>,
}

pub struct WeakParkableImage {
    inner: Weak<ParkableImageInner>,
}

struct ParkableImageInner {
    manager: ParkableImageManager,
    state: Mutex<ParkableImageState>,
}

enum ParkableImageState {
    Mutable(Vec<u8>),
    Frozen(FrozenImageState),
}

struct FrozenImageState {
    storage: FrozenImageStorage,
    frozen_at: Instant,
    used: bool,
    len: usize,
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
#[derive(Clone)]
pub struct ParkableImageSnapshot {
    data: Arc<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkableImageMutationError {
    Frozen,
    AlreadyFrozen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkOutcome {
    Parked,
    AlreadyParked,
    NotFrozen,
    BelowMinimum,
    Delayed { remaining: Duration },
    InUse,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkableImageStorageState {
    Mutable,
    Resident,
    Parking,
    ResidentWithDiskBackup,
    Parked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParkableImageDiagnostics {
    pub len: usize,
    pub storage: ParkableImageStorageState,
    pub used: bool,
    pub snapshot_count: usize,
    pub retained_memory_bytes: usize,
    pub retained_disk_bytes: usize,
}

impl ParkableImage {
    pub(crate) fn new_mutable(manager: ParkableImageManager, initial_capacity: usize) -> Self {
        Self {
            inner: Arc::new(ParkableImageInner {
                manager,
                state: Mutex::new(ParkableImageState::Mutable(Vec::with_capacity(
                    initial_capacity,
                ))),
            }),
        }
    }

    pub(crate) fn new_frozen(manager: ParkableImageManager, bytes: Vec<u8>) -> Self {
        Self {
            inner: Arc::new(ParkableImageInner {
                manager,
                state: Mutex::new(ParkableImageState::Frozen(FrozenImageState {
                    len: bytes.len(),
                    storage: FrozenImageStorage::Resident {
                        bytes: Arc::new(bytes),
                        disk_backup: None,
                    },
                    frozen_at: Instant::now(),
                    used: false,
                })),
            }),
        }
    }

    pub fn append(&self, bytes: &[u8]) -> Result<(), ParkableImageMutationError> {
        let mut state = self.inner.state.lock();
        let ParkableImageState::Mutable(buffer) = &mut *state else {
            return Err(ParkableImageMutationError::Frozen);
        };
        buffer.extend_from_slice(bytes);
        Ok(())
    }

    pub fn freeze(&self) -> Result<(), ParkableImageMutationError> {
        let mut state = self.inner.state.lock();
        let ParkableImageState::Mutable(_) = &*state else {
            return Err(ParkableImageMutationError::AlreadyFrozen);
        };
        let ParkableImageState::Mutable(bytes) =
            std::mem::replace(&mut *state, ParkableImageState::Mutable(Vec::new()))
        else {
            unreachable!("checked mutable parkable image state")
        };
        *state = ParkableImageState::Frozen(FrozenImageState {
            len: bytes.len(),
            storage: FrozenImageStorage::Resident {
                bytes: Arc::new(bytes),
                disk_backup: None,
            },
            frozen_at: Instant::now(),
            used: false,
        });
        Ok(())
    }

    pub fn is_frozen(&self) -> bool {
        matches!(*self.inner.state.lock(), ParkableImageState::Frozen(_))
    }

    pub fn len(&self) -> usize {
        match &*self.inner.state.lock() {
            ParkableImageState::Mutable(bytes) => bytes.len(),
            ParkableImageState::Frozen(frozen) => frozen.len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns whether two handles refer to the same encoded image backing.
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Returns a zero-copy read-only snapshot of resident bytes. A parked image
    /// is synchronously read back into memory first.
    pub fn snapshot(&self) -> io::Result<ParkableImageSnapshot> {
        let mut state = self.inner.state.lock();
        match &mut *state {
            ParkableImageState::Mutable(bytes) => Ok(ParkableImageSnapshot {
                data: Arc::new(bytes.clone()),
            }),
            ParkableImageState::Frozen(frozen) => {
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
                    return Ok(ParkableImageSnapshot { data: bytes });
                }
                let bytes = match &frozen.storage {
                    FrozenImageStorage::Resident { bytes, .. }
                    | FrozenImageStorage::Parking { bytes } => Arc::clone(bytes),
                    FrozenImageStorage::Parked { .. } => {
                        unreachable!("parked storage should have been restored")
                    }
                };
                Ok(ParkableImageSnapshot { data: bytes })
            }
        }
    }

    pub fn data(&self) -> io::Result<Vec<u8>> {
        self.snapshot().map(|snapshot| snapshot.to_vec())
    }

    pub fn read_range(&self, offset: usize, max_len: usize) -> io::Result<Vec<u8>> {
        let snapshot = self.snapshot()?;
        let Some(remaining) = snapshot.get(offset..) else {
            return Ok(Vec::new());
        };
        Ok(remaining[..remaining.len().min(max_len)].to_vec())
    }

    /// Attempts to discard resident bytes according to the Blink parking
    /// policy. A successful first park writes exactly one disk extent.
    pub fn maybe_park(&self) -> io::Result<ParkOutcome> {
        let (pool, chunk, write_bytes) = {
            let mut state = self.inner.state.lock();
            let ParkableImageState::Frozen(frozen) = &mut *state else {
                return Ok(ParkOutcome::NotFrozen);
            };
            let policy = self.inner.manager.policy();
            if frozen.len < policy.min_size_to_park {
                return Ok(ParkOutcome::BelowMinimum);
            }
            match &frozen.storage {
                FrozenImageStorage::Resident { .. } => {}
                FrozenImageStorage::Parking { .. } => return Ok(ParkOutcome::InUse),
                FrozenImageStorage::Parked { .. } => return Ok(ParkOutcome::AlreadyParked),
            }
            if !frozen.used {
                let elapsed = frozen.frozen_at.elapsed();
                if elapsed < policy.parking_delay {
                    return Ok(ParkOutcome::Delayed {
                        remaining: policy.parking_delay - elapsed,
                    });
                }
            }

            let (bytes, disk_backup) = match &frozen.storage {
                FrozenImageStorage::Resident { bytes, disk_backup } => {
                    if Arc::strong_count(bytes) != 1 {
                        return Ok(ParkOutcome::InUse);
                    }
                    (Arc::clone(bytes), disk_backup.clone())
                }
                FrozenImageStorage::Parking { .. } | FrozenImageStorage::Parked { .. } => {
                    unreachable!("non-resident storage returned before parking eligibility")
                }
            };

            if let Some(disk) = disk_backup {
                frozen.storage = FrozenImageStorage::Parked { disk };
                return Ok(ParkOutcome::Parked);
            }

            let Some(pool) = self.inner.manager.disk_pool() else {
                return Ok(ParkOutcome::Unavailable);
            };
            let Some(chunk) = pool.try_reserve_chunk(frozen.len) else {
                return Ok(ParkOutcome::Unavailable);
            };
            frozen.storage = FrozenImageStorage::Parking {
                bytes: Arc::clone(&bytes),
            };
            (pool, chunk, bytes)
        };

        // The pool uses positioned I/O, so the image lock is not needed while
        // the blocking write is in progress. A concurrent snapshot may retain
        // a reader lease; in that case the completed disk extent is kept as a
        // backup and a later sweep can discard memory without writing again.
        let write_result = pool.write(chunk, write_bytes.as_slice());

        let mut state = self.inner.state.lock();
        let ParkableImageState::Frozen(frozen) = &mut *state else {
            unreachable!("a frozen parkable image cannot become mutable")
        };
        let (parking_bytes, no_live_snapshots) = match &frozen.storage {
            FrozenImageStorage::Parking { bytes } => {
                let no_live_snapshots = Arc::strong_count(bytes) == 2;
                (Arc::clone(bytes), no_live_snapshots)
            }
            FrozenImageStorage::Resident { .. } | FrozenImageStorage::Parked { .. } => {
                unreachable!("only snapshots may run while an image is parking")
            }
        };
        match write_result {
            Ok(disk) if no_live_snapshots => {
                frozen.storage = FrozenImageStorage::Parked {
                    disk: Arc::new(disk),
                };
                drop((parking_bytes, write_bytes));
                Ok(ParkOutcome::Parked)
            }
            Ok(disk) => {
                frozen.storage = FrozenImageStorage::Resident {
                    bytes: parking_bytes,
                    disk_backup: Some(Arc::new(disk)),
                };
                drop(write_bytes);
                Ok(ParkOutcome::InUse)
            }
            Err(error) => {
                frozen.storage = FrozenImageStorage::Resident {
                    bytes: parking_bytes,
                    disk_backup: None,
                };
                drop(write_bytes);
                Err(error)
            }
        }
    }

    pub fn diagnostics(&self) -> ParkableImageDiagnostics {
        match &*self.inner.state.lock() {
            ParkableImageState::Mutable(bytes) => ParkableImageDiagnostics {
                len: bytes.len(),
                storage: ParkableImageStorageState::Mutable,
                used: false,
                snapshot_count: 0,
                retained_memory_bytes: bytes.capacity(),
                retained_disk_bytes: 0,
            },
            ParkableImageState::Frozen(frozen) => {
                let (storage, snapshot_count, retained_memory_bytes, retained_disk_bytes) =
                    match &frozen.storage {
                        FrozenImageStorage::Resident {
                            bytes,
                            disk_backup: None,
                        } => (
                            ParkableImageStorageState::Resident,
                            Arc::strong_count(bytes).saturating_sub(1),
                            bytes.capacity(),
                            0,
                        ),
                        FrozenImageStorage::Resident {
                            bytes,
                            disk_backup: Some(disk),
                        } => (
                            ParkableImageStorageState::ResidentWithDiskBackup,
                            Arc::strong_count(bytes).saturating_sub(1),
                            bytes.capacity(),
                            disk.len(),
                        ),
                        FrozenImageStorage::Parking { bytes } => (
                            ParkableImageStorageState::Parking,
                            Arc::strong_count(bytes).saturating_sub(2),
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
        }
    }

    pub fn downgrade(&self) -> WeakParkableImage {
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
    pub fn upgrade(&self) -> Option<ParkableImage> {
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
