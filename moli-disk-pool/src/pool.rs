use std::{fmt, io, sync::Arc};

use parking_lot::Mutex;

use crate::{
    allocation::{AllocationState, Extent, find_free_chunk, release_chunk},
    data::DiskData,
    file::PoolFile,
    reservation::ReservedChunk,
};

/// A thread-safe allocator over one anonymous temporary file.
#[derive(Clone)]
pub struct DiskPool {
    inner: Arc<DiskPoolInner>,
}

struct DiskPoolInner {
    file: PoolFile,
    max_capacity: Option<u64>,
    state: Mutex<AllocationState>,
}

/// A point-in-time snapshot of allocator state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskPoolDiagnostics {
    pub may_write: bool,
    pub disk_footprint_bytes: u64,
    pub free_bytes: usize,
    pub free_chunk_count: usize,
}

impl DiskPool {
    /// Creates a pool backed by one delete-on-close temporary file.
    ///
    /// `max_capacity` limits the high-water file footprint. Released extents
    /// remain reusable within that limit, matching Blink's allocator.
    pub fn new(max_capacity: Option<u64>) -> io::Result<Self> {
        let file = tempfile::tempfile()?;
        Ok(Self {
            inner: Arc::new(DiskPoolInner {
                file: PoolFile::new(file),
                max_capacity,
                state: Mutex::new(AllocationState {
                    may_write: true,
                    ..AllocationState::default()
                }),
            }),
        })
    }

    /// Whether future reservations may succeed.
    ///
    /// This is deliberately only a hint: a later disk write can still fail.
    pub fn may_write(&self) -> bool {
        self.inner.state.lock().may_write
    }

    /// Reserves an extent, preferring an exact free extent and then the
    /// largest fitting free extent before growing the file tail.
    pub fn try_reserve_chunk(&self, size: usize) -> Option<ReservedChunk> {
        if size == 0 {
            return None;
        }

        let mut state = self.inner.state.lock();
        if !state.may_write {
            return None;
        }

        let extent = find_free_chunk(&mut state, size).or_else(|| {
            let size = u64::try_from(size).ok()?;
            let new_tail = state.file_tail.checked_add(size)?;
            if self
                .inner
                .max_capacity
                .is_some_and(|capacity| new_tail > capacity)
            {
                return None;
            }
            let extent = Extent {
                offset: state.file_tail,
                len: usize::try_from(size).ok()?,
            };
            state.file_tail = new_tail;
            Some(extent)
        })?;

        Some(ReservedChunk::new(self.clone(), extent))
    }

    /// Writes exactly one reserved extent.
    ///
    /// As in Blink, a disk write failure releases the extent and disables
    /// later writes because failures such as a full disk are rarely transient.
    pub fn write(&self, mut chunk: ReservedChunk, data: &[u8]) -> io::Result<DiskData> {
        if !chunk.belongs_to(self) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reserved chunk belongs to another disk pool",
            ));
        }

        let extent = chunk.take_extent();
        if extent.len != data.len() {
            self.release(extent);
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "data length does not match reserved disk extent",
            ));
        }

        if let Err(error) = self.inner.file.write_all_at(data, extent.offset) {
            let mut state = self.inner.state.lock();
            release_chunk(&mut state, extent);
            state.may_write = false;
            return Err(error);
        }

        Ok(DiskData::new(self.clone(), extent))
    }

    /// Reserves and writes one extent.
    ///
    /// `Ok(None)` means the allocator is disabled or has reached its capacity;
    /// an `Err` means an attempted disk write failed.
    pub fn store(&self, data: &[u8]) -> io::Result<Option<DiskData>> {
        let Some(chunk) = self.try_reserve_chunk(data.len()) else {
            return Ok(None);
        };
        self.write(chunk, data).map(Some)
    }

    pub fn diagnostics(&self) -> DiskPoolDiagnostics {
        let state = self.inner.state.lock();
        DiskPoolDiagnostics {
            may_write: state.may_write,
            disk_footprint_bytes: state.file_tail,
            free_bytes: state.free_bytes,
            free_chunk_count: state.free_chunks_by_offset.len(),
        }
    }

    pub(crate) fn shares_state_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub(crate) fn release(&self, extent: Extent) {
        release_chunk(&mut self.inner.state.lock(), extent);
    }

    pub(crate) fn read_exact_file_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<()> {
        self.inner.file.read_exact_at(buffer, offset)
    }

    #[cfg(test)]
    pub(crate) fn free_chunks_for_test(&self) -> Vec<(u64, usize)> {
        self.inner
            .state
            .lock()
            .free_chunks_by_offset
            .iter()
            .map(|(&offset, &len)| (offset, len))
            .collect()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn fail_next_write_for_test(&self) {
        self.inner.file.fail_next_write_for_test();
    }

    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn truncate_file_for_test(&self, len: u64) -> io::Result<()> {
        self.inner.file.set_len(len)
    }
}

impl fmt::Debug for DiskPool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiskPool")
            .field("diagnostics", &self.diagnostics())
            .finish_non_exhaustive()
    }
}
