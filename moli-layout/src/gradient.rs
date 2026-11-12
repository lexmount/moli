mod interpolation;
mod stops;

use style::{
    OwnedSlice,
    color::{AbsoluteColor, mix::ColorInterpolationMethod},
    values::{
        computed::{
            Angle, AngleOrPercentage, CSSPixelLength, Gradient as StyloGradient, LengthPercentage,
            LineDirection, Percentage,
        },
        generics::{
            NonNegative,
            color::GenericColor,
            image::{
                EndingShape, GenericCircle, GenericEllipse, GenericEndingShape, GenericGradient,
                GenericGradientItem, GradientFlags, ShapeExtent,
            },
            position::GenericPosition,
        },
        specified::{
            percentage::ToPercentage,
            position::{HorizontalPositionKeyword, VerticalPositionKeyword},
        },
    },
};

use self::{
    interpolation::finalize_stops,
    stops::{
        AbsoluteGradientStop, clamp_negative_radial_offsets, clip_stops_to_unit_domain,
        normalize_stops, resolve_stops, stops_require_normalization,
    },
};
use crate::{
    LayoutPoint, LayoutRect, LayoutTransform2D, PaintBrush, PaintConicGradient,
    PaintGradientExtend, PaintLinearGradient, PaintRadialGradient,
};

type GradientItem<T> = GenericGradientItem<GenericColor<Percentage>, T>;
type LinearGradient<'a> = (
    &'a LineDirection,
    &'a [GradientItem<LengthPercentage>],
    GradientFlags,
    ColorInterpolationMethod,
);
type RadialGradient<'a> = (
    &'a EndingShape<NonNegative<CSSPixelLength>, NonNegative<LengthPercentage>>,
    &'a GenericPosition<LengthPercentage, LengthPercentage>,
    &'a OwnedSlice<GradientItem<LengthPercentage>>,
    GradientFlags,
    ColorInterpolationMethod,
);
type ConicGradient<'a> = (
    &'a Angle,
    &'a GenericPosition<LengthPercentage, LengthPercentage>,
    &'a OwnedSlice<GradientItem<AngleOrPercentage>>,
    GradientFlags,
    ColorInterpolationMethod,
);

pub(crate) fn project_gradient(
    gradient: &StyloGradient,
    rect: LayoutRect,
    current_color: &AbsoluteColor,
) -> Option<PaintBrush> {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return None;
    }
    match gradient {
        GenericGradient::Linear {
            direction,
            color_interpolation_method,
            items,
            flags,
            ..
        } => linear_gradient(
            (direction, items, *flags, *color_interpolation_method),
            rect,
            current_color,
        ),
        GenericGradient::Radial {
            shape,
            position,
            color_interpolation_method,
            items,
            flags,
            ..
        } => radial_gradient(
            (shape, position, items, *flags, *color_interpolation_method),
            rect,
            current_color,
        ),
        GenericGradient::Conic {
            angle,
            position,
            color_interpolation_method,
            items,
            flags,
        } => conic_gradient(
            (angle, position, items, *flags, *color_interpolation_method),
            rect,
            current_color,
        ),
    }
}

fn linear_gradient(
    (direction, items, flags, interpolation_method): LinearGradient<'_>,
    rect: LayoutRect,
    current_color: &AbsoluteColor,
) -> Option<PaintBrush> {
    let center = LayoutPoint::new(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    let (mut start, mut end) = match direction {
        LineDirection::Angle(angle) => {
            let angle = -angle.radians64() + std::f64::consts::PI;
            let offset_length = f64::from(rect.width) * 0.5 * angle.sin().abs()
                + f64::from(rect.height) * 0.5 * angle.cos().abs();
            let offset_x = (angle.sin() * offset_length) as f32;
            let offset_y = (angle.cos() * offset_length) as f32;
            (
                LayoutPoint::new(center.x - offset_x, center.y - offset_y),
                LayoutPoint::new(center.x + offset_x, center.y + offset_y),
            )
        }
        LineDirection::Horizontal(horizontal) => {
            let left = LayoutPoint::new(rect.x, center.y);
            let right = LayoutPoint::new(rect.right(), center.y);
            match horizontal {
                HorizontalPositionKeyword::Right => (left, right),
                HorizontalPositionKeyword::Left => (right, left),
            }
        }
        LineDirection::Vertical(vertical) => {
            let top = LayoutPoint::new(center.x, rect.y);
            let bottom = LayoutPoint::new(center.x, rect.bottom());
            match vertical {
                VerticalPositionKeyword::Top => (bottom, top),
                VerticalPositionKeyword::Bottom => (top, bottom),
            }
        }
        LineDirection::Corner(horizontal, vertical) => {
            let (start_x, end_x) = match horizontal {
                HorizontalPositionKeyword::Right => (rect.x, rect.right()),
                HorizontalPositionKeyword::Left => (rect.right(), rect.x),
            };
            let (start_y, end_y) = match vertical {
                VerticalPositionKeyword::Top => (rect.bottom(), rect.y),
                VerticalPositionKeyword::Bottom => (rect.y, rect.bottom()),
            };
            (
                LayoutPoint::new(start_x, start_y),
                LayoutPoint::new(end_x, end_y),
            )
        }
    };
    let length = ((end.x - start.x).hypot(end.y - start.y)).max(f32::EPSILON);
    let repeating = flags.contains(GradientFlags::REPEATING);
    let absolute_stops = resolve_stops(
        items,
        CSSPixelLength::new(length),
        current_color,
        interpolation_method,
        |length, position| {
            position
                .to_percentage_of(length)
                .and_then(|percentage| percentage.to_percentage())
        },
    );
    let requires_normalization = stops_require_normalization(&absolute_stops, repeating);
    let normalized = normalize_stops(absolute_stops, requires_normalization, repeating);
    if normalized.normalized {
        let original_start = start;
        let dx = end.x - original_start.x;
        let dy = end.y - original_start.y;
        start = LayoutPoint::new(
            original_start.x + dx * normalized.first_offset,
            original_start.y + dy * normalized.first_offset,
        );
        end = LayoutPoint::new(
            original_start.x + dx * normalized.last_offset,
            original_start.y + dy * normalized.last_offset,
        );
    }
    let (stops, interpolation) = finalize_stops(normalized.stops, interpolation_method, repeating);
    Some(PaintBrush::LinearGradient(PaintLinearGradient {
        start,
        end,
        stops,
        extend: gradient_extend(repeating),
        interpolation,
        transform: LayoutTransform2D::IDENTITY,
    }))
}

fn radial_gradient(
    (shape, position, items, flags, interpolation_method): RadialGradient<'_>,
    rect: LayoutRect,
    current_color: &AbsoluteColor,
) -> Option<PaintBrush> {
    let center = resolve_position(position, rect);
    let left = (center.x - rect.x).max(0.0);
    let right = (rect.right() - center.x).max(0.0);
    let top = (center.y - rect.y).max(0.0);
    let bottom = (rect.bottom() - center.y).max(0.0);
    let (mut radius_x, mut radius_y) = match shape {
        GenericEndingShape::Circle(circle) => {
            let radius = match circle {
                GenericCircle::Extent(ShapeExtent::FarthestSide) => {
                    left.max(right).max(top.max(bottom))
                }
                GenericCircle::Extent(ShapeExtent::ClosestSide) => {
                    left.min(right).min(top.min(bottom))
                }
                GenericCircle::Extent(ShapeExtent::FarthestCorner) => [
                    left.hypot(top),
                    right.hypot(top),
                    right.hypot(bottom),
                    left.hypot(bottom),
                ]
                .into_iter()
                .fold(0.0_f32, f32::max),
                GenericCircle::Extent(ShapeExtent::ClosestCorner) => [
                    left.hypot(top),
                    right.hypot(top),
                    right.hypot(bottom),
                    left.hypot(bottom),
                ]
                .into_iter()
                .fold(f32::INFINITY, f32::min),
                GenericCircle::Extent(_) => 0.0,
                GenericCircle::Radius(radius) => radius.0.px(),
            };
            (radius, radius)
        }
        GenericEndingShape::Ellipse(ellipse) => match ellipse {
            GenericEllipse::Extent(ShapeExtent::FarthestSide) => (left.max(right), top.max(bottom)),
            GenericEllipse::Extent(ShapeExtent::ClosestSide) => (left.min(right), top.min(bottom)),
            GenericEllipse::Extent(ShapeExtent::FarthestCorner) => (
                left.max(right) * 2.0_f32.sqrt(),
                top.max(bottom) * 2.0_f32.sqrt(),
            ),
            GenericEllipse::Extent(ShapeExtent::ClosestCorner) => (
                left.min(right) * 2.0_f32.sqrt(),
                top.min(bottom) * 2.0_f32.sqrt(),
            ),
            GenericEllipse::Extent(_) => (0.0, 0.0),
            GenericEllipse::Radii(x, y) => (
                x.0.resolve(CSSPixelLength::new(rect.width)).px(),
                y.0.resolve(CSSPixelLength::new(rect.height)).px(),
            ),
        },
    };
    radius_x = radius_x.max(f32::EPSILON);
    radius_y = radius_y.max(f32::EPSILON);
    let repeating = flags.contains(GradientFlags::REPEATING);
    let mut absolute_stops = resolve_stops(
        items,
        CSSPixelLength::new(radius_x),
        current_color,
        interpolation_method,
        |length, position| {
            position
                .to_percentage_of(length)
                .and_then(|percentage| percentage.to_percentage())
        },
    );
    let requires_normalization = stops_require_normalization(&absolute_stops, repeating);
    if requires_normalization && !repeating {
        clamp_negative_radial_offsets(&mut absolute_stops, interpolation_method);
    }
    let mut normalized = normalize_stops(absolute_stops, requires_normalization, repeating);
    let mut start_radius = 0.0;
    let mut end_radius = 1.0;
    if normalized.normalized {
        start_radius = normalized.first_offset;
        end_radius = normalized.last_offset;
        if repeating && start_radius < 0.0 {
            let radius_span = end_radius - start_radius;
            if radius_span > f32::EPSILON {
                let shift = radius_span * (-start_radius / radius_span).ceil();
                start_radius += shift;
                end_radius += shift;
            }
        }
    } else if requires_normalization && !repeating {
        // Chromium represents this as two coincident radii. Vello's radial
        // shader requires a non-degenerate radius interval, so encode the
        // equivalent sharp transition without changing visible pixels.
        let transition_radius = normalized.first_offset.max(0.0);
        if transition_radius <= f32::EPSILON {
            if let Some(last) = normalized.stops.last().copied() {
                normalized.stops = vec![
                    AbsoluteGradientStop {
                        offset: 0.0,
                        color: last.color,
                    },
                    AbsoluteGradientStop {
                        offset: 1.0,
                        color: last.color,
                    },
                ];
            }
        } else {
            end_radius = transition_radius;
        }
    }
    let (stops, interpolation) = finalize_stops(normalized.stops, interpolation_method, repeating);
    Some(PaintBrush::RadialGradient(PaintRadialGradient {
        start_center: LayoutPoint::ZERO,
        start_radius,
        end_center: LayoutPoint::ZERO,
        end_radius,
        stops,
        extend: gradient_extend(repeating),
        interpolation,
        transform: LayoutTransform2D::new([
            f64::from(radius_x),
            0.0,
            0.0,
            f64::from(radius_y),
            f64::from(center.x),
            f64::from(center.y),
        ]),
    }))
}

fn conic_gradient(
    (angle, position, items, flags, interpolation_method): ConicGradient<'_>,
    rect: LayoutRect,
    current_color: &AbsoluteColor,
) -> Option<PaintBrush> {
    let center = resolve_position(position, rect);
    let repeating = flags.contains(GradientFlags::REPEATING);
    let absolute_stops = resolve_stops(
        items,
        CSSPixelLength::new(1.0),
        current_color,
        interpolation_method,
        |_length, position| match position {
            AngleOrPercentage::Angle(angle) => {
                Some(angle.radians() / (std::f64::consts::TAU as f32))
            }
            AngleOrPercentage::Percentage(percentage) => percentage.to_percentage(),
        },
    );
    let (absolute_stops, start_angle_radians, end_angle_radians) = if repeating {
        let normalized = normalize_stops(absolute_stops, true, true);
        let (start, end) = if normalized.normalized {
            (
                std::f32::consts::TAU * normalized.first_offset,
                std::f32::consts::TAU * normalized.last_offset,
            )
        } else {
            (0.0, std::f32::consts::TAU)
        };
        (normalized.stops, start, end)
    } else {
        // Vello sweep gradients derive their parameter from an angle in the
        // visible `[0, 2π)` turn. Clip non-repeating CSS stops to that turn
        // before applying the CSS `from` angle as a local brush transform;
        // passing negative or >1 stops directly changes the visible domain.
        (
            clip_stops_to_unit_domain(absolute_stops, interpolation_method),
            0.0,
            std::f32::consts::TAU,
        )
    };
    let (stops, interpolation) = finalize_stops(absolute_stops, interpolation_method, repeating);
    let transform = LayoutTransform2D::translation(center.x, center.y).concatenate(
        LayoutTransform2D::rotation(f64::from(angle.radians() - std::f32::consts::FRAC_PI_2)),
    );
    Some(PaintBrush::ConicGradient(PaintConicGradient {
        center: LayoutPoint::ZERO,
        start_angle_radians,
        end_angle_radians,
        stops,
        extend: gradient_extend(repeating),
        interpolation,
        transform,
    }))
}

fn resolve_position(
    position: &GenericPosition<LengthPercentage, LengthPercentage>,
    rect: LayoutRect,
) -> LayoutPoint {
    LayoutPoint::new(
        rect.x
            + position
                .horizontal
                .resolve(CSSPixelLength::new(rect.width))
                .px(),
        rect.y
            + position
                .vertical
                .resolve(CSSPixelLength::new(rect.height))
                .px(),
    )
}

fn gradient_extend(repeating: bool) -> PaintGradientExtend {
    if repeating {
        PaintGradientExtend::Repeat
    } else {
        PaintGradientExtend::Pad
    }
}
