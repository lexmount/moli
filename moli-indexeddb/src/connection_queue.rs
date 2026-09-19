//! FIFO coordination for IDBFactory open/delete algorithms. Queue ownership
//! spans event loops; callbacks only wake the accepting loop and carry no JS values.

use crate::IndexedDbName;
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
    next_id: u64,
    queues: BTreeMap<(String, IndexedDbName), VecDeque<QueuedRequest>>,
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
    key: (String, IndexedDbName),
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
        name: impl Into<IndexedDbName>,
        wake: ConnectionRequestWake,
    ) -> ConnectionRequestLease {
        let key = (storage_key.to_owned(), name.into());
        let mut state = self.state.lock();
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
        ConnectionRequestLease(ConnectionRequestHandle {
            queues: Arc::downgrade(self),
            key,
            id,
        })
    }
}

impl ConnectionRequestHandle {
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
