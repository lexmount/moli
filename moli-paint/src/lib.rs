//! PDF output and CPU rasterization for owned Moli paint snapshots.
//!
//! The implementation is intentionally a narrow adaptation of the AnyRender
//! screenshot path used by Blitz at commit
//! `d788124ab881f9bb537cb452ec1d837604a374a8`. The referenced Blitz and
//! AnyRender code is licensed under MIT OR Apache-2.0. No Blitz DOM or retained
//! layout state is copied into this crate.

mod error;
mod geometry;
pub mod pdf;
mod raster;
mod scene;
mod validation;

pub use error::PaintError;
pub use moli_image::RgbaImage as RasterImage;
pub use raster::{
    MAX_TRANSIENT_RASTER_BYTES, RasterBufferLayout, RasterPixelFormat, raster_buffer_layout,
    raster_snapshot, raster_snapshot_into,
};
#[cfg(feature = "molio")]
pub use raster::{raster_damage_rows, raster_snapshot_rows_into};
