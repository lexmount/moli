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

#[cfg(feature = "molio")]
use crate::geometry::to_backend_rect;
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
#[cfg(feature = "molio")]
use std::ops::Range;
use std::sync::Arc;

/// Maximum estimated RGBA bytes simultaneously owned by the output and nested
/// filter surfaces in one raster call.
pub const MAX_TRANSIENT_RASTER_BYTES: usize = 256 * 1024 * 1024;

/// The device-pixel dimensions and storage size of a tightly packed 32-bit target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterBufferLayout {
    pub width: u32,
    pub height: u32,
    pub byte_len: usize,
}

/// The channel order of a caller's straight-alpha pixel buffer.
#[derive(Clone, Copy)]
pub enum RasterPixelFormat {
    Rgba8,
    Bgra8,
}

/// Rasterizes an owned snapshot to row-major, straight-alpha RGBA8 pixels with Vello CPU.
pub fn raster_snapshot(snapshot: &PaintSnapshot) -> Result<RasterImage, PaintError> {
    let dimensions = validated_snapshot_layout(snapshot)?;
    let mut rgba = vec![0; dimensions.byte_len];
    render_snapshot(snapshot, dimensions, &mut rgba);
    unpremultiply::<false>(&mut rgba);
    Ok(RasterImage {
        width: dimensions.width,
        height: dimensions.height,
        rgba,
    })
}

/// Computes the required device-pixel buffer layout before storage is allocated.
pub fn raster_buffer_layout(
    css_width: f32,
    css_height: f32,
    device_scale: f32,
) -> Result<RasterBufferLayout, PaintError> {
    let dimensions = validate_dimensions(PaintCaptureSurface::new(
        css_width,
        css_height,
        device_scale,
    ))?;
    validate_resource_budget(dimensions, 0)?;
    Ok(dimensions)
}

/// Paints directly into the caller's tightly packed pixel buffer.
/// The slice must contain exactly the number of bytes required by the snapshot.
pub fn raster_snapshot_into(
    snapshot: &PaintSnapshot,
    pixels: &mut [u8],
    format: RasterPixelFormat,
) -> Result<RasterBufferLayout, PaintError> {
    let dimensions = validated_snapshot_layout(snapshot)?;
    if pixels.len() != dimensions.byte_len {
        return Err(PaintError::UnexpectedBufferLength {
            expected: dimensions.byte_len,
            actual: pixels.len(),
        });
    }
    render_snapshot(snapshot, dimensions, pixels);
    convert_raster_output(pixels, format);
    Ok(dimensions)
}

/// Finds the affected device-pixel rows when a snapshot only changes raster
/// image contents. Anything that may change other paint or sample outside the
/// damaged rows conservatively invalidates the viewport.
#[cfg(feature = "molio")]
pub fn raster_damage_rows(
    previous: &PaintSnapshot,
    current: &PaintSnapshot,
    layout: RasterBufferLayout,
) -> Range<u32> {
    let full = 0..layout.height;
    if previous.viewport != current.viewport
        || previous.surface != current.surface
        || previous.raster_crop.is_some()
        || current.raster_crop.is_some()
        || previous.canvas_color != current.canvas_color
        || previous.fragments != current.fragments
        || previous.fonts != current.fonts
        || previous.svg_images != current.svg_images
        || previous.images.len() != current.images.len()
    {
        return full;
    }
    if previous
        .images
        .iter()
        .zip(&current.images)
        .all(|(old, new)| Arc::ptr_eq(&old.image, &new.image))
    {
        return 0..0;
    }
    let scale = f64::from(current.surface.device_scale);
    let grid = current.pixel_grid();
    let mut top = layout.height;
    let mut bottom = 0;
    for fragment in &current.fragments {
        let (id, rect, transform) = match fragment {
            PaintFragment::PushLayer {
                filter: Some(_), ..
            } => return full,
            PaintFragment::Image(image) => (
                image.image,
                grid.snap_rect(image.destination).and_then(to_backend_rect),
                image.transform,
            ),
            PaintFragment::Tiles(tiles) => match tiles.source {
                moli_layout::PaintTileSource::Image(id, _) => (
                    id,
                    crate::scene::tiles::grid_coverage(tiles.grid),
                    tiles.transform,
                ),
                _ => continue,
            },
            _ => continue,
        };
        let Some((old, new)) = previous.image(id).zip(current.image(id)) else {
            return full;
        };
        if Arc::ptr_eq(&old.image, &new.image) {
            continue;
        }
        let Some(rect) = rect else { return full };
        let bounds = device_transform(transform, scale).transform_rect_bbox(rect);
        if !bounds.is_finite() {
            return full;
        }
        // Widen to include the antialiasing fringe at transformed edges.
        if bounds.x1 > -2.0 && bounds.x0 < f64::from(layout.width) + 2.0 {
            let first = (bounds.y0.floor() - 2.0) as u32;
            let end = ((bounds.y1.ceil() + 2.0) as u32).min(layout.height);
            if first < end {
                top = top.min(first);
                bottom = bottom.max(end);
            }
        }
    }
    if top < bottom { top..bottom } else { 0..0 }
}

/// Repaints full-width rows of an existing viewport buffer, rounded out to
/// backend tile rows. Other rows retain the pixels established by damage
/// tracking. Filter-bearing snapshots use the regular full-surface path.
#[cfg(feature = "molio")]
pub fn raster_snapshot_rows_into(
    snapshot: &PaintSnapshot,
    pixels: &mut [u8],
    format: RasterPixelFormat,
    rows: Range<u32>,
) -> Result<RasterBufferLayout, PaintError> {
    let layout = validated_snapshot_layout(snapshot)?;
    if pixels.len() != layout.byte_len {
        return Err(PaintError::UnexpectedBufferLength {
            expected: layout.byte_len,
            actual: pixels.len(),
        });
    }
    let start = rows.start.min(layout.height);
    let end = rows.end.min(layout.height);
    if start >= end {
        return Ok(layout);
    }
    // Vello CPU rasterizes 4-pixel-high tile rows. Keep the original device
    // alignment so edges take the same coverage path as a full viewport.
    const TILE_HEIGHT: u32 = 4;
    let start = start / TILE_HEIGHT * TILE_HEIGHT;
    let end = (end.div_ceil(TILE_HEIGHT) * TILE_HEIGHT).min(layout.height);
    if (start == 0 && end == layout.height)
        || snapshot.fragments.iter().any(|fragment| {
            matches!(
                fragment,
                PaintFragment::PushLayer {
                    filter: Some(_),
                    ..
                }
            )
        })
    {
        render_snapshot(snapshot, layout, pixels);
        convert_raster_output(pixels, format);
        return Ok(layout);
    }
    let row_bytes = layout.width as usize * 4;
    let target = &mut pixels[start as usize * row_bytes..end as usize * row_bytes];
    let region = RasterBufferLayout {
        width: layout.width,
        height: end - start,
        byte_len: target.len(),
    };
    let mut recording = anyrender::Scene::new();
    paint_fragment_stream(
        &mut recording,
        snapshot,
        &snapshot.fragments,
        f64::from(snapshot.surface.device_scale),
        layout,
        Rect::new(
            0.0,
            f64::from(start),
            f64::from(layout.width),
            f64::from(end),
        ),
    );
    let mut renderer = VelloCpuImageRenderer::new(region.width, region.height);
    renderer.render(
        |scene| {
            paint_canvas(scene, snapshot.canvas_color, region);
            scene.append_scene(recording, Affine::translate((0.0, -f64::from(start))));
        },
        target,
    );
    convert_raster_output(target, format);
    Ok(layout)
}

fn convert_raster_output(pixels: &mut [u8], format: RasterPixelFormat) {
    match format {
        RasterPixelFormat::Rgba8 => unpremultiply::<false>(pixels),
        RasterPixelFormat::Bgra8 => unpremultiply::<true>(pixels),
    }
}

fn validated_snapshot_layout(snapshot: &PaintSnapshot) -> Result<RasterBufferLayout, PaintError> {
    let dimensions = validate_dimensions(snapshot.surface)?;
    validate_resource_budget(dimensions, validate_fragment_stream(snapshot)?)?;
    Ok(dimensions)
}

/// Paints premultiplied RGBA into the supplied pixel buffer.
fn render_snapshot(snapshot: &PaintSnapshot, dimensions: RasterBufferLayout, pixels: &mut [u8]) {
    let scale = f64::from(snapshot.surface.device_scale);

    let mut renderer = VelloCpuImageRenderer::new(dimensions.width, dimensions.height);
    renderer.render(
        |scene| {
            paint_canvas(scene, snapshot.canvas_color, dimensions);
            paint_fragment_stream(
                scene,
                snapshot,
                &snapshot.fragments,
                scale,
                dimensions,
                Rect::new(
                    0.0,
                    0.0,
                    f64::from(dimensions.width),
                    f64::from(dimensions.height),
                ),
            );
        },
        pixels,
    );
}

// Vello's Pixmap stores premultiplied channels; RasterImage and our software
// color filters consume straight-alpha RGBA. Convert at each backend readback.
fn unpremultiply<const BGRA: bool>(rgba: &mut [u8]) {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha == 0 {
            pixel[..3].fill(0);
        } else if alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
        if BGRA {
            pixel.swap(0, 2);
        }
    }
}

fn validate_resource_budget(
    dimensions: RasterBufferLayout,
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

fn validate_dimensions(surface: PaintCaptureSurface) -> Result<RasterBufferLayout, PaintError> {
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

    Ok(RasterBufferLayout {
        width,
        height,
        byte_len,
    })
}

fn paint_canvas(
    scene: &mut impl PaintScene,
    canvas_color: PaintColor,
    dimensions: RasterBufferLayout,
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
    dimensions: RasterBufferLayout,
    output: Rect,
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
            paint_fragment(scene, snapshot, fragment, scale, output);
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
                    output,
                );
            },
            &mut filtered_rgba,
        );
        unpremultiply::<false>(&mut filtered_rgba);
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
