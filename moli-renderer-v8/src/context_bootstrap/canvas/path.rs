//! Canvas 2D current-path geometry, owned by the context's native state.

use moli_layout::{
    LayoutPoint, LayoutRect, LayoutTransform2D, PaintPath, PaintPathElement, PaintRect,
};
use moli_svg::StrokeQueryLimitExceeded;

mod containment;
pub(super) mod coordinates;
mod viewport;
use coordinates::{Point, Scalar};

#[derive(Debug)]
pub(super) struct NumericQueryLimitExceeded;

#[derive(Clone, Copy, Debug)]
enum PathElement {
    MoveTo(Point),
    LineTo(Point),
    QuadTo(Point, Point),
    CubicTo(Point, Point, Point),
    Close,
}

impl PathElement {
    fn map(self, map: impl Fn(Point) -> Point) -> Self {
        match self {
            Self::MoveTo(p) => Self::MoveTo(map(p)),
            Self::LineTo(p) => Self::LineTo(map(p)),
            Self::QuadTo(a, p) => Self::QuadTo(map(a), map(p)),
            Self::CubicTo(a, b, p) => Self::CubicTo(map(a), map(b), map(p)),
            Self::Close => Self::Close,
        }
    }
    fn relative_to(self, origin: Point) -> Self {
        self.map(|p| p.relative_to(origin))
    }
    fn points(&self) -> impl Iterator<Item = Point> {
        match *self {
            Self::MoveTo(p) | Self::LineTo(p) => [Some(p), None, None],
            Self::QuadTo(a, p) => [Some(a), Some(p), None],
            Self::CubicTo(a, b, p) => [Some(a), Some(b), Some(p)],
            Self::Close => [None; 3],
        }
        .into_iter()
        .flatten()
    }
    fn paint(self) -> PaintPathElement {
        let point = |p: Point| LayoutPoint::new(p.x.lossy() as f32, p.y.lossy() as f32);
        match self {
            Self::MoveTo(p) => PaintPathElement::MoveTo(point(p)),
            Self::LineTo(p) => PaintPathElement::LineTo(point(p)),
            Self::QuadTo(a, p) => PaintPathElement::QuadTo(point(a), point(p)),
            Self::CubicTo(a, b, p) => PaintPathElement::CubicTo(point(a), point(b), point(p)),
            Self::Close => PaintPathElement::Close,
        }
    }
    fn bezier(self) -> kurbo::PathEl {
        let point = |p: Point| kurbo::Point::new(p.x.lossy(), p.y.lossy());
        match self {
            Self::MoveTo(p) => kurbo::PathEl::MoveTo(point(p)),
            Self::LineTo(p) => kurbo::PathEl::LineTo(point(p)),
            Self::QuadTo(a, p) => kurbo::PathEl::QuadTo(point(a), point(p)),
            Self::CubicTo(a, b, p) => kurbo::PathEl::CurveTo(point(a), point(b), point(p)),
            Self::Close => kurbo::PathEl::ClosePath,
        }
    }
}

/// A Bezier path in the same flat form as a kurbo path: one `MoveTo` starts a
/// subpath, `Close` ends it, and a new `MoveTo` begins the next one.
#[derive(Clone, Debug)]
pub(crate) struct Canvas2dPathState {
    // Default-path geometry is already in canvas coordinates. The current
    // transform only affects newly recorded commands, never earlier elements.
    elements: Vec<PathElement>,
    current: Point,
    current_subpath_start: Point,
    has_subpath: bool,
    just_closed: bool,
    transform: LayoutTransform2D,
}

impl Default for Canvas2dPathState {
    fn default() -> Self {
        Self {
            elements: Vec::new(),
            current: Point::ZERO,
            current_subpath_start: Point::ZERO,
            has_subpath: false,
            just_closed: false,
            transform: LayoutTransform2D::IDENTITY,
        }
    }
}

const TWO_PI: f64 = std::f64::consts::TAU;

impl Canvas2dPathState {
    /// An owned bitmap-coordinate polygon, without narrowing native endpoints
    /// or changing the context's current default path.
    pub(super) fn from_closed_polygon(points: [Point; 4]) -> Self {
        let mut state = Self::default();
        state.move_to_canvas(points[0]);
        for point in points[1..].iter().copied() {
            state.line_to_canvas(point);
        }
        state.close_path();
        state
    }

    pub(super) fn from_svg(raw: &str) -> Self {
        let mut state = Self::default();
        for element in moli_svg::parse_path_data(raw).elements() {
            match *element {
                kurbo::PathEl::MoveTo(p) => state.move_to(p.x, p.y),
                kurbo::PathEl::LineTo(p) => state.line_to(p.x, p.y),
                kurbo::PathEl::QuadTo(a, p) => state.quadratic_curve_to(a.x, a.y, p.x, p.y),
                kurbo::PathEl::CurveTo(a, b, p) => {
                    state.bezier_curve_to(a.x, a.y, b.x, b.y, p.x, p.y);
                }
                kurbo::PathEl::ClosePath => state.close_path(),
            }
        }
        if state.has_subpath {
            state.move_to_canvas(state.current);
        }
        state
    }

    /// Append a snapshot in local coordinates, then start a new subpath at its
    /// final point. The caller snapshots before borrowing the destination so
    /// addPath(self, transform) never borrows the same RefCell twice.
    pub(super) fn add_path(&mut self, source: &Self, matrix: [f64; 6]) {
        if !source.has_subpath || !matrix.into_iter().all(f64::is_finite) {
            return;
        }
        let affine = kurbo::Affine::new(matrix);
        self.elements.extend(
            source
                .elements
                .iter()
                .map(|element| element.map(|p| p.transform(affine))),
        );
        self.move_to_canvas(source.current.transform(affine));
    }

    /// Freeze a clip's geometry in bitmap coordinates without modifying either
    /// the author's path or the default path's recorded transform.
    pub(super) fn transformed_snapshot(&self, transform: LayoutTransform2D) -> Self {
        let affine = kurbo::Affine::new(transform.coefficients);
        if !affine.is_finite() {
            return Self::default();
        }
        let mut snapshot = self.clone();
        snapshot.elements = snapshot
            .elements
            .into_iter()
            .map(|element| element.map(|point| point.transform(affine)))
            .collect();
        snapshot.current = snapshot.current.transform(affine);
        snapshot.current_subpath_start = snapshot.current_subpath_start.transform(affine);
        snapshot.transform = LayoutTransform2D::IDENTITY;
        snapshot
    }

    pub(super) fn begin_path(&mut self) {
        self.elements.clear();
        self.current = Point::ZERO;
        self.current_subpath_start = Point::ZERO;
        self.has_subpath = false;
        self.just_closed = false;
    }

    pub(super) fn move_to(&mut self, x: f64, y: f64) {
        if ![x, y].into_iter().all(f64::is_finite) || !self.transform_is_finite() {
            return;
        }
        self.move_to_canvas(self.map_point(x, y));
    }

    fn move_to_canvas(&mut self, point: Point) {
        self.elements.push(PathElement::MoveTo(point));
        self.current = point;
        self.current_subpath_start = point;
        self.has_subpath = true;
        self.just_closed = false;
    }

    /// Reopens a closed subpath; callers establish the initial point for an empty path.
    fn ensure_open_subpath(&mut self) -> bool {
        if !self.has_subpath {
            return false;
        }
        if self.just_closed {
            self.elements.push(PathElement::MoveTo(self.current));
            self.current_subpath_start = self.current;
            self.just_closed = false;
        }
        true
    }

    pub(super) fn line_to(&mut self, x: f64, y: f64) {
        if ![x, y].into_iter().all(f64::is_finite) || !self.transform_is_finite() {
            return;
        }
        self.line_to_canvas(self.map_point(x, y));
    }

    fn line_to_canvas(&mut self, point: Point) {
        if !self.ensure_open_subpath() {
            self.move_to_canvas(point);
            return;
        }
        self.elements.push(PathElement::LineTo(point));
        self.current = point;
    }

    pub(super) fn quadratic_curve_to(&mut self, cpx: f64, cpy: f64, x: f64, y: f64) {
        if ![cpx, cpy, x, y].into_iter().all(f64::is_finite) || !self.transform_is_finite() {
            return;
        }
        if !self.ensure_open_subpath() {
            self.move_to(cpx, cpy);
        }
        let control = self.map_point(cpx, cpy);
        let point = self.map_point(x, y);
        self.elements.push(PathElement::QuadTo(control, point));
        self.current = point;
    }

    pub(super) fn bezier_curve_to(
        &mut self,
        c1x: f64,
        c1y: f64,
        c2x: f64,
        c2y: f64,
        x: f64,
        y: f64,
    ) {
        if ![c1x, c1y, c2x, c2y, x, y].into_iter().all(f64::is_finite)
            || !self.transform_is_finite()
        {
            return;
        }
        if !self.ensure_open_subpath() {
            self.move_to(c1x, c1y);
        }
        let first = self.map_point(c1x, c1y);
        let second = self.map_point(c2x, c2y);
        let point = self.map_point(x, y);
        self.elements
            .push(PathElement::CubicTo(first, second, point));
        self.current = point;
    }

    pub(super) fn close_path(&mut self) {
        if !self.has_subpath || self.just_closed {
            return;
        }
        self.elements.push(PathElement::Close);
        self.current = self.current_subpath_start;
        self.just_closed = true;
    }

    pub(super) fn rect(&mut self, x: f64, y: f64, width: f64, height: f64) {
        if ![x, y, width, height].into_iter().all(f64::is_finite)
            || !coordinates::is_invertible(kurbo::Affine::new(self.transform.coefficients))
        {
            return;
        }
        let left = Scalar::new(x);
        let top = Scalar::new(y);
        let right = left + Scalar::new(width);
        let bottom = top + Scalar::new(height);
        let matrix = kurbo::Affine::new(self.transform.coefficients);
        self.move_to_canvas(Point { x: left, y: top }.transform(matrix));
        self.line_to_canvas(Point { x: right, y: top }.transform(matrix));
        self.line_to_canvas(
            Point {
                x: right,
                y: bottom,
            }
            .transform(matrix),
        );
        self.line_to_canvas(Point { x: left, y: bottom }.transform(matrix));
        self.close_path();
    }

    /// Canvas arcs use positive angles clockwise in the screen's y-down space.
    pub(super) fn arc(
        &mut self,
        x: f64,
        y: f64,
        radius: f64,
        start: f64,
        end: f64,
        ccw: bool,
    ) -> bool {
        self.ellipse(x, y, radius, radius, 0.0, start, end, ccw)
    }

    pub(super) fn ellipse(
        &mut self,
        x: f64,
        y: f64,
        radius_x: f64,
        radius_y: f64,
        rotation: f64,
        start: f64,
        end: f64,
        ccw: bool,
    ) -> bool {
        if ![x, y, radius_x, radius_y, rotation, start, end]
            .into_iter()
            .all(f64::is_finite)
            || radius_x < 0.0
            || radius_y < 0.0
            || !coordinates::is_invertible(kurbo::Affine::new(self.transform.coefficients))
        {
            return false;
        }
        let (start, sweep) = arc_angles(start, end, ccw);
        self.append_arc(kurbo::Arc::new(
            (x, y),
            (radius_x, radius_y),
            start,
            sweep,
            rotation.rem_euclid(TWO_PI),
        ));
        true
    }

    pub(super) fn arc_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, radius: f64) -> bool {
        if ![x1, y1, x2, y2, radius].into_iter().all(f64::is_finite) || radius < 0.0 {
            return false;
        }
        if !self.has_subpath {
            self.move_to(x1, y1);
            return true;
        }
        let Some(inverse) = self.inverse_transform() else {
            self.line_to(x1, y1);
            return true;
        };
        // arcTo constructs a circular arc in the current user coordinate
        // system. The previous point was recorded under a possibly older CTM.
        let Some(p0) = self.current.transform(inverse).finite() else {
            self.line_to(x1, y1);
            return true;
        };
        let p1 = kurbo::Point::new(x1, y1);
        let p2 = kurbo::Point::new(x2, y2);
        let incoming = p0 - p1;
        let outgoing = p2 - p1;
        if incoming.hypot() == 0.0 || outgoing.hypot() == 0.0 || radius == 0.0 {
            self.line_to(x1, y1);
            return true;
        }
        // Both rays originate at the corner. There is no radius clamp: the
        // tangent points may lie beyond either of the supplied line segments.
        let u = incoming / incoming.hypot();
        let v = outgoing / outgoing.hypot();
        let cross = u.cross(v);
        if cross == 0.0 || !cross.is_finite() {
            self.line_to(x1, y1);
            return true;
        }
        let tangent_distance = radius * ((1.0 + u.dot(v).clamp(-1.0, 1.0)) / cross.abs());
        let tangent = p1 + u * tangent_distance;
        let center = tangent + kurbo::Vec2::new(-u.y, u.x) * (radius * cross.signum());
        let end = p1 + v * tangent_distance;
        let start_angle = (tangent - center).atan2();
        let end_angle = (end - center).atan2();
        let (start, sweep) = arc_angles(start_angle, end_angle, cross > 0.0);
        self.append_arc(kurbo::Arc::new(center, (radius, radius), start, sweep, 0.0));
        true
    }

    fn append_arc(&mut self, arc: kurbo::Arc) {
        use kurbo::{PathEl, Shape};
        if ![arc.center.x, arc.center.y, arc.radii.x, arc.radii.y]
            .into_iter()
            .all(f64::is_finite)
        {
            return;
        }
        let radius = arc.radii.x.max(arc.radii.y);
        if radius == 0.0 {
            self.line_to(arc.center.x, arc.center.y);
            return;
        }
        let tolerance = (0.01_f64.max(radius / 4096.0)) / radius;
        let (sin, cos) = arc.x_rotation.sin_cos();
        let ellipse = kurbo::Affine::new([
            arc.radii.x * cos,
            arc.radii.x * sin,
            -arc.radii.y * sin,
            arc.radii.y * cos,
            arc.center.x,
            arc.center.y,
        ]);
        let transform = kurbo::Affine::new(self.transform.coefficients);
        let map = |p: kurbo::Point| Point::new(p.x, p.y).transform(ellipse).transform(transform);
        let unit = kurbo::Arc::new(
            (0.0, 0.0),
            (1.0, 1.0),
            arc.start_angle,
            arc.sweep_angle,
            0.0,
        );
        for element in unit.path_elements(tolerance) {
            match element {
                PathEl::MoveTo(p) => self.line_to_canvas(map(p)),
                PathEl::CurveTo(a, b, p) => {
                    let p = map(p);
                    self.elements.push(PathElement::CubicTo(map(a), map(b), p));
                    self.current = p;
                }
                _ => unreachable!("kurbo arcs contain only a start point and cubic segments"),
            }
        }
    }

    pub(super) fn set_transform(&mut self, a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) {
        if [a, b, c, d, e, f].into_iter().all(f64::is_finite) {
            self.transform = LayoutTransform2D::new([a, b, c, d, e, f]);
        }
    }

    pub(super) fn restore_transform(&mut self, transform: LayoutTransform2D) {
        // Restoring native state is not a new setTransform call: multiplication
        // of finite input matrices can already have overflowed a coefficient.
        self.transform = transform;
    }

    pub(super) fn reset_transform(&mut self) {
        self.transform = LayoutTransform2D::IDENTITY;
    }

    pub(super) fn translate(&mut self, x: f64, y: f64) {
        self.concatenate_transform(1.0, 0.0, 0.0, 1.0, x, y);
    }

    pub(super) fn scale(&mut self, x: f64, y: f64) {
        self.concatenate_transform(x, 0.0, 0.0, y, 0.0, 0.0);
    }

    pub(super) fn rotate(&mut self, radians: f64) {
        let (sin, cos) = radians.sin_cos();
        self.concatenate_transform(cos, sin, -sin, cos, 0.0, 0.0);
    }

    pub(super) fn concatenate_transform(&mut self, a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) {
        if [a, b, c, d, e, f].into_iter().all(f64::is_finite) {
            self.transform = self
                .transform
                .concatenate(LayoutTransform2D::new([a, b, c, d, e, f]));
        }
    }

    pub(super) fn transform(&self) -> LayoutTransform2D {
        self.transform
    }

    fn map_point(&self, x: f64, y: f64) -> Point {
        Point::new(x, y).transform(kurbo::Affine::new(self.transform.coefficients))
    }

    fn transform_is_finite(&self) -> bool {
        self.transform.coefficients.into_iter().all(f64::is_finite)
    }

    pub(super) fn inverse_transform(&self) -> Option<kurbo::Affine> {
        coordinates::inverse(kurbo::Affine::new(self.transform.coefficients))
    }

    /// Stroke metrics are in the *current* user space. Bring the frozen path
    /// back to that space and let the painter transform the resulting stroke,
    /// so anisotropic line widths, joins and dashes change without moving it.
    pub(super) fn stroke_path(&self) -> Option<PaintPath> {
        let inverse = self.inverse_transform()?;
        let elements = self
            .elements
            .iter()
            .map(|element| element.map(|p| p.transform(inverse)).paint())
            .collect::<Vec<_>>();
        Some(PaintPath {
            bounds: path_bounds(&elements),
            elements,
        })
    }

    /// Builds an owned `PaintPath` with conservative local bounds.
    pub(super) fn paint_path(&self) -> PaintPath {
        let elements = self
            .elements
            .iter()
            .map(|element| element.paint())
            .collect::<Vec<_>>();
        PaintPath {
            bounds: path_bounds(&elements),
            elements,
        }
    }

    pub(super) fn clip_paint_path(
        &self,
        width: u32,
        height: u32,
    ) -> Result<PaintPath, moli_paint::PaintError> {
        viewport::paint_path(self, width, height)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Stroke tracing removes zero-length segments and isolated move points.
    /// This must precede compositing, which can affect pixels beyond the path.
    pub(super) fn has_stroke_segments(&self) -> bool {
        let mut current = Point::default();
        let mut start = current;
        for element in &self.elements {
            let end = match *element {
                PathElement::MoveTo(point) => {
                    current = point;
                    start = point;
                    continue;
                }
                PathElement::LineTo(point) => point,
                PathElement::QuadTo(control, point) => {
                    if control != current {
                        return true;
                    }
                    point
                }
                PathElement::CubicTo(first, second, point) => {
                    if first != current || second != current {
                        return true;
                    }
                    point
                }
                PathElement::Close => start,
            };
            if end != current {
                return true;
            }
            current = end;
        }
        false
    }

    pub(super) fn contains_point(
        &self,
        x: f64,
        y: f64,
        transform: LayoutTransform2D,
        fill_rule: moli_svg::SvgFillRule,
    ) -> Result<bool, NumericQueryLimitExceeded> {
        let affine = kurbo::Affine::new(transform.coefficients);
        if ![x, y].into_iter().all(f64::is_finite) || !affine.is_finite() {
            return Ok(false);
        }
        let query = Point::new(x, y);
        let transformed;
        let elements = if affine == kurbo::Affine::IDENTITY {
            &self.elements
        } else {
            transformed = self
                .elements
                .iter()
                .map(|element| element.map(|p| p.transform(affine)))
                .collect::<Vec<_>>();
            &transformed
        };
        if query.x.is_small_for_f64_geometry()
            && query.y.is_small_for_f64_geometry()
            && elements
                .iter()
                .flat_map(PathElement::points)
                .all(|p| p.x.is_small_for_f64_geometry() && p.y.is_small_for_f64_geometry())
        {
            let path =
                kurbo::BezPath::from_vec(elements.iter().map(|element| element.bezier()).collect());
            return Ok(moli_svg::is_point_in_path_with_rule(
                &path,
                moli_svg::SvgGeometryPoint::new(x, y),
                fill_rule,
            ));
        }
        containment::contains(elements, query, fill_rule)
    }

    fn bezier_path(&self, affine: kurbo::Affine) -> kurbo::BezPath {
        kurbo::BezPath::from_vec(
            self.elements
                .iter()
                .map(|element| element.map(|p| p.transform(affine)).bezier())
                .collect(),
        )
    }

    pub(super) fn contains_stroke_point(
        &self,
        point: kurbo::Point,
        geometry_transform: kurbo::Affine,
        style: &kurbo::Stroke,
    ) -> Result<bool, StrokeQueryLimitExceeded> {
        moli_svg::is_point_in_stroke_path(self.bezier_path(geometry_transform), point, style)
    }
}

fn path_bounds(elements: &[PaintPathElement]) -> PaintRect {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for element in elements {
        match *element {
            PaintPathElement::MoveTo(point) | PaintPathElement::LineTo(point) => {
                expand(&mut min_x, &mut min_y, &mut max_x, &mut max_y, point);
            }
            PaintPathElement::QuadTo(first, second) => {
                expand(&mut min_x, &mut min_y, &mut max_x, &mut max_y, first);
                expand(&mut min_x, &mut min_y, &mut max_x, &mut max_y, second);
            }
            PaintPathElement::CubicTo(first, second, third) => {
                expand(&mut min_x, &mut min_y, &mut max_x, &mut max_y, first);
                expand(&mut min_x, &mut min_y, &mut max_x, &mut max_y, second);
                expand(&mut min_x, &mut min_y, &mut max_x, &mut max_y, third);
            }
            PaintPathElement::Close => {}
        }
    }
    if !min_x.is_finite() {
        return LayoutRect::ZERO;
    }
    LayoutRect::new(
        min_x as f32,
        min_y as f32,
        (max_x - min_x) as f32,
        (max_y - min_y) as f32,
    )
}

fn expand(min_x: &mut f64, min_y: &mut f64, max_x: &mut f64, max_y: &mut f64, point: LayoutPoint) {
    let x = f64::from(point.x);
    let y = f64::from(point.y);
    *min_x = (*min_x).min(x);
    *min_y = (*min_y).min(y);
    *max_x = (*max_x).max(x);
    *max_y = (*max_y).max(y);
}

/// Normalize in constant time, including opposite-sign finite angles whose
/// subtraction overflows. Never repeatedly add/subtract TAU: at large f64
/// magnitudes that operation does not advance at all.
fn arc_angles(start: f64, end: f64, ccw: bool) -> (f64, f64) {
    let difference = end - start;
    let normalized_start = start.rem_euclid(TWO_PI);
    let sweep = if !ccw && difference >= TWO_PI {
        TWO_PI
    } else if ccw && difference <= -TWO_PI {
        -TWO_PI
    } else if difference == 0.0 {
        0.0
    } else {
        let remainder = if difference.is_finite() {
            difference.rem_euclid(TWO_PI)
        } else {
            (end.rem_euclid(TWO_PI) - normalized_start).rem_euclid(TWO_PI)
        };
        if ccw {
            // Like Blink's AdjustEndAngle, preserve a whole turn for opposite-
            // direction endpoints separated by an exact multiple of TAU.
            if remainder == 0.0 && difference < 0.0 {
                0.0
            } else {
                remainder - TWO_PI
            }
        } else if remainder == 0.0 && difference < 0.0 {
            TWO_PI
        } else {
            remainder
        }
    };
    (normalized_start, sweep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    #[test]
    fn arc_angles_preserve_direction_full_turns_and_equal_endpoints() {
        for (start, end, ccw, expected) in [
            (0.0, FRAC_PI_2, false, FRAC_PI_2),
            (0.0, FRAC_PI_2, true, -3.0 * FRAC_PI_2),
            (0.0, -FRAC_PI_2, true, -FRAC_PI_2),
            (0.0, -FRAC_PI_2, false, 3.0 * FRAC_PI_2),
            (0.0, TWO_PI, false, TWO_PI),
            (0.0, -TWO_PI, true, -TWO_PI),
            (0.0, TWO_PI, true, -TWO_PI),
            (0.0, -TWO_PI, false, TWO_PI),
            (2.0, 2.0, true, 0.0),
            (2.0, 2.0, false, 0.0),
        ] {
            assert!((arc_angles(start, end, ccw).1 - expected).abs() < 1e-12);
        }
    }

    #[test]
    fn finite_extreme_angles_produce_bounded_path_work() {
        for start in [0.0, 1e20, -1e20, f64::MAX, -f64::MAX] {
            for end in [0.0, 1e20, -1e20, f64::MAX, -f64::MAX] {
                for ccw in [false, true] {
                    let (angle, sweep) = arc_angles(start, end, ccw);
                    assert!(angle.is_finite() && (0.0..TWO_PI).contains(&angle));
                    assert!(sweep.is_finite() && sweep.abs() <= TWO_PI);
                    assert!(if ccw { sweep <= 0.0 } else { sweep >= 0.0 });
                    let mut state = Canvas2dPathState::default();
                    assert!(state.arc(10.0, 10.0, 5.0, start, end, ccw));
                    assert!(state.elements.len() <= 7);
                }
            }
        }
    }

    #[test]
    fn arc_to_has_the_correct_tangent_and_endpoint() {
        let mut state = Canvas2dPathState::default();
        state.move_to(0.0, 0.0);
        assert!(state.arc_to(10.0, 0.0, 10.0, 10.0, 5.0));
        let PathElement::LineTo(tangent) = state.elements[1] else {
            panic!("arcTo must connect to its first tangent");
        };
        assert!(
            (tangent.x.finite().unwrap() - 5.0).abs() < 1e-5
                && tangent.y.finite().unwrap().abs() < 1e-5
        );
        assert!((state.current.x.finite().unwrap() - 10.0).abs() < 1e-5);
        assert!((state.current.y.finite().unwrap() - 5.0).abs() < 1e-5);
    }

    #[test]
    fn rejected_arc_geometry_does_not_mutate_the_path() {
        let mut state = Canvas2dPathState::default();
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            assert!(!state.arc(0.0, 0.0, invalid, 0.0, 1.0, false));
            assert!(!state.ellipse(0.0, 0.0, 5.0, invalid, 0.0, 0.0, 1.0, false));
            assert!(!state.arc_to(0.0, 0.0, 1.0, 1.0, invalid));
            assert!(state.is_empty());
        }
    }
}
