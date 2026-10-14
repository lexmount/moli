//! Native Canvas fill and stroke hit testing after WebIDL conversion.

use super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasDrawPath.isPointInPath")]
struct PointArgs {
    #[webidl(required, converter = "unrestricted_double")]
    x: f64,
    #[webidl(required, converter = "unrestricted_double")]
    y: f64,
    #[webidl(default = CanvasFillRule::Nonzero, converter = "enum")]
    rule: CanvasFillRule,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasDrawPath.isPointInPath")]
struct PathPointArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Path2D)]
    path: v8::Local<'s, v8::Object>,
    #[webidl(required, converter = "unrestricted_double")]
    x: f64,
    #[webidl(required, converter = "unrestricted_double")]
    y: f64,
    #[webidl(default = CanvasFillRule::Nonzero, converter = "enum")]
    rule: CanvasFillRule,
}

pub(super) fn is_point_in_path_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let explicit_path = args.length() >= 4
        || (args.length() == 3
            && path_object(
                scope,
                args.get(0),
                webidl::Context::argument("CanvasDrawPath.isPointInPath", 1),
            )
            .is_ok());
    let (path, x, y, rule) = if explicit_path {
        let Some(parsed) = webidl::parse_args::<PathPointArgs>(scope, &args) else {
            return;
        };
        let target = web_api_object_target(scope, parsed.path).expect("validated Path2D argument");
        (Some(target), parsed.x, parsed.y, parsed.rule)
    } else {
        let Some(parsed) = webidl::parse_args::<PointArgs>(scope, &args) else {
            return;
        };
        (None, parsed.x, parsed.y, parsed.rule)
    };
    if !x.is_finite() || !y.is_finite() {
        rv.set_bool(false);
        return;
    }
    // Read native state after all author conversions: valueOf can reset the
    // context or append geometry. Only Path2D geometry uses the current CTM;
    // the default path already froze each command's transform when recorded.
    let context = web_api_object_target(scope, args.this()).expect("validated Canvas 2D receiver");
    let context_state = canvas_path_state(scope, context);
    let transform = if path.is_some() {
        context_state.borrow().transform()
    } else {
        moli_layout::LayoutTransform2D::IDENTITY
    };
    let path_state = path
        .map(|object| canvas_path_state(scope, object))
        .unwrap_or(context_state);
    let contains = path_state
        .borrow()
        .contains_point(x, y, transform, rule.svg_rule());
    rv.set_bool(contains);
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasDrawPath.isPointInStroke")]
struct StrokePointArgs {
    #[webidl(required, converter = "unrestricted_double")]
    x: f64,
    #[webidl(required, converter = "unrestricted_double")]
    y: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasDrawPath.isPointInStroke")]
struct StrokePathPointArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Path2D)]
    path: v8::Local<'s, v8::Object>,
    #[webidl(required, converter = "unrestricted_double")]
    x: f64,
    #[webidl(required, converter = "unrestricted_double")]
    y: f64,
}

pub(super) fn is_point_in_stroke_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let (path, x, y) = if args.length() >= 3 {
        let Some(parsed) = webidl::parse_args::<StrokePathPointArgs>(scope, &args) else {
            return;
        };
        let target = web_api_object_target(scope, parsed.path).expect("validated Path2D argument");
        (Some(target), parsed.x, parsed.y)
    } else {
        let Some(parsed) = webidl::parse_args::<StrokePointArgs>(scope, &args) else {
            return;
        };
        (None, parsed.x, parsed.y)
    };
    if !x.is_finite() || !y.is_finite() {
        rv.set_bool(false);
        return;
    }
    let context = web_api_object_target(scope, args.this()).expect("validated Canvas 2D receiver");
    let context_state = canvas_path_state(scope, context);
    let Some(inverse) = context_state.borrow().inverse_transform() else {
        rv.set_bool(false);
        return;
    };
    let style = super::super::context2d::context_stroke_style(scope, context);
    let geometry_transform = if path.is_some() {
        kurbo::Affine::IDENTITY
    } else {
        inverse
    };
    let state = path
        .map(|object| canvas_path_state(scope, object))
        .unwrap_or(context_state);
    // Stroke widths, caps and dashes are in the current user space. Transform
    // both the query and the frozen default geometry back to that space;
    // explicit Path2D geometry is already there.
    match state.borrow().contains_stroke_point(
        inverse * kurbo::Point::new(x, y),
        geometry_transform,
        &style,
    ) {
        Ok(contains) => rv.set_bool(contains),
        Err(_) => {
            let message = crate::util::v8str(
                scope,
                "Stroke hit testing exceeded the geometry allocation limit.",
            );
            let error = v8::Exception::range_error(scope, message);
            scope.throw_exception(error);
        }
    }
}
