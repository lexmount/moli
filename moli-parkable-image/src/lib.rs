//! Parkable encoded image storage.
//!
//! This follows Blink's `ParkableImage` boundary: callers append encoded bytes,
//! freeze the completed image, decode through read-only snapshots, and park an
//! eligible image into one `DiskPool` extent. Reading a parked image unparks it
//! synchronously; the retained disk extent lets a later park discard memory
//! without another write. The manager keeps resident and parked registries
//! separate and exposes the next resident parking deadline to its runtime.

mod image;
mod manager;
mod policy;
mod registry;

pub use image::{
    ParkOutcome, ParkableImage, ParkableImageDiagnostics, ParkableImageMutationError,
    ParkableImageSnapshot, ParkableImageStorageState, WeakParkableImage,
};
pub use manager::{
    ParkableImageManager, ParkableImageManagerDiagnostics, ParkableImageSweepReport,
};
pub use policy::{DEFAULT_MIN_SIZE_TO_PARK, DEFAULT_PARKING_DELAY, ParkableImagePolicy};

#[cfg(test)]
mod tests;
