use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use moli_disk_pool::DiskPool;
use parking_lot::Mutex;

use crate::{
    ParkableImageManager, ParkableImagePolicy,
    image::{ParkOutcome, ParkableImageStorageState},
};

fn manager(policy: ParkableImagePolicy) -> (DiskPool, ParkableImageManager) {
    let pool = DiskPool::new(None).unwrap();
    let manager = ParkableImageManager::new(Some(pool.clone()), policy);
    (pool, manager)
}

fn immediate_policy() -> ParkableImagePolicy {
    ParkableImagePolicy {
        min_size_to_park: 1,
        parking_delay: Duration::ZERO,
        reader_release_delay: Duration::ZERO,
    }
}

fn reference_bytes(len: usize) -> Vec<u8> {
    (0..len)
        .map(|offset| u8::try_from((offset * 37) % 251).unwrap())
        .collect()
}

#[test]
fn default_policy_keeps_unused_and_recently_read_images_resident() {
    let policy = ParkableImagePolicy::default();
    assert_eq!(policy.min_size_to_park, 1024);
    assert_eq!(policy.parking_delay, Duration::from_secs(30));
    assert_eq!(policy.reader_release_delay, Duration::from_secs(2));
}

#[test]
fn unused_image_observes_delay_but_used_image_can_park_immediately() {
    let (_, manager) = manager(ParkableImagePolicy {
        min_size_to_park: 1,
        parking_delay: Duration::from_secs(30),
        reader_release_delay: Duration::ZERO,
    });
    let image = manager.from_frozen_bytes(vec![7; 128]);
    assert!(matches!(
        image.maybe_park().unwrap(),
        ParkOutcome::Delayed { .. }
    ));

    let snapshot = image.snapshot().unwrap();
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::InUse);
    drop(snapshot);
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::Parked);
}

#[test]
fn forced_sweep_bypasses_deadlines_but_not_live_readers() {
    let (_, manager) = manager(ParkableImagePolicy {
        min_size_to_park: 1,
        parking_delay: Duration::from_secs(30),
        reader_release_delay: Duration::from_secs(30),
    });
    let image = manager.from_frozen_bytes(vec![7; 128]);
    let snapshot = image.snapshot().unwrap();

    let report = manager.park_images_now_with_report();
    assert_eq!(report.in_use, 1);
    assert_eq!(report.parked, 0);

    drop(snapshot);
    let report = manager.park_images_now_with_report();
    assert_eq!(report.parked, 1);
}

#[test]
fn one_extent_round_trips_and_reparks_without_another_write() {
    let (pool, manager) = manager(immediate_policy());
    let bytes = vec![42; 64 * 1024];
    let image = manager.from_frozen_bytes(bytes.clone());

    assert_eq!(
        image.diagnostics().storage,
        ParkableImageStorageState::Resident
    );
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::Parked);
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::AlreadyParked);
    assert_eq!(
        image.diagnostics().storage,
        ParkableImageStorageState::Parked
    );
    assert_eq!(pool.diagnostics().disk_footprint_bytes, bytes.len() as u64);
    let snapshot = image.snapshot().unwrap();
    assert_eq!(&*snapshot, bytes);
    assert_eq!(
        image.diagnostics().storage,
        ParkableImageStorageState::ResidentWithDiskBackup
    );
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::InUse);
    drop(snapshot);

    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::Parked);
    assert_eq!(
        image.diagnostics().storage,
        ParkableImageStorageState::Parked
    );
    assert_eq!(pool.diagnostics().disk_footprint_bytes, bytes.len() as u64);
    assert_eq!(image.snapshot().unwrap().as_ref(), bytes);
}

#[test]
fn parked_data_can_be_read_in_segments_and_at_boundaries() {
    let (_, manager) = manager(immediate_policy());
    let bytes = reference_bytes(3 * 4096 + 2048);
    let image = manager.from_frozen_bytes(bytes.clone());
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::Parked);

    let mut restored = Vec::new();
    let mut offset = 0;
    while offset < image.len() {
        let segment = image.read_range(offset, 4096).unwrap();
        assert!(!segment.is_empty());
        offset += segment.len();
        restored.extend_from_slice(&segment);
    }

    assert_eq!(restored, bytes);
    assert_eq!(
        image.read_range(image.len(), 4096).unwrap(),
        Vec::<u8>::new()
    );
    assert_eq!(
        image.read_range(image.len() + 1, 4096).unwrap(),
        Vec::<u8>::new()
    );
    assert_eq!(image.read_range(100, 0).unwrap(), Vec::<u8>::new());
    assert_eq!(
        image.diagnostics().storage,
        ParkableImageStorageState::ResidentWithDiskBackup
    );
}

#[test]
fn snapshot_can_outlive_the_image_and_its_manager() {
    let bytes = reference_bytes(3 * 4096 + 2048);
    let snapshot = {
        let (_, manager) = manager(immediate_policy());
        let image = manager.from_frozen_bytes(bytes.clone());
        image.snapshot().unwrap()
    };

    assert_eq!(&*snapshot, bytes);
}

#[test]
fn live_snapshot_vetoes_parking_without_copying() {
    let (_, manager) = manager(immediate_policy());
    let image = manager.from_frozen_bytes(vec![5; 16]);
    let first = image.snapshot().unwrap();
    let second = first.clone();
    assert_eq!(image.diagnostics().snapshot_count, 2);
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::InUse);
    drop((first, second));
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::Parked);
}

#[test]
fn below_minimum_and_capacity_failure_keep_memory() {
    let limited_pool = DiskPool::new(Some(4)).unwrap();
    let manager = ParkableImageManager::new(
        Some(limited_pool),
        ParkableImagePolicy {
            min_size_to_park: 2,
            parking_delay: Duration::ZERO,
            reader_release_delay: Duration::ZERO,
        },
    );
    let small = manager.from_frozen_bytes(vec![1]);
    assert_eq!(small.maybe_park().unwrap(), ParkOutcome::BelowMinimum);

    let large = manager.from_frozen_bytes(vec![2; 8]);
    assert_eq!(large.maybe_park().unwrap(), ParkOutcome::Unavailable);
    assert_eq!(
        large.diagnostics().storage,
        ParkableImageStorageState::Resident
    );
    assert_eq!(large.snapshot().unwrap().as_ref(), vec![2; 8]);
}

#[test]
fn failed_write_keeps_the_image_resident_and_disables_the_pool() {
    let (pool, manager) = manager(immediate_policy());
    let bytes = reference_bytes(3 * 4096 + 2048);
    let image = manager.from_frozen_bytes(bytes.clone());
    pool.fail_next_write_for_test();

    let report = manager.park_images_now_with_report();

    assert_eq!(report.considered, 1);
    assert_eq!(report.write_failures, 1);
    assert!(!pool.may_write());
    assert_eq!(pool.diagnostics().free_bytes, bytes.len());
    assert_eq!(
        image.diagnostics().storage,
        ParkableImageStorageState::Resident
    );
    assert_eq!(image.snapshot().unwrap().as_ref(), bytes);
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::Unavailable);
}

#[test]
fn limited_capacity_is_reclaimed_for_the_next_image() {
    const CAPACITY: usize = 32 * 1024;
    let pool = DiskPool::new(Some(u64::try_from(CAPACITY).unwrap())).unwrap();
    let manager = ParkableImageManager::new(Some(pool.clone()), immediate_policy());
    let first = manager.from_frozen_bytes(vec![1; CAPACITY]);
    let second = manager.from_frozen_bytes(vec![2; CAPACITY]);

    assert_eq!(first.maybe_park().unwrap(), ParkOutcome::Parked);
    assert_eq!(second.maybe_park().unwrap(), ParkOutcome::Unavailable);
    drop(first);
    assert_eq!(pool.diagnostics().free_bytes, CAPACITY);

    assert_eq!(second.maybe_park().unwrap(), ParkOutcome::Parked);
    assert_eq!(pool.diagnostics().disk_footprint_bytes, CAPACITY as u64);
    assert_eq!(second.snapshot().unwrap().as_ref(), vec![2; CAPACITY]);
}

#[test]
fn dropping_a_parked_image_on_another_thread_releases_its_extent() {
    const IMAGE_SIZE: usize = 16 * 1024;
    let (pool, manager) = manager(immediate_policy());
    let image = manager.from_frozen_bytes(vec![3; IMAGE_SIZE]);
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::Parked);
    assert_eq!(pool.diagnostics().free_bytes, 0);

    std::thread::spawn(move || drop(image)).join().unwrap();

    assert_eq!(pool.diagnostics().free_bytes, IMAGE_SIZE);
    assert_eq!(manager.diagnostics().image_count, 0);
}

#[test]
fn manager_sweeps_only_live_registered_images() {
    let (_, manager) = manager(immediate_policy());
    let first = manager.from_frozen_bytes(vec![1; 8]);
    let second = manager.from_frozen_bytes(vec![2; 8]);
    let small = manager.from_frozen_bytes(Vec::new());
    drop(second);

    let report = manager.park_images_now_with_report();
    assert_eq!(report.considered, 1);
    assert_eq!(report.parked, 1);
    assert_eq!(report.ineligible, 0);
    let diagnostics = manager.diagnostics();
    assert_eq!(diagnostics.image_count, 1);
    assert_eq!(diagnostics.parked_count, 1);
    assert_eq!(diagnostics.resident_count, 0);
    assert_eq!(diagnostics.parking_count, 0);
    assert_eq!(diagnostics.resident_with_disk_backup_count, 0);
    assert!(small.is_empty());
    drop((first, small));
    assert_eq!(manager.diagnostics().image_count, 0);
}

#[test]
fn weak_handle_does_not_extend_image_lifetime() {
    let (_, manager) = manager(immediate_policy());
    let image = manager.from_frozen_bytes(vec![1; 8]);
    let weak = image.downgrade();
    assert!(weak.upgrade().is_some());
    drop(image);
    assert!(weak.upgrade().is_none());
}

#[test]
fn empty_images_are_not_registered_or_parked() {
    let (pool, manager) = manager(immediate_policy());
    let image = manager.from_frozen_bytes(Vec::new());

    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::BelowMinimum);
    assert_eq!(manager.diagnostics().image_count, 0);
    assert_eq!(pool.diagnostics().disk_footprint_bytes, 0);
    assert_eq!(pool.diagnostics().free_bytes, 0);
}

#[test]
fn minimum_size_boundary_is_inclusive() {
    const MINIMUM: usize = 4096;
    let (pool, manager) = manager(ParkableImagePolicy {
        min_size_to_park: MINIMUM,
        parking_delay: Duration::ZERO,
        reader_release_delay: Duration::ZERO,
    });
    let below = manager.from_frozen_bytes(vec![1; MINIMUM - 1]);
    let exact = manager.from_frozen_bytes(vec![2; MINIMUM]);

    assert_eq!(below.maybe_park().unwrap(), ParkOutcome::BelowMinimum);
    assert_eq!(exact.maybe_park().unwrap(), ParkOutcome::Parked);
    assert_eq!(pool.diagnostics().disk_footprint_bytes, MINIMUM as u64);
}

#[test]
fn manager_without_a_pool_reports_unavailable_and_preserves_data() {
    let manager = ParkableImageManager::new(None, immediate_policy());
    let bytes = reference_bytes(4096);
    let image = manager.from_frozen_bytes(bytes.clone());

    let report = manager.park_images_now_with_report();

    assert_eq!(report.considered, 1);
    assert_eq!(report.unavailable, 1);
    assert_eq!(report.parked, 0);
    assert_eq!(
        image.diagnostics().storage,
        ParkableImageStorageState::Resident
    );
    assert_eq!(image.snapshot().unwrap().as_ref(), bytes);
}

#[test]
fn cloned_handles_share_one_entry_and_release_only_on_last_drop() {
    const IMAGE_SIZE: usize = 8192;
    let (pool, manager) = manager(immediate_policy());
    let image = manager.from_frozen_bytes(vec![7; IMAGE_SIZE]);
    let clone = image.clone();

    assert!(image.shares_storage_with(&clone));
    assert_eq!(manager.diagnostics().image_count, 1);
    assert_eq!(clone.maybe_park().unwrap(), ParkOutcome::Parked);
    drop(image);
    assert_eq!(manager.diagnostics().image_count, 1);
    assert_eq!(pool.diagnostics().free_bytes, 0);

    drop(clone);
    assert_eq!(manager.diagnostics().image_count, 0);
    assert_eq!(pool.diagnostics().free_bytes, IMAGE_SIZE);
}

#[test]
fn concurrent_park_attempts_create_only_one_disk_extent() {
    const THREADS: usize = 8;
    let (pool, manager) = manager(immediate_policy());
    let bytes = reference_bytes(128 * 1024);
    let image = manager.from_frozen_bytes(bytes.clone());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(THREADS));

    let workers = (0..THREADS)
        .map(|_| {
            let image = image.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                image.maybe_park().unwrap()
            })
        })
        .collect::<Vec<_>>();
    let outcomes = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(
        outcomes
            .iter()
            .filter(|&&outcome| outcome == ParkOutcome::Parked)
            .count(),
        1
    );
    assert!(outcomes.iter().all(|outcome| matches!(
        outcome,
        ParkOutcome::Parked | ParkOutcome::AlreadyParked | ParkOutcome::InUse
    )));
    assert_eq!(pool.diagnostics().disk_footprint_bytes, bytes.len() as u64);
    assert_eq!(image.snapshot().unwrap().as_ref(), bytes);
}

#[test]
fn concurrent_reads_unpark_once_and_preserve_every_copy() {
    const THREADS: usize = 8;
    let (pool, manager) = manager(immediate_policy());
    let bytes = reference_bytes(128 * 1024);
    let image = manager.from_frozen_bytes(bytes.clone());
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::Parked);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(THREADS));

    let workers = (0..THREADS)
        .map(|_| {
            let image = image.clone();
            let bytes = bytes.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                assert_eq!(image.snapshot().unwrap().as_ref(), bytes);
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }

    let diagnostics = image.diagnostics();
    assert_eq!(
        diagnostics.storage,
        ParkableImageStorageState::ResidentWithDiskBackup
    );
    assert_eq!(diagnostics.snapshot_count, 0);
    assert_eq!(pool.diagnostics().disk_footprint_bytes, bytes.len() as u64);
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::Parked);
}

#[test]
fn next_deadline_tracks_creation_and_the_last_snapshot_release_grace() {
    let delay = Duration::from_secs(30);
    let release_delay = Duration::from_secs(10);
    let wakeups = Arc::new(AtomicUsize::new(0));
    let wakeups_for_callback = Arc::clone(&wakeups);
    let pool = DiskPool::new(None).unwrap();
    let manager = ParkableImageManager::new_with_schedule_wakeup(
        Some(pool),
        ParkableImagePolicy {
            min_size_to_park: 1,
            parking_delay: delay,
            reader_release_delay: release_delay,
        },
        move || {
            wakeups_for_callback.fetch_add(1, Ordering::Relaxed);
        },
    );
    let before_creation = Instant::now();
    let image = manager.from_frozen_bytes(b"encoded image".to_vec());
    let deadline = manager
        .next_parking_deadline()
        .expect("creating an unused eligible image must schedule its delay");
    assert!(deadline >= before_creation + delay);
    assert!(deadline <= Instant::now() + delay);

    let snapshot = image.snapshot().unwrap();
    assert_eq!(
        manager.next_parking_deadline(),
        None,
        "a live snapshot must remove the image from deadline candidates"
    );
    let wakeups_before_drop = wakeups.load(Ordering::Relaxed);
    let before_drop = Instant::now();
    drop(snapshot);
    assert!(wakeups.load(Ordering::Relaxed) > wakeups_before_drop);
    let deadline = manager
        .next_parking_deadline()
        .expect("the last snapshot release must schedule its grace period");
    assert!(deadline >= before_drop + release_delay);
    assert!(deadline <= Instant::now() + release_delay);
    assert!(matches!(
        image.maybe_park().unwrap(),
        ParkOutcome::Delayed { .. }
    ));
}

#[test]
fn last_snapshot_release_is_visible_before_the_scheduler_wakeup() {
    let manager_slot = Arc::new(Mutex::new(None::<ParkableImageManager>));
    let manager_slot_for_callback = Arc::clone(&manager_slot);
    let wakeups = Arc::new(AtomicUsize::new(0));
    let wakeups_for_callback = Arc::clone(&wakeups);
    let observed_due = Arc::new(AtomicBool::new(false));
    let observed_due_for_callback = Arc::clone(&observed_due);
    let manager = ParkableImageManager::new_with_schedule_wakeup(
        Some(DiskPool::new(None).unwrap()),
        immediate_policy(),
        move || {
            wakeups_for_callback.fetch_add(1, Ordering::SeqCst);
            let manager = manager_slot_for_callback.lock().clone();
            let due = manager.is_some_and(|manager| {
                manager
                    .next_parking_deadline()
                    .is_some_and(|deadline| deadline <= Instant::now())
            });
            observed_due_for_callback.store(due, Ordering::SeqCst);
        },
    );
    *manager_slot.lock() = Some(manager.clone());
    let image = manager.from_frozen_bytes(vec![7; 4096]);
    let first = image.snapshot().unwrap();
    let second = first.clone();

    observed_due.store(false, Ordering::SeqCst);
    let wakeups_before_drop = wakeups.load(Ordering::SeqCst);
    drop(first);
    assert_eq!(wakeups.load(Ordering::SeqCst), wakeups_before_drop);

    drop(second);
    assert_eq!(wakeups.load(Ordering::SeqCst), wakeups_before_drop + 1);
    assert!(
        observed_due.load(Ordering::SeqCst),
        "the wakeup must observe the reader count after the last lease is released"
    );
}

#[test]
fn concurrent_park_and_unpark_keep_storage_and_registry_aligned() {
    const ITERATIONS: usize = 500;
    let (_, manager) = manager(immediate_policy());
    let image = manager.from_frozen_bytes(vec![5; 4096]);
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::Parked);
    let start = Arc::new(std::sync::Barrier::new(2));

    let reader = {
        let image = image.clone();
        let start = Arc::clone(&start);
        std::thread::spawn(move || {
            start.wait();
            for _ in 0..ITERATIONS {
                let snapshot = image.snapshot().unwrap();
                assert_eq!(snapshot.len(), 4096);
                drop(snapshot);
                std::thread::yield_now();
            }
        })
    };
    let parker = {
        let image = image.clone();
        let start = Arc::clone(&start);
        std::thread::spawn(move || {
            start.wait();
            for _ in 0..ITERATIONS {
                image.maybe_park().unwrap();
                std::thread::yield_now();
            }
        })
    };
    reader.join().unwrap();
    parker.join().unwrap();

    let snapshot = image.snapshot().unwrap();
    drop(snapshot);
    let report = manager.park_images_due_with_report();
    assert_eq!(report.considered, 1);
    assert_eq!(report.parked, 1);
    assert_eq!(manager.diagnostics().parked_count, 1);
}

#[test]
fn parked_images_leave_the_resident_schedule_until_they_are_unparked() {
    let (_, manager) = manager(immediate_policy());
    let image = manager.from_frozen_bytes(vec![3; 4096]);

    let report = manager.park_images_due_with_report();
    assert_eq!(report.considered, 1);
    assert_eq!(report.parked, 1);
    assert_eq!(manager.next_parking_deadline(), None);
    assert_eq!(
        manager.park_images_now_with_report().considered,
        0,
        "the parked registry must not be part of resident sweeps"
    );
    assert_eq!(manager.diagnostics().parked_count, 1);

    let snapshot = image.snapshot().unwrap();
    assert_eq!(manager.diagnostics().parked_count, 0);
    assert_eq!(manager.diagnostics().resident_with_disk_backup_count, 1);
    assert_eq!(manager.next_parking_deadline(), None);
    drop(snapshot);

    let report = manager.park_images_due_with_report();
    assert_eq!(report.considered, 1);
    assert_eq!(report.parked, 1);
    assert_eq!(manager.diagnostics().parked_count, 1);
}

#[test]
fn capacity_failure_can_be_retried_after_an_extent_is_released() {
    const IMAGE_SIZE: usize = 4096;
    let pool = DiskPool::new(Some(IMAGE_SIZE as u64)).unwrap();
    let released_before_wakeup = Arc::new(AtomicBool::new(false));
    let released_before_wakeup_for_callback = Arc::clone(&released_before_wakeup);
    let pool_for_callback = pool.clone();
    let manager =
        ParkableImageManager::new_with_schedule_wakeup(Some(pool), immediate_policy(), move || {
            if pool_for_callback.diagnostics().free_bytes >= IMAGE_SIZE {
                released_before_wakeup_for_callback.store(true, Ordering::SeqCst);
            }
        });
    let first = manager.from_frozen_bytes(vec![1; IMAGE_SIZE]);
    let second = manager.from_frozen_bytes(vec![2; IMAGE_SIZE]);
    assert_eq!(first.maybe_park().unwrap(), ParkOutcome::Parked);

    let report = manager.park_images_due_with_report();
    assert_eq!(report.considered, 1);
    assert_eq!(report.unavailable, 1);
    assert_eq!(
        manager.next_parking_deadline(),
        None,
        "a capacity miss must wait for a state change instead of spinning on an expired deadline"
    );
    assert_eq!(manager.park_images_due_with_report().considered, 0);

    released_before_wakeup.store(false, Ordering::SeqCst);
    drop(first);
    assert!(released_before_wakeup.load(Ordering::SeqCst));
    assert!(
        manager
            .next_parking_deadline()
            .is_some_and(|deadline| deadline <= Instant::now()),
        "dropping the parked owner must release its extent before waking the scheduler"
    );
    assert_eq!(manager.park_images_due_with_report().parked, 1);
    assert_eq!(
        second.diagnostics().storage,
        ParkableImageStorageState::Parked
    );
}

#[test]
fn surviving_snapshot_does_not_keep_the_disk_extent_reserved() {
    const IMAGE_SIZE: usize = 16 * 1024;
    let (pool, manager) = manager(immediate_policy());
    let image = manager.from_frozen_bytes(vec![9; IMAGE_SIZE]);
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::Parked);
    let snapshot = image.snapshot().unwrap();
    assert_eq!(
        image.diagnostics().storage,
        ParkableImageStorageState::ResidentWithDiskBackup
    );

    drop(image);

    assert_eq!(pool.diagnostics().free_bytes, IMAGE_SIZE);
    assert_eq!(&*snapshot, vec![9; IMAGE_SIZE]);
    assert_eq!(manager.diagnostics().image_count, 0);
}
