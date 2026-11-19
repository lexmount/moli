use kurbo::BezPath;
use svgtypes::{SimplePathSegment, SimplifyingPathParser};

/// Parse SVG path data, preserving the complete segments before any parse error.
pub fn parse_path_data(raw: &str) -> BezPath {
    let mut path = BezPath::new();
    for segment in SimplifyingPathParser::from(raw) {
        let Ok(segment) = segment else {
            break;
        };
        match segment {
            SimplePathSegment::MoveTo { x, y } => path.move_to((x, y)),
            SimplePathSegment::LineTo { x, y } => path.line_to((x, y)),
            SimplePathSegment::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => path.curve_to((x1, y1), (x2, y2), (x, y)),
            SimplePathSegment::Quadratic { x1, y1, x, y } => {
                path.quad_to((x1, y1), (x, y));
            }
            SimplePathSegment::ClosePath => path.close_path(),
        }
    }
    path
}
