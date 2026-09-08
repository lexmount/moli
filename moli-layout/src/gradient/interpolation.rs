//! Maps CSS color interpolation into the backend-neutral paint contract.
//!
//! Peniko represents most Stylo color spaces directly. The two methods it
//! cannot represent are expanded into a bounded number of resolved sRGB stops
//! here, while the owning one-shot layout snapshot still has the original
//! `AbsoluteColor` values and Stylo interpolation implementation available.

use style::{
    color::{
        AbsoluteColor, ColorSpace,
        mix::{ColorInterpolationMethod, ColorMixItem, HueInterpolationMethod, mix_many},
    },
    values::generics::color::ColorMixFlags,
};

use super::stops::AbsoluteGradientStop;
use crate::{
    PaintColor, PaintGradientColorSpace, PaintGradientHueDirection, PaintGradientInterpolation,
    PaintGradientStop, style::absolute_paint_color,
};

const UNSUPPORTED_INTERPOLATION_SAMPLE_STEPS: usize = 16;

pub(super) fn finalize_stops(
    stops: Vec<AbsoluteGradientStop>,
    interpolation_method: ColorInterpolationMethod,
    repeating: bool,
) -> (Vec<PaintGradientStop>, PaintGradientInterpolation) {
    let (mut stops, interpolation) =
        if let Some(interpolation) = paint_interpolation(interpolation_method) {
            (
                stops
                    .into_iter()
                    .map(|stop| PaintGradientStop {
                        offset: stop.offset.clamp(0.0, 1.0),
                        color: absolute_paint_color(stop.color),
                    })
                    .collect::<Vec<_>>(),
                interpolation,
            )
        } else {
            (
                sample_interpolation(stops, interpolation_method),
                PaintGradientInterpolation::default(),
            )
        };

    if stops.is_empty() {
        return (
            vec![
                PaintGradientStop {
                    offset: 0.0,
                    color: PaintColor::TRANSPARENT,
                },
                PaintGradientStop {
                    offset: 1.0,
                    color: PaintColor::TRANSPARENT,
                },
            ],
            interpolation,
        );
    }
    if stops.len() == 1 {
        let color = stops[0].color;
        stops = vec![
            PaintGradientStop { offset: 0.0, color },
            PaintGradientStop { offset: 1.0, color },
        ];
    } else if !repeating {
        if stops[0].offset > 0.0 {
            stops.insert(
                0,
                PaintGradientStop {
                    offset: 0.0,
                    color: stops[0].color,
                },
            );
        }
        if stops.last().is_some_and(|stop| stop.offset < 1.0) {
            let color = stops.last().expect("one gradient stop").color;
            stops.push(PaintGradientStop { offset: 1.0, color });
        }
    }
    (stops, interpolation)
}

fn sample_interpolation(
    stops: Vec<AbsoluteGradientStop>,
    interpolation_method: ColorInterpolationMethod,
) -> Vec<PaintGradientStop> {
    let Some(first) = stops.first().copied() else {
        return Vec::new();
    };
    let mut sampled = Vec::with_capacity(
        stops
            .len()
            .saturating_mul(UNSUPPORTED_INTERPOLATION_SAMPLE_STEPS),
    );
    sampled.push(PaintGradientStop {
        offset: first.offset.clamp(0.0, 1.0),
        color: absolute_paint_color(first.color),
    });
    for pair in stops.windows(2) {
        let left = pair[0];
        let right = pair[1];
        if right.offset <= left.offset {
            sampled.push(PaintGradientStop {
                offset: right.offset.clamp(0.0, 1.0),
                color: absolute_paint_color(right.color),
            });
            continue;
        }
        for step in 1..=UNSUPPORTED_INTERPOLATION_SAMPLE_STEPS {
            let progress = step as f32 / UNSUPPORTED_INTERPOLATION_SAMPLE_STEPS as f32;
            sampled.push(PaintGradientStop {
                offset: (left.offset + (right.offset - left.offset) * progress).clamp(0.0, 1.0),
                color: absolute_paint_color(interpolate_color(
                    left.color,
                    right.color,
                    progress,
                    interpolation_method,
                )),
            });
        }
    }
    sampled
}

fn paint_interpolation(method: ColorInterpolationMethod) -> Option<PaintGradientInterpolation> {
    let color_space = match method.space {
        ColorSpace::Srgb => PaintGradientColorSpace::Srgb,
        ColorSpace::SrgbLinear => PaintGradientColorSpace::LinearSrgb,
        ColorSpace::Hsl => PaintGradientColorSpace::Hsl,
        ColorSpace::Hwb => PaintGradientColorSpace::Hwb,
        ColorSpace::Lab => PaintGradientColorSpace::Lab,
        ColorSpace::Lch => PaintGradientColorSpace::Lch,
        ColorSpace::Oklab => PaintGradientColorSpace::Oklab,
        ColorSpace::Oklch => PaintGradientColorSpace::Oklch,
        ColorSpace::DisplayP3 => PaintGradientColorSpace::DisplayP3,
        ColorSpace::DisplayP3Linear => return None,
        ColorSpace::A98Rgb => PaintGradientColorSpace::A98Rgb,
        ColorSpace::ProphotoRgb => PaintGradientColorSpace::ProphotoRgb,
        ColorSpace::Rec2020 => PaintGradientColorSpace::Rec2020,
        ColorSpace::XyzD50 => PaintGradientColorSpace::XyzD50,
        ColorSpace::XyzD65 => PaintGradientColorSpace::XyzD65,
    };
    let hue_direction = match method.hue {
        HueInterpolationMethod::Shorter => PaintGradientHueDirection::Shorter,
        HueInterpolationMethod::Longer => PaintGradientHueDirection::Longer,
        HueInterpolationMethod::Increasing => PaintGradientHueDirection::Increasing,
        HueInterpolationMethod::Decreasing => PaintGradientHueDirection::Decreasing,
        HueInterpolationMethod::Specified => return None,
    };
    Some(PaintGradientInterpolation {
        color_space,
        hue_direction,
        ..Default::default()
    })
}

pub(super) fn interpolate_color(
    left: AbsoluteColor,
    right: AbsoluteColor,
    progress: f32,
    method: ColorInterpolationMethod,
) -> AbsoluteColor {
    mix_many(
        method,
        [
            ColorMixItem::new(left, 1.0 - progress),
            ColorMixItem::new(right, progress),
        ],
        ColorMixFlags::NORMALIZE_WEIGHTS,
    )
}
