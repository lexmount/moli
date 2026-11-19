//! Transform access to the canvas drawing state.

use crate::web_api_interfaces;
use moli_geometry::DomMatrixComponents;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CanvasRenderingContext2D, enumerable)]
struct CanvasTransformPrototypeDeclaration {
    #[webapi(method = "getTransform", length = 0, callback = get_transform_callback)]
    get_transform: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::OffscreenCanvasRenderingContext2D, enumerable)]
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

pub(super) use moli_webapi::canvas_context_values::transform::{
    set_transform_arguments, transform_arguments,
};
