// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Narrowly adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8:
// - examples/screenshot.rs
// - packages/blitz-paint/src/render.rs
// - crates/anyrender_svg (parsed SVG tree execution)
// The source project is licensed MIT OR Apache-2.0.

use std::{error::Error, fmt, sync::Arc};

mod filters;

use crate::RasterImage;
use anyrender::{Filter, ImageRenderer, PaintScene, filters::FilterEffect};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use filters::{apply_software_color_filter, backend_filter, is_software_color_filter};
use moli_layout::{
    PaintBlendMode, PaintBorderColors, PaintBorderStyle, PaintBorderStyles, PaintBoxShadow,
    PaintBrush, PaintCaptureSurface, PaintColor, PaintCompositeMode, PaintCornerRadii,
    PaintCornerRadius, PaintEdgeSizes, PaintFragment, PaintGradientColorSpace, PaintGradientExtend,
    PaintGradientHueDirection, PaintImage, PaintImageSampling, PaintLineCap, PaintLineJoin,
    PaintPath, PaintPathElement, PaintRect, PaintShape, PaintSnapshot, PaintStroke, PaintSvgImage,
    PaintTextDecoration, PaintTextDecorationStyle, PaintTextShadow, PaintTransform2D,
    pixel_snap_paint_axis, pixel_snap_paint_axis_allowing_zero, pixel_snap_paint_rect,
};
use peniko::{
    BlendMode, Blob, Color, Compose, Extend, Fill, Gradient, ImageAlphaType, ImageBrush, ImageData,
    ImageFormat, ImageQuality, ImageSampler, Mix,
    color::{ColorSpaceTag, HueDirection},
    kurbo::{
        Affine, BezPath, Cap, Circle, Ellipse, Insets, Join, PathEl, Point, Rect, RoundedRect,
        RoundedRectRadii, Shape as _, Stroke, Vec2,
    },
};

/// Maximum number of pixels admitted to one screenshot/screencast raster.
pub const MAX_RASTER_PIXELS: u64 = 16 * 1024 * 1024;
/// Maximum estimated RGBA bytes simultaneously owned by the output and nested
/// filter surfaces in one raster call.
pub const MAX_TRANSIENT_RASTER_BYTES: usize = 256 * 1024 * 1024;
/// Maximum backend-neutral paint work units admitted to one raster call.
///
/// One fragment costs one unit; nested path elements, gradient stops, dash
/// entries, and font variation coordinates are charged separately.
pub const MAX_PAINT_OPERATIONS: usize = 1_000_000;
/// Maximum positioned glyphs admitted to one raster call.
pub const MAX_PAINT_GLYPHS: usize = 4_000_000;
/// Maximum nested clip/effect scopes admitted to one raster call.
pub const MAX_PAINT_LAYER_DEPTH: usize = 4_096;

/// Failure while validating or rasterizing a paint snapshot.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintError {
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
    /// The requested surface exceeds the bounded product pixel budget.
    PixelBudgetExceeded {
        width: u32,
        height: u32,
        pixels: u64,
        max_pixels: u64,
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
            Self::PixelBudgetExceeded {
                width,
                height,
                pixels,
                max_pixels,
            } => write!(
                formatter,
                "raster surface {width}x{height} contains {pixels} pixels, exceeding the {max_pixels}-pixel capture budget"
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RasterDimensions {
    width: u32,
    height: u32,
    byte_len: usize,
}

/// Rasterizes an owned snapshot to row-major RGBA8 pixels with Vello CPU.
pub fn raster_snapshot(snapshot: &PaintSnapshot) -> Result<RasterImage, PaintError> {
    let dimensions = validate_dimensions(snapshot.surface)?;
    let stream = validate_fragment_stream(snapshot)?;
    validate_resource_budget(dimensions, stream)?;
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

    Ok(RasterImage {
        width: dimensions.width,
        height: dimensions.height,
        rgba,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PaintStreamMetrics {
    max_filter_depth: usize,
    glyph_count: usize,
    operation_count: usize,
}

fn validate_fragment_stream(snapshot: &PaintSnapshot) -> Result<PaintStreamMetrics, PaintError> {
    let mut layers = Vec::new();
    let mut filter_depth = 0usize;
    let mut metrics = PaintStreamMetrics {
        operation_count: snapshot.fragments.len(),
        ..Default::default()
    };
    for (fragment_index, fragment) in snapshot.fragments.iter().enumerate() {
        metrics.operation_count = metrics
            .operation_count
            .saturating_add(fragment_auxiliary_units(snapshot, fragment));
        validate_paint_operation_budget(metrics.operation_count)?;
        match fragment {
            PaintFragment::PushLayer { filter, .. } => {
                let filtered = filter.is_some();
                layers.push(filtered);
                if layers.len() > MAX_PAINT_LAYER_DEPTH {
                    return Err(PaintError::PaintLayerDepthBudgetExceeded {
                        depth: layers.len(),
                        max_depth: MAX_PAINT_LAYER_DEPTH,
                    });
                }
                if filtered {
                    filter_depth = filter_depth.saturating_add(1);
                    metrics.max_filter_depth = metrics.max_filter_depth.max(filter_depth);
                }
            }
            PaintFragment::PushClip { .. } => {
                layers.push(false);
                if layers.len() > MAX_PAINT_LAYER_DEPTH {
                    return Err(PaintError::PaintLayerDepthBudgetExceeded {
                        depth: layers.len(),
                        max_depth: MAX_PAINT_LAYER_DEPTH,
                    });
                }
            }
            PaintFragment::PopLayer => {
                let Some(filtered) = layers.pop() else {
                    return Err(PaintError::LayerStackUnderflow { fragment_index });
                };
                if filtered {
                    filter_depth -= 1;
                }
            }
            PaintFragment::GlyphRun(run) if snapshot.font(run.font).is_none() => {
                return Err(PaintError::MissingFontResource {
                    index: run.font.index(),
                });
            }
            PaintFragment::TextShadow(shadow) if snapshot.font(shadow.run.font).is_none() => {
                return Err(PaintError::MissingFontResource {
                    index: shadow.run.font.index(),
                });
            }
            PaintFragment::Image(image) if snapshot.image(image.image).is_none() => {
                return Err(PaintError::MissingImageResource {
                    index: image.image.index(),
                });
            }
            PaintFragment::SvgImage(image) if snapshot.svg_image(image.image).is_none() => {
                return Err(PaintError::MissingSvgImageResource {
                    index: image.image.index(),
                });
            }
            PaintFragment::GlyphRun(run) => {
                metrics.glyph_count = metrics.glyph_count.saturating_add(run.glyphs.len());
            }
            PaintFragment::TextShadow(shadow) => {
                metrics.glyph_count = metrics.glyph_count.saturating_add(shadow.run.glyphs.len());
            }
            PaintFragment::Fill { .. }
            | PaintFragment::Stroke(_)
            | PaintFragment::Border { .. }
            | PaintFragment::BoxShadow(_)
            | PaintFragment::TextDecoration(_)
            | PaintFragment::Image(_)
            | PaintFragment::SvgImage(_) => {}
        }
    }
    if !layers.is_empty() {
        return Err(PaintError::UnclosedLayerStack {
            depth: layers.len(),
        });
    }
    if metrics.glyph_count > MAX_PAINT_GLYPHS {
        return Err(PaintError::PaintGlyphBudgetExceeded {
            glyphs: metrics.glyph_count,
            max_glyphs: MAX_PAINT_GLYPHS,
        });
    }
    Ok(metrics)
}

fn validate_paint_operation_budget(operations: usize) -> Result<(), PaintError> {
    if operations > MAX_PAINT_OPERATIONS {
        return Err(PaintError::PaintOperationBudgetExceeded {
            operations,
            max_operations: MAX_PAINT_OPERATIONS,
        });
    }
    Ok(())
}

fn fragment_auxiliary_units(snapshot: &PaintSnapshot, fragment: &PaintFragment) -> usize {
    match fragment {
        PaintFragment::PushLayer { clip, .. } => shape_units(clip),
        PaintFragment::PushClip { shape, .. } => shape_units(shape),
        PaintFragment::Fill { shape, brush, .. } => {
            shape_units(shape).saturating_add(brush_units(brush))
        }
        PaintFragment::Stroke(stroke) => stroke
            .path
            .elements
            .len()
            .saturating_add(stroke.dash_pattern.len()),
        PaintFragment::TextShadow(shadow) => shadow.run.normalized_coords.len(),
        PaintFragment::GlyphRun(run) => run.normalized_coords.len(),
        PaintFragment::SvgImage(image) => snapshot
            .svg_image(image.image)
            .map_or(0, |resource| resource.image.paint_work_units()),
        PaintFragment::PopLayer
        | PaintFragment::Border { .. }
        | PaintFragment::BoxShadow(_)
        | PaintFragment::TextDecoration(_)
        | PaintFragment::Image(_) => 0,
    }
}

fn shape_units(shape: &PaintShape) -> usize {
    match shape {
        PaintShape::Path(path) => path.elements.len(),
        PaintShape::Rect(_) | PaintShape::RoundedRect { .. } => 0,
    }
}

fn brush_units(brush: &PaintBrush) -> usize {
    match brush {
        PaintBrush::Solid(_) => 0,
        PaintBrush::LinearGradient(gradient) => gradient.stops.len(),
        PaintBrush::RadialGradient(gradient) => gradient.stops.len(),
        PaintBrush::ConicGradient(gradient) => gradient.stops.len(),
    }
}

fn validate_resource_budget(
    dimensions: RasterDimensions,
    stream: PaintStreamMetrics,
) -> Result<(), PaintError> {
    let pixels = u64::from(dimensions.width) * u64::from(dimensions.height);
    if pixels > MAX_RASTER_PIXELS {
        return Err(PaintError::PixelBudgetExceeded {
            width: dimensions.width,
            height: dimensions.height,
            pixels,
            max_pixels: MAX_RASTER_PIXELS,
        });
    }
    // Backend filters may crop allocations to content bounds, whereas the
    // software color-filter fallback uses a complete surface. Charging every
    // nested filter as a complete surface is conservative and backend-neutral.
    let surface_count = stream.max_filter_depth.saturating_add(1);
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

fn paint_fragment(
    scene: &mut impl PaintScene,
    snapshot: &PaintSnapshot,
    fragment: &PaintFragment,
    scale: f64,
) {
    match fragment {
        PaintFragment::PushLayer {
            opacity,
            blend_mode,
            composite,
            clip,
            transform,
            filter,
        } => {
            let clip = to_backend_shape(clip);
            scene.push_layer(
                to_backend_blend_mode(*blend_mode, *composite),
                sanitize_unit(*opacity),
                device_transform(*transform, scale),
                &clip,
                filter.as_ref().and_then(backend_filter).map(Arc::new),
                None,
            );
        }
        PaintFragment::PushClip { shape, transform } => {
            let shape = to_backend_shape(shape);
            scene.push_clip_layer(device_transform(*transform, scale), &shape);
        }
        PaintFragment::PopLayer => scene.pop_layer(),
        PaintFragment::Fill {
            shape,
            brush,
            transform,
        } => {
            let shape = to_backend_shape(shape);
            paint_fill(scene, &shape, brush, device_transform(*transform, scale));
        }
        PaintFragment::Stroke(stroke) => paint_stroke(scene, stroke, scale),
        PaintFragment::Border {
            rect,
            widths,
            colors,
            styles,
            radii,
            transform,
        } => paint_border(
            scene,
            *rect,
            *widths,
            *colors,
            *styles,
            *radii,
            device_transform(*transform, scale),
            scale,
        ),
        PaintFragment::BoxShadow(shadow) => paint_box_shadow(scene, shadow, scale),
        PaintFragment::TextDecoration(decoration) => {
            paint_text_decoration(scene, decoration, scale)
        }
        PaintFragment::TextShadow(shadow) => paint_text_shadow(scene, snapshot, shadow, scale),
        PaintFragment::GlyphRun(run) => paint_glyph_run(scene, snapshot, run, run.color, scale),
        PaintFragment::Image(image) => paint_image(scene, snapshot, image, scale),
        PaintFragment::SvgImage(image) => paint_svg_image(scene, snapshot, image, scale),
    }
}

fn paint_image(
    scene: &mut impl PaintScene,
    snapshot: &PaintSnapshot,
    image: &PaintImage,
    scale: f64,
) {
    let Some(resource) = snapshot.image(image.image) else {
        return;
    };
    let Some(destination) = pixel_snapped_image_destination(image.destination) else {
        return;
    };
    if resource.image.width == 0 || resource.image.height == 0 {
        return;
    }
    let quality = match image.sampling {
        PaintImageSampling::Nearest => ImageQuality::Low,
        PaintImageSampling::Linear => ImageQuality::Medium,
    };
    let brush = ImageBrush {
        image: ImageData {
            data: Blob::new(resource.image.clone()),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: resource.image.width,
            height: resource.image.height,
        },
        sampler: ImageSampler {
            x_extend: Extend::Pad,
            y_extend: Extend::Pad,
            quality,
            alpha: 1.0,
        },
    };
    let image_to_destination =
        Affine::translate((f64::from(destination.x), f64::from(destination.y)))
            * Affine::scale_non_uniform(
                f64::from(destination.width) / f64::from(resource.image.width),
                f64::from(destination.height) / f64::from(resource.image.height),
            );
    scene.draw_image(
        brush.as_ref(),
        device_transform(image.transform, scale) * image_to_destination,
    );
}

fn paint_svg_image(
    scene: &mut impl PaintScene,
    snapshot: &PaintSnapshot,
    image: &PaintSvgImage,
    scale: f64,
) {
    let Some(resource) = snapshot.svg_image(image.image) else {
        return;
    };
    let Some(destination) = pixel_snapped_image_destination(image.destination) else {
        return;
    };
    let tree_size = resource.image.tree().size();
    if tree_size.width() <= 0.0 || tree_size.height() <= 0.0 {
        return;
    }
    let image_to_destination =
        Affine::translate((f64::from(destination.x), f64::from(destination.y)))
            * Affine::scale_non_uniform(
                f64::from(destination.width) / f64::from(tree_size.width()),
                f64::from(destination.height) / f64::from(tree_size.height()),
            );
    let transform = device_transform(image.transform, scale) * image_to_destination;

    // Blitz's SVG executor reports unsupported nodes by painting red boxes.
    // A product screenshot must not invent visible fallback pixels, so retain
    // supported vector content and silently omit unsupported masks, filters,
    // patterns, or embedded raster nodes until their backends are wired.
    anyrender_svg::render_svg_tree_with(
        scene,
        resource.image.tree(),
        transform,
        &mut |_scene, _node| {},
    );
}

fn pixel_snapped_image_destination(destination: PaintRect) -> Option<PaintRect> {
    // Image geometry remains fractional for layout and CSSOM. Snapshot
    // projection has already applied ordinary paint offsets while preserving
    // CSS/scroll property transforms; rasterization only snaps the image quad
    // in that pre-transform paint space. Adjacent fractional images therefore
    // share one pixel boundary, matching Blink's
    // ToPixelSnappedRect/SnapSizeToPixel contract.
    pixel_snap_paint_rect(destination)
}

fn paint_border(
    scene: &mut impl PaintScene,
    rect: PaintRect,
    widths: PaintEdgeSizes,
    colors: PaintBorderColors,
    styles: PaintBorderStyles,
    radii: PaintCornerRadii,
    transform: Affine,
    _device_scale: f64,
) {
    let Some(rect) = sanitize_border_rect(rect) else {
        return;
    };
    let widths = sanitize_border_widths(widths, rect.width() as f32, rect.height() as f32);
    let Some((rect, widths)) = pixel_snapped_border_geometry(rect, widths) else {
        return;
    };
    let radii = normalize_radii(radii, rect.width(), rect.height());
    if paint_uniform_solid_border(scene, rect, widths, colors, styles, radii, transform) {
        return;
    }
    let edges = [
        (BorderEdge::Top, widths.top, colors.top, styles.top),
        (BorderEdge::Right, widths.right, colors.right, styles.right),
        (
            BorderEdge::Bottom,
            widths.bottom,
            colors.bottom,
            styles.bottom,
        ),
        (BorderEdge::Left, widths.left, colors.left, styles.left),
    ];
    for (edge, width, color, style) in edges {
        paint_border_edge(
            scene, rect, widths, radii, edge, width, color, style, transform,
        );
    }
}

/// Resolve the outer and inner border edges onto the layout-pixel grid.
///
/// Blink independently snaps `PixelSnappedContouredBorder` and
/// `PixelSnappedContouredInnerBorder`. Reconstructing the four used widths
/// from those two rectangles preserves that contract for asymmetric and
/// fractional borders instead of merely rounding the outer origin.
fn pixel_snapped_border_geometry(
    rect: Rect,
    widths: PaintEdgeSizes,
) -> Option<(Rect, PaintEdgeSizes)> {
    let (outer_x, outer_width) = pixel_snap_paint_axis(rect.x0 as f32, rect.width() as f32);
    let (outer_y, outer_height) = pixel_snap_paint_axis(rect.y0 as f32, rect.height() as f32);
    if outer_width <= 0.0 || outer_height <= 0.0 {
        return None;
    }

    let inner_x = rect.x0 as f32 + widths.left;
    let inner_y = rect.y0 as f32 + widths.top;
    let inner_width = (rect.width() as f32 - widths.left - widths.right).max(0.0);
    let inner_height = (rect.height() as f32 - widths.top - widths.bottom).max(0.0);
    let (inner_x, inner_width) = pixel_snap_paint_axis_allowing_zero(inner_x, inner_width);
    let (inner_y, inner_height) = pixel_snap_paint_axis_allowing_zero(inner_y, inner_height);

    let outer = Rect::new(
        f64::from(outer_x),
        f64::from(outer_y),
        f64::from(outer_x + outer_width),
        f64::from(outer_y + outer_height),
    );
    let widths = PaintEdgeSizes::new(
        (inner_y - outer_y).clamp(0.0, outer_height),
        (outer.x1 as f32 - (inner_x + inner_width)).clamp(0.0, outer_width),
        (outer.y1 as f32 - (inner_y + inner_height)).clamp(0.0, outer_height),
        (inner_x - outer_x).clamp(0.0, outer_width),
    );
    Some((
        outer,
        sanitize_border_widths(widths, outer_width, outer_height),
    ))
}

#[derive(Clone, Copy)]
enum BorderEdge {
    Top,
    Right,
    Bottom,
    Left,
}

#[allow(clippy::too_many_arguments)]
fn paint_border_edge(
    scene: &mut impl PaintScene,
    rect: Rect,
    widths: PaintEdgeSizes,
    radii: PaintCornerRadii,
    edge: BorderEdge,
    width: f32,
    color: PaintColor,
    style: PaintBorderStyle,
    transform: Affine,
) {
    if width <= 0.0
        || color.alpha <= 0.0
        || matches!(style, PaintBorderStyle::None | PaintBorderStyle::Hidden)
    {
        return;
    }

    match style {
        PaintBorderStyle::None | PaintBorderStyle::Hidden => {}
        PaintBorderStyle::Solid => {
            paint_border_ring_slice(scene, rect, widths, radii, edge, 0.0, 1.0, color, transform)
        }
        PaintBorderStyle::Double if width >= 3.0 => {
            paint_border_ring_slice(
                scene,
                rect,
                widths,
                radii,
                edge,
                0.0,
                1.0 / 3.0,
                color,
                transform,
            );
            paint_border_ring_slice(
                scene,
                rect,
                widths,
                radii,
                edge,
                2.0 / 3.0,
                1.0,
                color,
                transform,
            );
        }
        PaintBorderStyle::Double => {
            paint_border_ring_slice(scene, rect, widths, radii, edge, 0.0, 1.0, color, transform)
        }
        PaintBorderStyle::Groove | PaintBorderStyle::Ridge => {
            let ridge = style == PaintBorderStyle::Ridge;
            let (outer, inner) = grooved_colors(color, edge, ridge);
            paint_border_ring_slice(scene, rect, widths, radii, edge, 0.0, 0.5, outer, transform);
            paint_border_ring_slice(scene, rect, widths, radii, edge, 0.5, 1.0, inner, transform);
        }
        PaintBorderStyle::Inset | PaintBorderStyle::Outset => {
            let inset = style == PaintBorderStyle::Inset;
            paint_border_ring_slice(
                scene,
                rect,
                widths,
                radii,
                edge,
                0.0,
                1.0,
                beveled_color(color, edge, inset),
                transform,
            );
        }
        PaintBorderStyle::Dashed => {
            paint_dashed_border_edge(scene, rect, widths, radii, edge, width, color, transform)
        }
        PaintBorderStyle::Dotted => {
            paint_dotted_border_edge(scene, rect, widths, radii, edge, width, color, transform)
        }
    }
}

fn paint_uniform_solid_border(
    scene: &mut impl PaintScene,
    rect: Rect,
    widths: PaintEdgeSizes,
    colors: PaintBorderColors,
    styles: PaintBorderStyles,
    radii: PaintCornerRadii,
    transform: Affine,
) -> bool {
    if [styles.top, styles.right, styles.bottom, styles.left]
        .iter()
        .any(|style| *style != PaintBorderStyle::Solid)
        || [colors.right, colors.bottom, colors.left]
            .iter()
            .any(|color| *color != colors.top)
        || colors.top.alpha <= 0.0
    {
        return false;
    }
    let width = widths.top;
    if width <= 0.0
        || [widths.right, widths.bottom, widths.left]
            .iter()
            .any(|candidate| (*candidate - width).abs() > f32::EPSILON)
    {
        return false;
    }

    let width = f64::from(width);
    let stroke = Stroke::new(width).with_join(Join::Miter);
    let corners = [
        radii.top_left,
        radii.top_right,
        radii.bottom_right,
        radii.bottom_left,
    ];
    let circular = corners
        .iter()
        .all(|radius| (radius.x - radius.y).abs() < 0.01);
    if circular
        && corners
            .iter()
            .all(|radius| radius.x == 0.0 || f64::from(radius.x) >= width)
    {
        let half_width = width / 2.0;
        let centerline = rect - Insets::uniform(half_width);
        let centerline_radius =
            |radius: PaintCornerRadius| (f64::from(radius.x) - half_width).max(0.0);
        let radii = RoundedRectRadii::new(
            centerline_radius(radii.top_left),
            centerline_radius(radii.top_right),
            centerline_radius(radii.bottom_right),
            centerline_radius(radii.bottom_left),
        );
        scene.stroke(
            &stroke,
            transform,
            to_backend_color(colors.top),
            None,
            &RoundedRect::from_rect(centerline, radii),
        );
        return true;
    }

    let half_x = rect.width() / 2.0;
    let half_y = rect.height() / 2.0;
    let is_half = |radius: PaintCornerRadius| {
        (f64::from(radius.x) - half_x).abs() < 0.01 && (f64::from(radius.y) - half_y).abs() < 0.01
    };
    if corners.into_iter().all(is_half) {
        let radius_x = (rect.width() + (rect.width() - width * 2.0).max(0.0)) / 4.0;
        let radius_y = (rect.height() + (rect.height() - width * 2.0).max(0.0)) / 4.0;
        scene.stroke(
            &stroke,
            transform,
            to_backend_color(colors.top),
            None,
            &Ellipse::new(rect.center(), (radius_x, radius_y), 0.0),
        );
        return true;
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn paint_dashed_border_edge(
    scene: &mut impl PaintScene,
    rect: Rect,
    widths: PaintEdgeSizes,
    radii: PaintCornerRadii,
    edge: BorderEdge,
    width: f32,
    color: PaintColor,
    transform: Affine,
) {
    let thickness = f64::from(width);
    if thickness <= 0.0 {
        return;
    }
    let (dash_ratio, gap_ratio) = dashed_ratios(thickness);
    let (centerline, dash, gap) = if !radii.is_zero() {
        let center = border_slice(rect, widths, radii, 0.5);
        let centerline = rounded_rect_path(center.0, center.1, true);
        let perimeter = centerline.perimeter(0.1);
        if perimeter <= 0.0 {
            return;
        }
        let initial_period = (dash_ratio + gap_ratio) * thickness;
        let count = (perimeter / initial_period).round().max(1.0);
        let period = perimeter / count;
        let dash = period * dash_ratio / (dash_ratio + gap_ratio);
        (centerline, dash, period - dash)
    } else {
        let half = thickness / 2.0;
        let (start, end, length) = match edge {
            BorderEdge::Top => (
                Point::new(rect.x0, rect.y0 + half),
                Point::new(rect.x1, rect.y0 + half),
                rect.width(),
            ),
            BorderEdge::Bottom => (
                Point::new(rect.x0, rect.y1 - half),
                Point::new(rect.x1, rect.y1 - half),
                rect.width(),
            ),
            BorderEdge::Left => (
                Point::new(rect.x0 + half, rect.y0),
                Point::new(rect.x0 + half, rect.y1),
                rect.height(),
            ),
            BorderEdge::Right => (
                Point::new(rect.x1 - half, rect.y0),
                Point::new(rect.x1 - half, rect.y1),
                rect.height(),
            ),
        };
        if length <= 0.0 {
            return;
        }
        let initial_dash = dash_ratio * thickness;
        let initial_gap = gap_ratio * thickness;
        let count = ((length + initial_gap) / (initial_dash + initial_gap))
            .round()
            .max(1.0);
        let ratio = dash_ratio / gap_ratio;
        let gap = length / (count * ratio + count - 1.0);
        let mut line = BezPath::new();
        line.move_to(start);
        line.line_to(end);
        (line, ratio * gap, gap)
    };

    let stroke = Stroke::new(thickness)
        .with_caps(Cap::Butt)
        .with_dashes(0.0, [dash, gap]);
    let clip = border_edge_clip(rect, widths, edge);
    scene.push_clip_layer(transform, &clip);
    scene.stroke(
        &stroke,
        transform,
        to_backend_color(color),
        None,
        &centerline,
    );
    scene.pop_layer();
}

#[allow(clippy::too_many_arguments)]
fn paint_dotted_border_edge(
    scene: &mut impl PaintScene,
    rect: Rect,
    widths: PaintEdgeSizes,
    radii: PaintCornerRadii,
    edge: BorderEdge,
    width: f32,
    color: PaintColor,
    transform: Affine,
) {
    let thickness = f64::from(width);
    if thickness <= 0.0 {
        return;
    }
    let radius = thickness / 2.0;
    let mut path = BezPath::new();
    if !radii.is_zero() {
        let center = border_slice(rect, widths, radii, 0.5);
        let centerline = rounded_rect_path(center.0, center.1, true);
        let perimeter = centerline.perimeter(0.1);
        if perimeter <= 0.0 {
            return;
        }
        let count = (perimeter / (2.0 * thickness)).round().max(1.0);
        let spacing = perimeter / count;
        for center in sample_points_along_path(&centerline, spacing, count as usize) {
            path.extend(Circle::new(center, radius).path_elements(0.1));
        }
    } else {
        let length = match edge {
            BorderEdge::Top | BorderEdge::Bottom => rect.width(),
            BorderEdge::Left | BorderEdge::Right => rect.height(),
        };
        if length <= 0.0 {
            return;
        }
        let dot_center = |along: f64| match edge {
            BorderEdge::Top => Point::new(rect.x0 + along, rect.y0 + radius),
            BorderEdge::Bottom => Point::new(rect.x0 + along, rect.y1 - radius),
            BorderEdge::Left => Point::new(rect.x0 + radius, rect.y0 + along),
            BorderEdge::Right => Point::new(rect.x1 - radius, rect.y0 + along),
        };
        let span = length - thickness;
        if span <= 0.0 {
            path.extend(Circle::new(dot_center(length / 2.0), radius).path_elements(0.1));
        } else {
            let gaps = (span / (2.0 * thickness)).round().max(1.0) as usize;
            let spacing = span / gaps as f64;
            for index in 0..=gaps {
                path.extend(
                    Circle::new(dot_center(radius + index as f64 * spacing), radius)
                        .path_elements(0.1),
                );
            }
        }
    }

    let clip = border_edge_clip(rect, widths, edge);
    scene.push_clip_layer(transform, &clip);
    scene.fill(
        Fill::NonZero,
        transform,
        to_backend_color(color),
        None,
        &path,
    );
    scene.pop_layer();
}

#[allow(clippy::too_many_arguments)]
fn paint_border_ring_slice(
    scene: &mut impl PaintScene,
    rect: Rect,
    widths: PaintEdgeSizes,
    radii: PaintCornerRadii,
    edge: BorderEdge,
    outer_fraction: f64,
    inner_fraction: f64,
    color: PaintColor,
    transform: Affine,
) {
    let clip = border_edge_clip(rect, widths, edge);
    let outer = border_slice(rect, widths, radii, outer_fraction);
    let inner = border_slice(rect, widths, radii, inner_fraction);
    let mut ring = rounded_rect_path(outer.0, outer.1, true);
    ring.extend(rounded_rect_path(inner.0, inner.1, true));
    scene.push_clip_layer(transform, &clip);
    scene.fill(
        Fill::EvenOdd,
        transform,
        to_backend_color(color),
        None,
        &ring,
    );
    scene.pop_layer();
}

fn border_slice(
    rect: Rect,
    widths: PaintEdgeSizes,
    radii: PaintCornerRadii,
    fraction: f64,
) -> (Rect, PaintCornerRadii) {
    let top = f64::from(widths.top) * fraction;
    let right = f64::from(widths.right) * fraction;
    let bottom = f64::from(widths.bottom) * fraction;
    let left = f64::from(widths.left) * fraction;
    let inset = Rect::new(
        (rect.x0 + left).min(rect.x1),
        (rect.y0 + top).min(rect.y1),
        (rect.x1 - right).max(rect.x0),
        (rect.y1 - bottom).max(rect.y0),
    );
    let radii = PaintCornerRadii {
        top_left: inset_radius(radii.top_left, left, top),
        top_right: inset_radius(radii.top_right, right, top),
        bottom_right: inset_radius(radii.bottom_right, right, bottom),
        bottom_left: inset_radius(radii.bottom_left, left, bottom),
    };
    (inset, normalize_radii(radii, inset.width(), inset.height()))
}

fn inset_radius(radius: PaintCornerRadius, horizontal: f64, vertical: f64) -> PaintCornerRadius {
    PaintCornerRadius::new(
        (f64::from(radius.x) - horizontal).max(0.0) as f32,
        (f64::from(radius.y) - vertical).max(0.0) as f32,
    )
}

fn border_edge_clip(rect: Rect, widths: PaintEdgeSizes, edge: BorderEdge) -> BezPath {
    let left = f64::from(widths.left);
    let right = f64::from(widths.right);
    let top = f64::from(widths.top);
    let bottom = f64::from(widths.bottom);
    let points = match edge {
        BorderEdge::Top => [
            (rect.x0, rect.y0),
            (rect.x1, rect.y0),
            (rect.x1 - right, rect.y0 + top),
            (rect.x0 + left, rect.y0 + top),
        ],
        BorderEdge::Right => [
            (rect.x1, rect.y0),
            (rect.x1, rect.y1),
            (rect.x1 - right, rect.y1 - bottom),
            (rect.x1 - right, rect.y0 + top),
        ],
        BorderEdge::Bottom => [
            (rect.x1, rect.y1),
            (rect.x0, rect.y1),
            (rect.x0 + left, rect.y1 - bottom),
            (rect.x1 - right, rect.y1 - bottom),
        ],
        BorderEdge::Left => [
            (rect.x0, rect.y1),
            (rect.x0, rect.y0),
            (rect.x0 + left, rect.y0 + top),
            (rect.x0 + left, rect.y1 - bottom),
        ],
    };
    let mut path = BezPath::new();
    path.move_to(points[0]);
    for point in &points[1..] {
        path.line_to(*point);
    }
    path.close_path();
    path
}

// Border shading and dash/dot distribution are DOM-neutral ports of
// `blitz-paint/src/render/border.rs` at Blitz d788124a. Geometry remains in
// snapshot CSS pixels and is scaled once by the fragment transform.
fn darken(color: PaintColor) -> PaintColor {
    let value = color.red.max(color.green).max(color.blue);
    if value == 0.0 {
        return color;
    }
    let multiplier = ((value - 0.33) / value).max(0.0);
    PaintColor::new(
        color.red * multiplier,
        color.green * multiplier,
        color.blue * multiplier,
        color.alpha,
    )
}

fn lighten(color: PaintColor) -> PaintColor {
    let value = color.red.max(color.green).max(color.blue);
    if value == 0.0 {
        return PaintColor::new(0.33, 0.33, 0.33, color.alpha);
    }
    let multiplier = (value + 0.33).min(1.0) / value;
    PaintColor::new(
        color.red * multiplier,
        color.green * multiplier,
        color.blue * multiplier,
        color.alpha,
    )
}

fn relative_luminance(color: PaintColor) -> f32 {
    let linear = |channel: f32| {
        let channel = channel.clamp(0.0, 1.0);
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.red) + 0.7152 * linear(color.green) + 0.0722 * linear(color.blue)
}

fn contrast_ratio(first: PaintColor, second: PaintColor) -> f32 {
    let first = relative_luminance(first) + 0.05;
    let second = relative_luminance(second) + 0.05;
    if first > second {
        first / second
    } else {
        second / first
    }
}

fn beveled_color(color: PaintColor, edge: BorderEdge, inset: bool) -> PaintColor {
    let top_or_left = matches!(edge, BorderEdge::Top | BorderEdge::Left);
    let should_darken = top_or_left == inset;
    let dark = darken(color);
    if should_darken {
        return dark;
    }
    if color.red >= 150.0 / 255.0 || color.green >= 92.0 / 255.0 {
        return color;
    }
    const MIN_CONTRAST_RATIO: f32 = 1.75;
    if contrast_ratio(color, dark) < MIN_CONTRAST_RATIO {
        lighten(color)
    } else {
        color
    }
}

fn grooved_colors(color: PaintColor, edge: BorderEdge, ridge: bool) -> (PaintColor, PaintColor) {
    (
        beveled_color(color, edge, !ridge),
        beveled_color(color, edge, ridge),
    )
}

fn dashed_ratios(thickness: f64) -> (f64, f64) {
    if thickness >= 3.0 {
        (2.0, 1.0)
    } else {
        (3.0, 2.0)
    }
}

fn sample_points_along_path(path: &BezPath, spacing: f64, count: usize) -> Vec<Point> {
    let mut polyline = Vec::new();
    peniko::kurbo::flatten(path.iter(), 0.1, |element| match element {
        PathEl::MoveTo(point) | PathEl::LineTo(point) => polyline.push(point),
        PathEl::ClosePath => {
            if let Some(&first) = polyline.first() {
                polyline.push(first);
            }
        }
        _ => {}
    });
    if polyline.len() < 2 {
        return Vec::new();
    }

    let mut points = Vec::with_capacity(count);
    let mut segment = 0;
    let mut segment_start = 0.0;
    let mut segment_length = (polyline[1] - polyline[0]).hypot();
    for index in 0..count {
        let target = index as f64 * spacing;
        while segment + 2 < polyline.len() && target > segment_start + segment_length {
            segment_start += segment_length;
            segment += 1;
            segment_length = (polyline[segment + 1] - polyline[segment]).hypot();
        }
        let progress = if segment_length > 0.0 {
            ((target - segment_start) / segment_length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        points.push(polyline[segment].lerp(polyline[segment + 1], progress));
    }
    points
}

fn paint_box_shadow(scene: &mut impl PaintScene, shadow: &PaintBoxShadow, scale: f64) {
    let Some(rect) = to_backend_rect(shadow.rect) else {
        return;
    };
    if shadow.color.alpha <= 0.0 {
        return;
    }
    let spread = finite_nonnegative(shadow.spread_radius) as f64;
    let blur = finite_nonnegative(shadow.blur_radius) as f64;
    let offset = Vec2::new(
        f64::from(finite_or_zero(shadow.offset.x)),
        f64::from(finite_or_zero(shadow.offset.y)),
    );
    let shadow_rect = rect.inflate(spread, spread);
    let radii = normalize_radii(
        outset_radii(shadow.radii, spread as f32),
        shadow_rect.width(),
        shadow_rect.height(),
    );
    let radius = average_radius(radii);
    let transform = device_transform(shadow.transform, scale) * Affine::translate(offset);
    let color = to_backend_color(shadow.color);

    if shadow.inset {
        let clip = rounded_rect_path(
            rect,
            normalize_radii(shadow.radii, rect.width(), rect.height()),
            true,
        );
        scene.push_layer(
            Mix::Normal,
            1.0,
            device_transform(shadow.transform, scale),
            &clip,
            None,
            None,
        );
        scene.fill(
            Fill::NonZero,
            device_transform(shadow.transform, scale),
            color,
            None,
            &clip,
        );
        scene.push_layer(
            Compose::DestOut,
            1.0,
            device_transform(shadow.transform, scale),
            &clip,
            None,
            None,
        );
        scene.draw_box_shadow(transform, shadow_rect, Color::WHITE, radius, blur);
        scene.pop_layer();
        scene.pop_layer();
    } else {
        let extent = spread + blur * 4.0 + offset.hypot() + 1.0;
        let outer = rect.inflate(extent, extent);
        let mut clip = rect_path(outer, true);
        clip.extend(rounded_rect_path(
            rect,
            normalize_radii(shadow.radii, rect.width(), rect.height()),
            false,
        ));
        scene.push_clip_layer(device_transform(shadow.transform, scale), &clip);
        scene.draw_box_shadow(transform, shadow_rect, color, radius, blur);
        scene.pop_layer();
    }
}

fn paint_text_shadow(
    scene: &mut impl PaintScene,
    snapshot: &PaintSnapshot,
    shadow: &PaintTextShadow,
    scale: f64,
) {
    let Some(font) = snapshot.font(shadow.run.font) else {
        return;
    };
    if shadow.color.alpha <= 0.0 {
        return;
    }
    let clip = Rect::new(
        0.0,
        0.0,
        f64::from(snapshot.viewport.css_width),
        f64::from(snapshot.viewport.css_height),
    );
    let blur = finite_nonnegative(shadow.blur_radius) * 0.5;
    if blur > 0.0 {
        scene.push_layer(
            Mix::Normal,
            1.0,
            Affine::scale(scale),
            &clip,
            Some(Arc::new(Filter::single(FilterEffect::blur(blur)))),
            None,
        );
    }
    let glyph_transform = backend_glyph_transform(shadow.run.glyph_skew_radians);
    let glyph_embolden = backend_glyph_embolden(shadow.run.glyph_embolden);
    scene.draw_glyphs(
        &font.font,
        shadow.run.font_size,
        glyph_embolden == Vec2::ZERO,
        &shadow.run.normalized_coords,
        glyph_embolden,
        Fill::NonZero,
        to_backend_color(shadow.color),
        1.0,
        device_transform(shadow.run.transform, scale),
        glyph_transform,
        shadow.run.glyphs.iter().map(|glyph| anyrender::Glyph {
            id: glyph.id,
            x: glyph.x + finite_or_zero(shadow.offset.x),
            y: glyph.y + finite_or_zero(shadow.offset.y),
        }),
    );
    if blur > 0.0 {
        scene.pop_layer();
    }
}

fn paint_glyph_run(
    scene: &mut impl PaintScene,
    snapshot: &PaintSnapshot,
    run: &moli_layout::PaintGlyphRun,
    color: PaintColor,
    scale: f64,
) {
    let Some(font) = snapshot.font(run.font) else {
        return;
    };
    let glyph_transform = backend_glyph_transform(run.glyph_skew_radians);
    let glyph_embolden = backend_glyph_embolden(run.glyph_embolden);
    scene.draw_glyphs(
        &font.font,
        run.font_size,
        glyph_embolden == Vec2::ZERO,
        &run.normalized_coords,
        glyph_embolden,
        Fill::NonZero,
        to_backend_color(color),
        1.0,
        device_transform(run.transform, scale),
        glyph_transform,
        run.glyphs.iter().map(|glyph| anyrender::Glyph {
            id: glyph.id,
            x: glyph.x,
            y: glyph.y,
        }),
    );
}

fn paint_text_decoration(
    scene: &mut impl PaintScene,
    decoration: &PaintTextDecoration,
    scale: f64,
) {
    let x = f64::from(finite_or_zero(decoration.x));
    let y = f64::from(finite_or_zero(decoration.y));
    let width = f64::from(finite_nonnegative(decoration.width));
    let thickness = f64::from(finite_nonnegative(decoration.thickness));
    if width <= 0.0 || thickness <= 0.0 || decoration.color.alpha <= 0.0 {
        return;
    }
    let transform = device_transform(decoration.transform, scale);
    let color = to_backend_color(decoration.color);
    match decoration.style {
        PaintTextDecorationStyle::Solid => {
            let rect = Rect::new(x, y - thickness * 0.5, x + width, y + thickness * 0.5);
            scene.fill(Fill::NonZero, transform, color, None, &rect);
        }
        PaintTextDecorationStyle::Double => {
            for center in [y - thickness, y + thickness] {
                let rect = Rect::new(
                    x,
                    center - thickness * 0.5,
                    x + width,
                    center + thickness * 0.5,
                );
                scene.fill(Fill::NonZero, transform, color, None, &rect);
            }
        }
        PaintTextDecorationStyle::Dotted | PaintTextDecorationStyle::Dashed => {
            let mut stroke = Stroke::new(thickness);
            stroke = if decoration.style == PaintTextDecorationStyle::Dotted {
                stroke
                    .with_caps(Cap::Round)
                    .with_dashes(0.0, [thickness * 0.08, thickness * 1.9])
            } else {
                stroke
                    .with_caps(Cap::Butt)
                    .with_dashes(0.0, [thickness * 3.0, thickness * 2.0])
            };
            scene.stroke(
                &stroke,
                transform,
                color,
                None,
                &peniko::kurbo::Line::new((x, y), (x + width, y)),
            );
        }
        PaintTextDecorationStyle::Wavy => {
            let amplitude = (thickness * 1.5).max(1.0);
            let half_wave = (thickness * 2.0).max(2.0);
            let mut path = BezPath::new();
            path.move_to((x, y));
            let mut current_x = x;
            let mut high = true;
            while current_x < x + width {
                current_x = (current_x + half_wave).min(x + width);
                path.line_to((current_x, y + if high { -amplitude } else { amplitude }));
                high = !high;
            }
            scene.stroke(
                &Stroke::new(thickness).with_caps(Cap::Round),
                transform,
                color,
                None,
                &path,
            );
        }
    }
}

fn backend_glyph_embolden(embolden: moli_layout::PaintPoint) -> Vec2 {
    Vec2::new(
        f64::from(finite_nonnegative(embolden.x)),
        f64::from(finite_nonnegative(embolden.y)),
    )
}

fn backend_glyph_transform(skew_radians: Option<f32>) -> Option<Affine> {
    skew_radians.map(|radians| {
        // A positive Fontique faux-oblique angle means a right lean. AnyRender
        // composes this transform in layout's Y-down space after flipping the
        // glyph outline from font-space Y-up, so preserve the CSS direction by
        // negating the horizontal shear at this boundary.
        Affine::skew(-f64::from(radians).tan(), 0.0)
    })
}

fn paint_stroke(scene: &mut impl PaintScene, stroke: &PaintStroke, scale: f64) {
    let width = f64::from(finite_nonnegative(stroke.width));
    if width <= 0.0 || stroke.color.alpha <= 0.0 {
        return;
    }
    let mut backend_stroke = Stroke::new(width);
    backend_stroke.join = match stroke.join {
        PaintLineJoin::Miter => Join::Miter,
        PaintLineJoin::Round => Join::Round,
        PaintLineJoin::Bevel => Join::Bevel,
    };
    backend_stroke.start_cap = to_backend_cap(stroke.start_cap);
    backend_stroke.end_cap = to_backend_cap(stroke.end_cap);
    backend_stroke.miter_limit = f64::from(finite_nonnegative(stroke.miter_limit));
    backend_stroke.dash_pattern = stroke
        .dash_pattern
        .iter()
        .map(|value| f64::from(finite_nonnegative(*value)))
        .collect();
    backend_stroke.dash_offset = f64::from(finite_or_zero(stroke.dash_offset));
    let path = to_backend_path(&stroke.path);
    scene.stroke(
        &backend_stroke,
        device_transform(stroke.transform, scale),
        to_backend_color(stroke.color),
        None,
        &path,
    );
}

fn paint_fill(scene: &mut impl PaintScene, shape: &BezPath, brush: &PaintBrush, transform: Affine) {
    match brush {
        PaintBrush::Solid(color) => scene.fill(
            Fill::NonZero,
            transform,
            to_backend_color(*color),
            None,
            shape,
        ),
        PaintBrush::LinearGradient(gradient) => {
            let gradient = Gradient::new_linear(
                (f64::from(gradient.start.x), f64::from(gradient.start.y)),
                (f64::from(gradient.end.x), f64::from(gradient.end.y)),
            )
            .with_extend(to_backend_extend(gradient.extend))
            .with_interpolation_cs(to_backend_gradient_color_space(
                gradient.interpolation.color_space,
            ))
            .with_hue_direction(to_backend_gradient_hue_direction(
                gradient.interpolation.hue_direction,
            ))
            .with_stops(backend_gradient_stops(&gradient.stops).as_slice());
            scene.fill(Fill::NonZero, transform, &gradient, None, shape);
        }
        PaintBrush::RadialGradient(gradient) => {
            let gradient_brush = Gradient::new_two_point_radial(
                (
                    f64::from(gradient.start_center.x),
                    f64::from(gradient.start_center.y),
                ),
                finite_nonnegative(gradient.start_radius),
                (
                    f64::from(gradient.end_center.x),
                    f64::from(gradient.end_center.y),
                ),
                finite_nonnegative(gradient.end_radius),
            )
            .with_extend(to_backend_extend(gradient.extend))
            .with_interpolation_cs(to_backend_gradient_color_space(
                gradient.interpolation.color_space,
            ))
            .with_hue_direction(to_backend_gradient_hue_direction(
                gradient.interpolation.hue_direction,
            ))
            .with_stops(backend_gradient_stops(&gradient.stops).as_slice());
            scene.fill(
                Fill::NonZero,
                transform,
                &gradient_brush,
                Some(to_backend_transform(gradient.transform)),
                shape,
            );
        }
        PaintBrush::ConicGradient(gradient) => {
            let gradient_brush = Gradient::new_sweep(
                (f64::from(gradient.center.x), f64::from(gradient.center.y)),
                finite_or_zero(gradient.start_angle_radians),
                finite_or_zero(gradient.end_angle_radians),
            )
            .with_extend(to_backend_extend(gradient.extend))
            .with_interpolation_cs(to_backend_gradient_color_space(
                gradient.interpolation.color_space,
            ))
            .with_hue_direction(to_backend_gradient_hue_direction(
                gradient.interpolation.hue_direction,
            ))
            .with_stops(backend_gradient_stops(&gradient.stops).as_slice());
            scene.fill(
                Fill::NonZero,
                transform,
                &gradient_brush,
                Some(to_backend_transform(gradient.transform)),
                shape,
            );
        }
    }
}

fn to_backend_gradient_color_space(color_space: PaintGradientColorSpace) -> ColorSpaceTag {
    match color_space {
        PaintGradientColorSpace::Srgb => ColorSpaceTag::Srgb,
        PaintGradientColorSpace::LinearSrgb => ColorSpaceTag::LinearSrgb,
        PaintGradientColorSpace::Hsl => ColorSpaceTag::Hsl,
        PaintGradientColorSpace::Hwb => ColorSpaceTag::Hwb,
        PaintGradientColorSpace::Lab => ColorSpaceTag::Lab,
        PaintGradientColorSpace::Lch => ColorSpaceTag::Lch,
        PaintGradientColorSpace::Oklab => ColorSpaceTag::Oklab,
        PaintGradientColorSpace::Oklch => ColorSpaceTag::Oklch,
        PaintGradientColorSpace::DisplayP3 => ColorSpaceTag::DisplayP3,
        PaintGradientColorSpace::A98Rgb => ColorSpaceTag::A98Rgb,
        PaintGradientColorSpace::ProphotoRgb => ColorSpaceTag::ProphotoRgb,
        PaintGradientColorSpace::Rec2020 => ColorSpaceTag::Rec2020,
        PaintGradientColorSpace::XyzD50 => ColorSpaceTag::XyzD50,
        PaintGradientColorSpace::XyzD65 => ColorSpaceTag::XyzD65,
    }
}

fn to_backend_gradient_hue_direction(hue_direction: PaintGradientHueDirection) -> HueDirection {
    match hue_direction {
        PaintGradientHueDirection::Shorter => HueDirection::Shorter,
        PaintGradientHueDirection::Longer => HueDirection::Longer,
        PaintGradientHueDirection::Increasing => HueDirection::Increasing,
        PaintGradientHueDirection::Decreasing => HueDirection::Decreasing,
    }
}

fn backend_gradient_stops(stops: &[moli_layout::PaintGradientStop]) -> Vec<(f32, Color)> {
    if stops.is_empty() {
        return vec![(0.0, Color::TRANSPARENT), (1.0, Color::TRANSPARENT)];
    }
    if stops.len() == 1 {
        let color = to_backend_color(stops[0].color);
        return vec![(0.0, color), (1.0, color)];
    }

    // Product snapshots already guarantee a normalized, monotonic stop
    // domain. Keep the backend trust boundary defensive because PaintSnapshot
    // is a public owned value and malformed offsets must not reach Vello's
    // shader assumptions or turn into a backend panic.
    let mut previous = 0.0;
    stops
        .iter()
        .map(|stop| {
            let offset = finite_or_zero(stop.offset).clamp(0.0, 1.0).max(previous);
            previous = offset;
            (offset, to_backend_color(stop.color))
        })
        .collect()
}

fn to_backend_shape(shape: &PaintShape) -> BezPath {
    match shape {
        PaintShape::Rect(rect) => to_backend_rect(*rect)
            .map(|rect| rect_path(rect, true))
            .unwrap_or_default(),
        PaintShape::RoundedRect { rect, radii } => to_backend_rect(*rect)
            .map(|rect| {
                rounded_rect_path(
                    rect,
                    normalize_radii(*radii, rect.width(), rect.height()),
                    true,
                )
            })
            .unwrap_or_default(),
        PaintShape::Path(path) => to_backend_path(path),
    }
}

fn to_backend_path(path: &PaintPath) -> BezPath {
    let mut backend = BezPath::new();
    for element in &path.elements {
        match *element {
            PaintPathElement::MoveTo(point) => backend.move_to((point.x, point.y)),
            PaintPathElement::LineTo(point) => backend.line_to((point.x, point.y)),
            PaintPathElement::QuadTo(control, point) => {
                backend.quad_to((control.x, control.y), (point.x, point.y));
            }
            PaintPathElement::CubicTo(first, second, point) => {
                backend.curve_to((first.x, first.y), (second.x, second.y), (point.x, point.y))
            }
            PaintPathElement::Close => backend.close_path(),
        }
    }
    backend
}

const fn to_backend_cap(cap: PaintLineCap) -> Cap {
    match cap {
        PaintLineCap::Butt => Cap::Butt,
        PaintLineCap::Round => Cap::Round,
        PaintLineCap::Square => Cap::Square,
    }
}

fn rounded_rect_path(rect: Rect, radii: PaintCornerRadii, clockwise: bool) -> BezPath {
    if radii.is_zero() {
        return rect_path(rect, clockwise);
    }
    const KAPPA: f64 = 0.552_284_749_830_793_6;
    let tl = radius_pair(radii.top_left);
    let tr = radius_pair(radii.top_right);
    let br = radius_pair(radii.bottom_right);
    let bl = radius_pair(radii.bottom_left);
    let mut path = BezPath::new();
    if clockwise {
        path.move_to((rect.x0 + tl.0, rect.y0));
        path.line_to((rect.x1 - tr.0, rect.y0));
        path.curve_to(
            (rect.x1 - tr.0 + tr.0 * KAPPA, rect.y0),
            (rect.x1, rect.y0 + tr.1 - tr.1 * KAPPA),
            (rect.x1, rect.y0 + tr.1),
        );
        path.line_to((rect.x1, rect.y1 - br.1));
        path.curve_to(
            (rect.x1, rect.y1 - br.1 + br.1 * KAPPA),
            (rect.x1 - br.0 + br.0 * KAPPA, rect.y1),
            (rect.x1 - br.0, rect.y1),
        );
        path.line_to((rect.x0 + bl.0, rect.y1));
        path.curve_to(
            (rect.x0 + bl.0 - bl.0 * KAPPA, rect.y1),
            (rect.x0, rect.y1 - bl.1 + bl.1 * KAPPA),
            (rect.x0, rect.y1 - bl.1),
        );
        path.line_to((rect.x0, rect.y0 + tl.1));
        path.curve_to(
            (rect.x0, rect.y0 + tl.1 - tl.1 * KAPPA),
            (rect.x0 + tl.0 - tl.0 * KAPPA, rect.y0),
            (rect.x0 + tl.0, rect.y0),
        );
    } else {
        path.move_to((rect.x0 + tl.0, rect.y0));
        path.curve_to(
            (rect.x0 + tl.0 - tl.0 * KAPPA, rect.y0),
            (rect.x0, rect.y0 + tl.1 - tl.1 * KAPPA),
            (rect.x0, rect.y0 + tl.1),
        );
        path.line_to((rect.x0, rect.y1 - bl.1));
        path.curve_to(
            (rect.x0, rect.y1 - bl.1 + bl.1 * KAPPA),
            (rect.x0 + bl.0 - bl.0 * KAPPA, rect.y1),
            (rect.x0 + bl.0, rect.y1),
        );
        path.line_to((rect.x1 - br.0, rect.y1));
        path.curve_to(
            (rect.x1 - br.0 + br.0 * KAPPA, rect.y1),
            (rect.x1, rect.y1 - br.1 + br.1 * KAPPA),
            (rect.x1, rect.y1 - br.1),
        );
        path.line_to((rect.x1, rect.y0 + tr.1));
        path.curve_to(
            (rect.x1, rect.y0 + tr.1 - tr.1 * KAPPA),
            (rect.x1 - tr.0 + tr.0 * KAPPA, rect.y0),
            (rect.x1 - tr.0, rect.y0),
        );
        path.line_to((rect.x0 + tl.0, rect.y0));
    }
    path.close_path();
    path
}

fn rect_path(rect: Rect, clockwise: bool) -> BezPath {
    let mut path = BezPath::new();
    if clockwise {
        path.move_to((rect.x0, rect.y0));
        path.line_to((rect.x1, rect.y0));
        path.line_to((rect.x1, rect.y1));
        path.line_to((rect.x0, rect.y1));
    } else {
        path.move_to((rect.x0, rect.y0));
        path.line_to((rect.x0, rect.y1));
        path.line_to((rect.x1, rect.y1));
        path.line_to((rect.x1, rect.y0));
    }
    path.close_path();
    path
}

fn normalize_radii(radii: PaintCornerRadii, rect_width: f64, rect_height: f64) -> PaintCornerRadii {
    let mut radii = PaintCornerRadii {
        top_left: sanitize_radius(radii.top_left),
        top_right: sanitize_radius(radii.top_right),
        bottom_right: sanitize_radius(radii.bottom_right),
        bottom_left: sanitize_radius(radii.bottom_left),
    };
    let factors = [
        ratio(rect_width, f64::from(radii.top_left.x + radii.top_right.x)),
        ratio(
            rect_width,
            f64::from(radii.bottom_left.x + radii.bottom_right.x),
        ),
        ratio(
            rect_height,
            f64::from(radii.top_left.y + radii.bottom_left.y),
        ),
        ratio(
            rect_height,
            f64::from(radii.top_right.y + radii.bottom_right.y),
        ),
    ];
    let factor = factors.into_iter().fold(1.0_f64, f64::min).min(1.0) as f32;
    for radius in [
        &mut radii.top_left,
        &mut radii.top_right,
        &mut radii.bottom_right,
        &mut radii.bottom_left,
    ] {
        radius.x *= factor;
        radius.y *= factor;
    }
    radii
}

fn ratio(available: f64, requested: f64) -> f64 {
    if requested > 0.0 {
        (available.max(0.0) / requested).max(0.0)
    } else {
        1.0
    }
}

fn sanitize_radius(radius: PaintCornerRadius) -> PaintCornerRadius {
    PaintCornerRadius::new(finite_nonnegative(radius.x), finite_nonnegative(radius.y))
}

fn radius_pair(radius: PaintCornerRadius) -> (f64, f64) {
    (f64::from(radius.x), f64::from(radius.y))
}

fn outset_radii(radii: PaintCornerRadii, spread: f32) -> PaintCornerRadii {
    let outset = |radius: PaintCornerRadius| {
        PaintCornerRadius::new((radius.x + spread).max(0.0), (radius.y + spread).max(0.0))
    };
    PaintCornerRadii {
        top_left: outset(radii.top_left),
        top_right: outset(radii.top_right),
        bottom_right: outset(radii.bottom_right),
        bottom_left: outset(radii.bottom_left),
    }
}

fn average_radius(radii: PaintCornerRadii) -> f64 {
    let values = [
        radii.top_left,
        radii.top_right,
        radii.bottom_right,
        radii.bottom_left,
    ];
    values
        .into_iter()
        .map(|radius| f64::from(radius.x + radius.y) * 0.5)
        .sum::<f64>()
        / 4.0
}

fn device_transform(transform: PaintTransform2D, scale: f64) -> Affine {
    Affine::scale(scale) * to_backend_transform(transform)
}

fn to_backend_transform(transform: PaintTransform2D) -> Affine {
    if transform.is_finite() {
        Affine::new(transform.coefficients)
    } else {
        Affine::IDENTITY
    }
}

fn to_backend_blend_mode(mode: PaintBlendMode, composite: PaintCompositeMode) -> BlendMode {
    if mode == PaintBlendMode::PlusLighter && composite == PaintCompositeMode::SrcOver {
        return Compose::PlusLighter.into();
    }
    let mix = match mode {
        PaintBlendMode::PlusLighter | PaintBlendMode::Normal => Mix::Normal,
        PaintBlendMode::Multiply => Mix::Multiply,
        PaintBlendMode::Screen => Mix::Screen,
        PaintBlendMode::Overlay => Mix::Overlay,
        PaintBlendMode::Darken => Mix::Darken,
        PaintBlendMode::Lighten => Mix::Lighten,
        PaintBlendMode::ColorDodge => Mix::ColorDodge,
        PaintBlendMode::ColorBurn => Mix::ColorBurn,
        PaintBlendMode::HardLight => Mix::HardLight,
        PaintBlendMode::SoftLight => Mix::SoftLight,
        PaintBlendMode::Difference => Mix::Difference,
        PaintBlendMode::Exclusion => Mix::Exclusion,
        PaintBlendMode::Hue => Mix::Hue,
        PaintBlendMode::Saturation => Mix::Saturation,
        PaintBlendMode::Color => Mix::Color,
        PaintBlendMode::Luminosity => Mix::Luminosity,
    };
    let compose = match composite {
        PaintCompositeMode::SrcOver => Compose::SrcOver,
        PaintCompositeMode::DestIn => Compose::DestIn,
        PaintCompositeMode::SrcOut => Compose::SrcOut,
        PaintCompositeMode::SrcIn => Compose::SrcIn,
        PaintCompositeMode::Xor => Compose::Xor,
    };
    BlendMode::new(mix, compose)
}

fn to_backend_extend(extend: PaintGradientExtend) -> Extend {
    match extend {
        PaintGradientExtend::Pad => Extend::Pad,
        PaintGradientExtend::Repeat => Extend::Repeat,
        PaintGradientExtend::Reflect => Extend::Reflect,
    }
}

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn finite_nonnegative(value: f32) -> f32 {
    finite_or_zero(value).max(0.0)
}

fn sanitize_unit(value: f32) -> f32 {
    finite_or_zero(value).clamp(0.0, 1.0)
}

fn sanitize_border_rect(rect: PaintRect) -> Option<Rect> {
    to_backend_rect(rect)
}

fn sanitize_border_widths(
    widths: PaintEdgeSizes,
    rect_width: f32,
    rect_height: f32,
) -> PaintEdgeSizes {
    fn finite_nonnegative(value: f32) -> f32 {
        if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        }
    }

    let mut widths = PaintEdgeSizes::new(
        finite_nonnegative(widths.top),
        finite_nonnegative(widths.right),
        finite_nonnegative(widths.bottom),
        finite_nonnegative(widths.left),
    );
    let horizontal = widths.left + widths.right;
    if horizontal > rect_width && horizontal > 0.0 {
        let scale = rect_width / horizontal;
        widths.left *= scale;
        widths.right *= scale;
    }
    let vertical = widths.top + widths.bottom;
    if vertical > rect_height && vertical > 0.0 {
        let scale = rect_height / vertical;
        widths.top *= scale;
        widths.bottom *= scale;
    }
    widths
}

fn to_backend_rect(rect: PaintRect) -> Option<Rect> {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || rect.width <= 0.0
        || rect.height <= 0.0
    {
        return None;
    }

    Some(Rect::new(
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.x) + f64::from(rect.width),
        f64::from(rect.y) + f64::from(rect.height),
    ))
}

fn to_backend_color(color: PaintColor) -> Color {
    fn sanitize(component: f32) -> f32 {
        if component.is_finite() {
            component.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    Color::new([
        sanitize(color.red),
        sanitize(color.green),
        sanitize(color.blue),
        sanitize(color.alpha),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_layout::{PaintFilter, PaintViewport};

    fn snapshot(width: u32, height: u32, scale: f32) -> PaintSnapshot {
        PaintSnapshot::new(PaintViewport::new(width, height, scale), PaintColor::WHITE)
    }

    #[test]
    fn rejects_zero_dimensions_before_backend_creation() {
        assert_eq!(
            raster_snapshot(&snapshot(0, 2, 1.0)),
            Err(PaintError::InvalidCaptureSurfaceDimensions {
                width: 0.0,
                height: 2.0,
            })
        );
        assert_eq!(
            raster_snapshot(&snapshot(2, 0, 1.0)),
            Err(PaintError::InvalidCaptureSurfaceDimensions {
                width: 2.0,
                height: 0.0,
            })
        );
    }

    #[test]
    fn rejects_dimensions_above_vello_cpu_limit() {
        assert!(matches!(
            raster_snapshot(&snapshot(u32::from(u16::MAX) + 1, 1, 1.0)),
            Err(PaintError::BackendDimensionLimit { .. })
        ));
        assert!(matches!(
            raster_snapshot(&snapshot(1, u32::from(u16::MAX) + 1, 1.0)),
            Err(PaintError::BackendDimensionLimit { .. })
        ));
        assert!(matches!(
            raster_snapshot(&snapshot(u32::from(u16::MAX), 1, 1.01)),
            Err(PaintError::BackendDimensionLimit { .. })
        ));
    }

    #[test]
    fn rejects_invalid_capture_device_scale() {
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(matches!(
                raster_snapshot(&snapshot(1, 1, scale)),
                Err(PaintError::InvalidCaptureDeviceScale { .. })
            ));
        }
    }

    #[test]
    fn rejects_surface_over_pixel_budget_before_backend_creation() {
        assert!(matches!(
            raster_snapshot(&snapshot(4_097, 4_096, 1.0)),
            Err(PaintError::PixelBudgetExceeded {
                width: 4_097,
                height: 4_096,
                ..
            })
        ));
    }

    #[test]
    fn rejects_paint_work_over_the_nested_resource_budget() {
        assert_eq!(
            validate_paint_operation_budget(MAX_PAINT_OPERATIONS + 1),
            Err(PaintError::PaintOperationBudgetExceeded {
                operations: MAX_PAINT_OPERATIONS + 1,
                max_operations: MAX_PAINT_OPERATIONS,
            })
        );
    }

    #[test]
    fn rejects_nested_filters_over_transient_surface_budget() {
        let mut snapshot = snapshot(4_096, 4_096, 1.0);
        for _ in 0..4 {
            snapshot.push_fragment(PaintFragment::PushLayer {
                opacity: 1.0,
                blend_mode: PaintBlendMode::Normal,
                composite: PaintCompositeMode::SrcOver,
                clip: PaintShape::Rect(PaintRect::new(0.0, 0.0, 4_096.0, 4_096.0)),
                transform: PaintTransform2D::IDENTITY,
                filter: Some(PaintFilter::Brightness(1.0)),
            });
        }
        for _ in 0..4 {
            snapshot.push_fragment(PaintFragment::PopLayer);
        }

        assert!(matches!(
            raster_snapshot(&snapshot),
            Err(PaintError::TransientRasterBudgetExceeded {
                required_bytes: 335_544_320,
                max_bytes: MAX_TRANSIENT_RASTER_BYTES,
            })
        ));
    }

    #[test]
    fn backend_gradient_stops_sanitize_public_snapshot_values() {
        let red = PaintColor::new(1.0, 0.0, 0.0, 1.0);
        let blue = PaintColor::new(0.0, 0.0, 1.0, 1.0);
        let single = backend_gradient_stops(&[moli_layout::PaintGradientStop {
            offset: f32::NAN,
            color: red,
        }]);
        assert_eq!(single.len(), 2);
        assert_eq!((single[0].0, single[1].0), (0.0, 1.0));

        let malformed = backend_gradient_stops(&[
            moli_layout::PaintGradientStop {
                offset: -2.0,
                color: red,
            },
            moli_layout::PaintGradientStop {
                offset: 0.75,
                color: blue,
            },
            moli_layout::PaintGradientStop {
                offset: 0.25,
                color: red,
            },
            moli_layout::PaintGradientStop {
                offset: f32::INFINITY,
                color: blue,
            },
        ]);
        assert_eq!(
            malformed.iter().map(|stop| stop.0).collect::<Vec<_>>(),
            [0.0, 0.75, 0.75, 0.75]
        );
    }

    #[test]
    fn rejects_excessive_layer_depth_before_backend_creation() {
        let mut snapshot = snapshot(1, 1, 1.0);
        for _ in 0..=MAX_PAINT_LAYER_DEPTH {
            snapshot.push_fragment(PaintFragment::push_clip(PaintRect::new(0.0, 0.0, 1.0, 1.0)));
        }
        assert_eq!(
            raster_snapshot(&snapshot),
            Err(PaintError::PaintLayerDepthBudgetExceeded {
                depth: MAX_PAINT_LAYER_DEPTH + 1,
                max_depth: MAX_PAINT_LAYER_DEPTH,
            })
        );
    }

    #[test]
    fn device_pixel_ratio_scales_output_and_fragments() {
        let mut snapshot = snapshot(2, 1, 2.0);
        snapshot.push_fragment(PaintFragment::solid_rect(
            PaintRect::new(1.0, 0.0, 1.0, 1.0),
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
        ));

        let image = raster_snapshot(&snapshot).expect("rasterization should succeed");
        assert_eq!((image.width, image.height), (4, 2));
        assert_eq!(pixel(&image, 0, 0), [255, 255, 255, 255]);
        assert_eq!(pixel(&image, 3, 1), [255, 0, 0, 255]);
    }

    #[test]
    fn synthetic_oblique_angles_preserve_css_direction_after_the_glyph_y_flip() {
        let right_leaning = raster_synthetic_oblique_box(14.0);
        let right_top = ink_centroid(&right_leaning, 15);
        let right_bottom = ink_centroid(&right_leaning, 45);
        assert!(
            right_top > right_bottom + 5.0,
            "positive oblique angle must move the glyph top right: top={right_top}, bottom={right_bottom}"
        );

        let left_leaning = raster_synthetic_oblique_box(-14.0);
        let left_top = ink_centroid(&left_leaning, 15);
        let left_bottom = ink_centroid(&left_leaning, 45);
        assert!(
            left_top + 5.0 < left_bottom,
            "negative oblique angle must move the glyph top left: top={left_top}, bottom={left_bottom}"
        );
    }

    #[test]
    fn snapshot_image_resource_rasterizes_without_copying_the_owned_pixels() {
        let pixels = std::sync::Arc::new(
            moli_image::RgbaImage::try_new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap(),
        );
        let mut snapshot = snapshot(4, 2, 1.0);
        let image = snapshot.add_image(pixels.clone());
        snapshot.push_fragment(PaintFragment::Image(PaintImage {
            image,
            destination: PaintRect::new(0.0, 0.0, 4.0, 2.0),
            sampling: PaintImageSampling::Nearest,
            transform: PaintTransform2D::IDENTITY,
        }));

        assert_eq!(std::sync::Arc::strong_count(&pixels), 2);
        let rendered = raster_snapshot(&snapshot).expect("image fragment should rasterize");
        assert_eq!(pixel(&rendered, 0, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&rendered, 1, 1), [255, 0, 0, 255]);
        assert_eq!(pixel(&rendered, 2, 0), [0, 255, 0, 255]);
        assert_eq!(pixel(&rendered, 3, 1), [0, 255, 0, 255]);
        drop(snapshot);
        assert_eq!(
            std::sync::Arc::strong_count(&pixels),
            1,
            "one-shot snapshot drop must release its shared image reference"
        );
    }

    #[test]
    fn snapshot_svg_resource_rasterizes_at_its_resolved_destination_size() {
        let svg = Arc::new(
            moli_image::decode_svg_image(
                br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1" viewBox="0 0 2 1"><rect width="1" height="1" fill="#ff0000"/><rect x="1" width="1" height="1" fill="#00ff00"/></svg>"##,
            )
            .expect("fixture SVG should parse"),
        );
        let mut snapshot = snapshot(20, 10, 1.0);
        let image = snapshot.add_svg_image(svg.clone());
        snapshot.push_fragment(PaintFragment::SvgImage(PaintSvgImage {
            image,
            destination: PaintRect::new(0.0, 0.0, 20.0, 10.0),
            transform: PaintTransform2D::IDENTITY,
        }));

        assert_eq!(Arc::strong_count(&svg), 2);
        let rendered = raster_snapshot(&snapshot).expect("SVG fragment should rasterize");
        assert_eq!(pixel(&rendered, 2, 5), [255, 0, 0, 255]);
        assert_eq!(pixel(&rendered, 17, 5), [0, 255, 0, 255]);
        drop(snapshot);
        assert_eq!(Arc::strong_count(&svg), 1);
    }

    #[test]
    fn fractional_svg_image_rows_share_pixel_snapped_boundaries() {
        let svg = Arc::new(
            moli_image::decode_svg_image(
                br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 12"><rect width="96" height="12" fill="#008000"/></svg>"##,
            )
            .expect("fixture SVG should parse"),
        );
        let mut snapshot = PaintSnapshot::new(
            PaintViewport::new(100, 101, 1.0),
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
        );
        let image = snapshot.add_svg_image(svg);
        for row in 0..8 {
            snapshot.push_fragment(PaintFragment::SvgImage(PaintSvgImage {
                image,
                destination: PaintRect::new(0.0, row as f32 * 12.5, 100.0, 12.5),
                transform: PaintTransform2D::IDENTITY,
            }));
        }

        let rendered = raster_snapshot(&snapshot).expect("SVG rows should rasterize");
        for y in 0..100 {
            for x in 0..100 {
                assert_eq!(
                    pixel(&rendered, x, y),
                    [0, 128, 0, 255],
                    "unexpected image coverage at ({x}, {y})"
                );
            }
        }
        for x in 0..100 {
            assert_eq!(
                pixel(&rendered, x, 100),
                [255, 0, 0, 255],
                "SVG content escaped its snapped destination at ({x}, 100)"
            );
        }
    }

    #[test]
    fn missing_snapshot_image_resource_is_rejected_before_backend_creation() {
        let mut snapshot = snapshot(1, 1, 1.0);
        snapshot.push_fragment(PaintFragment::Image(PaintImage {
            image: moli_layout::PaintImageId::from_index(0),
            destination: PaintRect::new(0.0, 0.0, 1.0, 1.0),
            sampling: PaintImageSampling::Linear,
            transform: PaintTransform2D::IDENTITY,
        }));

        assert_eq!(
            raster_snapshot(&snapshot),
            Err(PaintError::MissingImageResource { index: 0 })
        );
    }

    #[test]
    fn missing_snapshot_svg_resource_is_rejected_before_backend_creation() {
        let mut snapshot = snapshot(1, 1, 1.0);
        snapshot.push_fragment(PaintFragment::SvgImage(PaintSvgImage {
            image: moli_layout::PaintSvgImageId::from_index(0),
            destination: PaintRect::new(0.0, 0.0, 1.0, 1.0),
            transform: PaintTransform2D::IDENTITY,
        }));

        assert_eq!(
            raster_snapshot(&snapshot),
            Err(PaintError::MissingSvgImageResource { index: 0 })
        );
    }

    #[test]
    fn border_edges_are_rasterized_inside_the_border_box() {
        let red = PaintColor::new(1.0, 0.0, 0.0, 1.0);
        let mut snapshot = snapshot(8, 8, 1.0);
        snapshot.push_fragment(PaintFragment::border(
            PaintRect::new(1.0, 1.0, 6.0, 6.0),
            PaintEdgeSizes::new(1.0, 1.0, 1.0, 1.0),
            PaintBorderColors::all(red),
        ));

        let image = raster_snapshot(&snapshot).unwrap();
        assert_eq!(pixel(&image, 3, 1), [255, 0, 0, 255]);
        assert_eq!(pixel(&image, 1, 3), [255, 0, 0, 255]);
        assert_eq!(pixel(&image, 3, 3), [255, 255, 255, 255]);
    }

    #[test]
    fn fractional_box_borders_snap_before_rasterization() {
        let green = PaintColor::new(0.0, 1.0, 0.0, 1.0);
        let gray = PaintColor::new(128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0);
        let rect = PaintRect::new(1.09375, 1.0, 6.0, 6.0);
        let mut snapshot = snapshot(8, 8, 1.0);
        snapshot.push_fragment(PaintFragment::solid_rect(rect, green));
        snapshot.push_fragment(PaintFragment::border(
            rect,
            PaintEdgeSizes::new(1.0, 1.0, 1.0, 1.0),
            PaintBorderColors::all(gray),
        ));

        let image = raster_snapshot(&snapshot).unwrap();
        for coordinate in 1..7 {
            assert_eq!(pixel(&image, 1, coordinate), [128, 128, 128, 255]);
            assert_eq!(pixel(&image, 6, coordinate), [128, 128, 128, 255]);
        }
    }

    #[test]
    fn rectangular_clip_scopes_bound_following_fragments() {
        let mut snapshot = snapshot(4, 2, 1.0);
        snapshot.push_fragment(PaintFragment::push_clip(PaintRect::new(0.0, 0.0, 2.0, 2.0)));
        snapshot.push_fragment(PaintFragment::solid_rect(
            PaintRect::new(0.0, 0.0, 4.0, 2.0),
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
        ));
        snapshot.push_fragment(PaintFragment::PopLayer);

        let image = raster_snapshot(&snapshot).unwrap();
        assert_eq!(pixel(&image, 1, 1), [255, 0, 0, 255]);
        assert_eq!(pixel(&image, 3, 1), [255, 255, 255, 255]);
    }

    #[test]
    fn color_filter_layers_are_executed_by_the_one_shot_rgba_fallback() {
        let mut snapshot = snapshot(2, 2, 1.0);
        snapshot.push_fragment(PaintFragment::PushLayer {
            opacity: 1.0,
            blend_mode: PaintBlendMode::Normal,
            composite: PaintCompositeMode::SrcOver,
            clip: PaintShape::Rect(PaintRect::new(0.0, 0.0, 2.0, 2.0)),
            transform: PaintTransform2D::IDENTITY,
            filter: Some(PaintFilter::Brightness(0.0)),
        });
        snapshot.push_fragment(PaintFragment::solid_rect(
            PaintRect::new(0.0, 0.0, 2.0, 2.0),
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
        ));
        snapshot.push_fragment(PaintFragment::PopLayer);

        let image = raster_snapshot(&snapshot).unwrap();
        let pixel = pixel(&image, 1, 1);
        assert!(pixel[0] <= 2 && pixel[1] <= 2 && pixel[2] <= 2);
        assert_eq!(pixel[3], 255);
    }

    #[test]
    fn backend_filter_handles_inner_clip_layers_without_panicking() {
        let mut snapshot = snapshot(100, 100, 1.0);
        snapshot.push_fragment(PaintFragment::PushLayer {
            opacity: 1.0,
            blend_mode: PaintBlendMode::Normal,
            composite: PaintCompositeMode::SrcOver,
            clip: PaintShape::Rect(PaintRect::new(25.0, 25.0, 50.0, 50.0)),
            transform: PaintTransform2D::IDENTITY,
            filter: Some(PaintFilter::Blur(10.0)),
        });
        snapshot.push_fragment(PaintFragment::push_clip(PaintRect::new(
            30.0, 30.0, 40.0, 40.0,
        )));
        snapshot.push_fragment(PaintFragment::solid_rect(
            PaintRect::new(0.0, 0.0, 100.0, 100.0),
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
        ));
        snapshot.push_fragment(PaintFragment::PopLayer);
        snapshot.push_fragment(PaintFragment::PopLayer);

        let image = raster_snapshot(&snapshot).expect("nested clipped filter must not panic");
        let center = pixel(&image, 50, 50);
        let blurred_edge = pixel(&image, 27, 50);
        assert_eq!(center[0], 255);
        assert!(center[1] < 128 && center[1] == center[2]);
        assert_eq!(center[3], 255);
        assert_eq!(blurred_edge[0], 255);
        assert!(blurred_edge[1] > 0 && blurred_edge[1] < 255);
        assert_eq!(blurred_edge[1], blurred_edge[2]);
        assert!(center[1] < blurred_edge[1]);
        assert_eq!(pixel(&image, 10, 10), [255, 255, 255, 255]);
    }

    #[test]
    fn malformed_clip_and_font_references_are_structured_errors() {
        let mut pop = snapshot(2, 2, 1.0);
        pop.push_fragment(PaintFragment::PopLayer);
        assert_eq!(
            raster_snapshot(&pop),
            Err(PaintError::LayerStackUnderflow { fragment_index: 0 })
        );

        let mut push = snapshot(2, 2, 1.0);
        push.push_fragment(PaintFragment::push_clip(PaintRect::new(0.0, 0.0, 1.0, 1.0)));
        assert_eq!(
            raster_snapshot(&push),
            Err(PaintError::UnclosedLayerStack { depth: 1 })
        );

        let mut glyph = snapshot(2, 2, 1.0);
        glyph.push_fragment(PaintFragment::GlyphRun(moli_layout::PaintGlyphRun {
            font: moli_layout::PaintFontId::from_index(0),
            font_size: 12.0,
            normalized_coords: Vec::new(),
            color: PaintColor::BLACK,
            glyph_skew_radians: None,
            glyph_embolden: moli_layout::PaintPoint::ZERO,
            glyphs: vec![moli_layout::PaintGlyph {
                id: 1,
                x: 0.0,
                y: 1.0,
            }],
            transform: moli_layout::PaintTransform2D::IDENTITY,
        }));
        assert_eq!(
            raster_snapshot(&glyph),
            Err(PaintError::MissingFontResource { index: 0 })
        );
    }

    fn raster_synthetic_oblique_box(skew_degrees: f32) -> RasterImage {
        let mut snapshot = snapshot(80, 60, 1.0);
        let font = peniko::FontData::new(
            Blob::from(
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../moli-layout/tests/fixtures/moli-ahem.ttf"
                ))
                .to_vec(),
            ),
            0,
        );
        let font = snapshot.intern_font(&font);
        snapshot.push_fragment(PaintFragment::GlyphRun(moli_layout::PaintGlyphRun {
            font,
            font_size: 50.0,
            normalized_coords: Vec::new(),
            color: PaintColor::BLACK,
            glyph_skew_radians: Some(skew_degrees.to_radians()),
            glyph_embolden: moli_layout::PaintPoint::ZERO,
            // The fixture's .notdef glyph is a deterministic 600 x 800 box.
            glyphs: vec![moli_layout::PaintGlyph {
                id: 0,
                x: 20.0,
                y: 50.0,
            }],
            transform: moli_layout::PaintTransform2D::IDENTITY,
        }));
        raster_snapshot(&snapshot).expect("synthetic oblique fixture should rasterize")
    }

    fn ink_centroid(image: &RasterImage, y: u32) -> f32 {
        let (weighted_x, coverage) = (0..image.width).fold((0.0, 0.0), |acc, x| {
            let ink = f32::from(255 - pixel(image, x, y)[0]);
            (acc.0 + x as f32 * ink, acc.1 + ink)
        });
        assert!(coverage > 0.0, "fixture row {y} must contain glyph ink");
        weighted_x / coverage
    }

    fn pixel(image: &RasterImage, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * image.width + x) * 4) as usize;
        image.rgba[offset..offset + 4]
            .try_into()
            .expect("one RGBA pixel")
    }
}
