//! SVGMatrix slots, value algebra, and JavaScript bindings.

use crate::{dom_exception::throw_dom_exception_value, interfaces as web_api_interfaces};
use moli_svg::SvgMatrixComponents;
use moli_v8_util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};
use moli_webidl as webidl;

const SVG_MATRIX_OWNER_TRANSFORM_SLOT: &str = "__moliSvgMatrixOwnerTransform";
const SVG_MATRIX_A_SLOT: &str = "__moliSvgMatrixA";
const SVG_MATRIX_B_SLOT: &str = "__moliSvgMatrixB";
const SVG_MATRIX_C_SLOT: &str = "__moliSvgMatrixC";
const SVG_MATRIX_D_SLOT: &str = "__moliSvgMatrixD";
const SVG_MATRIX_E_SLOT: &str = "__moliSvgMatrixE";
const SVG_MATRIX_F_SLOT: &str = "__moliSvgMatrixF";
const SVG_MATRIX_ACCESSOR_NAMES: &[&str] = &["a", "b", "c", "d", "e", "f"];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::SVGMatrix, own_to_string_tag = "SVGMatrix")]
struct SvgMatrixObjectDeclaration {
    #[webapi(slot = SVG_MATRIX_A_SLOT)]
    a: f64,
    #[webapi(slot = SVG_MATRIX_B_SLOT)]
    b: f64,
    #[webapi(slot = SVG_MATRIX_C_SLOT)]
    c: f64,
    #[webapi(slot = SVG_MATRIX_D_SLOT)]
    d: f64,
    #[webapi(slot = SVG_MATRIX_E_SLOT)]
    e: f64,
    #[webapi(slot = SVG_MATRIX_F_SLOT)]
    f: f64,
    #[webapi(method, callback = svg_matrix_multiply_callback, length = 1)]
    multiply: (),
    #[webapi(method, callback = svg_matrix_inverse_callback, length = 0)]
    inverse: (),
    #[webapi(method, callback = svg_matrix_translate_callback, length = 2)]
    translate: (),
    #[webapi(method, callback = svg_matrix_scale_callback, length = 1)]
    scale: (),
    #[webapi(
        method,
        callback = svg_matrix_scale_non_uniform_callback,
        length = 2
    )]
    scale_non_uniform: (),
    #[webapi(method, callback = svg_matrix_rotate_callback, length = 1)]
    rotate: (),
    #[webapi(
        method,
        callback = svg_matrix_rotate_from_vector_callback,
        length = 2
    )]
    rotate_from_vector: (),
    #[webapi(method, callback = svg_matrix_flip_x_callback, length = 0)]
    flip_x: (),
    #[webapi(method, callback = svg_matrix_flip_y_callback, length = 0)]
    flip_y: (),
    #[webapi(method, callback = svg_matrix_skew_x_callback, length = 1)]
    skew_x: (),
    #[webapi(method, callback = svg_matrix_skew_y_callback, length = 1)]
    skew_y: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG matrix")]
pub struct SvgMatrixArg<'s> {
    #[webidl(required)]
    pub matrix: v8::Local<'s, v8::Value>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG angle")]
struct SvgAngleArg {
    #[webidl(required, converter = "double")]
    angle: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG matrix translate")]
struct SvgMatrixTranslateArgs {
    #[webidl(required, converter = "double")]
    x: f64,
    #[webidl(required, converter = "double")]
    y: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG matrix scale")]
struct SvgMatrixScaleArg {
    #[webidl(required, converter = "double")]
    scale_factor: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG matrix scaleNonUniform")]
struct SvgMatrixScaleNonUniformArgs {
    #[webidl(required, converter = "double")]
    scale_factor_x: f64,
    #[webidl(required, converter = "double")]
    scale_factor_y: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG matrix rotateFromVector")]
struct SvgMatrixRotateFromVectorArgs {
    #[webidl(required, converter = "double")]
    x: f64,
    #[webidl(required, converter = "double")]
    y: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGMatrix, enumerable)]
struct SvgMatrixTemplateMethodsDeclaration {
    #[webapi(method = "multiply", length = 1, callback = svg_matrix_multiply_callback)]
    multiply: (),

    #[webapi(method = "inverse", length = 0, callback = svg_matrix_inverse_callback)]
    inverse: (),

    #[webapi(method = "translate", length = 2, callback = svg_matrix_translate_callback)]
    translate: (),

    #[webapi(method = "scale", length = 1, callback = svg_matrix_scale_callback)]
    scale: (),

    #[webapi(
        method = "scaleNonUniform",
        length = 2,
        callback = svg_matrix_scale_non_uniform_callback
    )]
    scale_non_uniform: (),

    #[webapi(method = "rotate", length = 1, callback = svg_matrix_rotate_callback)]
    rotate: (),

    #[webapi(
        method = "rotateFromVector",
        length = 2,
        callback = svg_matrix_rotate_from_vector_callback
    )]
    rotate_from_vector: (),

    #[webapi(method = "flipX", length = 0, callback = svg_matrix_flip_x_callback)]
    flip_x: (),

    #[webapi(method = "flipY", length = 0, callback = svg_matrix_flip_y_callback)]
    flip_y: (),

    #[webapi(method = "skewX", length = 1, callback = svg_matrix_skew_x_callback)]
    skew_x: (),

    #[webapi(method = "skewY", length = 1, callback = svg_matrix_skew_y_callback)]
    skew_y: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGMatrix, enumerable)]
struct SvgMatrixTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "a",
        getter = svg_matrix_getter,
        setter = svg_matrix_setter,
        data = callback_data_index_value(scope, 0)
    )]
    a: (),

    #[webapi(
        accessor_property = "b",
        getter = svg_matrix_getter,
        setter = svg_matrix_setter,
        data = callback_data_index_value(scope, 1)
    )]
    b: (),

    #[webapi(
        accessor_property = "c",
        getter = svg_matrix_getter,
        setter = svg_matrix_setter,
        data = callback_data_index_value(scope, 2)
    )]
    c: (),

    #[webapi(
        accessor_property = "d",
        getter = svg_matrix_getter,
        setter = svg_matrix_setter,
        data = callback_data_index_value(scope, 3)
    )]
    d: (),

    #[webapi(
        accessor_property = "e",
        getter = svg_matrix_getter,
        setter = svg_matrix_setter,
        data = callback_data_index_value(scope, 4)
    )]
    e: (),

    #[webapi(
        accessor_property = "f",
        getter = svg_matrix_getter,
        setter = svg_matrix_setter,
        data = callback_data_index_value(scope, 5)
    )]
    f: (),
}

pub fn install_svg_matrix_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgMatrixTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
    SvgMatrixTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

pub fn build_svg_matrix<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    components: SvgMatrixComponents,
) -> v8::Local<'s, v8::Object> {
    SvgMatrixObjectDeclaration::new(
        components.a,
        components.b,
        components.c,
        components.d,
        components.e,
        components.f,
    )
    .bind(scope)
    .expect("SVGMatrix declaration should bind")
}

fn svg_matrix_value_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Local::<v8::Object>::try_from(value).ok();
    if let Some(object) = object
        && web_api_interfaces::SVGMatrix::is_instance(scope, object)
    {
        return Some(object);
    }
    webidl::throw_type_error(scope, "Argument 1 can not be converted to SVGMatrix");
    None
}

pub fn cloned_svg_matrix_value_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let matrix = svg_matrix_value_or_throw(scope, value)?;
    let components = synchronized_svg_matrix_components(scope, matrix);
    Some(build_svg_matrix(scope, components))
}

type MatrixOwnerCallback = for<'s> fn(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Object>);

#[derive(Clone, Copy)]
pub struct SvgMatrixOwnerHooks {
    pub synchronize: MatrixOwnerCallback,
    pub can_write: for<'s> fn(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Object>) -> bool,
    pub commit_write:
        for<'s> fn(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Object>, SvgMatrixComponents),
}

pub fn bind_svg_matrix_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
    hooks: SvgMatrixOwnerHooks,
) {
    if scope.get_slot::<SvgMatrixOwnerHooks>().is_none() {
        scope.set_slot(hooks);
    }
    set_private_value(scope, matrix, SVG_MATRIX_OWNER_TRANSFORM_SLOT, owner.into());
}

fn svg_matrix_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, SvgMatrixOwnerHooks)> {
    let owner = get_private_value(scope, matrix, SVG_MATRIX_OWNER_TRANSFORM_SLOT)?;
    let owner = v8::Local::<v8::Object>::try_from(owner).ok()?;
    Some((owner, *scope.get_slot::<SvgMatrixOwnerHooks>()?))
}

pub fn set_svg_matrix_components<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
    components: SvgMatrixComponents,
) {
    for (slot, value) in [
        (SVG_MATRIX_A_SLOT, components.a),
        (SVG_MATRIX_B_SLOT, components.b),
        (SVG_MATRIX_C_SLOT, components.c),
        (SVG_MATRIX_D_SLOT, components.d),
        (SVG_MATRIX_E_SLOT, components.e),
        (SVG_MATRIX_F_SLOT, components.f),
    ] {
        set_private_value(scope, matrix, slot, v8::Number::new(scope, value).into());
    }
}

fn synchronized_svg_matrix_components<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    matrix: v8::Local<'s, v8::Object>,
) -> SvgMatrixComponents {
    let matrix =
        web_api_object_target(scope, matrix).expect("SVGMatrix receiver validated by WebIDL");
    if let Some((owner, hooks)) = svg_matrix_owner(scope, matrix) {
        (hooks.synchronize)(scope, owner);
    }
    svg_matrix_components(scope, matrix)
}

pub fn svg_matrix_components<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> SvgMatrixComponents {
    SvgMatrixComponents {
        a: svg_number_slot(scope, object, SVG_MATRIX_A_SLOT).unwrap_or(1.0),
        b: svg_number_slot(scope, object, SVG_MATRIX_B_SLOT).unwrap_or(0.0),
        c: svg_number_slot(scope, object, SVG_MATRIX_C_SLOT).unwrap_or(0.0),
        d: svg_number_slot(scope, object, SVG_MATRIX_D_SLOT).unwrap_or(1.0),
        e: svg_number_slot(scope, object, SVG_MATRIX_E_SLOT).unwrap_or(0.0),
        f: svg_number_slot(scope, object, SVG_MATRIX_F_SLOT).unwrap_or(0.0),
    }
}

fn svg_matrix_slot(name: &str) -> Option<&'static str> {
    match name {
        "a" => Some(SVG_MATRIX_A_SLOT),
        "b" => Some(SVG_MATRIX_B_SLOT),
        "c" => Some(SVG_MATRIX_C_SLOT),
        "d" => Some(SVG_MATRIX_D_SLOT),
        "e" => Some(SVG_MATRIX_E_SLOT),
        "f" => Some(SVG_MATRIX_F_SLOT),
        _ => None,
    }
}

fn svg_matrix_default(slot: &str) -> f64 {
    match slot {
        SVG_MATRIX_A_SLOT | SVG_MATRIX_D_SLOT => 1.0,
        _ => 0.0,
    }
}

fn svg_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<f64> {
    get_private_value(scope, object, slot).and_then(|value| value.number_value(scope))
}

fn svg_matrix_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_MATRIX_ACCESSOR_NAMES,
        "SVGMatrix attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    let matrix =
        web_api_object_target(scope, args.this()).expect("SVGMatrix receiver validated by WebIDL");
    if let Some((owner, hooks)) = svg_matrix_owner(scope, matrix) {
        (hooks.synchronize)(scope, owner);
    }
    let slot = svg_matrix_slot(name).expect("SVGMatrix callback data must name a component");
    let value = svg_number_slot(scope, matrix, slot).unwrap_or(svg_matrix_default(slot));
    rv.set(v8::Number::new(scope, value).into());
}

fn svg_matrix_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_MATRIX_ACCESSOR_NAMES,
        "SVGMatrix attributes",
    ) else {
        return;
    };
    let matrix =
        web_api_object_target(scope, args.this()).expect("SVGMatrix receiver validated by WebIDL");
    let slot = svg_matrix_slot(name).expect("SVGMatrix callback data must name a component");
    let value = match webidl::convert::<webidl::Double>(
        scope,
        args.get(0),
        webidl::Context::member("SVGMatrix", "component"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let owner = svg_matrix_owner(scope, matrix);
    if let Some((owner, hooks)) = owner {
        (hooks.synchronize)(scope, owner);
        if !(hooks.can_write)(scope, owner) {
            return;
        }
    }
    set_private_value(scope, matrix, slot, v8::Number::new(scope, value).into());
    if let Some((owner, hooks)) = owner {
        let components = svg_matrix_components(scope, matrix);
        (hooks.commit_write)(scope, owner, components);
    }
}

fn svg_matrix_multiply_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgMatrixArg>(scope, &args) else {
        return;
    };
    let current = synchronized_svg_matrix_components(scope, args.this());
    let Some(other) = svg_matrix_value_or_throw(scope, parsed.matrix) else {
        return;
    };
    let other = synchronized_svg_matrix_components(scope, other);
    rv.set(build_svg_matrix(scope, current.multiply(other)).into());
}

fn svg_matrix_inverse_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let current = synchronized_svg_matrix_components(scope, args.this());
    if !current.is_invertible() {
        throw_dom_exception_value(scope, "The matrix is not invertible.", "InvalidStateError");
        return;
    }
    let components = current.inverse();
    rv.set(build_svg_matrix(scope, components).into());
}

fn svg_matrix_translate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgMatrixTranslateArgs>(scope, &args) else {
        return;
    };
    let matrix =
        synchronized_svg_matrix_components(scope, args.this()).then_translate(parsed.x, parsed.y);
    rv.set(build_svg_matrix(scope, matrix).into());
}

fn svg_matrix_scale_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgMatrixScaleArg>(scope, &args) else {
        return;
    };
    let matrix =
        synchronized_svg_matrix_components(scope, args.this()).then_scale(parsed.scale_factor);
    rv.set(build_svg_matrix(scope, matrix).into());
}

fn svg_matrix_scale_non_uniform_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgMatrixScaleNonUniformArgs>(scope, &args) else {
        return;
    };
    let matrix = synchronized_svg_matrix_components(scope, args.this())
        .then_scale_non_uniform(parsed.scale_factor_x, parsed.scale_factor_y);
    rv.set(build_svg_matrix(scope, matrix).into());
}

fn svg_matrix_rotate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgAngleArg>(scope, &args) else {
        return;
    };
    let matrix = synchronized_svg_matrix_components(scope, args.this()).then_rotate(parsed.angle);
    rv.set(build_svg_matrix(scope, matrix).into());
}

fn svg_matrix_rotate_from_vector_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgMatrixRotateFromVectorArgs>(scope, &args) else {
        return;
    };
    let Some(matrix) = synchronized_svg_matrix_components(scope, args.this())
        .then_rotate_from_vector(parsed.x, parsed.y)
    else {
        throw_dom_exception_value(scope, "Arguments cannot be zero.", "InvalidAccessError");
        return;
    };
    rv.set(build_svg_matrix(scope, matrix).into());
}

fn svg_matrix_flip_x_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let matrix = synchronized_svg_matrix_components(scope, args.this()).then_flip_x();
    rv.set(build_svg_matrix(scope, matrix).into());
}

fn svg_matrix_flip_y_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let matrix = synchronized_svg_matrix_components(scope, args.this()).then_flip_y();
    rv.set(build_svg_matrix(scope, matrix).into());
}

fn svg_matrix_skew_x_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgAngleArg>(scope, &args) else {
        return;
    };
    let matrix = synchronized_svg_matrix_components(scope, args.this()).then_skew_x(parsed.angle);
    rv.set(build_svg_matrix(scope, matrix).into());
}

fn svg_matrix_skew_y_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgAngleArg>(scope, &args) else {
        return;
    };
    let matrix = synchronized_svg_matrix_components(scope, args.this()).then_skew_y(parsed.angle);
    rv.set(build_svg_matrix(scope, matrix).into());
}
