use super::*;
use crate::{
    util::{callback_data_index_value, get_private_object, get_private_value},
    webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const DOM_QUAD_BRAND_SLOT: &str = "__moliDomQuadBrand";
const DOM_QUAD_P1_SLOT: &str = "__moliDomQuadP1";
const DOM_QUAD_P2_SLOT: &str = "__moliDomQuadP2";
const DOM_QUAD_P3_SLOT: &str = "__moliDomQuadP3";
const DOM_QUAD_P4_SLOT: &str = "__moliDomQuadP4";

#[derive(WebApiObject)]
#[webapi(interface = "DOMQuad", fallback_to_string_tag = "DOMQuad")]
struct DomQuadObjectDeclaration<'scope> {
    #[webapi(slot = DOM_QUAD_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = DOM_QUAD_P1_SLOT)]
    p1: v8::Local<'scope, v8::Object>,
    #[webapi(slot = DOM_QUAD_P2_SLOT)]
    p2: v8::Local<'scope, v8::Object>,
    #[webapi(slot = DOM_QUAD_P3_SLOT)]
    p3: v8::Local<'scope, v8::Object>,
    #[webapi(slot = DOM_QUAD_P4_SLOT)]
    p4: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct DomQuadJsonDeclaration<'scope> {
    p1: v8::Local<'scope, v8::Object>,
    p2: v8::Local<'scope, v8::Object>,
    p3: v8::Local<'scope, v8::Object>,
    p4: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMQuad")]
struct DomQuadPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = dom_quad_point_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    p1: (),
    #[webapi(
        accessor_property,
        getter = dom_quad_point_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    p2: (),
    #[webapi(
        accessor_property,
        getter = dom_quad_point_getter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    p3: (),
    #[webapi(
        accessor_property,
        getter = dom_quad_point_getter_callback,
        data = callback_data_index_value(scope, 3),
        enumerable
    )]
    p4: (),

    #[webapi(
        method = "getBounds",
        callback = dom_quad_get_bounds_callback,
        length = 0,
        enumerable
    )]
    get_bounds: (),

    #[webapi(
        method = "toJSON",
        callback = dom_quad_to_json_callback,
        length = 0,
        enumerable
    )]
    to_json: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMQuad", enumerable)]
struct DomQuadConstructorDeclaration {
    #[webapi(
        static_method = "fromRect",
        callback = dom_quad_from_rect_callback,
        length = 0
    )]
    from_rect: (),

    #[webapi(
        static_method = "fromQuad",
        callback = dom_quad_from_quad_callback,
        length = 0
    )]
    from_quad: (),
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "DOMQuadInit")]
struct DomQuadInit {
    #[webidl(with = optional_dom_point_member)]
    p1: Option<geometry_runtime::DomPointInit>,
    #[webidl(with = optional_dom_point_member)]
    p2: Option<geometry_runtime::DomPointInit>,
    #[webidl(with = optional_dom_point_member)]
    p3: Option<geometry_runtime::DomPointInit>,
    #[webidl(with = optional_dom_point_member)]
    p4: Option<geometry_runtime::DomPointInit>,
}

pub(super) fn dom_quad_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'DOMQuad': Please use the 'new' operator.",
        );
        return;
    }

    let mut points = [geometry_runtime::DomPointInit::default(); 4];
    for (index, point) in points.iter_mut().enumerate() {
        let Some(init) =
            geometry_runtime::optional_dom_point_init_arg(scope, &args, index as i32, "DOMQuad")
        else {
            return;
        };
        *point = init;
    }

    let [p1, p2, p3, p4] = points.map(|point| build_dom_point(scope, point));
    DomQuadObjectDeclaration::new(p1, p2, p3, p4)
        .initialize(scope, args.this())
        .expect("DOMQuad declaration should initialize object");
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn install_dom_quad_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name != "DOMQuad" {
        return;
    }
    DomQuadConstructorDeclaration::initialize_template(scope, template);
    DomQuadPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

fn dom_quad_from_rect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(rect) = dom_rect::dom_rect_init_arg(scope, &args, "DOMQuad.fromRect") else {
        return;
    };
    let p1 = point(rect.x, rect.y);
    let p2 = point(rect.x + rect.width, rect.y);
    let p3 = point(rect.x + rect.width, rect.y + rect.height);
    let p4 = point(rect.x, rect.y + rect.height);
    rv.set(build_dom_quad(scope, [p1, p2, p3, p4]).into());
}

fn dom_quad_from_quad_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(init) = dom_quad_init_arg(scope, &args) else {
        return;
    };
    rv.set(
        build_dom_quad(
            scope,
            [
                init.p1.unwrap_or_default(),
                init.p2.unwrap_or_default(),
                init.p3.unwrap_or_default(),
                init.p4.unwrap_or_default(),
            ],
        )
        .into(),
    );
}

fn dom_quad_point_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(scope, &args, DOM_QUAD_POINT_SLOTS, "DOMQuad point slots")
    else {
        rv.set_undefined();
        return;
    };
    if !dom_quad_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(point) = get_private_object(scope, args.this(), slot) else {
        rv.set_undefined();
        return;
    };
    rv.set(point.into());
}

fn dom_quad_get_bounds_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(points) = dom_quad_points(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let points = points.map(|point| geometry_runtime::dom_point_init_from_object(scope, point));
    let (x, width) = coordinate_bounds(points.map(|point| point.x));
    let (y, height) = coordinate_bounds(points.map(|point| point.y));
    rv.set(dom_rect::build_dom_rect_object(scope, x, y, width, height).into());
}

fn dom_quad_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some([p1, p2, p3, p4]) = dom_quad_points(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    let Ok(object) = DomQuadJsonDeclaration { p1, p2, p3, p4 }.bind(scope) else {
        return;
    };
    rv.set(object.into());
}

fn build_dom_quad<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    points: [geometry_runtime::DomPointInit; 4],
) -> v8::Local<'s, v8::Object> {
    let [p1, p2, p3, p4] = points.map(|point| build_dom_point(scope, point));
    DomQuadObjectDeclaration::new(p1, p2, p3, p4)
        .bind(scope)
        .expect("DOMQuad declaration should bind")
}

fn build_dom_point<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    point: geometry_runtime::DomPointInit,
) -> v8::Local<'s, v8::Object> {
    geometry_runtime::build_dom_point_object(scope, point.x, point.y, point.z, point.w)
}

fn point(x: f64, y: f64) -> geometry_runtime::DomPointInit {
    geometry_runtime::DomPointInit {
        x,
        y,
        ..Default::default()
    }
}

fn dom_quad_init_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<DomQuadInit> {
    if args.length() == 0 || args.get(0).is_undefined() {
        return Some(DomQuadInit::default());
    }
    match webidl::parse_dictionary::<DomQuadInit>(
        scope,
        args.get(0),
        webidl::Context::argument("DOMQuad.fromQuad", 1),
    ) {
        Ok(Some(init)) => Some(init),
        Ok(None) => Some(DomQuadInit::default()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn optional_dom_point_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Option<geometry_runtime::DomPointInit>, webidl::WebIdlError> {
    let context = webidl::Context::member("DOMQuadInit", name);
    let Some(value) = webidl::property_result(scope, object, name, context)? else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    webidl::parse_dictionary::<geometry_runtime::DomPointInit>(scope, value, context)
        .map(|init| Some(init.unwrap_or_default()))
}

fn dom_quad_points<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<[v8::Local<'s, v8::Object>; 4]> {
    if !dom_quad_receiver_branded(scope, receiver) {
        return None;
    }
    Some([
        get_private_object(scope, receiver, DOM_QUAD_P1_SLOT)?,
        get_private_object(scope, receiver, DOM_QUAD_P2_SLOT)?,
        get_private_object(scope, receiver, DOM_QUAD_P3_SLOT)?,
        get_private_object(scope, receiver, DOM_QUAD_P4_SLOT)?,
    ])
}

pub(super) fn dom_quad_clone_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<[[f64; 4]; 4]> {
    let points = dom_quad_points(scope, object)?;
    Some(points.map(|point| {
        let point = geometry_runtime::dom_point_init_from_object(scope, point);
        [point.x, point.y, point.z, point.w]
    }))
}

pub(super) fn build_dom_quad_clone_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    points: [[f64; 4]; 4],
) -> v8::Local<'s, v8::Object> {
    build_dom_quad(
        scope,
        points.map(|[x, y, z, w]| geometry_runtime::DomPointInit { x, y, z, w }),
    )
}

fn dom_quad_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, DOM_QUAD_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn coordinate_bounds(values: [f64; 4]) -> (f64, f64) {
    if values.iter().any(|value| value.is_nan()) {
        return (f64::NAN, f64::NAN);
    }
    let minimum = values.into_iter().fold(f64::INFINITY, f64::min);
    let maximum = values.into_iter().fold(f64::NEG_INFINITY, f64::max);
    (minimum, maximum - minimum)
}

const DOM_QUAD_POINT_SLOTS: &[&str] = &[
    DOM_QUAD_P1_SLOT,
    DOM_QUAD_P2_SLOT,
    DOM_QUAD_P3_SLOT,
    DOM_QUAD_P4_SLOT,
];
