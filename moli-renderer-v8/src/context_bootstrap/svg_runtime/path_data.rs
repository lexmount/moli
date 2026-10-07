use std::fmt::Write;
use style::values::specified::svg_path::{CoordPair, PathCommand, SVGPathData};

/// Keep native computed coordinates when passing a cascaded path to geometry.
/// CSSOM number serialization rounds values for display and is not a geometry
/// representation. Promote each binary32 coordinate before writing it so the
/// double-precision geometry parser receives that coordinate's exact value.
pub(super) fn geometry_path_data(path: &SVGPathData) -> String {
    let path = path.normalize(false);
    let mut output = String::new();
    for command in path.commands() {
        let result = match *command {
            PathCommand::Move { point } => {
                let [x, y] = coordinates(point);
                write!(output, "M{x} {y} ")
            }
            PathCommand::Line { point } => {
                let [x, y] = coordinates(point);
                write!(output, "L{x} {y} ")
            }
            PathCommand::HLine { x } => write!(output, "H{} ", f64::from(f32::from(x))),
            PathCommand::VLine { y } => write!(output, "V{} ", f64::from(f32::from(y))),
            PathCommand::CubicCurve {
                point,
                control1,
                control2,
            } => {
                let [x, y] = coordinates(point);
                let [x1, y1] = coordinates(control1);
                let [x2, y2] = coordinates(control2);
                write!(output, "C{x1} {y1} {x2} {y2} {x} {y} ")
            }
            PathCommand::QuadCurve { point, control1 } => {
                let [x, y] = coordinates(point);
                let [x1, y1] = coordinates(control1);
                write!(output, "Q{x1} {y1} {x} {y} ")
            }
            PathCommand::SmoothCubic { point, control2 } => {
                let [x, y] = coordinates(point);
                let [x2, y2] = coordinates(control2);
                write!(output, "S{x2} {y2} {x} {y} ")
            }
            PathCommand::SmoothQuad { point } => {
                let [x, y] = coordinates(point);
                write!(output, "T{x} {y} ")
            }
            PathCommand::Arc {
                point,
                radii,
                arc_sweep,
                arc_size,
                rotate,
            } => {
                let [x, y] = coordinates(point);
                let rx = f64::from(radii.rx);
                let ry = f64::from(radii.ry.as_ref().copied().unwrap_or(radii.rx));
                let angle = f64::from(rotate);
                write!(
                    output,
                    "A{rx} {ry} {angle} {} {} {x} {y} ",
                    arc_size as u8, arc_sweep as u8
                )
            }
            PathCommand::Close => output.write_str("Z "),
        };
        result.expect("writing SVG geometry path data to a String is infallible");
    }
    output
}

fn coordinates(point: impl Into<CoordPair>) -> [f64; 2] {
    let point = point.into();
    [f64::from(point.x), f64::from(point.y)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_svg::{SvgGeometryElement, point_at_length, segments_for_element};

    #[test]
    fn computed_path_coordinates_retain_precision_across_svg_commands() {
        for raw in [
            "M0 0L134217728 0",
            "M0 0h134217728v1",
            "M1 2c0 1 1 1 2 0s1 -1 2 0",
            "M1 2q1 1 2 0t2 0",
            "M0 0a3 2 45 1 1 5 2",
            "M1 2L4 2L4 6z",
        ] {
            let (native, valid) = SVGPathData::parse_bytes(raw.as_bytes());
            assert!(valid);
            let serialized = geometry_path_data(&native);
            let (reparsed, valid) = SVGPathData::parse_bytes(serialized.as_bytes());
            assert!(valid);
            assert_eq!(reparsed.normalize(false), native.normalize(false), "{raw}");
            let segments = segments_for_element(SvgGeometryElement::Path { d: serialized });
            let point = point_at_length(&segments, f64::MAX);
            let direct = segments_for_element(SvgGeometryElement::Path { d: raw.to_owned() });
            let expected = point_at_length(&direct, f64::MAX);
            assert_eq!([point.x, point.y], [expected.x, expected.y], "{raw}");
        }
    }
}
