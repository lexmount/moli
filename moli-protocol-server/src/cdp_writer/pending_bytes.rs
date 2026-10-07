use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
pub(super) struct PendingByteBudget {
    current: Arc<AtomicUsize>,
    limit: usize,
}

impl PendingByteBudget {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            current: Arc::new(AtomicUsize::new(0)),
            limit,
        }
    }

    pub(super) fn available(&self) -> usize {
        self.limit
            .saturating_sub(self.current.load(Ordering::Acquire))
    }

    pub(super) fn limit(&self) -> usize {
        self.limit
    }

    pub(super) fn try_reserve(&self, bytes: usize) -> Option<PendingByteReservation> {
        self.current
            .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(bytes)
                    .filter(|pending| *pending <= self.limit)
            })
            .ok()?;
        Some(PendingByteReservation {
            current: self.current.clone(),
            bytes,
        })
    }

    #[cfg(test)]
    pub(super) fn current(&self) -> usize {
        self.current.load(Ordering::Acquire)
    }
}

pub(super) struct PendingByteReservation {
    current: Arc<AtomicUsize>,
    bytes: usize,
}

impl Drop for PendingByteReservation {
    fn drop(&mut self) {
        let previous = self.current.fetch_sub(self.bytes, Ordering::AcqRel);
        debug_assert!(previous >= self.bytes);
    }
}
