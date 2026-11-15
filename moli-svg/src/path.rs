use kurbo::BezPath;
use svgtypes::{SimplePathSegment, SimplifyingPathParser};

pub(crate) fn path_geometry(raw: &str) -> Option<BezPath> {
    let mut path = BezPath::new();
    for segment in SimplifyingPathParser::from(path_data_before_invalid_number(raw)) {
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

// svgtypes accepts legacy trailing decimal points. SVG 2 aligns path numbers
// with CSS; bound its input before the first malformed number so complete
// preceding commands still contribute geometry.
// https://svgwg.org/svg2-draft/paths.html#PathDataBNF
fn path_data_before_invalid_number(raw: &str) -> &str {
    let bytes = raw.as_bytes();
    let mut position = 0;
    while let Some(&byte) = bytes.get(position) {
        match byte {
            b'\t' | b'\n' | b'\x0c' | b'\r' | b' ' | b',' | b'A' | b'C' | b'H' | b'L' | b'M'
            | b'Q' | b'S' | b'T' | b'V' | b'Z' | b'a' | b'c' | b'h' | b'l' | b'm' | b'q' | b's'
            | b't' | b'v' | b'z' => position += 1,
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let number_start = position;
                if matches!(bytes[position], b'+' | b'-') {
                    position += 1;
                }

                let integer_start = position;
                while bytes.get(position).is_some_and(u8::is_ascii_digit) {
                    position += 1;
                }
                let has_integer = position != integer_start;

                if bytes.get(position) == Some(&b'.') {
                    position += 1;
                    let fraction_start = position;
                    while bytes.get(position).is_some_and(u8::is_ascii_digit) {
                        position += 1;
                    }
                    if position == fraction_start {
                        return &raw[..number_start];
                    }
                } else if !has_integer {
                    return &raw[..number_start];
                }

                if matches!(bytes.get(position), Some(b'e' | b'E')) {
                    position += 1;
                    if matches!(bytes.get(position), Some(b'+' | b'-')) {
                        position += 1;
                    }
                    let exponent_start = position;
                    while bytes.get(position).is_some_and(u8::is_ascii_digit) {
                        position += 1;
                    }
                    if position == exponent_start {
                        return &raw[..number_start];
                    }
                }
            }
            _ => return raw,
        }
    }
    raw
}

#[cfg(test)]
mod number_grammar_tests {
    use super::*;

    #[test]
    fn malformed_numbers_preserve_only_complete_geometry_commands() {
        let prefix = path_geometry("M1 2L3 4").unwrap();
        for suffix in [
            "L1. 2L8 9",
            "L. 2L8 9",
            "L1e 2L8 9",
            "L1e+ 2L8 9",
            "L+ 2L8 9",
        ] {
            let raw = format!("M1 2L3 4{suffix}");
            assert_eq!(path_geometry(&raw).unwrap(), prefix, "{raw}");
        }
        assert!(path_geometry("M1. 2L3 4").unwrap().elements().is_empty());
    }

    #[test]
    fn valid_signed_fractional_and_exponent_numbers_retain_geometry() {
        let expected = path_geometry("M1 2L3 4L0.5 -0.25L10 20").unwrap();
        assert_eq!(
            path_geometry("M+1 +2L3e0 4E+0L.5-.25L1e1 2e1").unwrap(),
            expected
        );
        let raw = "M1 2L3 4L1é 2";
        assert_eq!(
            path_geometry(raw).unwrap(),
            path_geometry("M1 2L3 4").unwrap()
        );
    }
}
