use std::{
    sync::{
        Arc, Barrier, OnceLock, Weak,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use moli_disk_pool::DiskPool;

use super::{ParkableImageManager, ParkableImageManagerInner, ParkableImageSweepReport};
use crate::{
    ParkableImagePolicy,
    image::{ParkOutcome, ParkableImage, ParkableImageStorageState},
};

fn immediate_policy() -> ParkableImagePolicy {
    ParkableImagePolicy {
        min_size_to_park: 1,
        parking_delay: Duration::ZERO,
        reader_release_delay: Duration::ZERO,
    }
}

fn manager(capacity: Option<u64>) -> (DiskPool, ParkableImageManager) {
    let pool = DiskPool::new(capacity).unwrap();
    let manager = ParkableImageManager::new(Some(pool.clone()), immediate_policy());
    (pool, manager)
}

fn unavailable_report() -> ParkableImageSweepReport {
    ParkableImageSweepReport {
        considered: 1,
        unavailable: 1,
        ..ParkableImageSweepReport::default()
    }
}

fn assert_completes(operation: impl FnOnce() + Send + 'static) {
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        operation();
        let _ = sender.send(());
    });
    assert_ne!(
        receiver.recv_timeout(Duration::from_secs(30)),
        Err(mpsc::RecvTimeoutError::Timeout),
        "parking or a notification callback deadlocked"
    );
    worker.join().unwrap();
}

#[test]
fn scheduled_sweep_arms_wait_before_scanning() {
    let (_, manager) = manager(None);
    let image = manager.from_frozen_bytes(vec![1; 4]);
    assert!(manager.next_parking_deadline().is_some());

    assert!(manager.begin_scheduled_sweep());
    assert!(manager.next_parking_deadline().is_none());
    assert!(!manager.begin_scheduled_sweep());

    let report = manager.sweep(vec![image.clone()], ParkableImage::park);
    assert_eq!(report.parked, 1);
    manager.finish_sweep(report);
    assert!(manager.begin_scheduled_sweep());
    manager.finish_sweep(ParkableImageSweepReport::default());
}

#[test]
fn notification_before_sweep_does_not_hide_a_later_capacity_miss() {
    let manager = ParkableImageManager::default();
    manager.notify_schedule_changed();
    assert!(manager.begin_scheduled_sweep());
    manager.finish_sweep(unavailable_report());
    assert!(!manager.begin_scheduled_sweep());
}

#[test]
fn notification_during_unavailable_sweep_survives_completion() {
    for notifications in [1, 32] {
        let manager = ParkableImageManager::default();
        assert!(manager.begin_scheduled_sweep());
        for _ in 0..notifications {
            manager.notify_schedule_changed();
        }
        manager.finish_sweep(unavailable_report());
        assert!(manager.begin_scheduled_sweep());

        // The notification permits a retry, not unlimited failed sweeps.
        manager.finish_sweep(unavailable_report());
        assert!(!manager.begin_scheduled_sweep());
    }
}

#[test]
fn notification_after_unavailable_sweep_allows_retry() {
    let manager = ParkableImageManager::default();
    assert!(manager.begin_scheduled_sweep());
    manager.finish_sweep(unavailable_report());
    assert!(!manager.begin_scheduled_sweep());
    manager.notify_schedule_changed();
    assert!(manager.begin_scheduled_sweep());
    manager.finish_sweep(ParkableImageSweepReport::default());
}

#[test]
fn only_unavailable_outcomes_keep_a_sweep_waiting_for_change() {
    let reports = [
        ParkableImageSweepReport::default(),
        ParkableImageSweepReport {
            considered: 1,
            parked: 1,
            ..ParkableImageSweepReport::default()
        },
        ParkableImageSweepReport {
            considered: 1,
            already_parked: 1,
            ..ParkableImageSweepReport::default()
        },
        ParkableImageSweepReport {
            considered: 1,
            delayed: 1,
            ..ParkableImageSweepReport::default()
        },
        ParkableImageSweepReport {
            considered: 1,
            in_use: 1,
            ..ParkableImageSweepReport::default()
        },
        ParkableImageSweepReport {
            considered: 1,
            ineligible: 1,
            ..ParkableImageSweepReport::default()
        },
        ParkableImageSweepReport {
            considered: 1,
            write_failures: 1,
            ..ParkableImageSweepReport::default()
        },
    ];
    for report in reports {
        let manager = ParkableImageManager::default();
        assert!(manager.begin_scheduled_sweep());
        manager.finish_sweep(report);
        assert!(!*manager.inner.waiting_for_change.lock(), "{report:?}");

        assert!(manager.begin_scheduled_sweep());
        manager.finish_sweep(ParkableImageSweepReport {
            considered: report.considered + 1,
            unavailable: 1,
            ..report
        });
        assert!(!manager.begin_scheduled_sweep(), "{report:?}");
    }
}

#[test]
fn empty_and_future_deadline_sweeps_do_not_stay_waiting() {
    let pool = DiskPool::new(None).unwrap();
    let manager = ParkableImageManager::new(
        Some(pool),
        ParkableImagePolicy {
            parking_delay: Duration::from_secs(3600),
            ..immediate_policy()
        },
    );
    manager.park_images();
    assert!(!*manager.inner.waiting_for_change.lock());

    let image = manager.from_frozen_bytes(vec![1; 4]);
    let deadline = manager.next_parking_deadline().unwrap();
    assert!(deadline > Instant::now());
    for _ in 0..3 {
        assert_eq!(manager.park_images_with_report().considered, 0);
        assert_eq!(manager.next_parking_deadline(), Some(deadline));
    }
    assert_eq!(manager.force_park_images_with_report().parked, 1);
    assert_eq!(image.retained_memory_bytes(), 0);
    assert!(!*manager.inner.waiting_for_change.lock());
}

#[test]
fn failed_write_does_not_leave_the_sweep_in_progress() {
    let (pool, manager) = manager(None);
    let image = manager.from_frozen_bytes(vec![1; 4]);
    pool.fail_next_write_for_test();

    let report = manager.park_images_with_report();
    assert_eq!(report.write_failures, 1);
    assert!(!*manager.inner.waiting_for_change.lock());
    assert_eq!(manager.next_parking_deadline(), None);
    assert_eq!(manager.park_images_with_report().considered, 0);
    assert_eq!(image.snapshot().unwrap().as_ref(), &[1; 4]);
}

#[test]
fn registering_an_image_during_a_failed_sweep_preserves_its_retry() {
    let (_, manager) = manager(Some(4));
    let oversized = manager.from_frozen_bytes(vec![1; 8]);
    assert!(manager.begin_scheduled_sweep());
    let report = manager.sweep(vec![oversized.clone()], ParkableImage::park);
    assert_eq!(report, unavailable_report());

    let small = manager.from_frozen_bytes(vec![2; 4]);
    manager.finish_sweep(report);

    assert!(manager.next_parking_deadline().is_some());
    let retry = manager.park_images_with_report();
    assert_eq!(retry.considered, 2);
    assert_eq!(retry.parked, 1);
    assert_eq!(retry.unavailable, 1);
    assert_eq!(small.retained_memory_bytes(), 0);
    assert_eq!(oversized.retained_memory_bytes(), 8);
    assert_eq!(manager.next_parking_deadline(), None);
}

#[test]
fn last_reader_release_during_a_failed_sweep_preserves_its_retry() {
    assert_completes(|| {
        let (_, manager) = manager(Some(4));
        let oversized = manager.from_frozen_bytes(vec![1; 8]);
        let small = manager.from_frozen_bytes(vec![2; 4]);
        let first = small.snapshot().unwrap();
        let last = first.clone();
        assert!(manager.begin_scheduled_sweep());
        let report = manager.sweep(vec![oversized.clone()], ParkableImage::park);
        assert_eq!(report, unavailable_report());

        drop(first);
        assert!(*manager.inner.waiting_for_change.lock());
        thread::spawn(move || drop(last)).join().unwrap();
        manager.finish_sweep(report);

        let retry = manager.park_images_with_report();
        assert_eq!(retry.parked, 1);
        assert_eq!(retry.unavailable, 1);
        assert_eq!(small.retained_memory_bytes(), 0);
        assert_eq!(oversized.retained_memory_bytes(), 8);
    });
}

#[test]
fn acquiring_a_reader_during_a_failed_sweep_preserves_the_notification() {
    let (_, manager) = manager(Some(4));
    let oversized = manager.from_frozen_bytes(vec![1; 8]);
    assert!(manager.begin_scheduled_sweep());
    let report = manager.sweep(vec![oversized.clone()], ParkableImage::park);
    assert_eq!(report, unavailable_report());

    let snapshot = oversized.snapshot().unwrap();
    manager.finish_sweep(report);
    assert!(!*manager.inner.waiting_for_change.lock());
    assert_eq!(manager.park_images_with_report().considered, 0);

    drop(snapshot);
    assert!(manager.next_parking_deadline().is_some());
    assert_eq!(manager.park_images_with_report(), unavailable_report());
}

#[test]
fn releasing_an_extent_before_or_after_failed_completion_allows_retry() {
    assert_completes(|| {
        for release_before_completion in [true, false] {
            let (pool, manager) = manager(Some(4));
            let owner = manager.from_frozen_bytes(vec![1; 4]);
            assert_eq!(owner.park().unwrap(), ParkOutcome::Parked);
            let waiting = manager.from_frozen_bytes(vec![2; 4]);
            assert!(manager.begin_scheduled_sweep());
            let report = manager.sweep(vec![waiting.clone()], ParkableImage::park);
            assert_eq!(report, unavailable_report());

            if !release_before_completion {
                manager.finish_sweep(report);
                assert_eq!(manager.next_parking_deadline(), None);
            }
            thread::spawn(move || drop(owner)).join().unwrap();
            assert_eq!(pool.diagnostics().free_bytes, 4);
            if release_before_completion {
                manager.finish_sweep(report);
            }

            assert!(manager.next_parking_deadline().is_some());
            assert_eq!(manager.park_images_with_report().parked, 1);
            assert_eq!(waiting.snapshot().unwrap().as_ref(), &[2; 4]);
        }
    });
}

#[test]
fn force_retries_after_an_external_pool_owner_releases_capacity() {
    let (pool, manager) = manager(Some(4));
    let external = pool.store(&[1; 4]).unwrap().unwrap();
    let image = manager.from_frozen_bytes(vec![2; 4]);
    assert_eq!(manager.park_images_with_report(), unavailable_report());

    // A generic DiskData owner does not send image-manager notifications.
    drop(external);
    assert_eq!(pool.diagnostics().free_bytes, 4);
    assert_eq!(manager.park_images_with_report().considered, 0);

    assert_eq!(manager.force_park_images_with_report().parked, 1);
    assert!(!*manager.inner.waiting_for_change.lock());
    assert_eq!(image.retained_memory_bytes(), 0);
    assert_eq!(image.snapshot().unwrap().as_ref(), &[2; 4]);
}

#[test]
fn last_temporary_image_drop_inside_a_sweep_cannot_deadlock_or_lose_its_wakeup() {
    assert_completes(|| {
        for len in [4, 8] {
            let (pool, manager) = manager(Some(4));
            let image = manager.from_frozen_bytes(vec![1; len]);
            let images = manager.resident_images();
            drop(image);

            assert!(manager.begin_scheduled_sweep());
            // Consuming the only remaining handle unregisters the image and
            // notifies this same manager from within the sweep.
            let report = manager.sweep(images, ParkableImage::park);
            assert_eq!(report.parked, usize::from(len == 4));
            assert_eq!(report.unavailable, usize::from(len == 8));
            manager.finish_sweep(report);

            assert!(!*manager.inner.waiting_for_change.lock());
            assert_eq!(manager.diagnostics().image_count, 0);
            let disk = pool.diagnostics();
            assert_eq!(disk.disk_footprint_bytes, disk.free_bytes as u64);
        }
    });
}

#[test]
fn wakeup_callbacks_can_query_the_manager_without_holding_its_locks() {
    assert_completes(|| {
        let owner = Arc::new(OnceLock::<Weak<ParkableImageManagerInner>>::new());
        let callback_owner = Arc::clone(&owner);
        let wakeups = Arc::new(AtomicUsize::new(0));
        let callback_wakeups = Arc::clone(&wakeups);
        let manager = ParkableImageManager::new_with_schedule_wakeup(
            Some(DiskPool::new(None).unwrap()),
            immediate_policy(),
            move || {
                let inner = callback_owner.get().unwrap().upgrade().unwrap();
                assert!(inner.waiting_for_change.try_lock().is_some());
                assert!(inner.registry.try_lock().is_some());
                let manager = ParkableImageManager { inner };
                manager.next_parking_deadline();
                manager.diagnostics();
                callback_wakeups.fetch_add(1, Ordering::Relaxed);
            },
        );
        owner.set(Arc::downgrade(&manager.inner)).unwrap();

        let image = manager.from_frozen_bytes(vec![1; 4]);
        let snapshot = image.snapshot().unwrap();
        drop(snapshot);
        manager.park_images();
        drop(image);
        assert_eq!(wakeups.load(Ordering::Relaxed), 5);
    });
}

#[test]
fn overlapping_sweeps_preserve_notifications_in_every_event_order() {
    #[derive(Clone, Copy, Debug)]
    enum Event {
        Begin(usize),
        Finish(usize),
        Notify,
    }
    use Event::{Begin, Finish, Notify};

    // All six interleavings that preserve each sweep's begin-before-finish.
    let orders = [
        [Begin(0), Finish(0), Begin(1), Finish(1)],
        [Begin(0), Begin(1), Finish(0), Finish(1)],
        [Begin(0), Begin(1), Finish(1), Finish(0)],
        [Begin(1), Finish(1), Begin(0), Finish(0)],
        [Begin(1), Begin(0), Finish(1), Finish(0)],
        [Begin(1), Begin(0), Finish(0), Finish(1)],
    ];
    let mut scenarios = 0;
    for order in orders {
        for notification_at in 0..=order.len() {
            let mut events = order.to_vec();
            events.insert(notification_at, Notify);
            for forced in [[false, false], [false, true], [true, false], [true, true]] {
                for unavailable in [[false, false], [false, true], [true, false], [true, true]] {
                    let manager = ParkableImageManager::default();
                    let mut admitted = [false; 2];
                    let mut last_begin = 0;
                    for (step, event) in events.iter().enumerate() {
                        match *event {
                            Begin(id) => {
                                admitted[id] = if forced[id] {
                                    manager.begin_forced_sweep();
                                    true
                                } else {
                                    manager.begin_scheduled_sweep()
                                };
                                if admitted[id] {
                                    last_begin = step;
                                }
                            }
                            Finish(id) if admitted[id] => {
                                manager.finish_sweep(if unavailable[id] {
                                    unavailable_report()
                                } else {
                                    ParkableImageSweepReport::default()
                                });
                            }
                            Finish(_) => {}
                            Notify => manager.notify_schedule_changed(),
                        }
                    }
                    let waiting = *manager.inner.waiting_for_change.lock();
                    let missed_capacity = (0..2).any(|id| admitted[id] && unavailable[id]);
                    if notification_at > last_begin || !missed_capacity {
                        assert!(
                            !waiting,
                            "lost wakeup: {events:?}, forced={forced:?}, unavailable={unavailable:?}"
                        );
                    } else if (0..2).all(|id| !admitted[id] || unavailable[id]) {
                        assert!(
                            waiting,
                            "capacity misses must wait: {events:?}, forced={forced:?}"
                        );
                    }
                    scenarios += 1;
                }
            }
        }
    }
    assert_eq!(scenarios, 480);
}

#[test]
fn concurrent_normal_force_read_and_drop_operations_preserve_data_and_progress() {
    assert_completes(|| {
        const IMAGE_COUNT: usize = 8;
        const IMAGE_SIZE: usize = 4096;
        const ROUNDS: usize = 256;
        let (pool, manager) = manager(None);
        let images = (0..IMAGE_COUNT)
            .map(|id| manager.from_frozen_bytes(vec![id as u8; IMAGE_SIZE]))
            .collect::<Vec<_>>();
        let start = Arc::new(Barrier::new(4));
        let workers = (0..4)
            .map(|role| {
                let manager = manager.clone();
                let images = images.clone();
                let start = Arc::clone(&start);
                thread::spawn(move || {
                    start.wait();
                    for round in 0..ROUNDS {
                        match role {
                            0 => manager.park_images(),
                            1 => manager.force_park_images(),
                            2 => {
                                let id = round % IMAGE_COUNT;
                                let snapshot = images[id].snapshot().unwrap();
                                let clone = snapshot.clone();
                                drop(snapshot);
                                assert_eq!(clone.as_ref(), &[id as u8; IMAGE_SIZE]);
                            }
                            3 => {
                                let image = manager.from_frozen_bytes(vec![9; IMAGE_SIZE]);
                                let snapshot = image.snapshot().unwrap();
                                drop(image);
                                assert_eq!(snapshot.as_ref(), &[9; IMAGE_SIZE]);
                            }
                            _ => unreachable!(),
                        }
                        thread::yield_now();
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }

        for (id, image) in images.iter().enumerate() {
            assert_eq!(image.snapshot().unwrap().as_ref(), &[id as u8; IMAGE_SIZE]);
        }
        // Ordinary scheduling must still progress after concurrent activity.
        assert_eq!(manager.park_images_with_report().parked, IMAGE_COUNT);
        let diagnostics = manager.diagnostics();
        assert_eq!(diagnostics.image_count, IMAGE_COUNT);
        assert_eq!(diagnostics.parked_count, IMAGE_COUNT);
        assert_eq!(diagnostics.retained_memory_bytes, 0);
        assert_eq!(diagnostics.retained_disk_bytes, IMAGE_COUNT * IMAGE_SIZE);
        assert!(!*manager.inner.waiting_for_change.lock());
        for image in &images {
            assert_eq!(
                image.diagnostics().storage,
                ParkableImageStorageState::Parked
            );
        }
        let disk = pool.diagnostics();
        assert_eq!(
            disk.disk_footprint_bytes - disk.free_bytes as u64,
            (IMAGE_COUNT * IMAGE_SIZE) as u64
        );
        drop(images);
        assert_eq!(manager.diagnostics().image_count, 0);
        let disk = pool.diagnostics();
        assert_eq!(disk.disk_footprint_bytes, disk.free_bytes as u64);
    });
}
