// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Narrowly adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8:
// - examples/screenshot.rs
// - packages/blitz-paint/src/render.rs
// - crates/anyrender_svg (parsed SVG tree execution)
// The source project is licensed MIT OR Apache-2.0.

//! Lower owned paint fragments into AnyRender commands for CPU and PDF consumers.
//! This module records/draws primitives; the consumer chooses whether to rasterize them.

mod border;
mod brush;
mod effects;
mod image;
mod text;

use crate::geometry::{device_transform, finite_or_zero, to_backend_shape};
use anyrender::PaintScene;
use border::paint_border;
use brush::{paint_fill, paint_stroke};
pub(crate) use effects::backend_filter;
use effects::paint_box_shadow;
use image::{paint_image, paint_svg_image};
pub(crate) use image::paint_svg_tree;
use moli_layout::{PaintBlendMode, PaintColor, PaintCompositeMode, PaintFragment, PaintSnapshot};
use peniko::{BlendMode, Color, Compose, Mix};
use std::sync::Arc;
use text::{paint_glyph_run, paint_text_decoration, paint_text_shadow};

pub(crate) fn paint_fragment(
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
            fill_rule,
        } => {
            let shape = to_backend_shape(shape);
            let rule = match fill_rule {
                moli_layout::PaintFillRule::NonZero => peniko::Fill::NonZero,
                moli_layout::PaintFillRule::EvenOdd => peniko::Fill::EvenOdd,
            };
            paint_fill(
                scene,
                &shape,
                brush,
                device_transform(*transform, scale),
                rule,
            );
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

pub(crate) fn to_backend_blend_mode(
    mode: PaintBlendMode,
    composite: PaintCompositeMode,
) -> BlendMode {
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

pub(crate) fn sanitize_unit(value: f32) -> f32 {
    finite_or_zero(value).clamp(0.0, 1.0)
}

pub(crate) fn to_backend_color(color: PaintColor) -> Color {
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
