use std::{
    fmt,
    io::{self, Write},
};

use crate::{allocation::Extent, pool::DiskPool};

/// Immutable data stored in one pool extent.
///
/// Dropping this value returns its extent to the pool. The pool itself remains
/// alive for as long as any stored data refers to it.
pub struct DiskData {
    pool: DiskPool,
    extent: Extent,
}

impl DiskData {
    pub(crate) fn new(pool: DiskPool, extent: Extent) -> Self {
        Self { pool, extent }
    }

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
        self.pool.read_exact_file_at(buffer, absolute_offset)
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
