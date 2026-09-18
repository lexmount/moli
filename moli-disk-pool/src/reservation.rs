use std::fmt;

use crate::{allocation::Extent, pool::DiskPool};

/// A reserved extent that is automatically returned unless it is written.
pub struct ReservedChunk {
    pool: DiskPool,
    extent: Option<Extent>,
}

impl ReservedChunk {
    pub(crate) fn new(pool: DiskPool, extent: Extent) -> Self {
        Self {
            pool,
            extent: Some(extent),
        }
    }

    pub(crate) fn belongs_to(&self, pool: &DiskPool) -> bool {
        self.pool.shares_state_with(pool)
    }

    pub(crate) fn take_extent(&mut self) -> Extent {
        self.extent
            .take()
            .expect("reserved disk chunk should contain an extent")
    }

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
