//! Admission of regular transactions across connections and event loops.
//!
//! A transaction waits for every older, unfinished transaction whose scope
//! overlaps and whose mode conflicts, including transactions still waiting to
//! start. The accepting realm owns a lease until backend commit or rollback.

use parking_lot::Mutex;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Weak},
};

use crate::{ConnectionRequestWake, DatabaseHandle, IndexedDbName, TransactionMode};

#[derive(Default)]
pub(crate) struct TransactionRequestQueues {
    state: Mutex<QueueState>,
}

#[derive(Default)]
struct QueueState {
    next_id: u64,
    queues: BTreeMap<(String, IndexedDbName), VecDeque<QueuedRequest>>,
}

struct TransactionScope {
    stores: BTreeSet<IndexedDbName>,
    mode: TransactionMode,
}

impl TransactionScope {
    fn conflicts_with(&self, other: &Self) -> bool {
        (self.mode != TransactionMode::ReadOnly || other.mode != TransactionMode::ReadOnly)
            && !self.stores.is_disjoint(&other.stores)
    }
}

struct QueuedRequest {
    id: u64,
    database: DatabaseHandle,
    scope: Arc<TransactionScope>,
    ready: bool,
    wake: ConnectionRequestWake,
}

#[derive(Clone)]
pub struct TransactionRequestHandle {
    queues: Weak<TransactionRequestQueues>,
    key: (String, IndexedDbName),
    id: u64,
    database: DatabaseHandle,
    scope: Arc<TransactionScope>,
}

/// Drop only after committing or aborting the backend transaction. The lease
/// never retains the storage partition or any JS object.
pub struct TransactionRequestLease(TransactionRequestHandle);

impl TransactionRequestLease {
    pub fn handle(&self) -> &TransactionRequestHandle {
        &self.0
    }
}

impl Drop for TransactionRequestLease {
    fn drop(&mut self) {
        self.0.finish();
    }
}

impl TransactionRequestQueues {
    pub(crate) fn enqueue(
        self: &Arc<Self>,
        key: (String, IndexedDbName),
        database: DatabaseHandle,
        stores: BTreeSet<IndexedDbName>,
        mode: TransactionMode,
        wake: ConnectionRequestWake,
    ) -> TransactionRequestLease {
        let scope = Arc::new(TransactionScope { stores, mode });
        let mut state = self.state.lock();
        state.next_id = state
            .next_id
            .checked_add(1)
            .expect("IDB transaction id overflow");
        let id = state.next_id;
        let queue = state.queues.entry(key.clone()).or_default();
        let ready = !queue.iter().any(|older| scope.conflicts_with(&older.scope));
        queue.push_back(QueuedRequest {
            id,
            database,
            scope: scope.clone(),
            ready,
            wake,
        });
        // Initial readiness is returned to the caller. A wake here could run
        // before the accepting realm has attached its transaction wrapper.
        TransactionRequestLease(TransactionRequestHandle {
            queues: Arc::downgrade(self),
            key,
            id,
            database,
            scope,
        })
    }

    /// Called after rolling back the connection's backend transactions. Wakes
    /// may inspect this coordinator, but must not reenter the storage manager.
    pub(crate) fn cancel_connection(&self, database: DatabaseHandle) {
        let wakes = {
            let mut state = self.state.lock();
            let mut wakes = Vec::new();
            state.queues.retain(|_, queue| {
                let before = queue.len();
                queue.retain(|entry| entry.database != database);
                if before != queue.len() {
                    wake_newly_ready(queue, &mut wakes);
                }
                !queue.is_empty()
            });
            wakes
        };
        for wake in wakes {
            wake();
        }
    }
}

fn wake_newly_ready(queue: &mut VecDeque<QueuedRequest>, wakes: &mut Vec<ConnectionRequestWake>) {
    for index in 0..queue.len() {
        if !queue[index].ready
            && !queue
                .iter()
                .take(index)
                .any(|older| queue[index].scope.conflicts_with(&older.scope))
        {
            queue[index].ready = true;
            wakes.push(queue[index].wake.clone());
        }
    }
}

impl TransactionRequestHandle {
    pub fn is_ready(&self) -> bool {
        self.queues.upgrade().is_some_and(|queues| {
            queues
                .state
                .lock()
                .queues
                .get(&self.key)
                .is_some_and(|queue| queue.iter().any(|entry| entry.id == self.id && entry.ready))
        })
    }

    pub(crate) fn belongs_to(&self, queues: &Arc<TransactionRequestQueues>) -> bool {
        self.queues.ptr_eq(&Arc::downgrade(queues))
    }

    pub(crate) fn database(&self) -> DatabaseHandle {
        self.database
    }

    pub(crate) fn store_names(&self) -> Vec<IndexedDbName> {
        self.scope.stores.iter().cloned().collect()
    }

    pub(crate) fn mode(&self) -> TransactionMode {
        self.scope.mode
    }

    fn finish(&self) {
        let Some(queues) = self.queues.upgrade() else {
            return;
        };
        let wakes = {
            let mut state = queues.state.lock();
            let Some(queue) = state.queues.get_mut(&self.key) else {
                return;
            };
            let Some(index) = queue.iter().position(|entry| entry.id == self.id) else {
                return;
            };
            queue.remove(index);
            let mut wakes = Vec::new();
            wake_newly_ready(queue, &mut wakes);
            if queue.is_empty() {
                state.queues.remove(&self.key);
            }
            wakes
        };
        for wake in wakes {
            wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn request(
        queues: &Arc<TransactionRequestQueues>,
        connection: u64,
        stores: &[&str],
        mode: TransactionMode,
        wake: ConnectionRequestWake,
    ) -> TransactionRequestLease {
        queues.enqueue(
            ("origin".into(), "db".into()),
            DatabaseHandle::from_raw(connection),
            stores.iter().map(|store| (*store).into()).collect(),
            mode,
            wake,
        )
    }

    #[test]
    fn pending_writers_block_later_readers_across_connections_and_cancel_on_drop() {
        let queues = Arc::new(TransactionRequestQueues::default());
        let wakes = Arc::new(AtomicUsize::new(0));
        let wake: ConnectionRequestWake = {
            let queues = Arc::downgrade(&queues);
            let wakes = wakes.clone();
            Arc::new(move || {
                // Wakes must run without holding the coordinator lock.
                assert!(queues.upgrade().unwrap().state.try_lock().is_some());
                wakes.fetch_add(1, Ordering::SeqCst);
            })
        };
        let reader = request(&queues, 1, &["a"], TransactionMode::ReadOnly, wake.clone());
        let parallel_reader = request(&queues, 2, &["a"], TransactionMode::ReadOnly, wake.clone());
        let writer = request(
            &queues,
            2,
            &["a", "b"],
            TransactionMode::ReadWrite,
            wake.clone(),
        );
        let last_reader = request(&queues, 3, &["b"], TransactionMode::ReadOnly, wake.clone());
        let independent = request(&queues, 1, &["c"], TransactionMode::ReadWrite, wake);
        assert!(reader.0.is_ready());
        assert!(parallel_reader.0.is_ready());
        assert!(!writer.0.is_ready());
        assert!(!last_reader.0.is_ready());
        assert!(independent.0.is_ready());
        assert_eq!(wakes.load(Ordering::SeqCst), 0);
        drop(reader);
        assert!(!writer.0.is_ready());
        std::thread::spawn(move || drop(parallel_reader))
            .join()
            .unwrap();
        assert!(writer.0.is_ready());
        assert!(!last_reader.0.is_ready());
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        drop(writer);
        assert!(last_reader.0.is_ready());
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
        drop((last_reader, independent));
        assert!(queues.state.lock().queues.is_empty());
    }

    #[test]
    fn canceling_middle_writer_unblocks_readers_and_connection_cancellation_is_idempotent() {
        let queues = Arc::new(TransactionRequestQueues::default());
        let wake: ConnectionRequestWake = Arc::new(|| {});
        let first = request(&queues, 1, &["a"], TransactionMode::ReadOnly, wake.clone());
        let writer = request(
            &queues,
            2,
            &["a", "b"],
            TransactionMode::ReadWrite,
            wake.clone(),
        );
        let reader = request(
            &queues,
            3,
            &["a", "b"],
            TransactionMode::ReadOnly,
            wake.clone(),
        );
        let independent_writer =
            request(&queues, 2, &["c"], TransactionMode::ReadWrite, wake.clone());
        let independent_reader = request(&queues, 3, &["c"], TransactionMode::ReadOnly, wake);
        assert!(!reader.0.is_ready());
        assert!(!independent_reader.0.is_ready());
        queues.cancel_connection(DatabaseHandle::from_raw(2));
        assert!(first.0.is_ready());
        assert!(reader.0.is_ready());
        assert!(independent_reader.0.is_ready());
        assert!(!writer.0.is_ready());
        assert!(!independent_writer.0.is_ready());
        drop((writer, independent_writer));
        assert!(reader.0.is_ready());
        drop((first, reader, independent_reader));
        assert!(queues.state.lock().queues.is_empty());
    }

    #[test]
    fn database_names_and_storage_keys_are_independent_and_leases_do_not_retain_partition() {
        let queues = Arc::new(TransactionRequestQueues::default());
        let leases =
            [("origin", "db"), ("origin", "other"), ("bucket", "db")].map(|(origin, name)| {
                queues.enqueue(
                    (origin.into(), name.into()),
                    DatabaseHandle::from_raw(1),
                    BTreeSet::from(["store".into()]),
                    TransactionMode::ReadWrite,
                    Arc::new(|| {}),
                )
            });
        assert!(leases.iter().all(|lease| lease.0.is_ready()));
        drop(queues);
        assert!(leases.iter().all(|lease| !lease.0.is_ready()));
        drop(leases);
    }
}
