use moli_css_parse::{CssTransformFunction, UnitlessAngle, UnitlessLength};

use crate::matrix::{DOM_MATRIX_COMPONENT_COUNT, DomMatrixComponents};

pub fn dom_matrix_components_from_values(values: &[f64]) -> Option<DomMatrixComponents> {
    match values.len() {
        6 => Some(DomMatrixComponents {
            m11: values[0],
            m12: values[1],
            m21: values[2],
            m22: values[3],
            m41: values[4],
            m42: values[5],
            ..DomMatrixComponents::identity()
        }),
        DOM_MATRIX_COMPONENT_COUNT => Some(DomMatrixComponents {
            m11: values[0],
            m12: values[1],
            m13: values[2],
            m14: values[3],
            m21: values[4],
            m22: values[5],
            m23: values[6],
            m24: values[7],
            m31: values[8],
            m32: values[9],
            m33: values[10],
            m34: values[11],
            m41: values[12],
            m42: values[13],
            m43: values[14],
            m44: values[15],
        }),
        _ => None,
    }
}

pub fn parse_dom_matrix_value(text: &str) -> Option<DomMatrixComponents> {
    parse_dom_matrix_value_with_dimension(text).map(|(components, _)| components)
}

pub fn parse_dom_matrix_value_with_dimension(text: &str) -> Option<(DomMatrixComponents, bool)> {
    if text.is_empty() {
        return Some((DomMatrixComponents::identity(), true));
    }
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("none") || css_comments_wrap_none_keyword(trimmed) {
        return Some((DomMatrixComponents::identity(), true));
    }
    if trimmed.is_empty() {
        return None;
    }

    let functions = moli_css_parse::parse_transform_function_list(trimmed)?;
    if functions.is_empty() {
        return None;
    }
    let is_2d = functions.iter().all(is_2d_transform_function);
    let mut product = DomMatrixComponents::identity();
    for function in functions {
        product = product.multiply(components_from_transform_function(&function)?);
    }
    Some((product, is_2d))
}

fn is_2d_transform_function(function: &CssTransformFunction) -> bool {
    matches!(
        function.name.as_str(),
        "matrix"
            | "translate"
            | "translatex"
            | "translatey"
            | "scale"
            | "scalex"
            | "scaley"
            | "rotate"
            | "rotatez"
            | "skewx"
            | "skewy"
    )
}

fn components_from_transform_function(
    function: &CssTransformFunction,
) -> Option<DomMatrixComponents> {
    let arguments = &function.arguments;
    match function.name.as_str() {
        "matrix" if arguments.len() == 6 => {
            let values = css_number_arguments(arguments)?;
            Some(DomMatrixComponents {
                m11: values[0],
                m12: values[1],
                m21: values[2],
                m22: values[3],
                m41: values[4],
                m42: values[5],
                ..DomMatrixComponents::identity()
            })
        }
        "matrix3d" if arguments.len() == DOM_MATRIX_COMPONENT_COUNT => {
            let values = css_number_arguments(arguments)?;
            dom_matrix_components_from_values(&values)
        }
        "translate" if matches!(arguments.len(), 1 | 2) => {
            let tx = css_px_length_argument(&arguments[0])?;
            let ty = arguments
                .get(1)
                .map(|argument| css_px_length_argument(argument))
                .unwrap_or(Some(0.0))?;
            Some(DomMatrixComponents::translation(tx, ty, 0.0))
        }
        "translatex" if arguments.len() == 1 => Some(DomMatrixComponents::translation(
            css_px_length_argument(&arguments[0])?,
            0.0,
            0.0,
        )),
        "translatey" if arguments.len() == 1 => Some(DomMatrixComponents::translation(
            0.0,
            css_px_length_argument(&arguments[0])?,
            0.0,
        )),
        "translatez" if arguments.len() == 1 => Some(DomMatrixComponents::translation(
            0.0,
            0.0,
            css_px_length_argument(&arguments[0])?,
        )),
        "translate3d" if arguments.len() == 3 => Some(DomMatrixComponents::translation(
            css_px_length_argument(&arguments[0])?,
            css_px_length_argument(&arguments[1])?,
            css_px_length_argument(&arguments[2])?,
        )),
        "scale" if matches!(arguments.len(), 1 | 2) => {
            let sx = css_number_argument(&arguments[0])?;
            let sy = arguments
                .get(1)
                .map(|argument| css_number_argument(argument))
                .unwrap_or(Some(sx))?;
            Some(DomMatrixComponents::scale(sx, sy, 1.0))
        }
        "scalex" if arguments.len() == 1 => Some(DomMatrixComponents::scale(
            css_number_argument(&arguments[0])?,
            1.0,
            1.0,
        )),
        "scaley" if arguments.len() == 1 => Some(DomMatrixComponents::scale(
            1.0,
            css_number_argument(&arguments[0])?,
            1.0,
        )),
        "scalez" if arguments.len() == 1 => Some(DomMatrixComponents::scale(
            1.0,
            1.0,
            css_number_argument(&arguments[0])?,
        )),
        "scale3d" if arguments.len() == 3 => Some(DomMatrixComponents::scale(
            css_number_argument(&arguments[0])?,
            css_number_argument(&arguments[1])?,
            css_number_argument(&arguments[2])?,
        )),
        "rotate" | "rotatez" if arguments.len() == 1 => Some(
            DomMatrixComponents::identity().rotated_z(css_angle_degrees_argument(&arguments[0])?),
        ),
        "rotatex" if arguments.len() == 1 => Some(DomMatrixComponents::rotation_axis_angle(
            1.0,
            0.0,
            0.0,
            css_angle_degrees_argument(&arguments[0])?,
        )),
        "rotatey" if arguments.len() == 1 => Some(DomMatrixComponents::rotation_axis_angle(
            0.0,
            1.0,
            0.0,
            css_angle_degrees_argument(&arguments[0])?,
        )),
        "rotate3d" if arguments.len() == 4 => Some(DomMatrixComponents::rotation_axis_angle(
            css_number_argument(&arguments[0])?,
            css_number_argument(&arguments[1])?,
            css_number_argument(&arguments[2])?,
            css_angle_degrees_argument(&arguments[3])?,
        )),
        "perspective" if arguments.len() == 1 => {
            let distance = css_px_length_argument(&arguments[0])?;
            (distance > 0.0).then_some(DomMatrixComponents::perspective(distance))
        }
        "skewx" if arguments.len() == 1 => Some(DomMatrixComponents::skew_x(
            css_angle_degrees_argument(&arguments[0])?
                .to_radians()
                .tan(),
        )),
        "skewy" if arguments.len() == 1 => Some(DomMatrixComponents::skew_y(
            css_angle_degrees_argument(&arguments[0])?
                .to_radians()
                .tan(),
        )),
        _ => None,
    }
}

fn css_number_arguments(arguments: &[String]) -> Option<Vec<f64>> {
    arguments
        .iter()
        .map(|argument| css_number_argument(argument))
        .collect()
}

fn css_number_argument(raw: &str) -> Option<f64> {
    moli_css_parse::parse_number(raw)
}

fn css_px_length_argument(raw: &str) -> Option<f64> {
    moli_css_parse::parse_px_length(raw, UnitlessLength::ZeroOnly)
}

fn css_angle_degrees_argument(raw: &str) -> Option<f64> {
    moli_css_parse::parse_angle_degrees(raw, UnitlessAngle::ZeroOnly)
}

fn css_comments_wrap_none_keyword(raw: &str) -> bool {
    let without_empty_comments = raw.replace("/**/", "");
    without_empty_comments.trim().eq_ignore_ascii_case("none")
}
