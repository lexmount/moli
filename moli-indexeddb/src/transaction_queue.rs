//! Admission of regular transactions across connections and event loops.
//!
//! The accepting event loop owns an admission until it starts. Once started,
//! the backend retains admission until commit or rollback, even if the caller
//! drops its lease. Forced closure cancels both phases and wakes their owners.

use parking_lot::Mutex;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Weak},
};

use crate::{DatabaseHandle, TransactionMode};

/// Schedules the accepting event loop after readiness or cancellation changes.
pub type TransactionRequestWake = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionRequestStatus {
    Pending,
    Ready,
    Started,
    Canceled,
    Finished,
}

#[derive(Default)]
pub struct TransactionRequestQueues {
    state: Mutex<QueueState>,
}

#[derive(Default)]
struct QueueState {
    next_id: u64,
    queues: BTreeMap<(String, String), VecDeque<QueuedRequest>>,
}

struct TransactionScope {
    stores: BTreeSet<String>,
    mode: TransactionMode,
}

impl TransactionScope {
    // One SQLite connection has one writer. Readonly transactions can share
    // it while no older writer can modify their object-store scope.
    fn conflicts_with(&self, other: &Self) -> bool {
        self.mode == TransactionMode::VersionChange
            || other.mode == TransactionMode::VersionChange
            || (self.mode != TransactionMode::ReadOnly && other.mode != TransactionMode::ReadOnly)
            || ((self.mode != TransactionMode::ReadOnly || other.mode != TransactionMode::ReadOnly)
                && !self.stores.is_disjoint(&other.stores))
    }
}

struct RequestState {
    scope: TransactionScope,
    status: Mutex<TransactionRequestStatus>,
}

struct QueuedRequest {
    id: u64,
    database: DatabaseHandle,
    request: Arc<RequestState>,
    wake: TransactionRequestWake,
}

#[derive(Clone)]
pub struct TransactionRequestHandle {
    queues: Weak<TransactionRequestQueues>,
    key: (String, String),
    id: u64,
    database: DatabaseHandle,
    request: Arc<RequestState>,
}

impl std::fmt::Debug for TransactionRequestHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransactionRequestHandle")
            .field("id", &self.id)
            .field("database", &self.database)
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

/// Dropping an unstarted lease withdraws admission. A started admission belongs
/// to its backend transaction and is released only after commit or rollback.
pub struct TransactionRequestLease(TransactionRequestHandle);

impl TransactionRequestLease {
    pub fn handle(&self) -> &TransactionRequestHandle {
        &self.0
    }
}

impl Drop for TransactionRequestLease {
    fn drop(&mut self) {
        self.0.release(Release::LeaseDropped);
    }
}

enum Release {
    LeaseDropped,
    BackendFinished,
}

impl TransactionRequestQueues {
    pub fn enqueue(
        self: &Arc<Self>,
        key: (String, String),
        database: DatabaseHandle,
        stores: BTreeSet<String>,
        mode: TransactionMode,
        wake: TransactionRequestWake,
    ) -> TransactionRequestLease {
        let scope = TransactionScope { stores, mode };
        let mut state = self.state.lock();
        state.next_id = state
            .next_id
            .checked_add(1)
            .expect("IDB transaction id overflow");
        let id = state.next_id;
        let queue = state.queues.entry(key.clone()).or_default();
        let ready = !queue
            .iter()
            .any(|older| scope.conflicts_with(&older.request.scope));
        let request = Arc::new(RequestState {
            scope,
            status: Mutex::new(if ready {
                TransactionRequestStatus::Ready
            } else {
                TransactionRequestStatus::Pending
            }),
        });
        queue.push_back(QueuedRequest {
            id,
            database,
            request: request.clone(),
            wake,
        });
        // The caller must attach its wrapper before initial readiness is used.
        TransactionRequestLease(TransactionRequestHandle {
            queues: Arc::downgrade(self),
            key,
            id,
            database,
            request,
        })
    }

    pub(crate) fn cancel_database(&self, origin: &str, name: &str) {
        let queue = self
            .state
            .lock()
            .queues
            .remove(&(origin.to_owned(), name.to_owned()));
        if let Some(queue) = queue {
            for entry in &queue {
                *entry.request.status.lock() = TransactionRequestStatus::Canceled;
            }
            for entry in queue {
                (entry.wake)();
            }
        }
    }

    pub(crate) fn has_connection_requests(&self, database: DatabaseHandle) -> bool {
        self.state
            .lock()
            .queues
            .values()
            .any(|queue| queue.iter().any(|entry| entry.database == database))
    }

    /// Roll back backend work before calling this method. Cancellation remains
    /// observable through surviving handles after entries leave the queue.
    /// Callbacks run outside the queue lock and must not reenter the manager.
    pub fn cancel_connection(&self, database: DatabaseHandle) {
        self.cancel_connections(&BTreeSet::from([database]));
    }

    fn cancel_connections(&self, databases: &BTreeSet<DatabaseHandle>) {
        let wakes = {
            let mut state = self.state.lock();
            let mut wakes = Vec::new();
            state.queues.retain(|_, queue| {
                let before = queue.len();
                queue.retain(|entry| {
                    if !databases.contains(&entry.database) {
                        return true;
                    }
                    *entry.request.status.lock() = TransactionRequestStatus::Canceled;
                    wakes.push(entry.wake.clone());
                    false
                });
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

fn wake_newly_ready(queue: &mut VecDeque<QueuedRequest>, wakes: &mut Vec<TransactionRequestWake>) {
    for index in 0..queue.len() {
        let mut status = queue[index].request.status.lock();
        if *status == TransactionRequestStatus::Pending
            && !queue.iter().take(index).any(|older| {
                queue[index]
                    .request
                    .scope
                    .conflicts_with(&older.request.scope)
            })
        {
            *status = TransactionRequestStatus::Ready;
            wakes.push(queue[index].wake.clone());
        }
    }
}

impl TransactionRequestHandle {
    pub(crate) fn matches_key(&self, origin: &str, name: &str) -> bool {
        self.key.0 == origin && self.key.1 == name
    }
    pub fn status(&self) -> TransactionRequestStatus {
        let status = *self.request.status.lock();
        if status != TransactionRequestStatus::Finished && self.queues.strong_count() == 0 {
            TransactionRequestStatus::Canceled
        } else {
            status
        }
    }

    pub fn is_ready(&self) -> bool {
        self.status() == TransactionRequestStatus::Ready
    }

    pub fn belongs_to(&self, queues: &Arc<TransactionRequestQueues>) -> bool {
        self.queues.ptr_eq(&Arc::downgrade(queues))
    }

    pub fn database(&self) -> DatabaseHandle {
        self.database
    }
    pub fn store_names(&self) -> Vec<String> {
        self.request.scope.stores.iter().cloned().collect()
    }
    pub fn mode(&self) -> TransactionMode {
        self.request.scope.mode
    }

    pub(crate) fn claim_start(&self) -> bool {
        let Some(queues) = self.queues.upgrade() else {
            return false;
        };
        let state = queues.state.lock();
        let Some(entry) = state
            .queues
            .get(&self.key)
            .and_then(|queue| queue.iter().find(|entry| entry.id == self.id))
        else {
            return false;
        };
        let mut status = entry.request.status.lock();
        if *status != TransactionRequestStatus::Ready {
            return false;
        }
        *status = TransactionRequestStatus::Started;
        true
    }

    pub(crate) fn finish(&self) {
        self.release(Release::BackendFinished);
    }

    fn release(&self, release: Release) {
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
            {
                let mut status = queue[index].request.status.lock();
                if matches!(release, Release::LeaseDropped)
                    && *status == TransactionRequestStatus::Started
                {
                    return;
                }
                *status = TransactionRequestStatus::Finished;
            }
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
        wake: TransactionRequestWake,
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
        let wake: TransactionRequestWake = {
            let queues = Arc::downgrade(&queues);
            let wakes = wakes.clone();
            Arc::new(move || {
                // Wakes must run without holding the coordinator lock.
                assert!(queues.upgrade().unwrap().state.try_lock().is_some());
                wakes.fetch_add(1, Ordering::SeqCst);
            })
        };
        let reader = request(&queues, 1, &["a"], TransactionMode::ReadOnly, wake.clone());
        assert!(reader.handle().belongs_to(&queues));
        assert!(
            !reader
                .handle()
                .belongs_to(&Arc::new(TransactionRequestQueues::default()))
        );
        assert_eq!(reader.handle().database(), DatabaseHandle::from_raw(1));
        assert_eq!(reader.handle().store_names(), ["a"]);
        assert_eq!(reader.handle().mode(), TransactionMode::ReadOnly);
        let parallel_reader = request(&queues, 2, &["a"], TransactionMode::ReadOnly, wake.clone());
        let writer = request(
            &queues,
            2,
            &["a", "b"],
            TransactionMode::ReadWrite,
            wake.clone(),
        );
        let last_reader = request(&queues, 3, &["b"], TransactionMode::ReadOnly, wake.clone());
        let independent = request(&queues, 1, &["c"], TransactionMode::ReadOnly, wake);
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
    fn one_writer_allows_disjoint_readers_and_retains_admission_until_backend_finishes() {
        let queues = Arc::new(TransactionRequestQueues::default());
        let wake: TransactionRequestWake = Arc::new(|| {});
        let first = request(&queues, 1, &["a"], TransactionMode::ReadWrite, wake.clone());
        assert!(first.handle().claim_start());
        let first_handle = first.handle().clone();
        let overlapping_reader =
            request(&queues, 2, &["a"], TransactionMode::ReadOnly, wake.clone());
        let disjoint_reader = request(&queues, 2, &["b"], TransactionMode::ReadOnly, wake.clone());
        let second_writer = request(&queues, 3, &["c"], TransactionMode::ReadWrite, wake.clone());
        let later_reader = request(&queues, 4, &["c"], TransactionMode::ReadOnly, wake);
        assert!(!overlapping_reader.handle().is_ready());
        assert!(disjoint_reader.handle().is_ready());
        assert!(!second_writer.handle().is_ready());
        assert!(!later_reader.handle().is_ready());
        drop(first);
        assert_eq!(first_handle.status(), TransactionRequestStatus::Started);
        assert!(!overlapping_reader.handle().is_ready());
        assert!(!second_writer.handle().is_ready());
        first_handle.finish();
        assert!(overlapping_reader.handle().is_ready());
        assert!(disjoint_reader.handle().is_ready());
        assert!(second_writer.handle().is_ready());
        assert!(!later_reader.handle().is_ready());
        assert!(second_writer.handle().claim_start());
        second_writer.handle().finish();
        assert!(later_reader.handle().is_ready());
        drop((
            overlapping_reader,
            disjoint_reader,
            second_writer,
            later_reader,
        ));
        assert!(queues.state.lock().queues.is_empty());
    }

    #[test]
    fn canceling_middle_writer_unblocks_readers_and_connection_cancellation_is_idempotent() {
        let queues = Arc::new(TransactionRequestQueues::default());
        let wake: TransactionRequestWake = Arc::new(|| {});
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
