//! Bounded raster buffers, image codecs, and immutable SVG image trees.
//!
//! This crate deliberately does not know about DOM, layout, paint snapshots,
//! or protocol commands. It owns metadata-only probes and bounded decode/parse
//! contracts; renderer resource owners decide when and how long results live.

mod bounded;
mod budget;
mod jpeg;
mod png;
mod raster;
mod rgba;
mod svg;

pub use budget::{MAX_DECODED_RGBA_BYTES, MAX_ENCODED_IMAGE_BYTES};
pub use jpeg::{EncodedJpeg, JpegDecodeError, JpegEncodeError, decode_jpeg, encode_jpeg};
pub use png::{
    EncodedPng, PngDecodeError, PngEncodeError, PngEncodeOptions, decode_png, encode_png,
    encode_png_rgba8, encode_png_rgba8_with_options, encode_png_with_options,
};
pub use raster::{
    DecodedRasterImage, RasterDecodeError, RasterImageFormat, RasterImageMetadata,
    decode_raster_image, decode_raster_image_with_metadata, probe_raster_image,
    raster_image_dimensions,
};
pub use rgba::{RgbaImage, RgbaImageError};
pub use svg::{
    MAX_ENCODED_SVG_BYTES, MAX_SVG_PAINT_WORK_UNITS, SvgDecodeError, SvgImage, SvgImageMetadata,
    SvgTextResource, SvgTextSourceRange, decode_svg_image, decode_svg_image_with_metadata,
    decode_svg_image_with_source_elements, probe_svg_image,
    svg_image_metadata_from_root_attributes,
};
