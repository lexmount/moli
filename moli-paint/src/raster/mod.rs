// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Narrowly adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8:
// - examples/screenshot.rs
// - packages/blitz-paint/src/render.rs
// - crates/anyrender_svg (parsed SVG tree execution)
// The source project is licensed MIT OR Apache-2.0.

//! Vello CPU pixel rendering, RGBA readback and bounded software-filter surfaces.

mod filters;
#[cfg(test)]
mod tests;

use crate::geometry::{device_transform, to_backend_shape};
use crate::scene::{paint_fragment, sanitize_unit, to_backend_blend_mode, to_backend_color};
use crate::validation::validate_fragment_stream;
use crate::{PaintError, RasterImage};
use anyrender::ImageRenderer;
use anyrender::PaintScene;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use filters::{apply_software_color_filter, is_software_color_filter};
use moli_layout::{PaintCaptureSurface, PaintColor, PaintFragment, PaintSnapshot};
use peniko::kurbo::{Affine, Rect};
use peniko::{
    Blob, Extend, Fill, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality,
    ImageSampler,
};
use std::sync::Arc;

/// Maximum estimated RGBA bytes simultaneously owned by the output and nested
/// filter surfaces in one raster call.
pub const MAX_TRANSIENT_RASTER_BYTES: usize = 256 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RasterDimensions {
    width: u32,
    height: u32,
    byte_len: usize,
}

/// Rasterizes an owned snapshot to row-major, straight-alpha RGBA8 pixels with Vello CPU.
pub fn raster_snapshot(snapshot: &PaintSnapshot) -> Result<RasterImage, PaintError> {
    let dimensions = validate_dimensions(snapshot.surface)?;
    let max_filter_depth = validate_fragment_stream(snapshot)?;
    validate_resource_budget(dimensions, max_filter_depth)?;
    let scale = f64::from(snapshot.surface.device_scale);

    let mut renderer = VelloCpuImageRenderer::new(dimensions.width, dimensions.height);
    let mut rgba = Vec::new();
    renderer.render_to_vec(
        |scene| {
            paint_canvas(scene, snapshot.canvas_color, dimensions);
            paint_fragment_stream(scene, snapshot, &snapshot.fragments, scale, dimensions);
        },
        &mut rgba,
    );

    if rgba.len() != dimensions.byte_len {
        return Err(PaintError::UnexpectedBufferLength {
            expected: dimensions.byte_len,
            actual: rgba.len(),
        });
    }
    unpremultiply_rgba(&mut rgba);

    Ok(RasterImage {
        width: dimensions.width,
        height: dimensions.height,
        rgba,
    })
}

// Vello's Pixmap stores premultiplied channels; RasterImage and our software
// color filters consume straight-alpha RGBA. Convert at each backend readback.
fn unpremultiply_rgba(rgba: &mut [u8]) {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha == 0 {
            pixel[..3].fill(0);
        } else if alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}

fn validate_resource_budget(
    dimensions: RasterDimensions,
    max_filter_depth: usize,
) -> Result<(), PaintError> {
    // Backend filters may crop allocations to content bounds, whereas the
    // software color-filter fallback uses a complete surface. Charging every
    // nested filter as a complete surface is conservative and backend-neutral.
    let surface_count = max_filter_depth.saturating_add(1);
    let required_bytes = dimensions.byte_len.saturating_mul(surface_count);
    if required_bytes > MAX_TRANSIENT_RASTER_BYTES {
        return Err(PaintError::TransientRasterBudgetExceeded {
            required_bytes,
            max_bytes: MAX_TRANSIENT_RASTER_BYTES,
        });
    }
    Ok(())
}

fn validate_dimensions(surface: PaintCaptureSurface) -> Result<RasterDimensions, PaintError> {
    if !surface.css_width.is_finite()
        || !surface.css_height.is_finite()
        || surface.css_width <= 0.0
        || surface.css_height <= 0.0
    {
        return Err(PaintError::InvalidCaptureSurfaceDimensions {
            width: surface.css_width,
            height: surface.css_height,
        });
    }

    let scale = surface.device_scale;
    if !scale.is_finite() || scale <= 0.0 {
        return Err(PaintError::InvalidCaptureDeviceScale {
            device_scale: scale,
        });
    }

    let width = (f64::from(surface.css_width) * f64::from(scale)).ceil();
    let height = (f64::from(surface.css_height) * f64::from(scale)).ceil();
    const MAX_DIMENSION: u32 = u16::MAX as u32;
    if width > f64::from(MAX_DIMENSION) || height > f64::from(MAX_DIMENSION) {
        return Err(PaintError::BackendDimensionLimit {
            width,
            height,
            max_dimension: MAX_DIMENSION,
        });
    }

    let width = width as u32;
    let height = height as u32;
    let byte_len = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(PaintError::BufferLengthOverflow { width, height })?;

    Ok(RasterDimensions {
        width,
        height,
        byte_len,
    })
}

fn paint_canvas(
    scene: &mut impl PaintScene,
    canvas_color: PaintColor,
    dimensions: RasterDimensions,
) {
    let canvas = Rect::new(
        0.0,
        0.0,
        f64::from(dimensions.width),
        f64::from(dimensions.height),
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        to_backend_color(canvas_color),
        None,
        &canvas,
    );
}

fn paint_fragment_stream(
    scene: &mut impl PaintScene,
    snapshot: &PaintSnapshot,
    fragments: &[PaintFragment],
    scale: f64,
    dimensions: RasterDimensions,
) {
    let mut index = 0usize;
    while index < fragments.len() {
        let fragment = &fragments[index];
        let isolated_filter = match fragment {
            PaintFragment::PushLayer {
                filter: Some(filter),
                ..
            } if is_software_color_filter(filter) => Some(filter),
            _ => None,
        };
        let Some(filter) = isolated_filter else {
            paint_fragment(scene, snapshot, fragment, scale);
            index += 1;
            continue;
        };

        let end = matching_layer_pop(fragments, index);
        let PaintFragment::PushLayer {
            opacity,
            blend_mode,
            composite,
            clip,
            transform,
            ..
        } = fragment
        else {
            unreachable!("isolated filters are represented by PushLayer")
        };

        // Resolve a color-filter subtree into a temporary transparent image,
        // transform its pixels once, then composite it with the layer's normal
        // clip/opacity/blend contract. This surface belongs only to this raster
        // call and is never cached by screenshot or screencast.
        let mut filtered_rgba = Vec::new();
        let mut renderer = VelloCpuImageRenderer::new(dimensions.width, dimensions.height);
        renderer.render_to_vec(
            |filtered_scene| {
                paint_fragment_stream(
                    filtered_scene,
                    snapshot,
                    &fragments[index + 1..end],
                    scale,
                    dimensions,
                );
            },
            &mut filtered_rgba,
        );
        unpremultiply_rgba(&mut filtered_rgba);
        apply_software_color_filter(&mut filtered_rgba, filter);

        let clip = to_backend_shape(clip);
        scene.push_layer(
            to_backend_blend_mode(*blend_mode, *composite),
            sanitize_unit(*opacity),
            device_transform(*transform, scale),
            &clip,
            None,
            None,
        );
        let image = ImageBrush {
            image: ImageData {
                data: Blob::new(Arc::new(filtered_rgba)),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::Alpha,
                width: dimensions.width,
                height: dimensions.height,
            },
            sampler: ImageSampler {
                x_extend: Extend::Pad,
                y_extend: Extend::Pad,
                quality: ImageQuality::Low,
                alpha: 1.0,
            },
        };
        scene.draw_image(image.as_ref(), Affine::IDENTITY);
        scene.pop_layer();
        index = end + 1;
    }
}

fn matching_layer_pop(fragments: &[PaintFragment], push_index: usize) -> usize {
    let mut depth = 0usize;
    for (index, fragment) in fragments.iter().enumerate().skip(push_index) {
        match fragment {
            PaintFragment::PushLayer { .. } | PaintFragment::PushClip { .. } => depth += 1,
            PaintFragment::PopLayer => {
                depth -= 1;
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    unreachable!("paint stream validation rejects unclosed layers")
}
