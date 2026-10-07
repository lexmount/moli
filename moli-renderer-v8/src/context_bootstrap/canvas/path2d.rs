//! Native Path2D construction and shared CanvasPath bindings.

use super::*;
use super::{path::Canvas2dPathState, state::canvas_path_state, transform::TransformInit};
use moli_webapi_declare::web_api_object_target;

macro_rules! canvas_path_declaration {
    ($name:ident, $interface:path) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = $interface, enumerable, receiver)]
        struct $name {
            #[webapi(method = "closePath", length = 0, callback = canvas_context_close_path_callback)]
            close_path: (),
            #[webapi(method = "moveTo", length = 2, callback = canvas_context_move_to_callback)]
            move_to: (),
            #[webapi(method = "lineTo", length = 2, callback = canvas_context_line_to_callback)]
            line_to: (),
            #[webapi(method = "quadraticCurveTo", length = 4, callback = canvas_context_quadratic_curve_to_callback)]
            quadratic_curve_to: (),
            #[webapi(method = "bezierCurveTo", length = 6, callback = canvas_context_bezier_curve_to_callback)]
            bezier_curve_to: (),
            #[webapi(method = "arcTo", length = 5, callback = canvas_context_arc_to_callback)]
            arc_to: (),
            #[webapi(method = "arc", length = 5, callback = canvas_context_arc_callback)]
            arc: (),
            #[webapi(method = "ellipse", length = 7, callback = canvas_context_ellipse_callback)]
            ellipse: (),
            #[webapi(method = "rect", length = 4, callback = canvas_context_rect_callback)]
            rect: (),
        }
    };
}

canvas_path_declaration!(Path2dCanvasPathDeclaration, web_api_interfaces::Path2D);
canvas_path_declaration!(
    CanvasPathDeclaration,
    web_api_interfaces::CanvasRenderingContext2D
);
canvas_path_declaration!(
    OffscreenCanvasPathDeclaration,
    web_api_interfaces::OffscreenCanvasRenderingContext2D
);

macro_rules! canvas_draw_path_declaration {
    ($name:ident, $interface:path) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = $interface, enumerable, receiver)]
        struct $name {
            #[webapi(method = "fill", length = 0, callback = canvas_context_fill_callback)]
            fill: (),
            #[webapi(method = "stroke", length = 0, callback = canvas_context_stroke_callback)]
            stroke: (),
        }
    };
}

canvas_draw_path_declaration!(
    CanvasDrawPathDeclaration,
    web_api_interfaces::CanvasRenderingContext2D
);
canvas_draw_path_declaration!(
    OffscreenCanvasDrawPathDeclaration,
    web_api_interfaces::OffscreenCanvasRenderingContext2D
);

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Path2D, enumerable, receiver)]
struct Path2dPrototypeDeclaration {
    #[webapi(method = "addPath", length = 1, callback = add_path_callback)]
    add_path: (),
}

pub(crate) fn install_canvas_path_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: &str,
) {
    match interface {
        "Path2D" => {
            Path2dCanvasPathDeclaration::initialize_prototype_template(scope, prototype);
            Path2dPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CanvasRenderingContext2D" => {
            CanvasPathDeclaration::initialize_prototype_template(scope, prototype);
            CanvasDrawPathDeclaration::initialize_prototype_template(scope, prototype);
        }
        "OffscreenCanvasRenderingContext2D" => {
            OffscreenCanvasPathDeclaration::initialize_prototype_template(scope, prototype);
            OffscreenCanvasDrawPathDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => unreachable!("only CanvasPath interfaces have path bindings"),
    }
}

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
#[webidl(name = "CanvasFillRule")]
pub(super) enum CanvasFillRule {
    #[default]
    #[webidl(token = "nonzero")]
    Nonzero,
    #[webidl(token = "evenodd")]
    Evenodd,
}

impl CanvasFillRule {
    pub(super) fn paint_rule(self) -> moli_layout::PaintFillRule {
        match self {
            Self::Nonzero => moli_layout::PaintFillRule::NonZero,
            Self::Evenodd => moli_layout::PaintFillRule::EvenOdd,
        }
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasDrawPath.fill")]
struct FillRuleArgs {
    #[webidl(default = CanvasFillRule::Nonzero, converter = "enum")]
    rule: CanvasFillRule,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasDrawPath.fill")]
struct FillPathArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Path2D)]
    path: v8::Local<'s, v8::Object>,
    #[webidl(default = CanvasFillRule::Nonzero, converter = "enum")]
    rule: CanvasFillRule,
}

pub(super) fn fill_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(Option<v8::Local<'s, v8::Object>>, CanvasFillRule)> {
    if args.length() > 1 {
        let parsed = webidl::parse_args::<FillPathArgs>(scope, args)?;
        let target = web_api_object_target(scope, parsed.path).expect("validated Path2D argument");
        return Some((Some(target), parsed.rule));
    }
    if let Ok(object) = path_object(
        scope,
        args.get(0),
        webidl::Context::argument("CanvasDrawPath.fill", 1),
    ) {
        return Some((Some(object), CanvasFillRule::Nonzero));
    }
    let parsed = webidl::parse_args::<FillRuleArgs>(scope, args)?;
    Some((None, parsed.rule))
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasDrawPath.stroke")]
pub(super) struct StrokeArgs<'s> {
    #[webidl(interface = web_api_interfaces::Path2D)]
    pub(super) path: Option<v8::Local<'s, v8::Object>>,
}

/// Union selection uses the same native interface converter as derive fields;
/// it never consults author-visible constructors or prototype chains.
pub(super) fn path_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    webidl::convert_with_options::<webidl::InterfaceObject>(
        scope,
        value,
        context,
        &webidl::InterfaceOptions {
            name: web_api_interfaces::Path2D::NAME,
            brand_check: web_api_interfaces::Path2D::is_instance,
        },
    )
    .map(|object| {
        web_api_object_target(scope, object.0).expect("branded Path2D has native identity")
    })
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Path2D")]
struct ConstructorArgs<'s> {
    path: Option<v8::Local<'s, v8::Value>>,
}

pub(crate) fn path2d_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "Path2D requires the new operator");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    let context = webidl::Context::argument("Path2D", 1);
    let state = match parsed.path {
        None => Canvas2dPathState::default(),
        Some(value) => match path_object(scope, value, context) {
            Ok(object) => canvas_path_state(scope, object).borrow().clone(),
            Err(_) => match webidl::convert::<webidl::DomString>(scope, value, context) {
                Ok(text) => Canvas2dPathState::from_svg(&text.0),
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return;
                }
            },
        },
    };
    *canvas_path_state(scope, args.this()).borrow_mut() = state;
    rv.set(args.this().into());
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Path2D.addPath")]
struct AddPathArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Path2D)]
    path: v8::Local<'s, v8::Object>,
    #[webidl(dictionary)]
    transform: TransformInit,
}

fn add_path_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<AddPathArgs>(scope, &args) else {
        return;
    };
    let source = web_api_object_target(scope, parsed.path).expect("validated Path2D argument");
    let source = canvas_path_state(scope, source);
    if source.borrow().is_empty() {
        return;
    }
    let Some(matrix) = parsed.transform.validate(scope) else {
        return;
    };
    if !matrix.into_iter().all(f64::is_finite) {
        return;
    }
    let snapshot = source.borrow().clone();
    let target = web_api_object_target(scope, args.this()).expect("validated Path2D receiver");
    canvas_path_state(scope, target)
        .borrow_mut()
        .add_path(&snapshot, matrix);
}
