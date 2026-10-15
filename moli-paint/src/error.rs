// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Narrowly adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8:
// - examples/screenshot.rs
// - packages/blitz-paint/src/render.rs
// - crates/anyrender_svg (parsed SVG tree execution)
// The source project is licensed MIT OR Apache-2.0.

//! Shared errors for paint validation and rendering.

use std::{error::Error, fmt};

/// Failure while validating or rendering a paint snapshot.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintError {
    /// A native SVG image mapping contains a non-finite coefficient.
    InvalidSvgImageTransform { transform: [f64; 6] },
    /// At least one capture-surface dimension was non-finite or not positive.
    InvalidCaptureSurfaceDimensions {
        /// Width in CSS pixels.
        width: f32,
        /// Height in CSS pixels.
        height: f32,
    },
    /// Capture device scale was non-finite or not positive.
    InvalidCaptureDeviceScale {
        /// Rejected device scale.
        device_scale: f32,
    },
    /// A device-pixel dimension exceeds what the CPU backend represents.
    BackendDimensionLimit {
        /// Derived width in device pixels, before conversion to `u32`.
        width: f64,
        /// Derived height in device pixels, before conversion to `u32`.
        height: f64,
        /// Largest dimension accepted by the backend.
        max_dimension: u32,
    },
    /// Width × height × four bytes overflowed the host address space.
    BufferLengthOverflow {
        /// Width in device pixels.
        width: u32,
        /// Height in device pixels.
        height: u32,
    },
    /// Nested filters would exceed the transient RGBA budget.
    TransientRasterBudgetExceeded {
        required_bytes: usize,
        max_bytes: usize,
    },
    /// The snapshot contains too many backend-neutral paint work units.
    PaintOperationBudgetExceeded {
        operations: usize,
        max_operations: usize,
    },
    /// Exact clipping arithmetic would exceed its coordinate precision budget.
    CanvasClipCoordinateBudgetExceeded { bits: u64, max_bits: u64 },
    /// The snapshot contains too many positioned glyphs.
    PaintGlyphBudgetExceeded { glyphs: usize, max_glyphs: usize },
    /// The snapshot nests more clip/effect scopes than the backend boundary accepts.
    PaintLayerDepthBudgetExceeded { depth: usize, max_depth: usize },
    /// The backend returned a buffer with an unexpected length.
    UnexpectedBufferLength {
        /// Checked expected byte length.
        expected: usize,
        /// Actual byte length.
        actual: usize,
    },
    /// A layer pop appeared without a matching push.
    LayerStackUnderflow {
        /// Fragment index containing the unmatched pop.
        fragment_index: usize,
    },
    /// The snapshot ended while one or more layer scopes remained open.
    UnclosedLayerStack {
        /// Number of unmatched layer pushes.
        depth: usize,
    },
    /// A glyph run references a font outside the snapshot resource table.
    MissingFontResource {
        /// Invalid snapshot-local font index.
        index: usize,
    },
    /// An image command references an image outside the snapshot resource table.
    MissingImageResource {
        /// Invalid snapshot-local image index.
        index: usize,
    },
    /// An SVG command references an image outside the snapshot resource table.
    MissingSvgImageResource {
        /// Invalid snapshot-local SVG resource index.
        index: usize,
    },
}

impl fmt::Display for PaintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSvgImageTransform { transform } => write!(formatter, "SVG image mapping must be finite: {transform:?}"),
            Self::InvalidCaptureSurfaceDimensions { width, height } => {
                write!(
                    formatter,
                    "capture surface dimensions must be finite and positive, got {width}x{height}"
                )
            }
            Self::InvalidCaptureDeviceScale { device_scale } => write!(
                formatter,
                "capture device scale must be finite and positive, got {device_scale}"
            ),
            Self::BackendDimensionLimit {
                width,
                height,
                max_dimension,
            } => write!(
                formatter,
                "raster dimensions {width}x{height} exceed backend limit {max_dimension}"
            ),
            Self::BufferLengthOverflow { width, height } => write!(
                formatter,
                "RGBA byte length overflows the host address space for {width}x{height}"
            ),
            Self::TransientRasterBudgetExceeded {
                required_bytes,
                max_bytes,
            } => write!(
                formatter,
                "raster and nested filter surfaces conservatively account for {required_bytes} RGBA bytes, exceeding the {max_bytes}-byte transient budget"
            ),
            Self::PaintOperationBudgetExceeded {
                operations,
                max_operations,
            } => write!(
                formatter,
                "paint snapshot contains {operations} work units, exceeding the {max_operations}-unit budget"
            ),
            Self::CanvasClipCoordinateBudgetExceeded { bits, max_bits } => write!(
                formatter,
                "Canvas clipping requires {bits}-bit coordinates, exceeding the {max_bits}-bit budget"
            ),
            Self::PaintGlyphBudgetExceeded { glyphs, max_glyphs } => write!(
                formatter,
                "paint snapshot contains {glyphs} positioned glyphs, exceeding the {max_glyphs}-glyph budget"
            ),
            Self::PaintLayerDepthBudgetExceeded { depth, max_depth } => write!(
                formatter,
                "paint snapshot nests {depth} layer scopes, exceeding the {max_depth}-scope budget"
            ),
            Self::UnexpectedBufferLength { expected, actual } => write!(
                formatter,
                "raster backend returned {actual} bytes, expected {expected}"
            ),
            Self::LayerStackUnderflow { fragment_index } => write!(
                formatter,
                "paint command {fragment_index} pops an empty layer stack"
            ),
            Self::UnclosedLayerStack { depth } => {
                write!(formatter, "paint snapshot leaves {depth} layer scopes open")
            }
            Self::MissingFontResource { index } => {
                write!(
                    formatter,
                    "glyph run references missing font resource {index}"
                )
            }
            Self::MissingImageResource { index } => {
                write!(
                    formatter,
                    "image command references missing image resource {index}"
                )
            }
            Self::MissingSvgImageResource { index } => {
                write!(
                    formatter,
                    "SVG command references missing image resource {index}"
                )
            }
        }
    }
}

impl Error for PaintError {}
