//! Shared single-file storage for resource data.
//!
//! The allocation policy follows Blink's `DiskDataAllocator`: allocations are
//! represented by `(offset, len)` extents in one anonymous temporary file,
//! exact free extents are preferred, otherwise the largest fitting extent is
//! split, and adjacent free extents are coalesced on release.

use std::{
    collections::BTreeMap,
    fmt,
    fs::File,
    io::{self, Write},
    sync::Arc,
};

use parking_lot::Mutex;

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

#[derive(Debug, Default)]
struct AllocationState {
    may_write: bool,
    file_tail: u64,
    free_chunks: BTreeMap<u64, usize>,
    free_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Extent {
    offset: u64,
    len: usize,
}

/// A reserved extent that is automatically returned unless it is written.
pub struct ReservedChunk {
    pool: DiskPool,
    extent: Option<Extent>,
}

/// Immutable data stored in one pool extent.
///
/// Dropping this value returns its extent to the pool. The pool itself remains
/// alive for as long as any stored data refers to it.
pub struct DiskData {
    pool: DiskPool,
    extent: Extent,
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

        Some(ReservedChunk {
            pool: self.clone(),
            extent: Some(extent),
        })
    }

    /// Writes exactly one reserved extent.
    ///
    /// As in Blink, a disk write failure releases the extent and disables
    /// later writes because failures such as a full disk are rarely transient.
    pub fn write(&self, mut chunk: ReservedChunk, data: &[u8]) -> io::Result<DiskData> {
        if !Arc::ptr_eq(&self.inner, &chunk.pool.inner) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reserved chunk belongs to another disk pool",
            ));
        }

        let extent = chunk
            .extent
            .take()
            .expect("reserved disk chunk should contain an extent");
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

        Ok(DiskData {
            pool: self.clone(),
            extent,
        })
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
            free_chunk_count: state.free_chunks.len(),
        }
    }

    fn release(&self, extent: Extent) {
        release_chunk(&mut self.inner.state.lock(), extent);
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

impl ReservedChunk {
    pub fn offset(&self) -> u64 {
        self.extent.map_or(0, |extent| extent.offset)
    }

    pub fn len(&self) -> usize {
        self.extent.map_or(0, |extent| extent.len)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Debug for ReservedChunk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReservedChunk")
            .field("extent", &self.extent)
            .finish_non_exhaustive()
    }
}

impl Drop for ReservedChunk {
    fn drop(&mut self) {
        if let Some(extent) = self.extent.take() {
            self.pool.release(extent);
        }
    }
}

impl DiskData {
    pub fn offset(&self) -> u64 {
        self.extent.offset
    }

    pub fn len(&self) -> usize {
        self.extent.len
    }

    pub fn is_empty(&self) -> bool {
        self.extent.len == 0
    }

    /// Reads an exact range relative to this extent.
    pub fn read_exact_at(&self, offset: usize, buffer: &mut [u8]) -> io::Result<()> {
        let end = offset.checked_add(buffer.len()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "disk data range overflow")
        })?;
        if end > self.extent.len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "disk data read exceeds extent",
            ));
        }
        let absolute_offset = self
            .extent
            .offset
            .checked_add(u64::try_from(offset).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "disk data offset is too large")
            })?)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "disk data offset overflow")
            })?;
        self.pool.inner.file.read_exact_at(buffer, absolute_offset)
    }

    pub fn to_vec(&self) -> io::Result<Vec<u8>> {
        let mut data = vec![0; self.extent.len];
        self.read_exact_at(0, &mut data)?;
        Ok(data)
    }

    pub fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let mut offset = 0;
        let mut buffer = [0_u8; 64 * 1024];
        while offset < self.extent.len {
            let len = buffer.len().min(self.extent.len - offset);
            self.read_exact_at(offset, &mut buffer[..len])?;
            writer.write_all(&buffer[..len])?;
            offset += len;
        }
        Ok(())
    }
}

impl fmt::Debug for DiskData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiskData")
            .field("offset", &self.extent.offset)
            .field("len", &self.extent.len)
            .finish_non_exhaustive()
    }
}

impl Drop for DiskData {
    fn drop(&mut self) {
        self.pool.release(self.extent);
    }
}

fn find_free_chunk(state: &mut AllocationState, size: usize) -> Option<Extent> {
    let mut chosen = None;
    let mut worst_fit_size = 0;
    for (&offset, &chunk_size) in &state.free_chunks {
        if chunk_size == size {
            chosen = Some(Extent {
                offset,
                len: chunk_size,
            });
            break;
        }
        if chunk_size > size && chunk_size > worst_fit_size {
            chosen = Some(Extent {
                offset,
                len: chunk_size,
            });
            worst_fit_size = chunk_size;
        }
    }

    let mut chosen = chosen?;
    state.free_chunks.remove(&chosen.offset);
    state.free_bytes -= size;
    if chosen.len > size {
        let remainder_offset = chosen
            .offset
            .checked_add(u64::try_from(size).ok()?)
            .expect("free disk extent should not overflow");
        let previous = state
            .free_chunks
            .insert(remainder_offset, chosen.len - size);
        debug_assert!(previous.is_none());
        chosen.len = size;
    }
    Some(chosen)
}

fn release_chunk(state: &mut AllocationState, extent: Extent) {
    let original_len = extent.len;
    let mut merged = extent;

    if let Some((&left_offset, &left_len)) = state.free_chunks.range(..merged.offset).next_back() {
        let left_end = left_offset
            .checked_add(u64::try_from(left_len).expect("extent length should fit u64"))
            .expect("free disk extent should not overflow");
        debug_assert!(left_end <= merged.offset);
        if left_end == merged.offset {
            state.free_chunks.remove(&left_offset);
            merged.offset = left_offset;
            merged.len = merged
                .len
                .checked_add(left_len)
                .expect("merged disk extent should fit usize");
        }
    }

    if let Some((&right_offset, &right_len)) = state.free_chunks.range(merged.offset..).next() {
        let merged_end = merged
            .offset
            .checked_add(u64::try_from(merged.len).expect("extent length should fit u64"))
            .expect("free disk extent should not overflow");
        debug_assert!(merged_end <= right_offset);
        if merged_end == right_offset {
            state.free_chunks.remove(&right_offset);
            merged.len = merged
                .len
                .checked_add(right_len)
                .expect("merged disk extent should fit usize");
        }
    }

    let previous = state.free_chunks.insert(merged.offset, merged.len);
    debug_assert!(previous.is_none());
    state.free_bytes = state
        .free_bytes
        .checked_add(original_len)
        .expect("free disk bytes should fit usize");
}

struct PoolFile {
    #[cfg(any(unix, windows))]
    file: File,
    #[cfg(not(any(unix, windows)))]
    file: Mutex<File>,
}

impl PoolFile {
    fn new(file: File) -> Self {
        Self {
            #[cfg(any(unix, windows))]
            file,
            #[cfg(not(any(unix, windows)))]
            file: Mutex::new(file),
        }
    }

    fn read_exact_at(&self, mut buffer: &mut [u8], mut offset: u64) -> io::Result<()> {
        while !buffer.is_empty() {
            match self.read_at(buffer, offset) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "disk pool file ended inside an allocated extent",
                    ));
                }
                Ok(read) => {
                    offset = offset
                        .checked_add(u64::try_from(read).map_err(|_| {
                            io::Error::new(io::ErrorKind::InvalidInput, "read size is too large")
                        })?)
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::InvalidInput, "read offset overflow")
                        })?;
                    buffer = &mut buffer[read..];
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn write_all_at(&self, mut data: &[u8], mut offset: u64) -> io::Result<()> {
        while !data.is_empty() {
            match self.write_at(data, offset) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to write an allocated disk extent",
                    ));
                }
                Ok(written) => {
                    offset = offset
                        .checked_add(u64::try_from(written).map_err(|_| {
                            io::Error::new(io::ErrorKind::InvalidInput, "write size is too large")
                        })?)
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::InvalidInput, "write offset overflow")
                        })?;
                    data = &data[written..];
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    #[cfg(unix)]
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        use std::os::unix::fs::FileExt;

        self.file.read_at(buffer, offset)
    }

    #[cfg(unix)]
    fn write_at(&self, data: &[u8], offset: u64) -> io::Result<usize> {
        use std::os::unix::fs::FileExt;

        self.file.write_at(data, offset)
    }

    #[cfg(windows)]
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        use std::os::windows::fs::FileExt;

        self.file.seek_read(buffer, offset)
    }

    #[cfg(windows)]
    fn write_at(&self, data: &[u8], offset: u64) -> io::Result<usize> {
        use std::os::windows::fs::FileExt;

        self.file.seek_write(data, offset)
    }

    #[cfg(not(any(unix, windows)))]
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        use std::io::{Read, Seek};

        let mut file = self.file.lock();
        file.seek(io::SeekFrom::Start(offset))?;
        file.read(buffer)
    }

    #[cfg(not(any(unix, windows)))]
    fn write_at(&self, data: &[u8], offset: u64) -> io::Result<usize> {
        use std::io::Seek;

        let mut file = self.file.lock();
        file.seek(io::SeekFrom::Start(offset))?;
        file.write(data)
    }

    #[cfg(feature = "test-support")]
    fn set_len(&self, len: u64) -> io::Result<()> {
        #[cfg(any(unix, windows))]
        {
            self.file.set_len(len)
        }
        #[cfg(not(any(unix, windows)))]
        {
            self.file.lock().set_len(len)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn store(pool: &DiskPool, bytes: &[u8]) -> DiskData {
        pool.store(bytes)
            .expect("disk write should succeed")
            .expect("disk pool should have capacity")
    }

    #[test]
    fn reserved_chunk_is_released_on_drop() {
        let pool = DiskPool::new(None).unwrap();
        let first = pool.try_reserve_chunk(100).unwrap();
        assert_eq!(first.offset(), 0);
        let second = pool.try_reserve_chunk(100).unwrap();
        assert_eq!(second.offset(), 100);
        drop(second);

        let reused = pool.try_reserve_chunk(100).unwrap();
        assert_eq!(reused.offset(), 100);
    }

    #[test]
    fn writes_and_reads_without_a_shared_file_cursor() {
        let pool = DiskPool::new(None).unwrap();
        let left = store(&pool, b"left body");
        let right = store(&pool, b"right body");
        assert_eq!(left.offset(), 0);
        assert_eq!(right.offset(), 9);

        let mut middle = [0; 5];
        right.read_exact_at(1, &mut middle).unwrap();
        assert_eq!(&middle, b"ight ");
        assert_eq!(left.to_vec().unwrap(), b"left body");
        assert_eq!(right.to_vec().unwrap(), b"right body");
    }

    #[test]
    fn allocation_prefers_exact_fit_then_worst_fit() {
        let pool = DiskPool::new(None).unwrap();
        let exact_hole = store(&pool, &[1; 100]);
        let separator = store(&pool, &[2; 10]);
        let larger_hole = store(&pool, &[3; 200]);
        let tail = store(&pool, &[4; 10]);
        let exact_offset = exact_hole.offset();
        let larger_offset = larger_hole.offset();
        drop(exact_hole);
        drop(larger_hole);

        let exact = pool.try_reserve_chunk(100).unwrap();
        assert_eq!(exact.offset(), exact_offset);
        drop(exact);

        let worst_fit = pool.try_reserve_chunk(99).unwrap();
        assert_eq!(worst_fit.offset(), larger_offset);
        drop((separator, tail, worst_fit));
    }

    #[test]
    fn adjacent_free_chunks_are_coalesced() {
        let pool = DiskPool::new(None).unwrap();
        let first = store(&pool, &[1; 100]);
        let second = store(&pool, &[2; 100]);
        let third = store(&pool, &[3; 100]);
        let fourth = store(&pool, &[4; 100]);

        drop(first);
        drop(third);
        assert_eq!(pool.diagnostics().free_chunk_count, 2);
        drop(second);
        assert_eq!(pool.diagnostics().free_chunk_count, 1);
        assert_eq!(pool.diagnostics().free_bytes, 300);
        drop(fourth);

        assert_eq!(
            pool.diagnostics(),
            DiskPoolDiagnostics {
                may_write: true,
                disk_footprint_bytes: 400,
                free_bytes: 400,
                free_chunk_count: 1,
            }
        );
    }

    #[test]
    fn capacity_reuses_holes_but_does_not_extend_tail() {
        let pool = DiskPool::new(Some(100)).unwrap();
        let full = store(&pool, &[7; 100]);
        assert!(pool.try_reserve_chunk(1).is_none());
        drop(full);

        let reused = pool.try_reserve_chunk(60).unwrap();
        assert_eq!(reused.offset(), 0);
        assert!(pool.try_reserve_chunk(41).is_none());
        assert!(pool.try_reserve_chunk(40).is_some());
        assert_eq!(pool.diagnostics().disk_footprint_bytes, 100);
    }

    #[test]
    fn stored_data_keeps_pool_alive_and_supports_concurrent_reads() {
        let pool = DiskPool::new(None).unwrap();
        let diagnostics = pool.clone();
        let data = Arc::new(store(&pool, &[9; 128 * 1024]));
        drop(pool);

        let readers: Vec<_> = (0..4)
            .map(|_| {
                let data = Arc::clone(&data);
                std::thread::spawn(move || assert_eq!(data.to_vec().unwrap(), vec![9; data.len()]))
            })
            .collect();
        for reader in readers {
            reader.join().unwrap();
        }
        assert_eq!(diagnostics.diagnostics().free_bytes, 0);
        drop(data);
        assert_eq!(diagnostics.diagnostics().free_bytes, 128 * 1024);
    }
}
