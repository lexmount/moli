//! Detached, float-valued SVGPoint results (not DOMPoint/double lookalikes).

use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use super::{
    builders::{svg_matrix_components, svg_matrix_value_or_throw},
    *,
};
use crate::util::{callback_data_index_value, callback_data_item};
use crate::web_api_interfaces;

const X: &str = "__moliSvgPointX";
const Y: &str = "__moliSvgPointY";
const FIELDS: &[(&str, &str)] = &[("x", X), ("y", Y)];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::SVGPoint)]
struct SvgPointObject {
    #[webapi(slot = X)]
    x: f64,
    #[webapi(slot = Y)]
    y: f64,
}

pub(super) fn is_point<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, X).is_some()
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGPoint, receiver = is_point, enumerable)]
struct SvgPointBindings {
    #[webapi(accessor_property, getter = get_field, setter = set_field,
        data = callback_data_index_value(scope, 0))]
    x: (),
    #[webapi(accessor_property, getter = get_field, setter = set_field,
        data = callback_data_index_value(scope, 1))]
    y: (),
    #[webapi(method = "matrixTransform", callback = matrix_transform, length = 1)]
    matrix_transform: (),
}

pub(super) fn install_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    SvgPointBindings::initialize_prototype_template(scope, prototype);
}

pub(super) fn build<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    point: SvgGeometryPoint,
) -> v8::Local<'s, v8::Object> {
    SvgPointObject::new(f64::from(point.x as f32), f64::from(point.y as f32))
        .bind(scope)
        .expect("SVGPoint declaration should bind")
}

const READ_ONLY: &str = "__moliSvgPointReadOnly";

pub(super) fn set_read_only<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    point: v8::Local<'s, v8::Object>,
    read_only: bool,
) {
    set_private_value(
        scope,
        point,
        READ_ONLY,
        v8::Boolean::new(scope, read_only).into(),
    );
}

pub(super) fn components<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    point: v8::Local<'s, v8::Object>,
) -> Option<SvgGeometryPoint> {
    let x = get_private_value(scope, point, X)?.number_value(scope)?;
    let y = get_private_value(scope, point, Y)?.number_value(scope)?;
    Some(SvgGeometryPoint::new(x, y))
}

fn get_field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((_, slot)) = callback_data_item(scope, &args, FIELDS, "SVGPoint fields") else {
        return;
    };
    if let Some(value) = get_private_value(scope, args.this(), slot) {
        rv.set(value);
    }
}

fn set_field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((name, slot)) = callback_data_item(scope, &args, FIELDS, "SVGPoint fields") else {
        return;
    };
    let value = match webidl::convert::<webidl::Double>(
        scope,
        args.get(0),
        webidl::Context::member("SVGPoint", name),
    ) {
        Ok(value) => value.0 as f32,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if !value.is_finite() {
        webidl::throw_type_error(scope, "SVGPoint value is outside the finite float range.");
        return;
    }
    if get_private_value(scope, args.this(), READ_ONLY).is_some_and(|value| value.is_true()) {
        throw_dom_exception(
            scope,
            "NoModificationAllowedError",
            7,
            "The SVG point is read-only.",
        );
        return;
    }
    set_private_value(
        scope,
        args.this(),
        slot,
        v8::Number::new(scope, f64::from(value)).into(),
    );
    super::reflect_svg_point_mutation(scope, args.this());
}

fn matrix_transform<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgMatrixArg>(scope, &args) else {
        return;
    };
    let Some(matrix) = svg_matrix_value_or_throw(scope, parsed.matrix) else {
        return;
    };
    let matrix = svg_matrix_components(scope, matrix);
    let coordinate = |scope: &mut v8::PinScope<'s, '_>, slot| {
        get_private_value(scope, args.this(), slot)
            .and_then(|value| value.number_value(scope))
            .unwrap_or(0.0)
    };
    let (x, y) = (coordinate(scope, X), coordinate(scope, Y));
    rv.set(
        build(
            scope,
            SvgGeometryPoint::new(
                matrix.a * x + matrix.c * y + matrix.e,
                matrix.b * x + matrix.d * y + matrix.f,
            ),
        )
        .into(),
    );
}
