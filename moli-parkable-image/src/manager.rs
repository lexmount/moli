use std::{fmt, sync::Arc};

use moli_disk_pool::DiskPool;
use parking_lot::Mutex;

use crate::{
    image::{ParkOutcome, ParkableImage, ParkableImageStorageState, WeakParkableImage},
    policy::ParkableImagePolicy,
};

/// Browser-runtime scoped owner and diagnostics registry for parkable images.
#[derive(Clone)]
pub struct ParkableImageManager {
    inner: Arc<ParkableImageManagerInner>,
}

struct ParkableImageManagerInner {
    disk_pool: Option<DiskPool>,
    policy: ParkableImagePolicy,
    images: Mutex<Vec<WeakParkableImage>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParkableImageManagerDiagnostics {
    pub image_count: usize,
    pub mutable_count: usize,
    pub resident_count: usize,
    pub parking_count: usize,
    pub resident_with_disk_backup_count: usize,
    pub parked_count: usize,
    pub retained_memory_bytes: usize,
    pub retained_disk_bytes: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParkableImageSweepReport {
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
        Self {
            inner: Arc::new(ParkableImageManagerInner {
                disk_pool,
                policy,
                images: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn policy(&self) -> ParkableImagePolicy {
        self.inner.policy
    }

    pub fn create(&self, initial_capacity: usize) -> ParkableImage {
        let image = ParkableImage::new_mutable(self.clone(), initial_capacity);
        self.register(&image);
        image
    }

    pub fn from_frozen_bytes(&self, bytes: Vec<u8>) -> ParkableImage {
        let image = ParkableImage::new_frozen(self.clone(), bytes);
        self.register(&image);
        image
    }

    /// Attempts to park all currently live images.
    ///
    /// Disk I/O is synchronous. Callers that sweep many images should invoke
    /// this from a blocking worker, as Blink does for its writes.
    pub fn maybe_park_images(&self) -> ParkableImageSweepReport {
        let images = self.live_images();
        let mut report = ParkableImageSweepReport::default();
        for image in images {
            report.considered += 1;
            match image.maybe_park() {
                Ok(ParkOutcome::Parked) => report.parked += 1,
                Ok(ParkOutcome::AlreadyParked) => report.already_parked += 1,
                Ok(ParkOutcome::Delayed { .. }) => report.delayed += 1,
                Ok(ParkOutcome::InUse) => report.in_use += 1,
                Ok(ParkOutcome::Unavailable) => report.unavailable += 1,
                Ok(ParkOutcome::NotFrozen | ParkOutcome::BelowMinimum) => {
                    report.ineligible += 1;
                }
                Err(_) => report.write_failures += 1,
            }
        }
        report
    }

    pub fn diagnostics(&self) -> ParkableImageManagerDiagnostics {
        let mut diagnostics = ParkableImageManagerDiagnostics::default();
        for image in self.live_images() {
            let image = image.diagnostics();
            diagnostics.image_count += 1;
            match image.storage {
                ParkableImageStorageState::Mutable => diagnostics.mutable_count += 1,
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

    fn register(&self, image: &ParkableImage) {
        self.inner.images.lock().push(image.downgrade());
    }

    fn live_images(&self) -> Vec<ParkableImage> {
        let mut registered = self.inner.images.lock();
        let mut images = Vec::with_capacity(registered.len());
        registered.retain(|image| {
            if let Some(image) = image.upgrade() {
                images.push(image);
                true
            } else {
                false
            }
        });
        images
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
