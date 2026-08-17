use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use moli_disk_pool::DiskPool;

use crate::{
    ParkOutcome, ParkableImageManager, ParkableImageMutationError, ParkableImagePolicy,
    ParkableImageStorageState,
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
    }
}

fn reference_bytes(len: usize) -> Vec<u8> {
    (0..len)
        .map(|offset| u8::try_from((offset * 37) % 251).unwrap())
        .collect()
}

#[test]
fn initial_capacity_does_not_change_image_size() {
    let (_, manager) = manager(immediate_policy());
    let image = manager.create(32 * 1024);

    assert!(image.is_empty());
    assert_eq!(image.len(), 0);
    image.append(b"encoded image").unwrap();
    assert_eq!(image.len(), 13);
}

#[test]
fn append_then_freeze_makes_bytes_immutable() {
    let (_, manager) = manager(immediate_policy());
    let image = manager.create(8);
    image.append(b"abc").unwrap();
    image.append(b"def").unwrap();
    assert_eq!(&*image.snapshot().unwrap(), b"abcdef");
    image.freeze().unwrap();
    assert_eq!(
        image.append(b"late"),
        Err(ParkableImageMutationError::Frozen)
    );
    assert_eq!(
        image.freeze(),
        Err(ParkableImageMutationError::AlreadyFrozen)
    );
}

#[test]
fn mutable_snapshot_keeps_its_size_and_contents_after_append() {
    let (_, manager) = manager(immediate_policy());
    let image = manager.create(16);
    image.append(b"12345").unwrap();
    let snapshot = image.snapshot().unwrap();

    image.append(b"67890").unwrap();

    assert_eq!(&*snapshot, b"12345");
    assert_eq!(&*image.snapshot().unwrap(), b"1234567890");
}

#[test]
fn unused_image_observes_delay_but_used_image_can_park_immediately() {
    let (_, manager) = manager(ParkableImagePolicy {
        min_size_to_park: 1,
        parking_delay: Duration::from_secs(30),
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
    assert_eq!(image.data().unwrap(), bytes);
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
    assert_eq!(large.data().unwrap(), vec![2; 8]);
}

#[test]
fn failed_write_keeps_the_image_resident_and_disables_the_pool() {
    let (pool, manager) = manager(immediate_policy());
    let bytes = reference_bytes(3 * 4096 + 2048);
    let image = manager.from_frozen_bytes(bytes.clone());
    pool.fail_next_write_for_test();

    let report = manager.maybe_park_images();

    assert_eq!(report.considered, 1);
    assert_eq!(report.write_failures, 1);
    assert!(!pool.may_write());
    assert_eq!(pool.diagnostics().free_bytes, bytes.len());
    assert_eq!(
        image.diagnostics().storage,
        ParkableImageStorageState::Resident
    );
    assert_eq!(image.data().unwrap(), bytes);
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
    assert_eq!(second.data().unwrap(), vec![2; CAPACITY]);
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
fn manager_sweeps_live_images_and_prunes_dead_entries() {
    let (_, manager) = manager(immediate_policy());
    let first = manager.from_frozen_bytes(vec![1; 8]);
    let second = manager.from_frozen_bytes(vec![2; 8]);
    let mutable = manager.create(0);
    drop(second);

    let report = manager.maybe_park_images();
    assert_eq!(report.considered, 2);
    assert_eq!(report.parked, 1);
    assert_eq!(report.ineligible, 1);
    let diagnostics = manager.diagnostics();
    assert_eq!(diagnostics.image_count, 2);
    assert_eq!(diagnostics.parked_count, 1);
    assert_eq!(diagnostics.mutable_count, 1);
    assert_eq!(diagnostics.resident_count, 0);
    assert_eq!(diagnostics.parking_count, 0);
    assert_eq!(diagnostics.resident_with_disk_backup_count, 0);
    drop((first, mutable));
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
fn mutable_and_empty_frozen_images_never_allocate_disk_space() {
    let (pool, manager) = manager(immediate_policy());
    let image = manager.create(1024);

    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::NotFrozen);
    image.freeze().unwrap();
    assert_eq!(image.maybe_park().unwrap(), ParkOutcome::BelowMinimum);
    assert_eq!(pool.diagnostics().disk_footprint_bytes, 0);
    assert_eq!(pool.diagnostics().free_bytes, 0);
}

#[test]
fn minimum_size_boundary_is_inclusive() {
    const MINIMUM: usize = 4096;
    let (pool, manager) = manager(ParkableImagePolicy {
        min_size_to_park: MINIMUM,
        parking_delay: Duration::ZERO,
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

    let report = manager.maybe_park_images();

    assert_eq!(report.considered, 1);
    assert_eq!(report.unavailable, 1);
    assert_eq!(report.parked, 0);
    assert_eq!(
        image.diagnostics().storage,
        ParkableImageStorageState::Resident
    );
    assert_eq!(image.data().unwrap(), bytes);
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
    assert_eq!(image.data().unwrap(), bytes);
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
                assert_eq!(image.data().unwrap(), bytes);
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
fn next_deadline_tracks_freeze_use_and_the_last_snapshot_drop() {
    let delay = Duration::from_secs(30);
    let wakeups = Arc::new(AtomicUsize::new(0));
    let wakeups_for_callback = Arc::clone(&wakeups);
    let pool = DiskPool::new(None).unwrap();
    let manager = ParkableImageManager::new_with_schedule_wakeup(
        Some(pool),
        ParkableImagePolicy {
            min_size_to_park: 1,
            parking_delay: delay,
        },
        move || {
            wakeups_for_callback.fetch_add(1, Ordering::Relaxed);
        },
    );
    let image = manager.create(0);
    image.append(b"encoded image").unwrap();
    assert_eq!(manager.next_parking_deadline(), None);

    let before_freeze = Instant::now();
    image.freeze().unwrap();
    let deadline = manager
        .next_parking_deadline()
        .expect("freezing an unused eligible image must schedule its delay");
    assert!(deadline >= before_freeze + delay);
    assert!(deadline <= Instant::now() + delay);

    let snapshot = image.snapshot().unwrap();
    assert_eq!(
        manager.next_parking_deadline(),
        None,
        "a live snapshot must remove the image from deadline candidates"
    );
    let wakeups_before_drop = wakeups.load(Ordering::Relaxed);
    drop(snapshot);
    assert!(wakeups.load(Ordering::Relaxed) > wakeups_before_drop);
    assert!(
        manager
            .next_parking_deadline()
            .is_some_and(|deadline| deadline <= Instant::now()),
        "a used image becomes immediately due after its last snapshot drops"
    );
}

#[test]
fn parked_images_leave_the_resident_schedule_until_they_are_unparked() {
    let (_, manager) = manager(immediate_policy());
    let image = manager.from_frozen_bytes(vec![3; 4096]);

    let report = manager.park_images_due();
    assert_eq!(report.considered, 1);
    assert_eq!(report.parked, 1);
    assert_eq!(manager.next_parking_deadline(), None);
    assert_eq!(
        manager.maybe_park_images().considered,
        0,
        "the parked registry must not be part of resident sweeps"
    );
    assert_eq!(manager.diagnostics().parked_count, 1);

    let snapshot = image.snapshot().unwrap();
    assert_eq!(manager.diagnostics().parked_count, 0);
    assert_eq!(manager.diagnostics().resident_with_disk_backup_count, 1);
    assert_eq!(manager.next_parking_deadline(), None);
    drop(snapshot);

    let report = manager.park_images_due();
    assert_eq!(report.considered, 1);
    assert_eq!(report.parked, 1);
    assert_eq!(manager.diagnostics().parked_count, 1);
}

#[test]
fn capacity_failure_has_no_expired_deadline_until_an_extent_is_released() {
    const IMAGE_SIZE: usize = 4096;
    let pool = DiskPool::new(Some(IMAGE_SIZE as u64)).unwrap();
    let manager = ParkableImageManager::new(Some(pool), immediate_policy());
    let first = manager.from_frozen_bytes(vec![1; IMAGE_SIZE]);
    let second = manager.from_frozen_bytes(vec![2; IMAGE_SIZE]);
    assert_eq!(first.maybe_park().unwrap(), ParkOutcome::Parked);

    let report = manager.park_images_due();
    assert_eq!(report.considered, 1);
    assert_eq!(report.unavailable, 1);
    assert_eq!(
        manager.next_parking_deadline(),
        None,
        "a full pool must not leave an already-expired deadline spinning"
    );

    drop(first);
    assert!(
        manager
            .next_parking_deadline()
            .is_some_and(|deadline| deadline <= Instant::now())
    );
    assert_eq!(manager.park_images_due().parked, 1);
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
