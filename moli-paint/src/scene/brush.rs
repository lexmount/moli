// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Narrowly adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8:
// - examples/screenshot.rs
// - packages/blitz-paint/src/render.rs
// - crates/anyrender_svg (parsed SVG tree execution)
// The source project is licensed MIT OR Apache-2.0.

//! Fills, strokes and gradient conversion.

use super::to_backend_color;
use crate::geometry::{
    device_transform, finite_nonnegative, finite_or_zero, to_backend_cap, to_backend_path,
    to_backend_transform,
};
use anyrender::PaintScene;
use moli_layout::{
    PaintBrush, PaintGradientAlphaSpace, PaintGradientColorSpace, PaintGradientExtend, PaintGradientHueDirection,
    PaintLineJoin, PaintStroke,
};
use peniko::color::{ColorSpaceTag, HueDirection};
use peniko::kurbo::{Affine, BezPath, Join, Stroke};
use peniko::{Color, Extend, Fill, Gradient, InterpolationAlphaSpace};

pub(super) fn paint_stroke(scene: &mut impl PaintScene, stroke: &PaintStroke, scale: f64) {
    let width = f64::from(finite_nonnegative(stroke.width));
    if width <= 0.0 {
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
    let (brush, brush_transform) = backend_brush(&stroke.brush);
    scene.stroke(
        &backend_stroke,
        device_transform(stroke.transform, scale),
        peniko::BrushRef::from(&brush),
        brush_transform,
        &path,
    );
}

pub(super) fn paint_fill(
    scene: &mut impl PaintScene,
    shape: &BezPath,
    brush: &PaintBrush,
    transform: Affine,
    rule: Fill,
) {
    let (brush, brush_transform) = backend_brush(brush);
    scene.fill(
        rule,
        transform,
        peniko::BrushRef::from(&brush),
        brush_transform,
        shape,
    );
}

fn backend_brush(brush: &PaintBrush) -> (peniko::Brush, Option<Affine>) {
    match brush {
        PaintBrush::Solid(color) => (to_backend_color(*color).into(), None),
        PaintBrush::LinearGradient(gradient) => {
            let gradient_brush = Gradient::new_linear(
                (f64::from(gradient.start.x), f64::from(gradient.start.y)),
                (f64::from(gradient.end.x), f64::from(gradient.end.y)),
            )
            .with_extend(to_backend_extend(gradient.extend))
            .with_interpolation_cs(to_backend_gradient_color_space(
                gradient.interpolation.color_space,
            ))
            .with_interpolation_alpha_space(backend_gradient_alpha_space(
                gradient.interpolation.alpha_space,
            ))
            .with_hue_direction(to_backend_gradient_hue_direction(
                gradient.interpolation.hue_direction,
            ))
            .with_stops(backend_gradient_stops(&gradient.stops).as_slice());
            (
                gradient_brush.into(),
                Some(to_backend_transform(gradient.transform)),
            )
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
            .with_interpolation_alpha_space(backend_gradient_alpha_space(
                gradient.interpolation.alpha_space,
            ))
            .with_hue_direction(to_backend_gradient_hue_direction(
                gradient.interpolation.hue_direction,
            ))
            .with_stops(backend_gradient_stops(&gradient.stops).as_slice());
            (
                gradient_brush.into(),
                Some(to_backend_transform(gradient.transform)),
            )
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
            .with_interpolation_alpha_space(backend_gradient_alpha_space(
                gradient.interpolation.alpha_space,
            ))
            .with_hue_direction(to_backend_gradient_hue_direction(
                gradient.interpolation.hue_direction,
            ))
            .with_stops(backend_gradient_stops(&gradient.stops).as_slice());
            (
                gradient_brush.into(),
                Some(to_backend_transform(gradient.transform)),
            )
        }
    }
}

fn backend_gradient_alpha_space(space: PaintGradientAlphaSpace) -> InterpolationAlphaSpace {
    match space {
        PaintGradientAlphaSpace::Premultiplied => InterpolationAlphaSpace::Premultiplied,
        PaintGradientAlphaSpace::Unpremultiplied => InterpolationAlphaSpace::Unpremultiplied,
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

fn to_backend_extend(extend: PaintGradientExtend) -> Extend {
    match extend {
        PaintGradientExtend::Pad => Extend::Pad,
        PaintGradientExtend::Repeat => Extend::Repeat,
        PaintGradientExtend::Reflect => Extend::Reflect,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_layout::PaintColor;
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
}
