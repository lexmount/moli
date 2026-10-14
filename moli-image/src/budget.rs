/// Maximum encoded image payload retained for protocol or resource handoff.
pub const MAX_ENCODED_IMAGE_BYTES: usize = 128 * 1024 * 1024;

/// Maximum materialized straight-RGBA8 buffer owned by one codec operation.
///
/// Paint independently bounds transient raster allocations. This codec limit
/// also protects network image decode.
pub const MAX_DECODED_RGBA_BYTES: usize = 128 * 1024 * 1024;

/// Maximum retained layout surface, full precision colors and unconverted source
/// samples for one image. Renderer owners also impose an aggregate image budget.
pub(crate) const MAX_DECODED_COLOR_BYTES: usize = 256 * 1024 * 1024;
