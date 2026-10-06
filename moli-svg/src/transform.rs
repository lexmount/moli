use moli_css_parse::{UnitlessAngle, UnitlessLength};

use crate::helpers::number_len;
use crate::matrix::{SvgMatrixComponents, serialize_number};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvgTransformKind {
    Matrix,
    Translate,
    Scale,
    Rotate,
    SkewX,
    SkewY,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvgTransform {
    pub kind: SvgTransformKind,
    pub angle: f64,
    pub matrix: SvgMatrixComponents,
    pub rotation_center: (f64, f64),
}

impl SvgTransform {
    pub fn matrix(matrix: SvgMatrixComponents) -> Self {
        Self {
            kind: SvgTransformKind::Matrix,
            angle: 0.0,
            matrix,
            rotation_center: (0.0, 0.0),
        }
    }

    pub fn translate(tx: f64, ty: f64) -> Self {
        Self {
            kind: SvgTransformKind::Translate,
            angle: 0.0,
            matrix: SvgMatrixComponents::translate(tx, ty),
            rotation_center: (0.0, 0.0),
        }
    }

    pub fn scale(sx: f64, sy: f64) -> Self {
        Self {
            kind: SvgTransformKind::Scale,
            angle: 0.0,
            matrix: SvgMatrixComponents::scale(sx, sy),
            rotation_center: (0.0, 0.0),
        }
    }

    pub fn rotate(angle: f64, cx: f64, cy: f64) -> Self {
        Self {
            kind: SvgTransformKind::Rotate,
            angle,
            matrix: SvgMatrixComponents::rotate_around(angle, cx, cy),
            rotation_center: (cx, cy),
        }
    }

    pub fn skew_x(angle: f64) -> Self {
        Self {
            kind: SvgTransformKind::SkewX,
            angle,
            matrix: SvgMatrixComponents::skew_x(angle),
            rotation_center: (0.0, 0.0),
        }
    }

    pub fn skew_y(angle: f64) -> Self {
        Self {
            kind: SvgTransformKind::SkewY,
            angle,
            matrix: SvgMatrixComponents::skew_y(angle),
            rotation_center: (0.0, 0.0),
        }
    }

    pub fn serialize(self) -> String {
        match self.kind {
            SvgTransformKind::Matrix => self.matrix.serialize_transform_matrix(),
            SvgTransformKind::Translate => format!(
                "translate({} {})",
                serialize_number(self.matrix.e),
                serialize_number(self.matrix.f)
            ),
            SvgTransformKind::Scale => format!(
                "scale({} {})",
                serialize_number(self.matrix.a),
                serialize_number(self.matrix.d)
            ),
            SvgTransformKind::Rotate => format!(
                "rotate({} {} {})",
                serialize_number(self.angle),
                serialize_number(self.rotation_center.0),
                serialize_number(self.rotation_center.1)
            ),
            SvgTransformKind::SkewX => format!("skewX({})", serialize_number(self.angle)),
            SvgTransformKind::SkewY => format!("skewY({})", serialize_number(self.angle)),
        }
    }
}

pub fn parse_transform_attribute(raw: &str) -> Option<Vec<SvgTransform>> {
    parse_transform_attribute_functions(raw)?
        .into_iter()
        .map(transform_from_attribute_function)
        .collect()
}

pub fn consolidate_transform_matrices(
    matrices: impl IntoIterator<Item = SvgMatrixComponents>,
) -> Option<SvgMatrixComponents> {
    let mut matrices = matrices.into_iter();
    let first = matrices.next()?;
    Some(matrices.fold(first, SvgMatrixComponents::multiply))
}

pub fn serialize_transform_list(matrices: &[SvgMatrixComponents]) -> String {
    matrices
        .iter()
        .map(|matrix| matrix.serialize_transform_matrix())
        .collect::<Vec<_>>()
        .join(" ")
}

struct ParsedTransformFunction {
    name: String,
    arguments: String,
}

fn parse_transform_attribute_functions(raw: &str) -> Option<Vec<ParsedTransformFunction>> {
    let mut functions = Vec::new();
    let mut rest = raw.trim();
    if rest.is_empty() {
        return Some(functions);
    }
    while !rest.is_empty() {
        let open_index = rest.find('(')?;
        let name = rest[..open_index].trim().to_ascii_lowercase();
        if name.is_empty()
            || !name
                .chars()
                .all(|ch| ch.is_ascii_alphabetic() || ch.is_ascii_digit())
        {
            return None;
        }
        let function_len = open_index + moli_css_parse::balanced_function_len(&rest[open_index..])?;
        let close_index = function_len - 1;
        functions.push(ParsedTransformFunction {
            name,
            arguments: rest[open_index + 1..close_index].to_owned(),
        });
        rest = rest[function_len..].trim_start();
        if let Some(next) = rest.strip_prefix(',') {
            rest = next.trim_start();
            if rest.is_empty() {
                return None;
            }
        }
    }
    Some(functions)
}

fn transform_from_attribute_function(function: ParsedTransformFunction) -> Option<SvgTransform> {
    let arguments = transform_argument_tokens(&function.arguments)?;
    let matrix = match function.name.as_str() {
        "matrix" if arguments.len() == 6 => {
            let numbers = transform_number_arguments(&arguments)?;
            SvgMatrixComponents {
                a: numbers[0],
                b: numbers[1],
                c: numbers[2],
                d: numbers[3],
                e: numbers[4],
                f: numbers[5],
            }
        }
        "matrix3d" if arguments.len() == 16 => {
            affine_matrix3d_components(&transform_number_arguments(&arguments)?)?
        }
        "translate" if matches!(arguments.len(), 1 | 2) => SvgMatrixComponents::translate(
            transform_length_argument(&arguments[0])?,
            arguments
                .get(1)
                .map(|argument| transform_length_argument(argument))
                .unwrap_or(Some(0.0))?,
        ),
        "translatex" if arguments.len() == 1 => {
            SvgMatrixComponents::translate(transform_length_argument(&arguments[0])?, 0.0)
        }
        "translatey" if arguments.len() == 1 => {
            SvgMatrixComponents::translate(0.0, transform_length_argument(&arguments[0])?)
        }
        "translatez" if arguments.len() == 1 => {
            if transform_length_argument(&arguments[0])? != 0.0 {
                return None;
            }
            SvgMatrixComponents::identity()
        }
        "translate3d" if arguments.len() == 3 => {
            let z = transform_length_argument(&arguments[2])?;
            if z != 0.0 {
                return None;
            }
            SvgMatrixComponents::translate(
                transform_length_argument(&arguments[0])?,
                transform_length_argument(&arguments[1])?,
            )
        }
        "scale" if matches!(arguments.len(), 1 | 2) => {
            let x = transform_number_argument(&arguments[0])?;
            SvgMatrixComponents::scale(
                x,
                arguments
                    .get(1)
                    .map(|argument| transform_number_argument(argument))
                    .unwrap_or(Some(x))?,
            )
        }
        "scalex" if arguments.len() == 1 => {
            SvgMatrixComponents::scale(transform_number_argument(&arguments[0])?, 1.0)
        }
        "scaley" if arguments.len() == 1 => {
            SvgMatrixComponents::scale(1.0, transform_number_argument(&arguments[0])?)
        }
        "scalez" if arguments.len() == 1 => {
            if transform_number_argument(&arguments[0])? != 1.0 {
                return None;
            }
            SvgMatrixComponents::identity()
        }
        "scale3d" if arguments.len() == 3 => {
            let z = transform_number_argument(&arguments[2])?;
            if z != 1.0 {
                return None;
            }
            SvgMatrixComponents::scale(
                transform_number_argument(&arguments[0])?,
                transform_number_argument(&arguments[1])?,
            )
        }
        "rotate" if matches!(arguments.len(), 1 | 3) => SvgMatrixComponents::rotate_around(
            transform_angle_degrees_argument(&arguments[0])?,
            arguments
                .get(1)
                .map(|argument| transform_length_argument(argument))
                .unwrap_or(Some(0.0))?,
            arguments
                .get(2)
                .map(|argument| transform_length_argument(argument))
                .unwrap_or(Some(0.0))?,
        ),
        "rotatez" if arguments.len() == 1 => {
            SvgMatrixComponents::rotate(transform_angle_degrees_argument(&arguments[0])?)
        }
        "rotatex" | "rotatey" if arguments.len() == 1 => {
            if transform_angle_degrees_argument(&arguments[0])? != 0.0 {
                return None;
            }
            SvgMatrixComponents::identity()
        }
        "rotate3d" if arguments.len() == 4 => {
            let x = transform_number_argument(&arguments[0])?;
            let y = transform_number_argument(&arguments[1])?;
            let z = transform_number_argument(&arguments[2])?;
            if x != 0.0 || y != 0.0 || z == 0.0 {
                return None;
            }
            let angle = transform_angle_degrees_argument(&arguments[3])?;
            SvgMatrixComponents::rotate(if z < 0.0 { -angle } else { angle })
        }
        "skewx" if arguments.len() == 1 => {
            SvgMatrixComponents::skew_x(transform_angle_degrees_argument(&arguments[0])?)
        }
        "skewy" if arguments.len() == 1 => {
            SvgMatrixComponents::skew_y(transform_angle_degrees_argument(&arguments[0])?)
        }
        _ => return None,
    };
    let kind = match function.name.as_str() {
        "matrix" | "matrix3d" | "translatez" | "scalez" | "rotatex" | "rotatey" => {
            SvgTransformKind::Matrix
        }
        "translate" | "translatex" | "translatey" | "translate3d" => SvgTransformKind::Translate,
        "scale" | "scalex" | "scaley" | "scale3d" => SvgTransformKind::Scale,
        "rotate" | "rotatez" | "rotate3d" => SvgTransformKind::Rotate,
        "skewx" => SvgTransformKind::SkewX,
        "skewy" => SvgTransformKind::SkewY,
        _ => return None,
    };
    let angle = match function.name.as_str() {
        "rotate" | "rotatez" | "skewx" | "skewy" => {
            transform_angle_degrees_argument(&arguments[0])?
        }
        "rotate3d" => {
            let angle = transform_angle_degrees_argument(&arguments[3])?;
            if transform_number_argument(&arguments[2])? < 0.0 {
                -angle
            } else {
                angle
            }
        }
        _ => 0.0,
    };
    let rotation_center = if function.name == "rotate" && arguments.len() == 3 {
        (
            transform_length_argument(&arguments[1])?,
            transform_length_argument(&arguments[2])?,
        )
    } else {
        (0.0, 0.0)
    };
    Some(SvgTransform {
        kind,
        angle,
        matrix,
        rotation_center,
    })
}

fn transform_argument_tokens(raw: &str) -> Option<Vec<String>> {
    let mut values = Vec::new();
    let mut rest = raw.trim_start();
    while !rest.is_empty() {
        let token_len = transform_argument_token_len(rest)?;
        values.push(rest[..token_len].to_owned());

        let after_token = &rest[token_len..];
        let next = after_token.trim_start();
        let had_whitespace = next.len() != after_token.len();
        if next.is_empty() {
            return Some(values);
        }
        if let Some(after_comma) = next.strip_prefix(',') {
            rest = after_comma.trim_start();
            if rest.is_empty() || rest.starts_with(',') {
                return None;
            }
            continue;
        }
        if had_whitespace || next.starts_with(['+', '-']) {
            rest = next;
            continue;
        }
        return None;
    }
    Some(values)
}

fn transform_argument_token_len(raw: &str) -> Option<usize> {
    if moli_css_parse::starts_with_supported_math_function(raw) {
        return moli_css_parse::balanced_function_len(raw);
    }
    let number_len = number_len(raw)?;
    let mut token_len = number_len;
    while matches!(
        raw.as_bytes().get(token_len),
        Some(b'a'..=b'z' | b'A'..=b'Z' | b'%')
    ) {
        token_len += 1;
    }
    Some(token_len)
}

fn transform_number_arguments(arguments: &[String]) -> Option<Vec<f64>> {
    arguments
        .iter()
        .map(|argument| transform_number_argument(argument))
        .collect()
}

fn transform_number_argument(raw: &str) -> Option<f64> {
    moli_css_parse::parse_number(raw)
}

fn transform_length_argument(raw: &str) -> Option<f64> {
    moli_css_parse::parse_px_length(raw, UnitlessLength::Any)
}

fn transform_angle_degrees_argument(raw: &str) -> Option<f64> {
    moli_css_parse::parse_angle_degrees(raw, UnitlessAngle::Degrees)
}

fn affine_matrix3d_components(values: &[f64]) -> Option<SvgMatrixComponents> {
    if values.len() != 16
        || values[2] != 0.0
        || values[3] != 0.0
        || values[6] != 0.0
        || values[7] != 0.0
        || values[8] != 0.0
        || values[9] != 0.0
        || values[10] != 1.0
        || values[11] != 0.0
        || values[14] != 0.0
        || values[15] != 1.0
    {
        return None;
    }
    Some(SvgMatrixComponents {
        a: values[0],
        b: values[1],
        c: values[4],
        d: values[5],
        e: values[12],
        f: values[13],
    })
}

#[cfg(test)]
mod serialization_tests {
    use super::*;
    #[test]
    fn svg_transform_serialization_preserves_function_and_rotation_center() {
        for value in [
            SvgTransform::matrix(SvgMatrixComponents {
                a: 2.0,
                b: 3.0,
                c: 4.0,
                d: 5.0,
                e: 1024.0,
                f: -1024.0,
            }),
            SvgTransform::translate(0.1, 1024.0),
            SvgTransform::scale(2.5, 3.5),
            SvgTransform::rotate(0.0, 13.0, 17.0),
            SvgTransform::rotate(90.0, 13.0, 17.0),
            SvgTransform::skew_x(45.0),
            SvgTransform::skew_y(-45.0),
        ] {
            let parsed = parse_transform_attribute(&value.serialize()).unwrap();
            assert_eq!(parsed, vec![value]);
        }
    }
}
