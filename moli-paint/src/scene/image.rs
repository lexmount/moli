// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Narrowly adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8:
// - examples/screenshot.rs
// - packages/blitz-paint/src/render.rs
// - crates/anyrender_svg (parsed SVG tree execution)
// The source project is licensed MIT OR Apache-2.0.

//! Raster resource and parsed SVG commands with shared destination geometry.

use crate::geometry::device_transform;
use anyrender::PaintScene;
use moli_layout::{PaintImage, PaintImageSampling, PaintSnapshot, PaintSvgImage};
use peniko::kurbo::Affine;
use peniko::{
    Blob, Extend, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality, ImageSampler,
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
    let Some(destination) = snapshot.pixel_grid().snap_rect(image.destination) else {
        return;
    };
    if resource.image.width == 0 || resource.image.height == 0 {
        return;
    }
    let brush = raster_brush(&resource.image, image.sampling, Extend::Pad, Extend::Pad);
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

pub(super) fn raster_brush(
    image: &std::sync::Arc<moli_image::RgbaImage>,
    sampling: PaintImageSampling,
    x_extend: Extend,
    y_extend: Extend,
) -> ImageBrush {
    ImageBrush {
        image: ImageData {
            data: Blob::new(image.clone()),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: image.width,
            height: image.height,
        },
        sampler: ImageSampler {
            x_extend,
            y_extend,
            quality: match sampling {
                PaintImageSampling::Nearest => ImageQuality::Low,
                PaintImageSampling::Linear => ImageQuality::Medium,
            },
            alpha: 1.0,
        },
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
    let Some(destination) = snapshot.pixel_grid().snap_rect(image.destination) else {
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
    draw_svg(
        scene,
        &resource.image,
        device_transform(image.transform, scale) * image_to_destination,
    );
}

pub(super) fn draw_svg(
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
