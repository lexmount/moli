use crate::helpers::svg_number_list;
use crate::matrix::SvgMatrixComponents;
use crate::path::path_geometry;
use kurbo::{
    Arc, BezPath, Circle, Ellipse, Line, ParamCurve, ParamCurveArclen, ParamCurveExtrema,
    ParamCurveNearest, PathEl, PathSeg, Point, Rect, Shape,
};

const SHAPE_PATH_TOLERANCE: f64 = 0.1;
const ARC_LENGTH_ACCURACY: f64 = 1e-6;
const FILL_BOUNDARY_EPSILON: f64 = 1e-9;

#[derive(Clone, Copy, Debug)]
pub struct SvgGeometryPoint {
    pub x: f64,
    pub y: f64,
}

impl SvgGeometryPoint {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SvgGeometrySegment {
    inner: PathSeg,
}

impl SvgGeometrySegment {
    fn new(inner: PathSeg) -> Self {
        Self { inner }
    }

    pub fn length(&self) -> f64 {
        self.inner.arclen(ARC_LENGTH_ACCURACY)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SvgGeometryBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl SvgGeometryBox {
    pub fn union(self, other: Self) -> Self {
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = (self.x + self.width).max(other.x + other.width);
        let y1 = (self.y + self.height).max(other.y + other.height);
        Self {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        }
    }
}

pub enum SvgGeometryElement {
    Circle {
        cx: f64,
        cy: f64,
        r: f64,
    },
    Ellipse {
        cx: f64,
        cy: f64,
        rx: f64,
        ry: f64,
    },
    Line {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
    },
    Path {
        d: String,
    },
    Polygon {
        points: String,
    },
    Polyline {
        points: String,
    },
    Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        rx: f64,
        ry: f64,
    },
}

pub fn segments_for_element(element: SvgGeometryElement) -> Vec<SvgGeometrySegment> {
    let path = distance_query_path(&element).unwrap_or_default();
    let segments = path
        .segments()
        .map(SvgGeometrySegment::new)
        .collect::<Vec<_>>();
    if !segments.is_empty() {
        return segments;
    }
    path.elements()
        .iter()
        .find_map(|element| match element {
            PathEl::MoveTo(point) => Some(vec![SvgGeometrySegment::new(
                Line::new(*point, *point).into(),
            )]),
            _ => None,
        })
        .unwrap_or_default()
}

pub fn bounding_box_for_segments(segments: &[SvgGeometrySegment]) -> Option<SvgGeometryBox> {
    let mut segments = segments.iter();
    let first = segments.next()?;
    let bounds = segments.fold(
        ParamCurveExtrema::bounding_box(&first.inner),
        |bounds, segment| bounds.union(ParamCurveExtrema::bounding_box(&segment.inner)),
    );
    Some(SvgGeometryBox {
        x: bounds.x0,
        y: bounds.y0,
        width: bounds.width(),
        height: bounds.height(),
    })
}

pub fn bounding_box_for_element(element: &SvgGeometryElement) -> Option<SvgGeometryBox> {
    bounding_box_for_transformed_element(element, SvgMatrixComponents::identity())
}

pub fn bounding_box_for_transformed_element(
    element: &SvgGeometryElement,
    transform: SvgMatrixComponents,
) -> Option<SvgGeometryBox> {
    let affine = transform.to_affine();
    let analytic_bounds = match element {
        SvgGeometryElement::Circle { cx, cy, r } if *r > 0.0 => {
            Some((affine * Circle::new(Point::new(*cx, *cy), *r)).bounding_box())
        }
        SvgGeometryElement::Ellipse { cx, cy, rx, ry } => {
            let (rx, ry) = normalized_ellipse_radii(*rx, *ry);
            (rx > 0.0 && ry > 0.0).then(|| {
                (affine * Ellipse::new(Point::new(*cx, *cy), (rx, ry), 0.0)).bounding_box()
            })
        }
        _ => None,
    };
    if let Some(bounds) = analytic_bounds {
        return Some(svg_box(bounds));
    }
    let path = geometry_path(element)?;
    if path.elements().is_empty() {
        return None;
    }
    if path.segments().next().is_none() {
        let point = path
            .elements()
            .iter()
            .rev()
            .find_map(|element| match element {
                PathEl::MoveTo(point) => Some(affine * *point),
                _ => None,
            })?;
        return Some(SvgGeometryBox {
            x: point.x,
            y: point.y,
            width: 0.0,
            height: 0.0,
        });
    }
    let bounds = (affine * path).bounding_box();
    Some(svg_box(bounds))
}

fn svg_box(bounds: Rect) -> SvgGeometryBox {
    SvgGeometryBox {
        x: bounds.x0,
        y: bounds.y0,
        width: bounds.width(),
        height: bounds.height(),
    }
}

pub fn is_point_in_fill(element: &SvgGeometryElement, point: SvgGeometryPoint) -> bool {
    if matches!(element, SvgGeometryElement::Line { .. }) {
        return false;
    }
    geometry_path(element).is_some_and(|path| {
        let path = close_subpaths_for_fill(&path);
        let point = Point::new(point.x, point.y);
        path.winding(point) != 0
            || path.segments().any(|segment| {
                segment.nearest(point, FILL_BOUNDARY_EPSILON).distance_sq
                    <= FILL_BOUNDARY_EPSILON.powi(2)
            })
    })
}

pub fn point_at_length(segments: &[SvgGeometrySegment], distance: f64) -> SvgGeometryPoint {
    let Some(first) = segments.first() else {
        return SvgGeometryPoint::new(0.0, 0.0);
    };
    if distance <= 0.0 {
        return svg_point(first.inner.start());
    }
    let mut remaining = distance;
    for segment in segments {
        let length = segment.length();
        if length == 0.0 {
            continue;
        }
        if remaining <= length {
            let parameter = segment.inner.inv_arclen(remaining, ARC_LENGTH_ACCURACY);
            return svg_point(segment.inner.eval(parameter));
        }
        remaining -= length;
    }
    segments
        .last()
        .map(|segment| svg_point(segment.inner.end()))
        .unwrap_or(SvgGeometryPoint::new(0.0, 0.0))
}

fn distance_query_path(element: &SvgGeometryElement) -> Option<BezPath> {
    // A zero axis disables rendering, but distance queries can still compute
    // the equivalent path along the other axis.
    match element {
        SvgGeometryElement::Rect {
            x,
            y,
            width,
            height,
            ..
        } if (*width == 0.0 && *height > 0.0) || (*height == 0.0 && *width > 0.0) => {
            Some(Rect::new(*x, *y, *x + *width, *y + *height).to_path(SHAPE_PATH_TOLERANCE))
        }
        SvgGeometryElement::Ellipse { cx, cy, rx, ry }
            if (*rx == 0.0 && *ry > 0.0) || (*ry == 0.0 && *rx > 0.0) =>
        {
            // Collapsed quarter arcs are lines. Preserve SVG's starting point
            // and sweep instead of normalizing the ellipse's axes.
            let mut path = BezPath::new();
            path.move_to((*cx + *rx, *cy));
            path.line_to((*cx, *cy + *ry));
            path.line_to((*cx - *rx, *cy));
            path.line_to((*cx, *cy - *ry));
            path.close_path();
            Some(path)
        }
        _ => geometry_path(element),
    }
}

fn geometry_path(element: &SvgGeometryElement) -> Option<BezPath> {
    match element {
        SvgGeometryElement::Circle { cx, cy, r } => Some(if *r > 0.0 {
            Circle::new(Point::new(*cx, *cy), *r).to_path(SHAPE_PATH_TOLERANCE)
        } else {
            BezPath::new()
        }),
        SvgGeometryElement::Ellipse { cx, cy, rx, ry } => {
            let (rx, ry) = normalized_ellipse_radii(*rx, *ry);
            Some(if rx > 0.0 && ry > 0.0 {
                Ellipse::new(Point::new(*cx, *cy), (rx, ry), 0.0).to_path(SHAPE_PATH_TOLERANCE)
            } else {
                BezPath::new()
            })
        }
        SvgGeometryElement::Line { x1, y1, x2, y2 } => {
            let mut path = BezPath::new();
            path.move_to((*x1, *y1));
            path.line_to((*x2, *y2));
            Some(path)
        }
        SvgGeometryElement::Path { d } => path_geometry(d),
        SvgGeometryElement::Polygon { points } => poly_points_geometry_path(points, true),
        SvgGeometryElement::Polyline { points } => poly_points_geometry_path(points, false),
        SvgGeometryElement::Rect {
            x,
            y,
            width,
            height,
            rx,
            ry,
        } => Some(rect_path(*x, *y, *width, *height, *rx, *ry)),
    }
}

fn rect_path(x: f64, y: f64, width: f64, height: f64, rx: f64, ry: f64) -> BezPath {
    if width <= 0.0 || height <= 0.0 {
        return BezPath::new();
    }
    let (rx, ry) = normalized_rect_radii(width, height, rx, ry);
    if rx > 0.0 && ry > 0.0 {
        return rounded_rect_path(x, y, width, height, rx, ry);
    }
    Rect::new(x, y, x + width, y + height).to_path(SHAPE_PATH_TOLERANCE)
}

fn normalized_ellipse_radii(rx: f64, ry: f64) -> (f64, f64) {
    match (rx.is_sign_negative(), ry.is_sign_negative()) {
        (true, false) => (ry, ry),
        (false, true) => (rx, rx),
        _ => (rx, ry),
    }
}

fn normalized_rect_radii(width: f64, height: f64, rx: f64, ry: f64) -> (f64, f64) {
    if rx <= 0.0 || ry <= 0.0 {
        return (0.0, 0.0);
    }
    (rx.min(width / 2.0), ry.min(height / 2.0))
}

fn rounded_rect_path(x: f64, y: f64, width: f64, height: f64, rx: f64, ry: f64) -> BezPath {
    let mut path = BezPath::new();
    path.move_to((x + rx, y));
    path.line_to((x + width - rx, y));
    append_quarter_ellipse(&mut path, x + width - rx, y + ry, rx, ry, -90.0);
    path.line_to((x + width, y + height - ry));
    append_quarter_ellipse(&mut path, x + width - rx, y + height - ry, rx, ry, 0.0);
    path.line_to((x + rx, y + height));
    append_quarter_ellipse(&mut path, x + rx, y + height - ry, rx, ry, 90.0);
    path.line_to((x, y + ry));
    append_quarter_ellipse(&mut path, x + rx, y + ry, rx, ry, 180.0);
    path.close_path();
    path
}

fn append_quarter_ellipse(
    path: &mut BezPath,
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    start_degrees: f64,
) {
    path.extend(
        Arc::new(
            Point::new(cx, cy),
            (rx, ry),
            start_degrees.to_radians(),
            std::f64::consts::FRAC_PI_2,
            0.0,
        )
        .append_iter(SHAPE_PATH_TOLERANCE),
    );
}

fn poly_points_geometry_path(raw: &str, close: bool) -> Option<BezPath> {
    let coordinates = svg_number_list(raw)?;
    if coordinates.len() < 2 || !coordinates.len().is_multiple_of(2) {
        return None;
    }
    let mut coordinates = coordinates.as_chunks::<2>().0.iter();
    let first = coordinates.next()?;
    let mut path = BezPath::new();
    path.move_to((first[0], first[1]));
    for pair in coordinates {
        path.line_to((pair[0], pair[1]));
    }
    if close {
        path.close_path();
    }
    Some(path)
}

fn close_subpaths_for_fill(path: &BezPath) -> BezPath {
    let mut closed = BezPath::new();
    let mut subpath_is_open = false;
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(point) => {
                if subpath_is_open {
                    closed.close_path();
                }
                closed.move_to(point);
                subpath_is_open = true;
            }
            PathEl::ClosePath => {
                closed.close_path();
                subpath_is_open = false;
            }
            element => {
                closed.push(element);
                subpath_is_open = true;
            }
        }
    }
    if subpath_is_open {
        closed.close_path();
    }
    closed
}

fn svg_point(point: Point) -> SvgGeometryPoint {
    SvgGeometryPoint::new(point.x, point.y)
}
