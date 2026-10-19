use kurbo::BezPath;
use svgtypes::{PathParser, PathSegment, SimplePathSegment, SimplifyingPathParser};

pub(crate) fn path_geometry(raw: &str) -> Option<BezPath> {
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
    Some(path)
}

/// A parsed SVG command retaining its relative form and original coordinates.
/// Consumers choose their own numeric boundary, such as SVG DOM binary32.
#[derive(Debug, Clone, PartialEq)]
pub struct SvgPathSegment {
    pub command: char,
    pub values: Vec<f64>,
}

/// Read complete commands until the first grammar error, without CSS clamping.
pub fn parse_path_segments(raw: &str) -> impl Iterator<Item = SvgPathSegment> + '_ {
    PathParser::from(raw).map_while(Result::ok).map(|segment| {
        let command = segment.command() as char;
        let values = match segment {
            PathSegment::MoveTo { x, y, .. }
            | PathSegment::LineTo { x, y, .. }
            | PathSegment::SmoothQuadratic { x, y, .. } => vec![x, y],
            PathSegment::HorizontalLineTo { x, .. } => vec![x],
            PathSegment::VerticalLineTo { y, .. } => vec![y],
            PathSegment::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
                ..
            } => vec![x1, y1, x2, y2, x, y],
            PathSegment::SmoothCurveTo { x2, y2, x, y, .. } => vec![x2, y2, x, y],
            PathSegment::Quadratic { x1, y1, x, y, .. } => vec![x1, y1, x, y],
            PathSegment::EllipticalArc {
                rx,
                ry,
                x_axis_rotation,
                large_arc,
                sweep,
                x,
                y,
                ..
            } => vec![
                rx,
                ry,
                x_axis_rotation,
                f64::from(u8::from(large_arc)),
                f64::from(u8::from(sweep)),
                x,
                y,
            ],
            PathSegment::ClosePath { .. } => vec![],
        };
        SvgPathSegment {
            command: if command.eq_ignore_ascii_case(&'z') {
                'Z'
            } else {
                command
            },
            values,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_path_segments_preserve_commands_and_stop_at_invalid_suffixes() {
        let segments: Vec<_> =
            parse_path_segments("m1 2 3 4h5v6c1 2 3 4 5 6s7 8 9 10q1 2 3 4t5 6a2 3 45 1 0 7 8z")
                .collect();
        assert_eq!(
            segments.iter().map(|s| s.command).collect::<String>(),
            "mlhvcsqtaZ"
        );
        assert_eq!(segments[1].values, vec![3.0, 4.0]);
        assert_eq!(segments[8].values, vec![2.0, 3.0, 45.0, 1.0, 0.0, 7.0, 8.0]);
        assert!(segments[9].values.is_empty());
        for raw in ["M1 2L", "M1 2L1. 2L3 4", "M1 2L1e 2L3 4", "M1 2X3 4"] {
            let parsed: Vec<_> = parse_path_segments(raw).collect();
            assert_eq!(parsed.len(), 1, "{raw}");
            assert_eq!(parsed[0].command, 'M');
            assert_eq!(parsed[0].values, vec![1.0, 2.0]);
        }
        assert_eq!(parse_path_segments("Z").count(), 0);
        assert_eq!(parse_path_segments("L1 2").count(), 0);
        // Geometry users retain double coordinates; DOM users must reject,
        // rather than clamp, a value that overflows their binary32 boundary.
        let huge: Vec<_> = parse_path_segments("M0 0L1e40 2").collect();
        assert_eq!(huge[1].values[0], 1e40);
        assert!(!(huge[1].values[0] as f32).is_finite());
    }
}
