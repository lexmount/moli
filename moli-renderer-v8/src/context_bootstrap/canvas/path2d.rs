use super::context2d::{
    canvas_path_arc_callback, canvas_path_arc_to_callback, canvas_path_bezier_curve_to_callback,
    canvas_path_close_path_callback, canvas_path_ellipse_callback, canvas_path_line_to_callback,
    canvas_path_move_to_callback, canvas_path_quadratic_curve_to_callback,
    canvas_path_rect_callback,
};
use super::path::Canvas2dPathState;
use super::state::canvas_path_state;
use crate::{util::throw_type_error, web_api_interfaces, webidl};
use moli_webapi::canvas_context_values::transform::transform_init_coefficients;
use moli_webapi::geometry::DomMatrix2DInit;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Path2D, enumerable)]
struct Path2dMethods {
    #[webapi(method = "addPath", length = 1, callback = add_path_callback)]
    add_path: (),
    #[webapi(method = "closePath", length = 0, callback = canvas_path_close_path_callback)]
    close_path: (),
    #[webapi(method = "moveTo", length = 2, callback = canvas_path_move_to_callback)]
    move_to: (),
    #[webapi(method = "lineTo", length = 2, callback = canvas_path_line_to_callback)]
    line_to: (),
    #[webapi(method = "quadraticCurveTo", length = 4, callback = canvas_path_quadratic_curve_to_callback)]
    quadratic_curve_to: (),
    #[webapi(method = "bezierCurveTo", length = 6, callback = canvas_path_bezier_curve_to_callback)]
    bezier_curve_to: (),
    #[webapi(method = "arcTo", length = 5, callback = canvas_path_arc_to_callback)]
    arc_to: (),
    #[webapi(method = "rect", length = 4, callback = canvas_path_rect_callback)]
    rect: (),
    #[webapi(method = "arc", length = 5, callback = canvas_path_arc_callback)]
    arc: (),
    #[webapi(method = "ellipse", length = 7, callback = canvas_path_ellipse_callback)]
    ellipse: (),
}

pub(super) fn install_path_2d_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    Path2dMethods::initialize_prototype_template(scope, prototype);
}

pub(super) fn path_2d_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    web_api_interfaces::Path2D::is_instance(scope, object).then_some(object)
}

pub(crate) fn path_2d_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'Path2D': Please use the 'new' operator.",
        );
        return;
    }
    let input = args.get(0);
    if input.is_undefined() {
        return;
    }
    let path = if let Some(source) = path_2d_object(scope, input) {
        canvas_path_state(scope, source).borrow().clone()
    } else {
        match webidl::convert::<webidl::DomString>(
            scope,
            input,
            webidl::Context::argument("Path2D", 1),
        ) {
            Ok(data) => Canvas2dPathState::from_svg(&data.0),
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
        }
    };
    *canvas_path_state(scope, args.this()).borrow_mut() = path;
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Path2D.addPath")]
struct AddPathArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Path2D)]
    path: v8::Local<'s, v8::Object>,
    #[webidl(dictionary, default = DomMatrix2DInit::default())]
    transform: DomMatrix2DInit,
}

fn add_path_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<AddPathArgs>(scope, &args) else {
        return;
    };
    let source = canvas_path_state(scope, parsed.path);
    if source.borrow().is_empty() {
        return;
    }
    let Some(transform) = transform_init_coefficients(scope, &parsed.transform) else {
        return;
    };
    // Copy before borrowing the destination, since addPath can append itself.
    let source = source.borrow().clone();
    canvas_path_state(scope, args.this())
        .borrow_mut()
        .add_path(&source, transform);
}
