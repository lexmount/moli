//! Shared single-file storage for resource data.
//!
//! The allocation policy follows Blink's `DiskDataAllocator`: allocations are
//! represented by `(offset, len)` extents in one anonymous temporary file,
//! exact free extents are preferred, otherwise the largest fitting extent is
//! split, and adjacent free extents are coalesced on release.

mod allocation;
mod data;
mod file;
mod pool;
mod reservation;

pub use data::DiskData;
pub use pool::{DiskPool, DiskPoolDiagnostics};
pub use reservation::ReservedChunk;

#[cfg(test)]
mod tests;
