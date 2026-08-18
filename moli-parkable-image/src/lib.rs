//! Parkable encoded image storage.
//!
//! Completed encoded images are decoded through read-only snapshots and may be
//! parked into one `DiskPool` extent. Reading a parked image unparks it
//! synchronously; the retained disk extent lets a later park discard memory
//! without another write. The manager keeps resident and parked registries
//! separate and exposes the next resident parking deadline to its runtime.

mod image;
mod manager;
mod policy;
mod registry;

pub use image::{ParkableImage, ParkableImageSnapshot};
pub use manager::{ParkableImageManager, ParkableImageManagerDiagnostics};
pub use policy::ParkableImagePolicy;

#[cfg(test)]
mod tests;
