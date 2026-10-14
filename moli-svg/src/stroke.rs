//! Stroke outline containment and zero-length dash caps in user coordinates.

use kurbo::{ParamCurve, ParamCurveArclen, ParamCurveDeriv};

#[derive(Debug)]
pub struct StrokeQueryLimitExceeded;

const MAX_STROKE_QUERY_ELEMENTS: usize = 1_000_000;
const DASH_ACCURACY: f64 = 1e-6;

pub fn is_point_in_stroke_path(
    mut path: kurbo::BezPath,
    point: kurbo::Point,
    style: &kurbo::Stroke,
) -> Result<bool, StrokeQueryLimitExceeded> {
    if !point.is_finite()
        || !style.is_finite()
        || style.width <= 0.0
        || style.dash_pattern.iter().any(|dash| *dash < 0.0)
    {
        return Ok(false);
    }
    let mut style = style.clone();
    if style.dash_pattern.iter().all(|dash| *dash == 0.0) {
        style.dash_pattern.clear();
    } else if style.dash_pattern.len() % 2 == 1 {
        let repeated = style.dash_pattern.clone();
        style.dash_pattern.extend(repeated);
    }
    let mut scale = 1.0;
    // Avoid an infinite dash period: a negative offset normalized modulo
    // infinity would otherwise prevent the dash iterator from advancing.
    // Uniform power-of-two scaling preserves all geometric relationships.
    while style.dash_pattern.iter().sum::<f64>().is_infinite() {
        style.width *= 0.5;
        style.dash_offset *= 0.5;
        for dash in &mut style.dash_pattern {
            *dash *= 0.5;
        }
        scale *= 0.5;
    }
    let point = kurbo::Point::new(point.x * scale, point.y * scale);
    path.apply_affine(kurbo::Affine::scale(scale));
    if !path.is_finite() {
        return Ok(false);
    }
    if zero_dash_cap_contains(&path, point, &style)? {
        return Ok(true);
    }
    let path = if style.dash_pattern.is_empty() {
        path
    } else {
        let mut dashed = kurbo::BezPath::new();
        for element in kurbo::dash(path.iter(), style.dash_offset, &style.dash_pattern)
            .take(MAX_STROKE_QUERY_ELEMENTS + 1)
        {
            if dashed.elements().len() == MAX_STROKE_QUERY_ELEMENTS {
                return Err(StrokeQueryLimitExceeded);
            }
            dashed.push(element);
        }
        style.dash_pattern.clear();
        dashed
    };
    let tolerance = (0.001 * scale).max(style.width * 1e-6);
    let outline = kurbo::stroke(
        path.iter(),
        &style,
        &kurbo::StrokeOpts::default(),
        tolerance,
    );
    Ok(crate::is_point_in_path_with_rule(
        &outline,
        crate::SvgGeometryPoint::new(point.x, point.y),
        crate::SvgFillRule::NonZero,
    ))
}

/// Zero-on dashes are introduced after zero-length source segments have been
/// pruned. Their caps need the direction at that exact source arc length;
/// reconstructing a direction from the expanded zero-length segment loses it.
fn zero_dash_cap_contains(
    path: &kurbo::BezPath,
    point: kurbo::Point,
    style: &kurbo::Stroke,
) -> Result<bool, StrokeQueryLimitExceeded> {
    if style.start_cap == kurbo::Cap::Butt
        || !style
            .dash_pattern
            .iter()
            .step_by(2)
            .any(|dash| *dash == 0.0)
    {
        return Ok(false);
    }
    let mut first = 0;
    let mut visited = 0;
    for (index, element) in path.elements().iter().enumerate() {
        if index > first && matches!(element, kurbo::PathEl::MoveTo(_)) {
            if subpath_dash_cap_contains(
                &path.elements()[first..index],
                point,
                style,
                &mut visited,
            )? {
                return Ok(true);
            }
            first = index;
        }
    }
    subpath_dash_cap_contains(&path.elements()[first..], point, style, &mut visited)
}

fn subpath_dash_cap_contains(
    elements: &[kurbo::PathEl],
    point: kurbo::Point,
    style: &kurbo::Stroke,
    visited: &mut usize,
) -> Result<bool, StrokeQueryLimitExceeded> {
    let segments = kurbo::segments(elements.iter().copied())
        .map(|segment| (segment, segment.arclen(DASH_ACCURACY)))
        .filter(|(_, length)| *length > 0.0)
        .collect::<Vec<_>>();
    let length = segments.iter().map(|(_, length)| length).sum::<f64>();
    let period = style.dash_pattern.iter().sum::<f64>();
    if segments.is_empty() || period <= 0.0 {
        return Ok(false);
    }
    let mut prefix = 0.0;
    for (index, dash) in style.dash_pattern.iter().enumerate() {
        if index.is_multiple_of(2) && *dash == 0.0 {
            let difference = prefix - style.dash_offset;
            let mut position = if difference.is_finite() {
                difference.rem_euclid(period)
            } else {
                (prefix - style.dash_offset.rem_euclid(period)).rem_euclid(period)
            };
            let mut segment_index = 0;
            let mut segment_start = 0.0;
            while position <= length {
                *visited += 1;
                if *visited > MAX_STROKE_QUERY_ELEMENTS {
                    return Err(StrokeQueryLimitExceeded);
                }
                while segment_index + 1 < segments.len()
                    && position >= segment_start + segments[segment_index].1
                {
                    segment_start += segments[segment_index].1;
                    segment_index += 1;
                }
                let (segment, segment_length) = segments[segment_index];
                let t = segment.inv_arclen(
                    (position - segment_start).min(segment_length),
                    DASH_ACCURACY,
                );
                if dash_cap_contains(segment, t, point, style) {
                    return Ok(true);
                }
                let next = position + period;
                if next <= position {
                    return Err(StrokeQueryLimitExceeded);
                }
                position = next;
            }
        }
        prefix += dash;
    }
    Ok(false)
}

fn dash_cap_contains(
    segment: kurbo::PathSeg,
    t: f64,
    point: kurbo::Point,
    style: &kurbo::Stroke,
) -> bool {
    let center = segment.eval(t);
    let half_width = style.width * 0.5;
    if style.start_cap == kurbo::Cap::Round {
        return point.distance(center) <= half_width;
    }
    let Some(direction) = dash_direction(segment, t) else {
        return false;
    };
    let delta = point - center;
    delta.dot(direction).abs() <= half_width && delta.cross(direction).abs() <= half_width
}

fn dash_direction(segment: kurbo::PathSeg, t: f64) -> Option<kurbo::Vec2> {
    let tangent = match segment {
        kurbo::PathSeg::Line(line) => line.p1 - line.p0,
        kurbo::PathSeg::Quad(curve) => curve.deriv().eval(t).to_vec2(),
        kurbo::PathSeg::Cubic(curve) => curve.deriv().eval(t).to_vec2(),
    };
    let length = tangent.hypot();
    if length > 0.0 && length.is_finite() {
        return Some(tangent / length);
    }
    // Repeated controls can make the first derivative zero at an endpoint
    // or cusp. The first distinct control gives the one-sided direction.
    let at_end = t == 1.0;
    let remaining = if at_end {
        segment
    } else {
        segment.subsegment(t..1.0)
    };
    let vectors = match (remaining, at_end) {
        (kurbo::PathSeg::Line(line), _) => [line.p1 - line.p0; 3],
        (kurbo::PathSeg::Quad(curve), false) => {
            [curve.p1 - curve.p0, curve.p2 - curve.p0, kurbo::Vec2::ZERO]
        }
        (kurbo::PathSeg::Quad(curve), true) => {
            [curve.p2 - curve.p1, curve.p2 - curve.p0, kurbo::Vec2::ZERO]
        }
        (kurbo::PathSeg::Cubic(curve), false) => [
            curve.p1 - curve.p0,
            curve.p2 - curve.p0,
            curve.p3 - curve.p0,
        ],
        (kurbo::PathSeg::Cubic(curve), true) => [
            curve.p3 - curve.p2,
            curve.p3 - curve.p1,
            curve.p3 - curve.p0,
        ],
    };
    vectors.into_iter().find_map(|vector| {
        let length = vector.hypot();
        (length > 0.0 && length.is_finite()).then(|| vector / length)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stroke_query_preserves_finite_offsets_when_dash_period_overflows() {
        let mut path = kurbo::BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((100.0, 0.0));
        for (offset, expected) in [(0.0, true), (1e308, false), (-1e308, false)] {
            let style = kurbo::Stroke::new(2.0)
                .with_caps(kurbo::Cap::Butt)
                .with_dashes(offset, [1e308, 1e308]);
            assert_eq!(
                is_point_in_stroke_path(path.clone(), kurbo::Point::new(50.0, 0.0), &style,)
                    .unwrap(),
                expected,
                "offset {offset}"
            );
        }
    }

    #[test]
    fn stroke_query_reports_allocation_limit_for_extremely_dense_dashes() {
        let mut path = kurbo::BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((100.0, 0.0));
        let style = kurbo::Stroke::new(2.0).with_dashes(0.0, [1e-30, 1e-30]);
        assert!(
            is_point_in_stroke_path(path.clone(), kurbo::Point::new(50.0, 0.0), &style,).is_err()
        );
    }
}
