// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Narrowly adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8:
// - examples/screenshot.rs
// - packages/blitz-paint/src/render.rs
// - crates/anyrender_svg (parsed SVG tree execution)
// The source project is licensed MIT OR Apache-2.0.

//! Glyph and decoration commands, including visual shadow replays.

use super::to_backend_color;
use crate::geometry::{device_transform, finite_nonnegative, finite_or_zero};
use anyrender::PaintScene;
use anyrender::{Filter, filters::FilterEffect};
use moli_layout::{
    PaintColor, PaintSnapshot, PaintTextDecoration, PaintTextDecorationStyle, PaintTextShadow,
    PaintTextStroke,
};
use peniko::kurbo::{Affine, BezPath, Cap, Join, Rect, Stroke, Vec2};
use peniko::{Fill, Mix};
use std::sync::Arc;

pub(super) fn paint_text_shadow(
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

pub(super) fn paint_glyph_run(
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
    let transform = device_transform(run.transform, scale);
    let glyphs = || {
        run.glyphs.iter().map(|glyph| anyrender::Glyph {
            id: glyph.id,
            x: glyph.x,
            y: glyph.y,
        })
    };
    let stroke = run.stroke.and_then(|stroke| {
        let width = finite_nonnegative(stroke.width);
        (width > 0.0 && stroke.color.alpha > 0.0).then_some((stroke, width))
    });
    // Fill and stroke prepare the same expanded outline. Stroke width remains
    // independent of synthetic weight, preserving both sides of the contour.
    let paint_stroke = |scene: &mut _| {
        if let Some((stroke, width)) = stroke {
            PaintScene::draw_glyphs(
                scene,
                &font.font,
                run.font_size,
                false,
                &run.normalized_coords,
                glyph_embolden,
                &Stroke::new(f64::from(width))
                    .with_join(Join::Miter)
                    .with_miter_limit(f64::from(PaintTextStroke::MITER_LIMIT)),
                to_backend_color(stroke.color),
                1.0,
                transform,
                glyph_transform,
                glyphs(),
            );
        }
    };
    let stroke_first = stroke.is_some_and(|(stroke, _)| stroke.paint_first);
    if stroke_first {
        paint_stroke(scene);
    }
    scene.draw_glyphs(
        &font.font,
        run.font_size,
        glyph_embolden == Vec2::ZERO,
        &run.normalized_coords,
        glyph_embolden,
        Fill::NonZero,
        to_backend_color(color),
        1.0,
        transform,
        glyph_transform,
        glyphs(),
    );
    if !stroke_first {
        paint_stroke(scene);
    }
}

pub(super) fn paint_text_decoration(
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
