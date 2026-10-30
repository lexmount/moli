//! FIFO coordination for IDBFactory open/delete algorithms. Queue ownership
//! spans event loops; callbacks only wake the accepting loop and carry no JS values.

use parking_lot::Mutex;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Weak},
};

pub type ConnectionRequestWake = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub struct ConnectionRequestQueues {
    state: Mutex<QueueState>,
}

#[derive(Default)]
struct QueueState {
    closed: bool,
    next_id: u64,
    queues: BTreeMap<(String, crate::IndexedDbName), VecDeque<QueuedRequest>>,
}

struct QueuedRequest {
    id: u64,
    wake: ConnectionRequestWake,
}

/// A cancellation handle can outlive its accepting realm, but cannot keep the
/// storage partition alive. Only the request owns the RAII lease.
#[derive(Clone)]
pub struct ConnectionRequestHandle {
    queues: Weak<ConnectionRequestQueues>,
    key: (String, crate::IndexedDbName),
    id: u64,
}

pub struct ConnectionRequestLease(ConnectionRequestHandle);

impl ConnectionRequestLease {
    pub fn handle(&self) -> &ConnectionRequestHandle {
        &self.0
    }
}

impl Drop for ConnectionRequestLease {
    fn drop(&mut self) {
        self.0.finish();
    }
}

impl ConnectionRequestQueues {
    pub fn enqueue(
        self: &Arc<Self>,
        storage_key: &str,
        name: &str,
        wake: ConnectionRequestWake,
    ) -> ConnectionRequestLease {
        self.try_enqueue(storage_key, name, wake)
            .expect("connection request queue is open")
    }

    pub fn try_enqueue(
        self: &Arc<Self>,
        storage_key: &str,
        name: &str,
        wake: ConnectionRequestWake,
    ) -> Result<ConnectionRequestLease, crate::IndexedDbError> {
        self.try_enqueue_named(storage_key, &name.into(), wake)
    }

    pub fn try_enqueue_named(
        self: &Arc<Self>,
        storage_key: &str,
        name: &crate::IndexedDbName,
        wake: ConnectionRequestWake,
    ) -> Result<ConnectionRequestLease, crate::IndexedDbError> {
        let mut state = self.state.lock();
        if state.closed {
            return Err(crate::IndexedDbError::InvalidState(
                "IndexedDB connection queue is closed".into(),
            ));
        }
        let key = (storage_key.to_owned(), name.to_owned());
        state.next_id = state
            .next_id
            .checked_add(1)
            .expect("IndexedDB request id overflow");
        let id = state.next_id;
        state
            .queues
            .entry(key.clone())
            .or_default()
            .push_back(QueuedRequest { id, wake });
        Ok(ConnectionRequestLease(ConnectionRequestHandle {
            queues: Arc::downgrade(self),
            key,
            id,
        }))
    }
}

impl ConnectionRequestHandle {
    pub fn is_active(&self) -> bool {
        self.queues.upgrade().is_some_and(|queues| {
            queues
                .state
                .lock()
                .queues
                .get(&self.key)
                .is_some_and(|queue| queue.iter().any(|entry| entry.id == self.id))
        })
    }
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn is_head(&self) -> bool {
        self.queues.upgrade().is_some_and(|queues| {
            queues
                .state
                .lock()
                .queues
                .get(&self.key)
                .and_then(VecDeque::front)
                .is_some_and(|entry| entry.id == self.id)
        })
    }

    pub fn finish(&self) {
        let Some(queues) = self.queues.upgrade() else {
            return;
        };
        let wake = {
            let mut state = queues.state.lock();
            let Some(queue) = state.queues.get_mut(&self.key) else {
                return;
            };
            let Some(index) = queue.iter().position(|entry| entry.id == self.id) else {
                return;
            };
            queue.remove(index);
            let wake = (index == 0)
                .then(|| queue.front().map(|entry| entry.wake.clone()))
                .flatten();
            if queue.is_empty() {
                state.queues.remove(&self.key);
            }
            wake
        };
        // Wake outside the lock: the recipient may immediately query its queue.
        if let Some(wake) = wake {
            wake();
        }
    }
}

impl ConnectionRequestQueues {
    /// Cancel accepted algorithms after the storage owner has drained SQL.
    /// Wake every event loop so non-head requests can settle rather than hang.
    pub(crate) fn shutdown(&self) {
        let queues = {
            let mut state = self.state.lock();
            state.closed = true;
            std::mem::take(&mut state.queues)
        };
        for entry in queues.into_values().flatten() {
            (entry.wake)();
        }
    }

    /// Retire all accepted algorithms for one exact owner, including names
    /// which have not reached database registration. Wake outside the lock.
    pub(crate) fn cancel_owner(&self, owner: &str) {
        self.cancel_matching_owners(|key| key == owner);
    }

    pub(crate) fn cancel_owner_prefix(&self, prefix: &str) {
        self.cancel_matching_owners(|key| key.starts_with(prefix));
    }

    fn cancel_matching_owners(&self, matches: impl Fn(&str) -> bool) {
        let queues = self
            .state
            .lock()
            .queues
            .extract_if(.., |(key, _), _| matches(key))
            .map(|(_, queue)| queue)
            .collect::<Vec<_>>();
        for entry in queues.into_iter().flatten() {
            (entry.wake)();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn connection_requests_serialize_per_storage_key_and_name_and_wake_on_drop() {
        let queues = Arc::new(ConnectionRequestQueues::default());
        let wakes = Arc::new(AtomicUsize::new(0));
        let wake: ConnectionRequestWake = {
            let wakes = wakes.clone();
            Arc::new(move || {
                wakes.fetch_add(1, Ordering::SeqCst);
            })
        };
        let first = queues.enqueue("origin", "db", wake.clone());
        let second = queues.enqueue("origin", "db", wake.clone());
        let canceled = queues.enqueue("origin", "db", wake.clone());
        let third = queues.enqueue("origin", "db", wake.clone());
        let other_name = queues.enqueue("origin", "other", wake.clone());
        let other_storage = queues.enqueue("bucket", "db", wake);
        assert!(first.0.is_head());
        assert!(!second.0.is_head());
        assert!(other_name.0.is_head());
        assert!(other_storage.0.is_head());
        drop(canceled);
        assert_eq!(wakes.load(Ordering::SeqCst), 0);
        let cancellation = first.0.clone();
        std::thread::spawn(move || drop(first)).join().unwrap();
        assert!(second.0.is_head());
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        cancellation.finish();
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        drop(second);
        assert!(third.0.is_head());
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
        drop((third, other_name, other_storage));
        assert!(queues.state.lock().queues.is_empty());
    }

    #[test]
    fn connection_request_lease_does_not_retain_storage_partition() {
        let queues = Arc::new(ConnectionRequestQueues::default());
        let request = queues.enqueue("origin", "db", Arc::new(|| {}));
        drop(queues);
        assert!(!request.0.is_head());
        drop(request);
    }
}
