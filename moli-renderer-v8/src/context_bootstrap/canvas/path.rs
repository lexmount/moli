//! Canvas 2D current-path geometry, owned by the context's native state.

use moli_layout::{
    LayoutPoint, LayoutRect, LayoutTransform2D, PaintPath, PaintPathElement, PaintRect,
};

/// A Bezier path in the same flat form as a kurbo path: one `MoveTo` starts a
/// subpath, `Close` ends it, and a new `MoveTo` begins the next one.
#[derive(Clone, Debug)]
pub(crate) struct Canvas2dPathState {
    // Default-path geometry is already in canvas coordinates. The current
    // transform only affects newly recorded commands, never earlier elements.
    elements: Vec<PaintPathElement>,
    current: (f64, f64),
    current_subpath_start: (f64, f64),
    has_subpath: bool,
    just_closed: bool,
    transform: LayoutTransform2D,
}

impl Default for Canvas2dPathState {
    fn default() -> Self {
        Self {
            elements: Vec::new(),
            current: (0.0, 0.0),
            current_subpath_start: (0.0, 0.0),
            has_subpath: false,
            just_closed: false,
            transform: LayoutTransform2D::IDENTITY,
        }
    }
}

const TWO_PI: f64 = std::f64::consts::TAU;

impl Canvas2dPathState {
    pub(super) fn begin_path(&mut self) {
        self.elements.clear();
        self.current = (0.0, 0.0);
        self.current_subpath_start = (0.0, 0.0);
        self.has_subpath = false;
        self.just_closed = false;
    }

    pub(super) fn move_to(&mut self, x: f64, y: f64) {
        if ![x, y].into_iter().all(f64::is_finite) {
            return;
        }
        let (x, y) = self.map_point(x, y);
        self.elements.push(PaintPathElement::MoveTo(point(x, y)));
        self.current = (x, y);
        self.current_subpath_start = (x, y);
        self.has_subpath = true;
        self.just_closed = false;
    }

    /// Reopens a closed subpath; callers establish the initial point for an empty path.
    fn ensure_open_subpath(&mut self) -> bool {
        if !self.has_subpath {
            return false;
        }
        if self.just_closed {
            self.elements.push(PaintPathElement::MoveTo(point(
                self.current.0,
                self.current.1,
            )));
            self.current_subpath_start = self.current;
            self.just_closed = false;
        }
        true
    }

    pub(super) fn line_to(&mut self, x: f64, y: f64) {
        if ![x, y].into_iter().all(f64::is_finite) {
            return;
        }
        if !self.ensure_open_subpath() {
            self.move_to(x, y);
            return;
        }
        let (x, y) = self.map_point(x, y);
        self.elements.push(PaintPathElement::LineTo(point(x, y)));
        self.current = (x, y);
    }

    pub(super) fn quadratic_curve_to(&mut self, cpx: f64, cpy: f64, x: f64, y: f64) {
        if ![cpx, cpy, x, y].into_iter().all(f64::is_finite) {
            return;
        }
        if !self.ensure_open_subpath() {
            self.move_to(cpx, cpy);
        }
        let (cpx, cpy) = self.map_point(cpx, cpy);
        let (x, y) = self.map_point(x, y);
        self.elements
            .push(PaintPathElement::QuadTo(point(cpx, cpy), point(x, y)));
        self.current = (x, y);
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
        if ![c1x, c1y, c2x, c2y, x, y].into_iter().all(f64::is_finite) {
            return;
        }
        if !self.ensure_open_subpath() {
            self.move_to(c1x, c1y);
        }
        let (c1x, c1y) = self.map_point(c1x, c1y);
        let (c2x, c2y) = self.map_point(c2x, c2y);
        let (x, y) = self.map_point(x, y);
        self.elements.push(PaintPathElement::CubicTo(
            point(c1x, c1y),
            point(c2x, c2y),
            point(x, y),
        ));
        self.current = (x, y);
    }

    pub(super) fn close_path(&mut self) {
        if !self.has_subpath || self.just_closed {
            return;
        }
        self.elements.push(PaintPathElement::Close);
        self.current = self.current_subpath_start;
        self.just_closed = true;
    }

    pub(super) fn rect(&mut self, x: f64, y: f64, width: f64, height: f64) {
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
        let p0 = inverse * kurbo::Point::from(self.current);
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

    pub(super) fn set_transform(&mut self, a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) {
        if [a, b, c, d, e, f].into_iter().all(f64::is_finite) {
            self.transform = LayoutTransform2D::new([a, b, c, d, e, f]);
        }
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

    fn map_point(&self, x: f64, y: f64) -> (f64, f64) {
        let p = kurbo::Affine::new(self.transform.coefficients) * kurbo::Point::new(x, y);
        (p.x, p.y)
    }

    pub(super) fn inverse_transform(&self) -> Option<kurbo::Affine> {
        let transform = kurbo::Affine::new(self.transform.coefficients);
        let determinant = transform.determinant();
        // A small nonzero scale is still invertible; do not use EPSILON as
        // a singularity threshold (e.g. scale(1e-9,1e-9) is valid).
        (determinant != 0.0 && determinant.is_finite()).then(|| transform.inverse())
    }

    /// Stroke metrics are in the *current* user space. Bring the frozen path
    /// back to that space and let the painter transform the resulting stroke,
    /// so anisotropic line widths, joins and dashes change without moving it.
    pub(super) fn stroke_path(&self) -> Option<PaintPath> {
        let inverse = self.inverse_transform()?;
        let map = |p: LayoutPoint| {
            let p = inverse * kurbo::Point::new(f64::from(p.x), f64::from(p.y));
            point(p.x, p.y)
        };
        let elements = self
            .elements
            .iter()
            .map(|element| match *element {
                PaintPathElement::MoveTo(p) => PaintPathElement::MoveTo(map(p)),
                PaintPathElement::LineTo(p) => PaintPathElement::LineTo(map(p)),
                PaintPathElement::QuadTo(a, p) => PaintPathElement::QuadTo(map(a), map(p)),
                PaintPathElement::CubicTo(a, b, p) => {
                    PaintPathElement::CubicTo(map(a), map(b), map(p))
                }
                PaintPathElement::Close => PaintPathElement::Close,
            })
            .collect::<Vec<_>>();
        Some(PaintPath {
            bounds: path_bounds(&elements),
            elements,
        })
    }

    /// Builds an owned `PaintPath` with conservative local bounds.
    pub(super) fn paint_path(&self) -> PaintPath {
        PaintPath {
            elements: self.elements.clone(),
            bounds: path_bounds(&self.elements),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.elements.is_empty()
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

fn point(x: f64, y: f64) -> LayoutPoint {
    LayoutPoint::new(x as f32, y as f32)
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
        let PaintPathElement::LineTo(tangent) = state.elements[1] else {
            panic!("arcTo must connect to its first tangent");
        };
        assert!((tangent.x - 5.0).abs() < 1e-5 && tangent.y.abs() < 1e-5);
        assert!((state.current.0 - 10.0).abs() < 1e-5);
        assert!((state.current.1 - 5.0).abs() < 1e-5);
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
