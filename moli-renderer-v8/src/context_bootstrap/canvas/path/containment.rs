//! Ray crossings in native coordinate range. Ordinary paths use Kurbo; this
//! path preserves signs when finite affine products overflow or underflow f64.

use std::cmp::Ordering;

use super::coordinates::{Point, Scalar};
use super::{NumericQueryLimitExceeded, PathElement};

const MAX_QUERY_STEPS: usize = 1_000_000;
const MAX_CURVE_DEPTH: u16 = 4096;

struct QueryBudget(usize);

impl QueryBudget {
    fn step(&mut self) -> Result<(), NumericQueryLimitExceeded> {
        self.0 = self.0.checked_sub(1).ok_or(NumericQueryLimitExceeded)?;
        Ok(())
    }
}

/// An infinitesimal query displacement, first along `first`, then along
/// `second` at a smaller order. Only signs are evaluated: no numeric epsilon
/// can reliably move every coordinate of an extreme or very thin path.
#[derive(Clone, Copy, Default)]
struct QueryOffset {
    first: Point,
    second: Point,
}

impl QueryOffset {
    fn sign(value: Scalar, first: Scalar, second: Scalar) -> Ordering {
        value
            .partial_cmp(&Scalar::ZERO)
            .unwrap()
            .then_with(|| Scalar::ZERO.partial_cmp(&first).unwrap())
            .then_with(|| Scalar::ZERO.partial_cmp(&second).unwrap())
    }

    fn x(self, point: Point) -> Ordering {
        Self::sign(point.x, self.first.x, self.second.x)
    }

    fn y(self, point: Point) -> Ordering {
        Self::sign(point.y, self.first.y, self.second.y)
    }

    fn determinant(self, first: Point, last: Point) -> Scalar {
        let determinant = first.x * last.y - last.x * first.y;
        if !determinant.is_zero() {
            return determinant;
        }
        // det(p - q, r - q) = det(p, r) + q.y * (r.x - p.x)
        //                                  - q.x * (r.y - p.y).
        let coefficient =
            |direction: Point| direction.y * (last.x - first.x) - direction.x * (last.y - first.y);
        let leading = coefficient(self.first);
        if leading.is_zero() {
            coefficient(self.second)
        } else {
            leading
        }
    }
}

#[derive(Default)]
struct Hit {
    winding: i64,
    boundary: bool,
}

impl Hit {
    fn append(&mut self, other: Self) {
        self.winding += other.winding;
        self.boundary |= other.boundary;
    }
}

pub(super) fn contains(
    elements: &[PathElement],
    query: Point,
    rule: moli_svg::SvgFillRule,
) -> Result<bool, NumericQueryLimitExceeded> {
    let relative = elements
        .iter()
        .map(|element| element.relative_to(query))
        .collect::<Vec<_>>();
    let inside = |winding| match rule {
        moli_svg::SvgFillRule::NonZero => winding != 0,
        moli_svg::SvgFillRule::EvenOdd => winding % 2 != 0,
    };
    let mut budget = QueryBudget(MAX_QUERY_STEPS);
    let hit = crossings(&relative, QueryOffset::default(), &mut budget)?;
    if inside(hit.winding) {
        return Ok(true);
    }
    if !hit.boundary {
        return Ok(false);
    }

    // A boundary is included only if it borders a filled region. This keeps
    // isolated lines and overlapping paths cancelled by the fill rule empty.
    for x in [-1.0, 0.0, 1.0] {
        for y in [-1.0, 0.0, 1.0] {
            if x == 0.0 && y == 0.0 {
                continue;
            }
            let offset = QueryOffset {
                first: Point::new(x, y),
                second: Point::ZERO,
            };
            if inside(crossings(&relative, offset, &mut budget)?.winding) {
                return Ok(true);
            }
        }
    }
    // At an acute vertex none of the eight compass directions need enter the
    // area. Approach along each incident tangent and test its two sides at a
    // smaller order. Control points also supply the tangent of a Bezier whose
    // endpoint is the query. Both orders retain native coordinate range.
    for first in relative.iter().flat_map(PathElement::points) {
        if first == Point::ZERO {
            continue;
        }
        for sign in [-1.0, 1.0] {
            let sign = Scalar::new(sign);
            let offset = QueryOffset {
                first,
                second: Point {
                    x: -first.y * sign,
                    y: first.x * sign,
                },
            };
            if inside(crossings(&relative, offset, &mut budget)?.winding) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn crossings(
    elements: &[PathElement],
    offset: QueryOffset,
    budget: &mut QueryBudget,
) -> Result<Hit, NumericQueryLimitExceeded> {
    let mut hit = Hit::default();
    let mut current = Point::ZERO;
    let mut start = None;
    for element in elements {
        budget.step()?;
        match *element {
            PathElement::MoveTo(point) => {
                if let Some(first) = start {
                    hit.append(line(current, first, offset));
                }
                current = point;
                start = Some(point);
            }
            PathElement::LineTo(point) => {
                hit.append(line(current, point, offset));
                current = point;
            }
            PathElement::QuadTo(control, point) => {
                hit.append(curve([current, control, point, point], 3, offset, budget)?);
                current = point;
            }
            PathElement::CubicTo(a, b, point) => {
                hit.append(curve([current, a, b, point], 4, offset, budget)?);
                current = point;
            }
            PathElement::Close => {
                if let Some(first) = start.take() {
                    hit.append(line(current, first, offset));
                    current = first;
                }
            }
        }
    }
    if let Some(first) = start {
        hit.append(line(current, first, offset));
    }
    Ok(hit)
}

fn endpoint_winding(first: Point, last: Point, offset: QueryOffset) -> i64 {
    if offset.y(first) != Ordering::Greater && offset.y(last) == Ordering::Greater {
        -1
    } else if offset.y(last) != Ordering::Greater && offset.y(first) == Ordering::Greater {
        1
    } else {
        0
    }
}

fn line(first: Point, last: Point, offset: QueryOffset) -> Hit {
    let boundary = (first.x * last.y - last.x * first.y).is_zero()
        && ((first.x <= Scalar::ZERO && last.x >= Scalar::ZERO)
            || (last.x <= Scalar::ZERO && first.x >= Scalar::ZERO))
        && ((first.y <= Scalar::ZERO && last.y >= Scalar::ZERO)
            || (last.y <= Scalar::ZERO && first.y >= Scalar::ZERO));
    let determinant = offset.determinant(first, last);
    let direction = endpoint_winding(first, last, offset);
    let denominator = last.y - first.y;
    let left =
        determinant.is_zero() || (determinant < Scalar::ZERO) != (denominator < Scalar::ZERO);
    Hit {
        winding: if left { direction } else { 0 },
        boundary,
    }
}

fn curve(
    points: [Point; 4],
    count: usize,
    offset: QueryOffset,
    budget: &mut QueryBudget,
) -> Result<Hit, NumericQueryLimitExceeded> {
    let mut result = Hit::default();
    let mut stack = vec![(points, 0_u16)];
    while let Some((points, depth)) = stack.pop() {
        budget.step()?;
        let first = points[0];
        let last = points[count - 1];
        result.boundary |= first == Point::ZERO || last == Point::ZERO;
        let all_left = points[..count]
            .iter()
            .all(|point| offset.x(*point) == Ordering::Less);
        let all_right = points[..count]
            .iter()
            .all(|point| offset.x(*point) == Ordering::Greater);
        let all_above = points[..count]
            .iter()
            .all(|point| offset.y(*point) == Ordering::Greater);
        let all_below = points[..count]
            .iter()
            .all(|point| offset.y(*point) == Ordering::Less);
        if all_left {
            result.winding += endpoint_winding(first, last, offset);
            continue;
        }
        if all_right || all_above || all_below {
            continue;
        }
        let nonnegative_y = points[..count]
            .iter()
            .all(|point| offset.y(*point) != Ordering::Less);
        let nonpositive_y = points[..count]
            .iter()
            .all(|point| offset.y(*point) != Ordering::Greater);
        let horizontal = points[..count].iter().all(|point| point.y.is_zero());
        if !horizontal && nonnegative_y {
            if offset.y(first) == Ordering::Equal && offset.x(first) == Ordering::Less {
                result.winding -= 1;
            }
            if offset.y(last) == Ordering::Equal && offset.x(last) == Ordering::Less {
                result.winding += 1;
            }
            continue;
        }
        if !horizontal && nonpositive_y {
            continue;
        }
        if horizontal
            && ((first.x <= Scalar::ZERO && last.x >= Scalar::ZERO)
                || (last.x <= Scalar::ZERO && first.x >= Scalar::ZERO))
        {
            result.boundary = true;
            continue;
        }
        if depth == MAX_CURVE_DEPTH {
            return Err(NumericQueryLimitExceeded);
        }
        let mut triangle = points;
        let mut left = [Point::ZERO; 4];
        let mut right = [Point::ZERO; 4];
        left[0] = first;
        right[count - 1] = last;
        for level in 1..count {
            for index in 0..count - level {
                triangle[index] = triangle[index].midpoint(triangle[index + 1]);
            }
            left[level] = triangle[0];
            right[count - 1 - level] = triangle[count - 1 - level];
        }
        if left[..count] == points[..count] || right[..count] == points[..count] {
            result.append(line(first, last, offset));
            continue;
        }
        stack.push((right, depth + 1));
        stack.push((left, depth + 1));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    #[test]
    fn wide_coordinate_fill_matches_ordinary_bezier_winding() {
        use PathElement::*;
        let point = Point::new;
        let shapes = [
            vec![
                MoveTo(point(-10.0, -10.0)),
                LineTo(point(10.0, 10.0)),
                LineTo(point(-10.0, 10.0)),
                Close,
            ],
            vec![
                MoveTo(point(-10.0, -10.0)),
                QuadTo(point(0.0, 30.0), point(10.0, -10.0)),
                Close,
            ],
            vec![
                MoveTo(point(-10.0, -10.0)),
                CubicTo(point(-10.0, 20.0), point(10.0, 20.0), point(10.0, -10.0)),
                Close,
            ],
        ];
        for exponent in [-4096, -1024, 0, 1024, 4096] {
            let scale = |point: Point| Point {
                x: point.x.scaled(exponent),
                y: point.y.scaled(exponent),
            };
            for shape in &shapes {
                let ordinary = kurbo::BezPath::from_vec(
                    shape.iter().map(|element| element.bezier()).collect(),
                );
                let native = shape
                    .iter()
                    .map(|element| element.map(scale))
                    .collect::<Vec<_>>();
                for x in -12..=12 {
                    for y in -14..=14 {
                        // Avoid boundary tolerances in the independent Kurbo oracle.
                        let query = kurbo::Point::new(f64::from(x) + 0.123, f64::from(y) + 0.234);
                        let expected = ordinary.winding(query) != 0;
                        assert_eq!(
                            contains(
                                &native,
                                scale(point(query.x, query.y)),
                                moli_svg::SvgFillRule::NonZero
                            )
                            .unwrap(),
                            expected,
                            "exponent={exponent}, query={query:?}, shape={shape:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn wide_fill_boundaries_include_acute_sectors_and_respect_cancellation() {
        use PathElement::*;
        let point = Point::new;
        let acute = [
            MoveTo(Point::ZERO),
            LineTo(point(10.0, 1.0)),
            LineTo(point(10.0, 2.0)),
            Close,
        ];
        let diagonal = [
            MoveTo(point(-10.0, -10.0)),
            LineTo(point(10.0, 10.0)),
            LineTo(point(-10.0, 10.0)),
            Close,
        ];
        for exponent in [-4096, -1024, 0, 1024, 4096] {
            for reflect in [-1.0, 1.0] {
                let scale = |point: Point| Point {
                    x: point.x.scaled(exponent) * Scalar::new(reflect),
                    y: point.y.scaled(exponent),
                };
                for shape in [acute, diagonal] {
                    let native = shape.map(|element| element.map(scale));
                    assert!(
                        contains(&native, Point::ZERO, moli_svg::SvgFillRule::NonZero).unwrap()
                    );
                    assert!(
                        contains(&native, Point::ZERO, moli_svg::SvgFillRule::EvenOdd).unwrap()
                    );
                    let mut duplicate = native.to_vec();
                    duplicate.extend(native);
                    assert!(
                        !contains(&duplicate, Point::ZERO, moli_svg::SvgFillRule::EvenOdd).unwrap()
                    );
                }
                let line = [MoveTo(Point::ZERO), LineTo(scale(point(10.0, 1.0)))];
                assert!(!contains(&line, Point::ZERO, moli_svg::SvgFillRule::NonZero).unwrap());
            }
        }
    }
}
