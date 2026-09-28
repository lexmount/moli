use super::*;

fn flatten(value: DomMatrixComponents) -> DomMatrixComponents {
    DomMatrixComponents {
        m11: value.m11,
        m12: value.m12,
        m21: value.m21,
        m22: value.m22,
        m41: value.m41,
        m42: value.m42,
        ..DomMatrixComponents::identity()
    }
}

pub(super) fn matrix_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<DomMatrixComponents> {
    let (_, _, values) = geometry_runtime::dom_matrix_clone_data(scope, object)?;
    moli_geometry::dom_matrix_components_from_values(&values)
}

pub(super) fn component<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(DomMatrixComponents, bool)> {
    let kind = kind(scope, object)?;
    let is_2d = is_2d(scope, object);
    let items = operands(scope, object)?;
    let number = |scope: &mut v8::PinScope<'s, '_>, index, unit| {
        let value = item(scope, items, index)?;
        math::to_unit_number(scope, value, unit)
    };
    let matrix = match kind {
        Kind::Translate => DomMatrixComponents::translation(
            number(scope, 0, "px")?,
            number(scope, 1, "px")?,
            if is_2d { 0.0 } else { number(scope, 2, "px")? },
        ),
        Kind::Scale => DomMatrixComponents::scale(
            number(scope, 0, "number")?,
            number(scope, 1, "number")?,
            if is_2d {
                1.0
            } else {
                number(scope, 2, "number")?
            },
        ),
        Kind::Rotate => {
            let angle = number(scope, 3, "deg")?;
            if is_2d {
                DomMatrixComponents::identity().rotated_z(angle)
            } else {
                DomMatrixComponents::rotation_axis_angle(
                    number(scope, 0, "number")?,
                    number(scope, 1, "number")?,
                    number(scope, 2, "number")?,
                    angle,
                )
            }
        }
        Kind::Skew => DomMatrixComponents {
            m21: number(scope, 0, "rad")?.tan(),
            m12: number(scope, 1, "rad")?.tan(),
            ..DomMatrixComponents::identity()
        },
        Kind::SkewX => DomMatrixComponents::skew_x(number(scope, 0, "rad")?.tan()),
        Kind::SkewY => DomMatrixComponents::skew_y(number(scope, 0, "rad")?.tan()),
        Kind::Perspective => {
            let value = item(scope, items, 0)?;
            if web_api_interfaces::CSSKeywordValue::is_instance(scope, value) {
                if !values::serialize(scope, value)?.eq_ignore_ascii_case("none") {
                    throw_type_error(scope, "Invalid perspective keyword");
                    return None;
                }
                DomMatrixComponents::identity()
            } else {
                let length = number(scope, 0, "px")?;
                // CSS Transforms 2 clamps depths below one pixel for matrix
                // conversion. Preserve NaN instead of f64::max's suppression.
                DomMatrixComponents::perspective(if length < 1.0 { 1.0 } else { length })
            }
        }
        Kind::Matrix => {
            let object = item(scope, items, 0)?;
            let data = matrix_data(scope, object)?;
            if is_2d { flatten(data) } else { data }
        }
    };
    Some((matrix, is_2d))
}

fn bind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    matrix: DomMatrixComponents,
    is_2d: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let realm = owner.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, realm);
    Some(geometry_runtime::build_dom_matrix_clone_object(
        scope,
        true,
        is_2d,
        [
            matrix.m11, matrix.m12, matrix.m13, matrix.m14, matrix.m21, matrix.m22, matrix.m23,
            matrix.m24, matrix.m31, matrix.m32, matrix.m33, matrix.m34, matrix.m41, matrix.m42,
            matrix.m43, matrix.m44,
        ],
    ))
}

pub(super) fn component_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some((matrix, is_2d)) = component(scope, args.this())
        && let Some(result) = bind(scope, args.this(), matrix, is_2d)
    {
        rv.set(result.into());
    }
}

pub(super) fn value_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(items) = components(scope, args.this()) else {
        return;
    };
    let mut matrix = DomMatrixComponents::identity();
    let mut is_2d = true;
    for i in 0..items.length() {
        let Some((next, dimension)) =
            item(scope, items, i).and_then(|value| component(scope, value))
        else {
            return;
        };
        matrix = matrix.multiply(next);
        is_2d &= dimension;
    }
    if let Some(result) = bind(scope, args.this(), matrix, is_2d) {
        rv.set(result.into());
    }
}
