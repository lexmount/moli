//! Reservations follow schema snapshots, including old snapshots retained by
//! a completion or client. They are independent of executable task capacity.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::{IndexedDbError, read_budget::ReadBudget};

pub(super) const DEFAULT_METADATA_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(super) struct MetadataBudget(Arc<State>);

#[derive(Debug)]
struct State {
    limit: usize,
    used: AtomicUsize,
}

impl Default for MetadataBudget {
    fn default() -> Self {
        Self::new(DEFAULT_METADATA_BYTES)
    }
}

impl MetadataBudget {
    pub(super) fn new(limit: usize) -> Self {
        Self(Arc::new(State {
            limit,
            used: AtomicUsize::new(0),
        }))
    }

    pub(super) fn used(&self) -> usize {
        self.0.used.load(Ordering::Acquire)
    }

    pub(super) fn reservation(&self) -> MetadataReservation {
        MetadataReservation {
            budget: self.clone(),
            bytes: 0,
        }
    }
}

#[derive(Debug)]
pub(super) struct MetadataReservation {
    budget: MetadataBudget,
    bytes: usize,
}

impl ReadBudget for MetadataReservation {
    fn charge(&mut self, bytes: u64) -> Result<(), IndexedDbError> {
        let bytes = usize::try_from(bytes).map_err(|_| exhausted())?;
        self.budget
            .0
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|total| *total <= self.budget.0.limit)
            })
            .map_err(|_| exhausted())?;
        self.bytes += bytes;
        Ok(())
    }

    fn refund(&mut self, bytes: u64) {
        let bytes = usize::try_from(bytes).expect("refund fits its reservation");
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .expect("reserved metadata refund");
        self.budget.0.used.fetch_sub(bytes, Ordering::AcqRel);
    }
}

impl Drop for MetadataReservation {
    fn drop(&mut self) {
        self.budget.0.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

fn exhausted() -> IndexedDbError {
    IndexedDbError::Resource("IndexedDB schema metadata byte capacity exhausted".into())
}

/// A deliberately conservative node allowance, including an underfilled root
/// and child edges. BTreeMap does not expose allocator capacity. Charging this
/// per entry bounds its nodes without depending on their exact occupancy.
pub(super) fn map_entry_bytes<K, V>() -> u64 {
    (12 * (std::mem::size_of::<K>() + std::mem::size_of::<V>() + std::mem::size_of::<usize>()) + 64)
        as u64
}
