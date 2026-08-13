use std::time::Duration;

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
