use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use moli_disk_pool::DiskPool;
use parking_lot::Mutex;

use crate::{
    image::{ParkOutcome, ParkableImage, ParkableImageStorageState},
    policy::ParkableImagePolicy,
    registry::ParkableImageRegistry,
};

/// Browser-runtime scoped owner and deadline scheduler for parkable images.
#[derive(Clone)]
pub struct ParkableImageManager {
    inner: Arc<ParkableImageManagerInner>,
}

struct ParkableImageManagerInner {
    disk_pool: Option<DiskPool>,
    policy: ParkableImagePolicy,
    next_image_id: AtomicU64,
    registry: Mutex<ParkableImageRegistry>,
    // Suppress expired deadlines after a capacity miss. Arm before scanning;
    // notifications clear this flag, and an unavailable sweep leaves it
    // untouched so it cannot overwrite a concurrent wakeup.
    // Never hold this lock while scanning, doing I/O, or invoking callbacks.
    waiting_for_change: Mutex<bool>,
    schedule_wakeup: Option<ScheduleWakeup>,
}

type ScheduleWakeup = Arc<dyn Fn() + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParkableImageManagerDiagnostics {
    pub image_count: usize,
    pub resident_count: usize,
    pub parking_count: usize,
    pub resident_with_disk_backup_count: usize,
    pub parked_count: usize,
    pub retained_memory_bytes: usize,
    pub retained_disk_bytes: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ParkableImageSweepReport {
    pub considered: usize,
    pub parked: usize,
    pub already_parked: usize,
    pub delayed: usize,
    pub in_use: usize,
    pub ineligible: usize,
    pub unavailable: usize,
    pub write_failures: usize,
}

impl ParkableImageManager {
    pub fn new(disk_pool: Option<DiskPool>, policy: ParkableImagePolicy) -> Self {
        Self::build(disk_pool, policy, None)
    }

    /// Creates a manager whose owner is notified whenever its next parking
    /// deadline may have changed. The callback should only wake the owner's
    /// scheduler; parking work itself remains outside the callback.
    pub fn new_with_schedule_wakeup(
        disk_pool: Option<DiskPool>,
        policy: ParkableImagePolicy,
        wakeup: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self::build(disk_pool, policy, Some(Arc::new(wakeup)))
    }

    fn build(
        disk_pool: Option<DiskPool>,
        policy: ParkableImagePolicy,
        schedule_wakeup: Option<ScheduleWakeup>,
    ) -> Self {
        Self {
            inner: Arc::new(ParkableImageManagerInner {
                disk_pool,
                policy,
                next_image_id: AtomicU64::new(0),
                registry: Mutex::new(ParkableImageRegistry::default()),
                waiting_for_change: Mutex::new(false),
                schedule_wakeup,
            }),
        }
    }

    pub(crate) fn policy(&self) -> ParkableImagePolicy {
        self.inner.policy
    }

    pub fn from_frozen_bytes(&self, bytes: Vec<u8>) -> ParkableImage {
        let should_register = bytes.len() >= self.inner.policy.min_size_to_park;
        let image = ParkableImage::new(self.clone(), self.allocate_image_id(), bytes);
        if should_register {
            self.register_resident(&image);
        }
        image
    }

    /// Returns the earliest deadline among schedulable resident images.
    /// Parked images are kept in a separate registry and are not inspected.
    /// A sweep in progress or waiting for a change can suppress the deadline.
    pub fn next_parking_deadline(&self) -> Option<Instant> {
        if *self.inner.waiting_for_change.lock() {
            return None;
        }
        self.resident_images()
            .into_iter()
            .filter_map(|image| image.parking_deadline())
            .min()
    }

    /// Attempts to park resident images whose deadline has elapsed.
    ///
    /// This does not wait for future deadlines. Disk I/O is synchronous; the
    /// owner should wait for [`Self::next_parking_deadline`] and call this from
    /// a blocking worker.
    pub fn park_images(&self) {
        self.park_images_with_report();
    }

    pub(crate) fn park_images_with_report(&self) -> ParkableImageSweepReport {
        if !self.begin_scheduled_sweep() {
            return ParkableImageSweepReport::default();
        }
        let now = Instant::now();
        let due = self
            .resident_images()
            .into_iter()
            .filter(|image| {
                image
                    .parking_deadline()
                    .is_some_and(|deadline| deadline <= now)
            })
            .collect();
        let report = self.sweep(due, ParkableImage::park);
        self.finish_sweep(report);
        report
    }

    /// Attempts to park all resident images, ignoring deadlines and retrying
    /// after unavailable disk reservations.
    ///
    /// Live readers, in-flight writes, size thresholds, and pool limits still
    /// apply. Disk I/O is synchronous. Normal scheduling uses [`Self::park_images`].
    pub fn force_park_images(&self) {
        self.force_park_images_with_report();
    }

    pub(crate) fn force_park_images_with_report(&self) -> ParkableImageSweepReport {
        self.begin_forced_sweep();
        let images = self.resident_images();
        let report = self.sweep(images, ParkableImage::force_park);
        self.finish_sweep(report);
        report
    }

    pub fn diagnostics(&self) -> ParkableImageManagerDiagnostics {
        let images = self.inner.registry.lock().all_images();
        let mut diagnostics = ParkableImageManagerDiagnostics::default();
        for image in images {
            let image = image.diagnostics();
            diagnostics.image_count += 1;
            match image.storage {
                ParkableImageStorageState::Resident => diagnostics.resident_count += 1,
                ParkableImageStorageState::Parking => diagnostics.parking_count += 1,
                ParkableImageStorageState::ResidentWithDiskBackup => {
                    diagnostics.resident_with_disk_backup_count += 1;
                }
                ParkableImageStorageState::Parked => diagnostics.parked_count += 1,
            }
            diagnostics.retained_memory_bytes = diagnostics
                .retained_memory_bytes
                .saturating_add(image.retained_memory_bytes);
            diagnostics.retained_disk_bytes = diagnostics
                .retained_disk_bytes
                .saturating_add(image.retained_disk_bytes);
        }
        diagnostics
    }

    pub(crate) fn disk_pool(&self) -> Option<DiskPool> {
        self.inner.disk_pool.clone()
    }

    pub(crate) fn notify_schedule_changed(&self) {
        *self.inner.waiting_for_change.lock() = false;
        self.wake_scheduler();
    }

    /// Moves an image between the physical-residency indexes.
    ///
    /// The caller must hold the image state lock. The manager never acquires
    /// an image lock while holding the registry lock, so the global lock order
    /// is always image state -> manager registry.
    pub(crate) fn move_to_parked_while_image_locked(&self, image: &ParkableImage) {
        self.inner.registry.lock().move_to_parked(image);
    }

    /// See [`Self::move_to_parked_while_image_locked`] for the lock-order
    /// contract.
    pub(crate) fn move_to_resident_while_image_locked(&self, image: &ParkableImage) {
        self.inner.registry.lock().move_to_resident(image);
    }

    pub(crate) fn unregister(&self, id: u64) {
        let removed = self.inner.registry.lock().remove(id);
        if removed {
            self.notify_schedule_changed();
        }
    }

    fn allocate_image_id(&self) -> u64 {
        self.inner
            .next_image_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1)
    }

    fn register_resident(&self, image: &ParkableImage) {
        self.inner.registry.lock().register_resident(image);
        self.notify_schedule_changed();
    }

    fn resident_images(&self) -> Vec<ParkableImage> {
        self.inner.registry.lock().resident_images()
    }

    fn begin_scheduled_sweep(&self) -> bool {
        let mut waiting = self.inner.waiting_for_change.lock();
        if *waiting {
            return false;
        }
        *waiting = true;
        true
    }

    fn begin_forced_sweep(&self) {
        *self.inner.waiting_for_change.lock() = true;
    }

    fn finish_sweep(&self, report: ParkableImageSweepReport) {
        // Overlapping completions may allow an extra retry; they must never
        // re-arm the wait after a notification has cleared it.
        if report.unavailable == 0 {
            *self.inner.waiting_for_change.lock() = false;
        }
        self.wake_scheduler();
    }

    fn wake_scheduler(&self) {
        if let Some(wakeup) = &self.inner.schedule_wakeup {
            wakeup();
        }
    }

    fn sweep(
        &self,
        images: Vec<ParkableImage>,
        park: fn(&ParkableImage) -> std::io::Result<ParkOutcome>,
    ) -> ParkableImageSweepReport {
        let mut report = ParkableImageSweepReport::default();
        for image in images {
            report.considered += 1;
            match park(&image) {
                Ok(ParkOutcome::Parked) => report.parked += 1,
                Ok(ParkOutcome::AlreadyParked) => report.already_parked += 1,
                Ok(ParkOutcome::Delayed { .. }) => report.delayed += 1,
                Ok(ParkOutcome::InUse) => report.in_use += 1,
                Ok(ParkOutcome::Unavailable) => report.unavailable += 1,
                Ok(ParkOutcome::BelowMinimum) => {
                    report.ineligible += 1;
                }
                Err(_) => report.write_failures += 1,
            }
        }
        report
    }
}

impl Default for ParkableImageManager {
    fn default() -> Self {
        Self::new(None, ParkableImagePolicy::default())
    }
}

impl fmt::Debug for ParkableImageManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParkableImageManager")
            .field("policy", &self.policy())
            .field("diagnostics", &self.diagnostics())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
