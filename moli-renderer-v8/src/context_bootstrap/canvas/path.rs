//! Canvas 2D current-path geometry.
//!
//! The CanvasRenderingContext2D path methods are thin V8 callbacks that
//! translate into this module's geometry. The accumulated path is kept in a
//! per-context side table keyed by a private-slot id (V8 private slots can
//! only hold `Value`, so a numeric id + global map is used instead of storing
//! a `Vec` directly on the object).
//!
//! Path elements map 1:1 onto [`moli_layout::PaintPathElement`] so the
//! existing `moli_paint::raster_snapshot` pipeline (vello CPU) can fill and
//! stroke them.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use moli_layout::{
    LayoutPoint, LayoutRect, LayoutTransform2D, PaintPath, PaintPathElement, PaintRect,
};
use parking_lot::Mutex;

pub(crate) const CANVAS_CONTEXT_PATH_STATE_ID_SLOT: &str = "__moliCanvasContextPathStateId";

/// A Bezier path in the same flat form as a kurbo path: one `MoveTo` starts a
/// subpath, `Close` ends it, and a new `MoveTo` begins the next one.
#[derive(Clone, Debug)]
pub(crate) struct Canvas2dPathState {
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

static NEXT_CANVAS_PATH_STATE_ID: AtomicU64 = AtomicU64::new(1);
static CANVAS_PATH_STATES: OnceLock<Mutex<HashMap<u64, Canvas2dPathState>>> = OnceLock::new();

fn canvas_path_states() -> &'static Mutex<HashMap<u64, Canvas2dPathState>> {
    CANVAS_PATH_STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn allocate_canvas_path_state_id() -> u64 {
    NEXT_CANVAS_PATH_STATE_ID.fetch_add(1, Ordering::Relaxed)
}

/// Runs `update` against the current-path state for `id`, inserting a fresh
/// state when the context has never recorded one.
pub(crate) fn with_canvas_path_state_mut<T>(
    id: u64,
    update: impl FnOnce(&mut Canvas2dPathState) -> T,
) -> T {
    let mut states = canvas_path_states().lock();
    let state = states.entry(id).or_default();
    update(state)
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
        self.elements.push(PaintPathElement::MoveTo(point(x, y)));
        self.current = (x, y);
        self.current_subpath_start = (x, y);
        self.has_subpath = true;
        self.just_closed = false;
    }

    /// Returns `false` when the path has no subpath, matching the spec: line
    /// and curve commands must do nothing when the path has no subpaths.
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
        if !self.ensure_open_subpath() {
            return;
        }
        self.elements.push(PaintPathElement::LineTo(point(x, y)));
        self.current = (x, y);
    }

    pub(super) fn quadratic_curve_to(&mut self, cpx: f64, cpy: f64, x: f64, y: f64) {
        if !self.ensure_open_subpath() {
            return;
        }
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
        if !self.ensure_open_subpath() {
            return;
        }
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
        self.move_to(x, y);
        self.line_to(x + width, y);
        self.line_to(x + width, y + height);
        self.line_to(x, y + height);
        self.close_path();
    }

    /// Appends the path for `ctx.arc(...)`. Returns `false` when the arguments
    /// are non-finite or the radius is negative (caller reports the error).
    pub(super) fn arc(
        &mut self,
        x: f64,
        y: f64,
        radius: f64,
        start: f64,
        end: f64,
        ccw: bool,
    ) -> bool {
        if !x.is_finite()
            || !y.is_finite()
            || !radius.is_finite()
            || !start.is_finite()
            || !end.is_finite()
        {
            return false;
        }
        if radius < 0.0 {
            return false;
        }
        if radius == 0.0 {
            return true;
        }
        let (start, end) = normalized_arc_angles(start, end, ccw);
        let start_point = (x + radius * start.cos(), y + radius * start.sin());
        if !self.has_subpath {
            self.move_to(start_point.0, start_point.1);
        } else if self.current != start_point {
            self.line_to(start_point.0, start_point.1);
        }
        self.push_arc_cubics((x, y), radius, start, end, ccw);
        self.current = (x + radius * end.cos(), y + radius * end.sin());
        true
    }

    /// Appends the path for `ctx.ellipse(...)`.
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
        if !x.is_finite()
            || !y.is_finite()
            || !radius_x.is_finite()
            || !radius_y.is_finite()
            || !rotation.is_finite()
            || !start.is_finite()
            || !end.is_finite()
        {
            return false;
        }
        if radius_x < 0.0 || radius_y < 0.0 {
            return false;
        }
        if radius_x == 0.0 || radius_y == 0.0 {
            return true;
        }
        let (start, end) = normalized_arc_angles(start, end, ccw);
        // Map unit-circle angles through the ellipse's scale + rotation.
        let ellipse_transform = LayoutTransform2D::IDENTITY
            .concatenate(LayoutTransform2D::rotation(rotation))
            .concatenate(LayoutTransform2D::scale(radius_x, radius_y))
            .concatenate(LayoutTransform2D::translation(x as f32, y as f32));
        let start_point = transform_point(ellipse_transform, start.cos(), start.sin());
        if !self.has_subpath {
            self.move_to(start_point.0, start_point.1);
        } else if self.current != start_point {
            self.line_to(start_point.0, start_point.1);
        }
        self.push_ellipse_cubics(ellipse_transform, start, end, ccw);
        let end_point = transform_point(ellipse_transform, end.cos(), end.sin());
        self.current = end_point;
        true
    }

    /// Appends the path for `ctx.arcTo(...)`.
    pub(super) fn arc_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, radius: f64) -> bool {
        if !x1.is_finite()
            || !y1.is_finite()
            || !x2.is_finite()
            || !y2.is_finite()
            || !radius.is_finite()
        {
            return false;
        }
        if radius < 0.0 {
            return false;
        }
        if !self.has_subpath {
            return true;
        }
        let (x0, y0) = self.current;
        let (p1, p2) = ((x1, y1), (x2, y2));
        if (x0, y0) == p1 || p1 == p2 || radius == 0.0 {
            self.line_to(x1, y1);
            return true;
        }
        let (x01, y01) = (p1.0 - x0, p1.1 - y0);
        let (x12, y12) = (p2.0 - p1.0, p2.1 - p1.1);
        let d01 = x01.hypot(y01);
        let d12 = x12.hypot(y12);
        if d01 <= f64::EPSILON || d12 <= f64::EPSILON {
            self.line_to(x1, y1);
            return true;
        }
        let cross = x01 * y12 - y01 * x12;
        if cross.abs() <= f64::EPSILON {
            self.line_to(x1, y1);
            return true;
        }
        let (ux01, uy01) = (x01 / d01, y01 / d01);
        let (ux12, uy12) = (x12 / d12, y12 / d12);
        let cos_theta = (x01 * x12 + y01 * y12) / (d01 * d12);
        let cos_theta = cos_theta.clamp(-1.0, 1.0);
        let theta = cos_theta.acos();
        let half = theta / 2.0;
        let tan_half = half.tan();
        if !tan_half.is_finite() || tan_half.abs() <= f64::EPSILON {
            self.line_to(x1, y1);
            return true;
        }
        let max_radius = d01.min(d12) * tan_half;
        let radius = radius.min(max_radius);
        let tangent = radius / tan_half;
        let (ta_x, ta_y) = (p1.0 + ux01 * tangent, p1.1 + uy01 * tangent);
        let (tb_x, tb_y) = (p1.0 + ux12 * tangent, p1.1 + uy12 * tangent);
        let (bx, by) = normalize((ux01 + ux12, uy01 + uy12));
        let sin_half = half.sin();
        if !sin_half.is_finite() || sin_half.abs() <= f64::EPSILON {
            self.line_to(x1, y1);
            return true;
        }
        let center = (
            p1.0 + bx * (radius / sin_half),
            p1.1 + by * (radius / sin_half),
        );
        let start_angle = (ta_y - center.1).atan2(ta_x - center.0);
        let end_angle = (tb_y - center.1).atan2(tb_x - center.0);
        let ccw = cross < 0.0;
        self.push_arc_cubics(center, radius, start_angle, end_angle, ccw);
        self.current = (tb_x, tb_y);
        true
    }

    /// Adds a straight connector to the current point when `connect` and the
    /// current point differs from `target`, then appends cubic approximations
    /// of the arc from `start` to `end` around `center` with radius `radius`.
    fn push_arc_cubics(
        &mut self,
        center: (f64, f64),
        radius: f64,
        start: f64,
        end: f64,
        ccw: bool,
    ) {
        let sweep = signed_arc_sweep(start, end, ccw);
        if sweep.abs() <= f64::EPSILON {
            return;
        }
        let segments = (sweep.abs() / (std::f64::consts::FRAC_PI_2)).ceil() as usize;
        let segments = segments.max(1);
        let delta = sweep / segments as f64;
        let mut angle = start;
        for _ in 0..segments {
            let next = angle + delta;
            let (cos_a, sin_a) = angle.sin_cos();
            let (cos_b, sin_b) = next.sin_cos();
            let alpha = (4.0 / 3.0) * (1.0 - (delta / 2.0).cos()) / (delta / 2.0).sin();
            let (p0x, p0y) = (center.0 + radius * cos_a, center.1 + radius * sin_a);
            let (c1x, c1y) = (p0x - alpha * radius * sin_a, p0y + alpha * radius * cos_a);
            let (p3x, p3y) = (center.0 + radius * cos_b, center.1 + radius * sin_b);
            let (c2x, c2y) = (p3x + alpha * radius * sin_b, p3y - alpha * radius * cos_b);
            self.elements.push(PaintPathElement::CubicTo(
                point(c1x, c1y),
                point(c2x, c2y),
                point(p3x, p3y),
            ));
            self.current = (p3x, p3y);
            angle = next;
        }
    }

    fn push_ellipse_cubics(
        &mut self,
        transform: LayoutTransform2D,
        start: f64,
        end: f64,
        ccw: bool,
    ) {
        let sweep = signed_arc_sweep(start, end, ccw);
        if sweep.abs() <= f64::EPSILON {
            return;
        }
        let segments = (sweep.abs() / (std::f64::consts::FRAC_PI_2)).ceil() as usize;
        let segments = segments.max(1);
        let delta = sweep / segments as f64;
        let mut angle = start;
        for _ in 0..segments {
            let next = angle + delta;
            let (cos_a, sin_a) = angle.sin_cos();
            let (cos_b, sin_b) = next.sin_cos();
            let alpha = (4.0 / 3.0) * (1.0 - (delta / 2.0).cos()) / (delta / 2.0).sin();
            let (p0x, p0y) = transform_point(transform, cos_a, sin_a);
            let (p3x, p3y) = transform_point(transform, cos_b, sin_b);
            // Tangents on the unit circle, then mapped through the transform.
            let tangent_a = radius_tangent(transform, -sin_a, cos_a);
            let tangent_b = radius_tangent(transform, -sin_b, cos_b);
            let (c1x, c1y) = (p0x + alpha * tangent_a.0, p0y + alpha * tangent_a.1);
            let (c2x, c2y) = (p3x - alpha * tangent_b.0, p3y - alpha * tangent_b.1);
            self.elements.push(PaintPathElement::CubicTo(
                point(c1x, c1y),
                point(c2x, c2y),
                point(p3x, p3y),
            ));
            self.current = (p3x, p3y);
            angle = next;
        }
    }

    pub(super) fn set_transform(&mut self, a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) {
        self.transform = LayoutTransform2D::new([a, b, c, d, e, f]);
    }

    pub(super) fn reset_transform(&mut self) {
        self.transform = LayoutTransform2D::IDENTITY;
    }

    pub(super) fn translate(&mut self, x: f64, y: f64) {
        self.transform = self
            .transform
            .concatenate(LayoutTransform2D::translation(x as f32, y as f32));
    }

    pub(super) fn scale(&mut self, x: f64, y: f64) {
        self.transform = self.transform.concatenate(LayoutTransform2D::scale(x, y));
    }

    pub(super) fn rotate(&mut self, radians: f64) {
        self.transform = self
            .transform
            .concatenate(LayoutTransform2D::rotation(radians));
    }

    pub(super) fn concatenate_transform(&mut self, a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) {
        self.transform = self
            .transform
            .concatenate(LayoutTransform2D::new([a, b, c, d, e, f]));
    }

    pub(super) fn transform(&self) -> LayoutTransform2D {
        self.transform
    }

    /// Builds an owned `PaintPath` with conservative local bounds.
    pub(super) fn paint_path(&self) -> PaintPath {
        PaintPath {
            elements: self.elements.clone(),
            bounds: self.bounds(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    fn bounds(&self) -> PaintRect {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for element in &self.elements {
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

fn normalize(vector: (f64, f64)) -> (f64, f64) {
    let length = vector.0.hypot(vector.1);
    if length <= f64::EPSILON {
        (0.0, 0.0)
    } else {
        (vector.0 / length, vector.1 / length)
    }
}

fn normalized_arc_angles(start: f64, end: f64, ccw: bool) -> (f64, f64) {
    let mut end = end;
    if ccw {
        while end > start {
            end -= TWO_PI;
        }
    } else {
        while end < start {
            end += TWO_PI;
        }
    }
    (start, end)
}

/// Signed sweep angle (positive counterclockwise, negative clockwise) in
/// `(-TAU, TAU]`. An exact full-circle difference is preserved as ±TAU.
fn signed_arc_sweep(start: f64, end: f64, ccw: bool) -> f64 {
    let difference = end - start;
    if difference.abs() >= TWO_PI {
        return if ccw { TWO_PI } else { -TWO_PI };
    }
    if ccw {
        difference.rem_euclid(TWO_PI)
    } else {
        -((start - end).rem_euclid(TWO_PI))
    }
}

fn transform_point(transform: LayoutTransform2D, x: f64, y: f64) -> (f64, f64) {
    let mapped = transform.map_point(LayoutPoint::new(x as f32, y as f32));
    (f64::from(mapped.x), f64::from(mapped.y))
}

/// Maps a unit-circle tangent through the ellipse's linear part only
/// (scale + rotation, excluding translation).
fn radius_tangent(transform: LayoutTransform2D, x: f64, y: f64) -> (f64, f64) {
    let [a, b, c, d, _, _] = transform.coefficients;
    (a * x + c * y, b * x + d * y)
}
