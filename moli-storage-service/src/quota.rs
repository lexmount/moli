use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use parking_lot::{Condvar, Mutex};

use crate::StorageBucketLocator;

type QuotaReady = Box<dyn FnOnce(StorageQuotaReservation) + Send>;

/// Serialize aggregate quota publication for each persistent bucket. Async
/// callers wait here without occupying a storage or SQLite worker.
#[derive(Clone, Debug, Default)]
pub(crate) struct StorageQuotaCoordinator {
    inner: Arc<StorageQuotaCoordinatorInner>,
}

#[derive(Debug, Default)]
struct StorageQuotaCoordinatorInner {
    state: Mutex<QuotaState>,
    changed: Condvar,
}

#[derive(Default)]
struct QuotaState {
    active: BTreeSet<StorageBucketLocator>,
    waiting: BTreeMap<StorageBucketLocator, VecDeque<QuotaReady>>,
    ready: VecDeque<(StorageBucketLocator, QuotaReady)>,
    draining: bool,
}

impl fmt::Debug for QuotaState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuotaState")
            .field("active", &self.active)
            .field("waiting_buckets", &self.waiting.len())
            .field("ready", &self.ready.len())
            .field("draining", &self.draining)
            .finish()
    }
}

impl StorageQuotaCoordinator {
    pub(crate) fn try_reserve(
        &self,
        locator: &StorageBucketLocator,
    ) -> Option<StorageQuotaReservation> {
        let mut state = self.inner.state.lock();
        if !state.active.insert(locator.clone()) {
            return None;
        }
        Some(StorageQuotaReservation {
            coordinator: self.clone(),
            locator: Some(locator.clone()),
        })
    }

    pub(crate) fn reserve(&self, locator: &StorageBucketLocator) -> StorageQuotaReservation {
        let mut state = self.inner.state.lock();
        while state.active.contains(locator) {
            self.inner.changed.wait(&mut state);
        }
        state.active.insert(locator.clone());
        StorageQuotaReservation {
            coordinator: self.clone(),
            locator: Some(locator.clone()),
        }
    }

    /// The callback only enqueues work; it never performs storage I/O itself.
    pub(crate) fn reserve_async(
        &self,
        locator: StorageBucketLocator,
        ready: impl FnOnce(StorageQuotaReservation) + Send + 'static,
    ) {
        let ready: QuotaReady = Box::new(ready);
        let drain = {
            let mut state = self.inner.state.lock();
            if !state.active.insert(locator.clone()) {
                state.waiting.entry(locator).or_default().push_back(ready);
                return;
            }
            state.enqueue_ready(locator, ready)
        };
        if drain {
            self.drain_ready();
        }
    }

    fn drain_ready(&self) {
        loop {
            let next = {
                let mut state = self.inner.state.lock();
                let next = state.ready.pop_front();
                if next.is_none() {
                    state.draining = false;
                }
                next
            };
            let Some((locator, ready)) = next else {
                self.inner.changed.notify_all();
                return;
            };
            // A failed enqueue may immediately drop its reservation. Drain
            // handoffs iteratively, including panicking completion callbacks.
            let _ = catch_unwind(AssertUnwindSafe(|| {
                ready(StorageQuotaReservation {
                    coordinator: self.clone(),
                    locator: Some(locator),
                });
            }));
        }
    }

    /// Shutdown waits for accepted waiters to enqueue their work, rather than
    /// shutting down SQL while a quota callback still needs to submit to it.
    /// Synchronous reservations without pending async work do not delay this.
    pub(crate) fn wait_for_dispatch(&self) {
        let mut state = self.inner.state.lock();
        while !state.waiting.is_empty() || state.draining {
            self.inner.changed.wait(&mut state);
        }
    }

    fn release(&self, locator: StorageBucketLocator) {
        let drain = {
            let mut state = self.inner.state.lock();
            let next = state
                .waiting
                .get_mut(&locator)
                .and_then(VecDeque::pop_front);
            if state.waiting.get(&locator).is_some_and(VecDeque::is_empty) {
                state.waiting.remove(&locator);
            }
            match next {
                Some(ready) => state.enqueue_ready(locator, ready),
                None => {
                    let removed = state.active.remove(&locator);
                    debug_assert!(removed, "released bucket quota reservation must be active");
                    false
                }
            }
        };
        self.inner.changed.notify_all();
        if drain {
            self.drain_ready();
        }
    }
}

impl QuotaState {
    fn enqueue_ready(&mut self, locator: StorageBucketLocator, ready: QuotaReady) -> bool {
        self.ready.push_back((locator, ready));
        if self.draining {
            false
        } else {
            self.draining = true;
            true
        }
    }
}

/// Owned guard for one bucket's aggregate publication window. Release before
/// delivering the result; dropping on error also advances the next waiter.
#[derive(Debug)]
pub struct StorageQuotaReservation {
    coordinator: StorageQuotaCoordinator,
    locator: Option<StorageBucketLocator>,
}

impl StorageQuotaReservation {
    pub fn locator(&self) -> &StorageBucketLocator {
        self.locator
            .as_ref()
            .expect("live quota reservation must retain its locator")
    }
}

impl Drop for StorageQuotaReservation {
    fn drop(&mut self) {
        if let Some(locator) = self.locator.take() {
            self.coordinator.release(locator);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Barrier,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    use crate::{StorageBucketId, StorageBucketLocator};

    use super::StorageQuotaCoordinator;

    fn named_bucket(id: u64) -> StorageBucketLocator {
        StorageBucketLocator::named(
            "storage-key:v1;origin=https://quota.example",
            StorageBucketId::new(id).expect("test bucket ID should be non-zero"),
        )
    }

    #[test]
    fn same_bucket_quota_reservations_are_exclusive_and_drop_releases() {
        let coordinator = StorageQuotaCoordinator::default();
        let locator = named_bucket(1);
        let first = coordinator.reserve(&locator);
        let (entered_tx, entered_rx) = mpsc::channel();
        let waiting_coordinator = coordinator.clone();
        let waiting_locator = locator.clone();
        let waiter = thread::spawn(move || {
            let _second = waiting_coordinator.reserve(&waiting_locator);
            entered_tx
                .send(())
                .expect("test receiver should remain live");
        });

        assert!(entered_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(first);
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("dropping the first reservation should release the waiter");
        waiter.join().expect("quota waiter should not panic");
    }

    #[test]
    fn different_bucket_quota_reservations_can_overlap() {
        let coordinator = StorageQuotaCoordinator::default();
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let overlap = Arc::new(Barrier::new(3));
        let (entered_tx, entered_rx) = mpsc::channel();
        let mut workers = Vec::new();
        for id in [1, 2] {
            let coordinator = coordinator.clone();
            let active = active.clone();
            let peak = peak.clone();
            let overlap = overlap.clone();
            let entered_tx = entered_tx.clone();
            workers.push(thread::spawn(move || {
                let _reservation = coordinator.reserve(&named_bucket(id));
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                entered_tx
                    .send(())
                    .expect("test receiver should remain live");
                overlap.wait();
                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        drop(entered_tx);

        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first bucket should enter");
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second bucket should enter independently");
        overlap.wait();
        for worker in workers {
            worker.join().expect("quota worker should not panic");
        }
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn async_waiters_preserve_bucket_fifo_without_blocking_other_buckets() {
        let coordinator = StorageQuotaCoordinator::default();
        let first = coordinator.reserve(&named_bucket(1));
        let (sender, receiver) = mpsc::channel();
        for index in [1, 2] {
            let sender = sender.clone();
            coordinator.reserve_async(named_bucket(1), move |guard| {
                sender.send((index, guard)).unwrap();
            });
        }
        coordinator.reserve_async(named_bucket(2), move |guard| {
            sender.send((3, guard)).unwrap();
        });
        let (index, unrelated) = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(index, 3);
        drop(unrelated);
        assert!(receiver.try_recv().is_err());
        drop(first);
        let (index, second) = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(index, 1);
        assert!(receiver.try_recv().is_err());
        drop(second);
        let (index, third) = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(index, 2);
        coordinator.wait_for_dispatch();
        assert!(coordinator.try_reserve(&named_bucket(1)).is_none());
        drop(third);
        assert!(coordinator.try_reserve(&named_bucket(1)).is_some());
    }

    #[test]
    fn immediately_released_async_waiters_do_not_recurse_or_stall_after_a_panic() {
        let coordinator = StorageQuotaCoordinator::default();
        let first = coordinator.reserve(&named_bucket(1));
        let completed = Arc::new(AtomicUsize::new(0));
        for index in 0..10_000 {
            let completed = completed.clone();
            coordinator.reserve_async(named_bucket(1), move |_guard| {
                completed.fetch_add(1, Ordering::Relaxed);
                assert_ne!(index, 5_000, "injected callback panic");
            });
        }
        drop(first);
        coordinator.wait_for_dispatch();
        assert_eq!(completed.load(Ordering::Relaxed), 10_000);
        assert!(coordinator.try_reserve(&named_bucket(1)).is_some());
    }
}
