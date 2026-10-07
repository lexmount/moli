//! WebIDL conversion and DOMMatrix2DInit alias validation for Canvas transforms.

use crate::{util::throw_type_error, web_api_interfaces, webidl};
use moli_geometry::DomMatrixComponents;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CanvasRenderingContext2D, enumerable, receiver)]
struct CanvasTransformPrototypeDeclaration {
    #[webapi(method = "getTransform", length = 0, callback = get_transform_callback)]
    get_transform: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::OffscreenCanvasRenderingContext2D, enumerable, receiver)]
struct OffscreenCanvasTransformPrototypeDeclaration {
    #[webapi(method = "getTransform", length = 0, callback = get_transform_callback)]
    get_transform: (),
}

pub(crate) fn install_canvas_transform_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: &str,
) {
    match interface {
        "CanvasRenderingContext2D" => {
            CanvasTransformPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "OffscreenCanvasRenderingContext2D" => {
            OffscreenCanvasTransformPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => unreachable!("only 2D canvas contexts have transform bindings"),
    }
}

fn get_transform_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("validated canvas receiver has native identity");
    let [a, b, c, d, e, f] = super::state::canvas_path_state(scope, context)
        .borrow()
        .transform()
        .coefficients;
    let matrix = super::super::geometry_runtime::build_dom_matrix_object(
        scope,
        DomMatrixComponents {
            m11: a,
            m12: b,
            m21: c,
            m22: d,
            m41: e,
            m42: f,
            ..DomMatrixComponents::identity()
        },
    );
    rv.set(matrix.into());
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "DOMMatrix2DInit")]
struct TransformInit {
    a: Option<f64>,
    b: Option<f64>,
    c: Option<f64>,
    d: Option<f64>,
    e: Option<f64>,
    f: Option<f64>,
    m11: Option<f64>,
    m12: Option<f64>,
    m21: Option<f64>,
    m22: Option<f64>,
    m41: Option<f64>,
    m42: Option<f64>,
}

pub(super) fn transform_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    prefix: &'static str,
) -> Option<[f64; 6]> {
    if args.length() < 6 {
        throw_type_error(scope, &format!("{prefix} requires six arguments"));
        return None;
    }
    let mut values = [0.0; 6];
    for (index, value) in values.iter_mut().enumerate() {
        match webidl::argument::<webidl::UnrestrictedDouble>(
            scope,
            args,
            index as i32,
            webidl::Context::argument(prefix, index + 1),
        ) {
            Ok(parsed) => *value = parsed.0,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        }
    }
    Some(values)
}

pub(super) fn set_transform_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<[f64; 6]> {
    const PREFIX: &str = "CanvasRenderingContext2D.setTransform";
    if args.length() > 1 {
        return transform_arguments(scope, args, PREFIX);
    }
    let init = match webidl::parse_dictionary::<TransformInit>(
        scope,
        args.get(0),
        webidl::Context::argument(PREFIX, 1),
    ) {
        Ok(init) => init.unwrap_or_default(),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let pairs = [
        (init.a, init.m11, 1.0),
        (init.b, init.m12, 0.0),
        (init.c, init.m21, 0.0),
        (init.d, init.m22, 1.0),
        (init.e, init.m41, 0.0),
        (init.f, init.m42, 0.0),
    ];
    let mut values = [0.0; 6];
    for (value, (alias, component, default)) in values.iter_mut().zip(pairs) {
        if let (Some(a), Some(b)) = (alias, component)
            && a != b
            && !(a.is_nan() && b.is_nan())
        {
            throw_type_error(scope, "DOMMatrix2DInit contains inconsistent aliases");
            return None;
        }
        *value = alias.or(component).unwrap_or(default);
    }
    Some(values)
}
