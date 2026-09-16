//! Cross-event-loop connection discovery and versionchange acknowledgements.
//! Wake callbacks carry no script values and only schedule the accepting loop.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use parking_lot::Mutex;

use crate::{ConnectionRequestWake, DatabaseHandle};

#[derive(Default)]
pub struct ConnectionNotifications {
    connections: Mutex<BTreeMap<DatabaseHandle, RegisteredConnection>>,
}

struct RegisteredConnection {
    key: String,
    state: Arc<ConnectionState>,
    wake: ConnectionRequestWake,
    notifications: VecDeque<VersionChangeNotification>,
    observers: Vec<Weak<VersionChangeBatchState>>,
}

struct ConnectionState {
    open: AtomicBool,
    close_pending: AtomicBool,
}

#[derive(Clone)]
pub struct VersionChangeBatch(Arc<VersionChangeBatchState>);

struct VersionChangeBatchState {
    connections: Vec<Arc<ConnectionState>>,
    progress: Mutex<NotificationProgress>,
    wake: ConnectionRequestWake,
}

struct NotificationProgress {
    pending: usize,
    blocked: Option<bool>,
}

pub struct VersionChangeNotification {
    pub old_version: u64,
    pub new_version: Option<u64>,
    pub completion: VersionChangeCompletion,
}

/// The recipient retains this through its callback's microtask checkpoint.
/// Dropping queued work on connection teardown also acknowledges that work.
pub struct VersionChangeCompletion(Arc<VersionChangeBatchState>);

impl Drop for VersionChangeCompletion {
    fn drop(&mut self) {
        let completed = {
            let mut progress = self.0.progress.lock();
            assert!(
                progress.pending > 0,
                "versionchange acknowledgement underflow"
            );
            progress.pending -= 1;
            if progress.pending == 0 {
                progress.blocked = Some(self.0.has_open_connections());
                true
            } else {
                false
            }
        };
        if completed {
            (self.0.wake)();
        }
    }
}

impl VersionChangeBatchState {
    fn has_open_connections(&self) -> bool {
        self.connections
            .iter()
            .any(|connection| connection.open.load(Ordering::Acquire))
    }
}

impl VersionChangeBatch {
    /// Once decided, a later close must not cancel an already-required event.
    pub fn blocked_event_required(&self) -> Option<bool> {
        self.0.progress.lock().blocked
    }
}

impl ConnectionNotifications {
    pub fn register(&self, handle: DatabaseHandle, key: String, wake: ConnectionRequestWake) {
        let previous = self.connections.lock().insert(
            handle,
            RegisteredConnection {
                key,
                state: Arc::new(ConnectionState {
                    open: AtomicBool::new(true),
                    close_pending: AtomicBool::new(false),
                }),
                wake,
                notifications: VecDeque::new(),
                observers: Vec::new(),
            },
        );
        assert!(previous.is_none(), "connection registered twice");
    }

    pub fn has_connections(&self, key: &str) -> bool {
        self.connections
            .lock()
            .values()
            .any(|connection| connection.key == key)
    }

    pub fn contains(&self, handle: DatabaseHandle) -> bool {
        self.connections.lock().contains_key(&handle)
    }

    pub fn mark_close_pending(&self, handle: DatabaseHandle) {
        if let Some(connection) = self.connections.lock().get(&handle) {
            connection
                .state
                .close_pending
                .store(true, Ordering::Release);
        }
    }

    pub fn unregister(&self, handle: DatabaseHandle) {
        let connection = {
            let mut connections = self.connections.lock();
            let Some(connection) = connections.remove(&handle) else {
                return;
            };
            connection.state.open.store(false, Ordering::Release);
            connection
        };
        // Acknowledgements and wakes may schedule other event loops. Keep them
        // outside the registry lock, including when queued work is discarded.
        drop(connection.notifications);
        for observer in connection
            .observers
            .into_iter()
            .filter_map(|observer| observer.upgrade())
        {
            (observer.wake)();
        }
    }

    pub fn begin_version_change(
        &self,
        key: &str,
        old_version: u64,
        new_version: Option<u64>,
        wake: ConnectionRequestWake,
    ) -> VersionChangeBatch {
        let (batch, recipients) = {
            let mut connections = self.connections.lock();
            let snapshot: Vec<_> = connections
                .values()
                .filter(|connection| connection.key == key)
                .map(|connection| connection.state.clone())
                .collect();
            let pending = snapshot
                .iter()
                .filter(|state| !state.close_pending.load(Ordering::Acquire))
                .count();
            let blocked = (pending == 0).then(|| {
                snapshot
                    .iter()
                    .any(|state| state.open.load(Ordering::Acquire))
            });
            let batch = Arc::new(VersionChangeBatchState {
                connections: snapshot,
                progress: Mutex::new(NotificationProgress { pending, blocked }),
                wake,
            });
            let mut recipients = Vec::new();
            for connection in connections
                .values_mut()
                .filter(|connection| connection.key == key)
            {
                connection
                    .observers
                    .retain(|observer| observer.strong_count() > 0);
                connection.observers.push(Arc::downgrade(&batch));
                if connection.state.close_pending.load(Ordering::Acquire) {
                    continue;
                }
                connection
                    .notifications
                    .push_back(VersionChangeNotification {
                        old_version,
                        new_version,
                        completion: VersionChangeCompletion(batch.clone()),
                    });
                recipients.push(connection.wake.clone());
            }
            (batch, recipients)
        };
        for recipient in recipients {
            recipient();
        }
        let completed = batch.progress.lock().blocked.is_some();
        if completed {
            (batch.wake)();
        }
        VersionChangeBatch(batch)
    }

    pub fn take_notification(&self, handle: DatabaseHandle) -> Option<VersionChangeNotification> {
        self.connections
            .lock()
            .get_mut(&handle)?
            .notifications
            .pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn notifications_wait_for_every_checkpoint_and_preserve_a_decided_blocked_event() {
        let registry = Arc::new(ConnectionNotifications::default());
        let first = DatabaseHandle::from_raw(1);
        let second = DatabaseHandle::from_raw(2);
        let unrelated = DatabaseHandle::from_raw(3);
        for (handle, key) in [(first, "db"), (second, "db"), (unrelated, "other")] {
            registry.register(handle, key.into(), Arc::new(|| {}));
        }
        let wakes = Arc::new(AtomicUsize::new(0));
        let wake: ConnectionRequestWake = {
            let wakes = wakes.clone();
            Arc::new(move || {
                wakes.fetch_add(1, Ordering::SeqCst);
            })
        };
        let batch = registry.begin_version_change("db", 1, Some(2), wake);
        let notification = registry.take_notification(first).unwrap();
        assert_eq!(notification.old_version, 1);
        assert_eq!(notification.new_version, Some(2));
        assert!(registry.take_notification(unrelated).is_none());
        registry.unregister(first);
        assert_eq!(batch.blocked_event_required(), None);
        std::thread::spawn(move || drop(notification))
            .join()
            .unwrap();
        assert_eq!(batch.blocked_event_required(), None);
        drop(registry.take_notification(second).unwrap());
        assert_eq!(batch.blocked_event_required(), Some(true));
        let before_close = wakes.load(Ordering::SeqCst);
        registry.unregister(second);
        assert!(wakes.load(Ordering::SeqCst) > before_close);
        assert_eq!(batch.blocked_event_required(), Some(true));
        assert!(!registry.has_connections("db"));
        assert!(registry.has_connections("other"));
    }

    #[test]
    fn retiring_queued_recipients_and_closing_in_callbacks_complete_the_batch() {
        let registry = ConnectionNotifications::default();
        let first = DatabaseHandle::from_raw(1);
        let second = DatabaseHandle::from_raw(2);
        for handle in [first, second] {
            registry.register(handle, "db".into(), Arc::new(|| {}));
        }
        let batch = registry.begin_version_change("db", 3, None, Arc::new(|| {}));
        let notification = registry.take_notification(first).unwrap();
        registry.unregister(first);
        registry.unregister(second);
        assert_eq!(batch.blocked_event_required(), None);
        drop(notification);
        assert_eq!(batch.blocked_event_required(), Some(false));
        assert!(registry.take_notification(second).is_none());
    }

    #[test]
    fn close_pending_connections_block_without_receiving_another_notification() {
        let registry = ConnectionNotifications::default();
        let handle = DatabaseHandle::from_raw(1);
        registry.register(handle, "db".into(), Arc::new(|| {}));
        registry.mark_close_pending(handle);
        let batch = registry.begin_version_change("db", 1, None, Arc::new(|| {}));
        assert!(registry.take_notification(handle).is_none());
        assert_eq!(batch.blocked_event_required(), Some(true));
        registry.unregister(handle);
        assert!(!registry.contains(handle));
        assert_eq!(batch.blocked_event_required(), Some(true));
    }
}
