// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Narrowly adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8:
// - examples/screenshot.rs
// - packages/blitz-paint/src/render.rs
// - crates/anyrender_svg (parsed SVG tree execution)
// The source project is licensed MIT OR Apache-2.0.

//! Raster resource and parsed SVG commands with shared destination geometry.

use crate::geometry::{device_transform, to_backend_rect};
use anyrender::PaintScene;
use moli_layout::{
    PaintImage, PaintImageSampling, PaintRect, PaintSnapshot, PaintSvgImage, pixel_snap_paint_rect,
};
use peniko::kurbo::Affine;
use peniko::{
    Blob, Extend, Fill, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality,
    ImageSampler,
};

pub(super) fn paint_image(
    scene: &mut impl PaintScene,
    snapshot: &PaintSnapshot,
    image: &PaintImage,
    scale: f64,
) {
    let Some(resource) = snapshot.image(image.image) else {
        return;
    };
    let destination = if image.repeat_bounds.is_some() {
        // Snapping one tile would change its period and accumulate phase error
        // over the fill. Only the finite fill bounds use box pixel snapping.
        if to_backend_rect(image.destination).is_none() {
            return;
        }
        image.destination
    } else {
        let Some(destination) = pixel_snapped_image_destination(image.destination) else {
            return;
        };
        destination
    };
    if resource.image.width == 0 || resource.image.height == 0 {
        return;
    }
    let quality = match image.sampling {
        PaintImageSampling::Nearest => ImageQuality::Low,
        PaintImageSampling::Linear => ImageQuality::Medium,
    };
    let extend = if image.repeat_bounds.is_some() {
        Extend::Repeat
    } else {
        Extend::Pad
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
            x_extend: extend,
            y_extend: extend,
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
    if let Some(bounds) = image.repeat_bounds {
        let Some(bounds) = pixel_snap_paint_rect(bounds).and_then(to_backend_rect) else {
            return;
        };
        scene.fill(
            Fill::NonZero,
            device_transform(image.transform, scale),
            brush.as_ref(),
            Some(image_to_destination),
            &bounds,
        );
    } else {
        scene.draw_image(
            brush.as_ref(),
            device_transform(image.transform, scale) * image_to_destination,
        );
    }
}

pub(super) fn paint_svg_image(
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

    paint_svg_tree(scene, &resource.image, transform);
}

pub(crate) fn paint_svg_tree(
    scene: &mut impl PaintScene,
    image: &moli_image::SvgImage,
    transform: Affine,
) {
    // Blitz's SVG executor reports unsupported nodes by painting red boxes.
    // Paint output must not invent visible fallback pixels, so retain
    // supported vector content and silently omit unsupported masks, filters,
    // patterns, or embedded raster nodes until their backends are wired.
    anyrender_svg::render_svg_tree_with(scene, image.tree(), transform, &mut |_scene, _node| {});
}

fn pixel_snapped_image_destination(destination: PaintRect) -> Option<PaintRect> {
    // Image geometry remains fractional for layout and CSSOM. Snapshot
    // projection has already applied ordinary paint offsets while preserving
    // CSS/scroll property transforms; drawing only snaps the image quad
    // in that pre-transform paint space. Adjacent fractional images therefore
    // share one pixel boundary, matching Blink's
    // ToPixelSnappedRect/SnapSizeToPixel contract.
    pixel_snap_paint_rect(destination)
}
