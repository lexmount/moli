//! Browser-independent Canvas 2D current-path geometry.
//!
//! Backed by kurbo-native types (`kurbo::PathEl`, `kurbo::Affine`) so this
//! module has no dependency on page layout or any browser internals. Ported
//! from the renderer's per-context path state with corrected arc mathematics,
//! bounded angle normalization, and default-path transform semantics.
//!
//! Semantics matching the HTML spec and Chromium: the default-path transform is
//! applied to each newly recorded command at command time; earlier elements are
//! never moved by a later transform change. Stroke metrics (width, dash,
//! cap/join) are expressed in the *current* user space.

use kurbo::{Affine, Point, Rect, Vec2};

/// An owned Bézier path in canvas coordinates plus its conservative local
/// bounds. Elements use the same f32 truncation the browser adapter consumes,
/// while bounds are expressed in canvas pixel space.
#[derive(Clone, Debug)]
pub struct CanvasPathData {
    /// Path elements, in canvas (already-transformed) coordinates for fill.
    pub elements: Vec<kurbo::PathEl>,
    /// Conservative local bounds over the elements.
    pub bounds: Rect,
}

/// A Bezier path in the same flat form as a kurbo path: one `MoveTo` starts a
/// subpath, `ClosePath` ends it, and a new `MoveTo` begins the next one.
///
/// The current transform only affects newly recorded commands, never earlier
/// elements.
#[derive(Clone, Debug)]
pub struct CanvasPath {
    elements: Vec<kurbo::PathEl>,
    current: Point,
    current_subpath_start: Point,
    has_subpath: bool,
    just_closed: bool,
    transform: Affine,
}

impl Default for CanvasPath {
    fn default() -> Self {
        Self {
            elements: Vec::new(),
            current: Point::ZERO,
            current_subpath_start: Point::ZERO,
            has_subpath: false,
            just_closed: false,
            transform: Affine::IDENTITY,
        }
    }
}

const TWO_PI: f64 = std::f64::consts::TAU;

impl CanvasPath {
    pub fn begin_path(&mut self) {
        self.elements.clear();
        self.current = Point::ZERO;
        self.current_subpath_start = Point::ZERO;
        self.has_subpath = false;
        self.just_closed = false;
    }

    pub fn move_to(&mut self, x: f64, y: f64) {
        if ![x, y].into_iter().all(f64::is_finite) {
            return;
        }
        let (x, y) = self.map_point(x, y);
        self.elements.push(kurbo::PathEl::MoveTo(point(x, y)));
        self.current = Point::new(x, y);
        self.current_subpath_start = Point::new(x, y);
        self.has_subpath = true;
        self.just_closed = false;
    }

    /// Reopens a closed subpath; callers establish the initial point for an
    /// empty path.
    fn ensure_open_subpath(&mut self) -> bool {
        if !self.has_subpath {
            return false;
        }
        if self.just_closed {
            self.elements
                .push(kurbo::PathEl::MoveTo(point(self.current.x, self.current.y)));
            self.current_subpath_start = self.current;
            self.just_closed = false;
        }
        true
    }

    pub fn line_to(&mut self, x: f64, y: f64) {
        if ![x, y].into_iter().all(f64::is_finite) {
            return;
        }
        if !self.ensure_open_subpath() {
            self.move_to(x, y);
            return;
        }
        let (x, y) = self.map_point(x, y);
        self.elements.push(kurbo::PathEl::LineTo(point(x, y)));
        self.current = Point::new(x, y);
    }

    pub fn quadratic_curve_to(&mut self, cpx: f64, cpy: f64, x: f64, y: f64) {
        if ![cpx, cpy, x, y].into_iter().all(f64::is_finite) {
            return;
        }
        if !self.ensure_open_subpath() {
            self.move_to(cpx, cpy);
        }
        let (cpx, cpy) = self.map_point(cpx, cpy);
        let (x, y) = self.map_point(x, y);
        self.elements
            .push(kurbo::PathEl::QuadTo(point(cpx, cpy), point(x, y)));
        self.current = Point::new(x, y);
    }

    pub fn bezier_curve_to(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) {
        if ![c1x, c1y, c2x, c2y, x, y].into_iter().all(f64::is_finite) {
            return;
        }
        if !self.ensure_open_subpath() {
            self.move_to(c1x, c1y);
        }
        let (c1x, c1y) = self.map_point(c1x, c1y);
        let (c2x, c2y) = self.map_point(c2x, c2y);
        let (x, y) = self.map_point(x, y);
        self.elements.push(kurbo::PathEl::CurveTo(
            point(c1x, c1y),
            point(c2x, c2y),
            point(x, y),
        ));
        self.current = Point::new(x, y);
    }

    pub fn close_path(&mut self) {
        if !self.has_subpath || self.just_closed {
            return;
        }
        self.elements.push(kurbo::PathEl::ClosePath);
        self.current = self.current_subpath_start;
        self.just_closed = true;
    }

    pub fn rect(&mut self, x: f64, y: f64, width: f64, height: f64) {
        if ![x, y, width, height].into_iter().all(f64::is_finite)
            || self.inverse_transform().is_none()
        {
            return;
        }
        self.move_to(x, y);
        self.line_to(x + width, y);
        self.line_to(x + width, y + height);
        self.line_to(x, y + height);
        self.close_path();
    }

    /// Canvas arcs use positive angles clockwise in the screen's y-down space.
    pub fn arc(&mut self, x: f64, y: f64, radius: f64, start: f64, end: f64, ccw: bool) -> bool {
        self.ellipse(x, y, radius, radius, 0.0, start, end, ccw)
    }

    pub fn ellipse(
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
            || self.inverse_transform().is_none()
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

    pub fn arc_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, radius: f64) -> bool {
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
        let p0 = inverse * self.current;
        let p1 = Point::new(x1, y1);
        let p2 = Point::new(x2, y2);
        let incoming = p0 - p1;
        let outgoing = p2 - p1;
        if incoming.hypot() == 0.0 || outgoing.hypot() == 0.0 || radius == 0.0 {
            self.line_to(x1, y1);
            return true;
        }
        // Both rays originate at the corner. There is no radius clamp.
        let u = incoming / incoming.hypot();
        let v = outgoing / outgoing.hypot();
        let cross = u.cross(v);
        if cross == 0.0 || !cross.is_finite() {
            self.line_to(x1, y1);
            return true;
        }
        let tangent_distance = radius * ((1.0 + u.dot(v).clamp(-1.0, 1.0)) / cross.abs());
        let tangent = p1 + u * tangent_distance;
        let center = tangent + Vec2::new(-u.y, u.x) * (radius * cross.signum());
        let end = p1 + v * tangent_distance;
        let start_angle = (tangent - center).atan2();
        let end_angle = (end - center).atan2();
        let (start, sweep) = arc_angles(start_angle, end_angle, cross > 0.0);
        self.append_arc(kurbo::Arc::new(center, (radius, radius), start, sweep, 0.0));
        true
    }

    fn append_arc(&mut self, arc: kurbo::Arc) {
        use kurbo::{PathEl, Shape};

        // A relative floor also bounds subdivision for enormous but finite
        // radii. It prevents resource usage growing without bound with radius.
        let tolerance = 0.01_f64.max(arc.radii.x.max(arc.radii.y) / 4096.0);
        for element in arc.path_elements(tolerance) {
            match element {
                PathEl::MoveTo(p) => self.line_to(p.x, p.y),
                PathEl::CurveTo(a, b, p) => self.bezier_curve_to(a.x, a.y, b.x, b.y, p.x, p.y),
                _ => unreachable!("kurbo arcs contain only a start point and cubic segments"),
            }
        }
    }

    pub fn set_transform(&mut self, a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) {
        if [a, b, c, d, e, f].into_iter().all(f64::is_finite) {
            self.transform = Affine::new([a, b, c, d, e, f]);
        }
    }

    pub fn reset_transform(&mut self) {
        self.transform = Affine::IDENTITY;
    }

    pub fn translate(&mut self, x: f64, y: f64) {
        self.concatenate_transform(1.0, 0.0, 0.0, 1.0, x, y);
    }

    pub fn scale(&mut self, x: f64, y: f64) {
        self.concatenate_transform(x, 0.0, 0.0, y, 0.0, 0.0);
    }

    pub fn rotate(&mut self, radians: f64) {
        let (sin, cos) = radians.sin_cos();
        self.concatenate_transform(cos, sin, -sin, cos, 0.0, 0.0);
    }

    pub fn concatenate_transform(&mut self, a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) {
        if [a, b, c, d, e, f].into_iter().all(f64::is_finite) {
            self.transform *= Affine::new([a, b, c, d, e, f]);
        }
    }

    /// The current transform, mapping user space to canvas space.
    pub fn transform(&self) -> Affine {
        self.transform
    }

    fn map_point(&self, x: f64, y: f64) -> (f64, f64) {
        let p = self.transform * Point::new(x, y);
        (p.x, p.y)
    }

    /// Inverse of the current transform, `None` for singular transforms. A
    /// small nonzero scale is still invertible; do not use `EPSILON` as a
    /// singularity threshold (e.g. `scale(1e-9, 1e-9)` is valid).
    pub fn inverse_transform(&self) -> Option<Affine> {
        let determinant = self.transform.determinant();
        (determinant != 0.0 && determinant.is_finite()).then(|| self.transform.inverse())
    }

    /// Returns the frozen path mapped back to the *current* user space, so a
    /// later transform can run the painter over the path for anisotropic
    /// strokes and dashes. Returns `None` when the transform is singular.
    pub fn stroke_path(&self) -> Option<CanvasPathData> {
        let inverse = self.inverse_transform()?;
        let map = |p: Point| {
            let p = inverse * p;
            point(p.x, p.y)
        };
        let elements = self
            .elements
            .iter()
            .map(|element| match *element {
                kurbo::PathEl::MoveTo(p) => kurbo::PathEl::MoveTo(map(p)),
                kurbo::PathEl::LineTo(p) => kurbo::PathEl::LineTo(map(p)),
                kurbo::PathEl::QuadTo(a, p) => kurbo::PathEl::QuadTo(map(a), map(p)),
                kurbo::PathEl::CurveTo(a, b, p) => kurbo::PathEl::CurveTo(map(a), map(b), map(p)),
                kurbo::PathEl::ClosePath => kurbo::PathEl::ClosePath,
            })
            .collect::<Vec<_>>();
        Some(CanvasPathData {
            bounds: path_bounds(&elements),
            elements,
        })
    }

    /// Builds an owned `CanvasPathData` with conservative local bounds.
    pub fn paint_path(&self) -> CanvasPathData {
        CanvasPathData {
            elements: self.elements.clone(),
            bounds: path_bounds(&self.elements),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
}

/// Casts a coordinate pair to the same f32 truncation the browser's painted
/// path consumes, preserving observable behavior of the original adapter.
fn point(x: f64, y: f64) -> Point {
    Point::new((x as f32) as f64, (y as f32) as f64)
}

fn path_bounds(elements: &[kurbo::PathEl]) -> Rect {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for element in elements {
        match *element {
            kurbo::PathEl::MoveTo(point) | kurbo::PathEl::LineTo(point) => {
                expand(&mut min_x, &mut min_y, &mut max_x, &mut max_y, point);
            }
            kurbo::PathEl::QuadTo(first, second) => {
                expand(&mut min_x, &mut min_y, &mut max_x, &mut max_y, first);
                expand(&mut min_x, &mut min_y, &mut max_x, &mut max_y, second);
            }
            kurbo::PathEl::CurveTo(first, second, third) => {
                expand(&mut min_x, &mut min_y, &mut max_x, &mut max_y, first);
                expand(&mut min_x, &mut min_y, &mut max_x, &mut max_y, second);
                expand(&mut min_x, &mut min_y, &mut max_x, &mut max_y, third);
            }
            kurbo::PathEl::ClosePath => {}
        }
    }
    if !min_x.is_finite() {
        return Rect::ZERO;
    }
    Rect::new(
        (min_x as f32) as f64,
        (min_y as f32) as f64,
        ((max_x - min_x) as f32) as f64,
        ((max_y - min_y) as f32) as f64,
    )
}

fn expand(min_x: &mut f64, min_y: &mut f64, max_x: &mut f64, max_y: &mut f64, point: Point) {
    *min_x = (*min_x).min(point.x);
    *min_y = (*min_y).min(point.y);
    *max_x = (*max_x).max(point.x);
    *max_y = (*max_y).max(point.y);
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
            // Like Blink's AdjustEndAngle, preserve a whole turn for
            // opposite-direction endpoints separated by an exact TAU multiple.
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
                    let mut state = CanvasPath::default();
                    assert!(state.arc(10.0, 10.0, 5.0, start, end, ccw));
                    assert!(state.elements.len() <= 7);
                }
            }
        }
    }

    #[test]
    fn arc_to_has_the_correct_tangent_and_endpoint() {
        let mut state = CanvasPath::default();
        state.move_to(0.0, 0.0);
        assert!(state.arc_to(10.0, 0.0, 10.0, 10.0, 5.0));
        let kurbo::PathEl::LineTo(tangent) = state.elements[1] else {
            panic!("arcTo must connect to its first tangent");
        };
        assert!((tangent.x - 5.0).abs() < 1e-5 && tangent.y.abs() < 1e-5);
        assert!((state.current.x - 10.0).abs() < 1e-5);
        assert!((state.current.y - 5.0).abs() < 1e-5);
    }

    #[test]
    fn rejected_arc_geometry_does_not_mutate_the_path() {
        let mut state = CanvasPath::default();
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            assert!(!state.arc(0.0, 0.0, invalid, 0.0, 1.0, false));
            assert!(!state.ellipse(0.0, 0.0, 5.0, invalid, 0.0, 0.0, 1.0, false));
            assert!(!state.arc_to(0.0, 0.0, 1.0, 1.0, invalid));
            assert!(state.is_empty());
        }
    }

    #[test]
    fn default_path_transform_applies_at_command_time_only() {
        let mut state = CanvasPath::default();
        state.move_to(0.0, 0.0);
        state.line_to(10.0, 0.0);
        let before = state.paint_path();
        // A later transform must not move already-recorded elements.
        state.set_transform(2.0, 0.0, 0.0, 2.0, 100.0, 100.0);
        let after = state.paint_path();
        assert_eq!(before.elements.len(), after.elements.len());
        assert_eq!(
            before.elements[1], after.elements[1],
            "earlier commands keep their transform"
        );
        // New commands after the transform change are transformed.
        state.line_to(1.0, 1.0);
        let kurbo::PathEl::LineTo(q) = state.paint_path().elements[2] else {
            panic!("expected line")
        };
        assert!((q.x - 102.0).abs() < 1e-6 && (q.y - 102.0).abs() < 1e-6);
    }
}
