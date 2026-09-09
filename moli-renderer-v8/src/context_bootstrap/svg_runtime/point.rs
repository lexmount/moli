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

fn is_point<'s>(scope: &mut v8::PinScope<'s, '_>, receiver: v8::Local<'s, v8::Object>) -> bool {
    get_private_value(scope, receiver, X).is_some()
}

pub(super) fn is_svg_root<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, receiver)
        .ok()
        .is_some_and(|(host, handle)| {
            unsafe { &*host }
                .dom_host()
                .node(handle)
                .and_then(crate::dom::native::Node::as_element)
                .is_some_and(|element| element.is_svg_element("svg"))
        })
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

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(build(scope, SvgGeometryPoint::new(0.0, 0.0)).into());
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
    set_private_value(
        scope,
        args.this(),
        slot,
        v8::Number::new(scope, f64::from(value)).into(),
    );
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
