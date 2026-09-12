use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use super::ProcessEnvironmentChange;

const TIMEZONE: u8 = 1;
const LOCALE: u8 = 2;
const CLOSED: u8 = 4;

/// One bounded mailbox per isolate, shared by its idle, interrupt and pause
/// routes. These are pending cache invalidations, not configuration versions:
/// ICU already contains the latest values, so unconsumed changes can be merged.
#[derive(Clone, Debug, Default)]
pub struct ProcessEnvironmentNotifications {
    pending: Arc<AtomicU8>,
}

impl ProcessEnvironmentNotifications {
    pub fn notifier(&self, wake: impl Fn() + Send + Sync + 'static) -> ProcessEnvironmentNotifier {
        ProcessEnvironmentNotifier {
            notifications: self.clone(),
            wake: Arc::new(wake),
        }
    }

    pub fn has_pending(&self) -> bool {
        self.pending.load(Ordering::Acquire) & (LOCALE | TIMEZONE) != 0
    }

    /// Claim only the work published so far. A concurrent publication either
    /// joins this batch or remains pending for the next owner turn; it cannot
    /// be cleared by finishing the current notification.
    pub fn take(&self) -> Option<ProcessEnvironmentChange> {
        let pending = self.pending.fetch_and(CLOSED, Ordering::AcqRel);
        if pending & LOCALE != 0 {
            Some(ProcessEnvironmentChange::LocaleChanged)
        } else if pending & TIMEZONE != 0 {
            Some(ProcessEnvironmentChange::TimezoneChanged)
        } else {
            None
        }
    }

    /// Owner disposal permanently rejects late publishers, including clones
    /// retained by an in-flight process-wide publication.
    pub fn close(&self) {
        self.pending.store(CLOSED, Ordering::Release);
    }

    fn publish(&self, change: ProcessEnvironmentChange) -> bool {
        let bits = match change {
            ProcessEnvironmentChange::LocaleChanged => LOCALE | TIMEZONE,
            ProcessEnvironmentChange::TimezoneChanged => TIMEZONE,
        };
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                (pending & CLOSED == 0).then_some(pending | bits)
            })
            == Ok(0)
    }
}

/// A typed publication target, not a callback that receives environment state.
/// Publication only touches the mailbox. Its owner wake runs separately, after
/// the process controller has released the ICU/configuration mutex.
#[derive(Clone)]
pub struct ProcessEnvironmentNotifier {
    notifications: ProcessEnvironmentNotifications,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl ProcessEnvironmentNotifier {
    /// Post a cache invalidation outside a process configuration transaction.
    /// A burst schedules a wake only on the idle-to-pending transition.
    pub fn notify(&self, change: ProcessEnvironmentChange) {
        if self.publish(change) {
            self.wake();
        }
    }

    pub(super) fn publish(&self, change: ProcessEnvironmentChange) -> bool {
        self.notifications.publish(change)
    }

    pub(super) fn wake(&self) {
        if self.notifications.has_pending() {
            (self.wake)();
        }
    }

    pub(crate) fn close(&self) {
        self.notifications.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProcessEnvironmentChange::{LocaleChanged, TimezoneChanged};
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn bursts_merge_both_axes_into_one_notification_and_one_wake() {
        let notifications = ProcessEnvironmentNotifications::default();
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = wakes.clone();
        let notifier = notifications.notifier(move || {
            count.fetch_add(1, Ordering::Relaxed);
        });
        for _ in 0..10_000 {
            notifier.notify(TimezoneChanged);
            notifier.notify(LocaleChanged);
        }
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        assert_eq!(notifications.take(), Some(LocaleChanged));
        assert_eq!(notifications.take(), None);
        notifier.notify(TimezoneChanged);
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
        assert_eq!(notifications.take(), Some(TimezoneChanged));
    }

    #[test]
    fn publication_during_consumption_survives_and_rearms_the_wake() {
        let notifications = ProcessEnvironmentNotifications::default();
        let (wake_tx, wake_rx) = std::sync::mpsc::channel();
        let notifier = notifications.notifier(move || wake_tx.send(()).unwrap());
        notifier.notify(LocaleChanged);
        wake_rx.try_recv().unwrap();
        let applying = notifications.take().unwrap();
        // The first invalidation has been claimed but has not finished applying.
        notifier.notify(TimezoneChanged);
        assert_eq!(applying, LocaleChanged);
        wake_rx.try_recv().unwrap();
        assert_eq!(notifications.take(), Some(TimezoneChanged));
        assert_eq!(notifications.take(), None);
    }

    #[test]
    fn concurrent_publication_and_claim_never_lose_either_axis() {
        let notifications = ProcessEnvironmentNotifications::default();
        let barrier = std::sync::Barrier::new(2);
        let mut outcomes = Vec::new();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..1_000 {
                    barrier.wait();
                    notifications.publish(LocaleChanged);
                    barrier.wait();
                }
            });
            for _ in 0..1_000 {
                notifications.publish(TimezoneChanged);
                barrier.wait();
                let first = notifications.take();
                barrier.wait();
                let second = notifications.take();
                outcomes.push((first, second, notifications.has_pending()));
            }
        });
        // Check after both threads finish so a regression cannot strand the
        // producer at a barrier while the assertion unwinds the consumer.
        for (first, second, pending) in outcomes {
            assert!(first == Some(LocaleChanged) || second == Some(LocaleChanged));
            assert!(
                first.is_some(),
                "the earlier timezone must also be consumed"
            );
            assert!(!pending);
        }
    }

    #[test]
    fn delayed_wake_observes_merged_state_and_never_overwrites_newer_work() {
        let notifications = ProcessEnvironmentNotifications::default();
        let (wake_tx, wake_rx) = std::sync::mpsc::channel();
        let notifier = notifications.notifier(move || wake_tx.send(()).unwrap());
        assert!(notifier.publish(TimezoneChanged));
        assert!(!notifier.publish(LocaleChanged));
        notifier.wake();
        wake_rx.try_recv().unwrap();
        assert_eq!(notifications.take(), Some(LocaleChanged));
        notifier.notify(TimezoneChanged);
        wake_rx.try_recv().unwrap();
        assert_eq!(notifications.take(), Some(TimezoneChanged));
        notifier.wake(); // An old delayed wake carries no configuration value.
        assert!(wake_rx.try_recv().is_err());
    }

    #[test]
    fn concurrent_disposal_and_publication_leave_mailboxes_permanently_closed() {
        let notifications: Vec<_> = (0..1_000)
            .map(|_| ProcessEnvironmentNotifications::default())
            .collect();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for mailbox in &notifications {
                    barrier.wait();
                    mailbox.publish(LocaleChanged);
                    barrier.wait();
                }
            });
            for mailbox in &notifications {
                barrier.wait();
                mailbox.close();
                barrier.wait();
            }
        });
        for mailbox in notifications {
            assert_eq!(mailbox.take(), None);
            assert!(!mailbox.publish(TimezoneChanged));
            assert_eq!(mailbox.take(), None);
        }
    }

    #[test]
    fn disposal_cancels_delayed_wakes_and_cannot_be_reopened() {
        let notifications = ProcessEnvironmentNotifications::default();
        let notifier = notifications.notifier(|| panic!("closed mailbox woke its owner"));
        assert!(notifier.publish(LocaleChanged));
        let delayed = notifier.clone();
        notifications.close();
        delayed.wake();
        for change in [LocaleChanged, TimezoneChanged] {
            delayed.notify(change);
        }
        assert_eq!(notifications.take(), None);
        assert!(!notifications.has_pending());
    }
}
