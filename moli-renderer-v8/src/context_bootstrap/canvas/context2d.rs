use super::backing_store::canvas_owner_from_context;

use super::state::canvas_path_state;
use super::*;
use crate::util::{get_private_value, set_private_value};
use crate::web_api_interfaces;
use crate::webidl;
use moli_canvas::{DEFAULT_FONT, draw_text, measure_text_width};
use moli_layout::{
    PaintBrush, PaintColor, PaintFragment, PaintLineCap, PaintLineJoin, PaintShape, PaintSnapshot,
    PaintStroke, PaintTransform2D, PaintViewport,
};
use moli_webapi_declare::WebApiObject;
use std::str::FromStr;

const DEFAULT_IMAGE_SMOOTHING_QUALITY: &str = "low";
const TEXT_METRICS_WIDTH_SLOT: &str = "__moliTextMetricsWidth";

pub(super) fn reset_canvas_context_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) {
    super::helpers::init_canvas_like_context_object(scope, context);
    super::drawing_state::clear_canvas_drawing_stack(scope, context);
    super::state::reset_canvas_native_state(scope, context);
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TextMetrics)]
struct CanvasTextMetricsDeclaration {
    #[webapi(slot = TEXT_METRICS_WIDTH_SLOT)]
    width: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TextMetrics, enumerable)]
struct TextMetricsPrototypeDeclaration {
    #[webapi(accessor_property, getter = text_metrics_width_getter)]
    width: (),
}

pub(super) fn install_text_metrics_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    TextMetricsPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

macro_rules! canvas_rect_declaration {
    ($name:ident, $interface:path) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = $interface, enumerable, receiver)]
        struct $name {
            #[webapi(method = "fillRect", length = 4, callback = canvas_context_fill_rect_callback)]
            fill_rect: (),
            #[webapi(method = "clearRect", length = 4, callback = canvas_context_clear_rect_callback)]
            clear_rect: (),
        }
    };
}

canvas_rect_declaration!(
    CanvasRectDeclaration,
    web_api_interfaces::CanvasRenderingContext2D
);
canvas_rect_declaration!(
    OffscreenCanvasRectDeclaration,
    web_api_interfaces::OffscreenCanvasRenderingContext2D
);

pub(crate) fn install_canvas_rect_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: &str,
) {
    match interface {
        "CanvasRenderingContext2D" => {
            CanvasRectDeclaration::initialize_prototype_template(scope, prototype);
        }
        "OffscreenCanvasRenderingContext2D" => {
            OffscreenCanvasRectDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => unreachable!("only Canvas 2D contexts have fillRect bindings"),
    }
}

fn text_metrics_width_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(width) = get_private_value(scope, args.this(), TEXT_METRICS_WIDTH_SLOT) else {
        throw_type_error(scope, "TextMetrics.width called on incompatible receiver.");
        return;
    };
    rv.set(width);
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::IntoStaticStr, webidl::WebIdlEnum,
)]
#[webidl(name = "ImageSmoothingQuality", parse_with = Self::parse)]
#[strum(serialize_all = "lowercase")]
enum ImageSmoothingQuality {
    Low,
    Medium,
    High,
}

impl ImageSmoothingQuality {
    fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    fn label(self) -> &'static str {
        self.into()
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.fillText")]
struct CanvasContextFillTextArgs {
    #[webidl(required)]
    text: String,
    #[webidl(required)]
    x: f64,
    #[webidl(required)]
    y: f64,
    #[webidl(name = "maxWidth")]
    _max_width: Option<f64>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.strokeText")]
struct CanvasContextStrokeTextArgs {
    #[webidl(required)]
    text: String,
    #[webidl(required)]
    x: f64,
    #[webidl(required)]
    y: f64,
    #[webidl(name = "maxWidth")]
    _max_width: Option<f64>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.measureText")]
struct CanvasContextMeasureTextArgs {
    #[webidl(required)]
    text: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.moveTo")]
struct CanvasContextMoveToArgs {
    #[webidl(required)]
    x: f64,
    #[webidl(required)]
    y: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.lineTo")]
struct CanvasContextLineToArgs {
    #[webidl(required)]
    x: f64,
    #[webidl(required)]
    y: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.quadraticCurveTo")]
struct CanvasContextQuadraticCurveToArgs {
    #[webidl(required)]
    cpx: f64,
    #[webidl(required)]
    cpy: f64,
    #[webidl(required)]
    x: f64,
    #[webidl(required)]
    y: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.bezierCurveTo")]
struct CanvasContextBezierCurveToArgs {
    #[webidl(required)]
    cp1x: f64,
    #[webidl(required)]
    cp1y: f64,
    #[webidl(required)]
    cp2x: f64,
    #[webidl(required)]
    cp2y: f64,
    #[webidl(required)]
    x: f64,
    #[webidl(required)]
    y: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.rect")]
struct CanvasContextRectPathArgs {
    #[webidl(required)]
    x: f64,
    #[webidl(required)]
    y: f64,
    #[webidl(required)]
    width: f64,
    #[webidl(required)]
    height: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.fillRect")]
struct CanvasContextFillRectArgs {
    #[webidl(required)]
    x: f64,
    #[webidl(required)]
    y: f64,
    #[webidl(required)]
    width: f64,
    #[webidl(required)]
    height: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.clearRect")]
struct CanvasContextClearRectArgs {
    #[webidl(required, converter = "unrestricted_double")]
    x: f64,
    #[webidl(required, converter = "unrestricted_double")]
    y: f64,
    #[webidl(required, converter = "unrestricted_double")]
    width: f64,
    #[webidl(required, converter = "unrestricted_double")]
    height: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.arc")]
struct CanvasContextArcArgs {
    #[webidl(required)]
    x: f64,
    #[webidl(required)]
    y: f64,
    #[webidl(required)]
    radius: f64,
    #[webidl(required, name = "startAngle")]
    start_angle: f64,
    #[webidl(required, name = "endAngle")]
    end_angle: f64,
    #[webidl(default = false)]
    counterclockwise: bool,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.arcTo")]
struct CanvasContextArcToArgs {
    #[webidl(required)]
    x1: f64,
    #[webidl(required)]
    y1: f64,
    #[webidl(required)]
    x2: f64,
    #[webidl(required)]
    y2: f64,
    #[webidl(required)]
    radius: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.ellipse")]
struct CanvasContextEllipseArgs {
    #[webidl(required)]
    x: f64,
    #[webidl(required)]
    y: f64,
    #[webidl(required, name = "radiusX")]
    radius_x: f64,
    #[webidl(required, name = "radiusY")]
    radius_y: f64,
    #[webidl(required)]
    rotation: f64,
    #[webidl(required, name = "startAngle")]
    start_angle: f64,
    #[webidl(required, name = "endAngle")]
    end_angle: f64,
    #[webidl(default = false)]
    counterclockwise: bool,
}

fn require_canvas_context_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &str,
) -> bool {
    if web_api_interfaces::CanvasRenderingContext2D::is_instance(scope, receiver)
        || web_api_interfaces::OffscreenCanvasRenderingContext2D::is_instance(scope, receiver)
    {
        return true;
    }
    throw_type_error(
        scope,
        &format!("CanvasRenderingContext2D.{member} called on incompatible receiver."),
    );
    false
}

pub(crate) fn canvas_context_font_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "font getter") {
        return;
    }
    let value = context_string_slot(scope, args.this(), CANVAS_CONTEXT_FONT_SLOT)
        .unwrap_or_else(|| "10px sans-serif".to_owned());
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set(v8::String::empty(scope).into());
    }
}

pub(crate) fn canvas_context_font_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "font setter") {
        return;
    }
    let Some(font) =
        canvas_context_dom_string_value(scope, args.get(0), "CanvasRenderingContext2D", "font")
    else {
        rv.set_undefined();
        return;
    };
    set_context_string_slot(scope, args.this(), CANVAS_CONTEXT_FONT_SLOT, &font);
    rv.set_undefined();
}

pub(crate) fn canvas_context_image_smoothing_enabled_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "imageSmoothingEnabled getter") {
        return;
    }
    let value = context_bool_slot(
        scope,
        args.this(),
        CANVAS_CONTEXT_IMAGE_SMOOTHING_ENABLED_SLOT,
    )
    .unwrap_or(true);
    rv.set(v8::Boolean::new(scope, value).into());
}

pub(crate) fn canvas_context_image_smoothing_enabled_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "imageSmoothingEnabled setter") {
        return;
    }
    let Some(value) = canvas_context_boolean_value(
        scope,
        args.get(0),
        "CanvasRenderingContext2D",
        "imageSmoothingEnabled",
    ) else {
        rv.set_undefined();
        return;
    };
    set_context_bool_slot(
        scope,
        args.this(),
        CANVAS_CONTEXT_IMAGE_SMOOTHING_ENABLED_SLOT,
        value,
    );
    rv.set_undefined();
}

pub(crate) fn canvas_context_image_smoothing_quality_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "imageSmoothingQuality getter") {
        return;
    }
    let value = context_string_slot(
        scope,
        args.this(),
        CANVAS_CONTEXT_IMAGE_SMOOTHING_QUALITY_SLOT,
    )
    .unwrap_or_else(|| DEFAULT_IMAGE_SMOOTHING_QUALITY.to_owned());
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8::String::empty(scope))
            .into(),
    );
}

pub(crate) fn canvas_context_image_smoothing_quality_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "imageSmoothingQuality setter") {
        return;
    }
    let Some(quality) = canvas_context_image_smoothing_quality_value(scope, args.get(0)) else {
        rv.set_undefined();
        return;
    };
    set_context_string_slot(
        scope,
        args.this(),
        CANVAS_CONTEXT_IMAGE_SMOOTHING_QUALITY_SLOT,
        quality.label(),
    );
    rv.set_undefined();
}

fn canvas_context_dom_string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) -> Option<String> {
    match webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::member(owner, property),
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn canvas_required_unrestricted_double_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    prefix: &'static str,
) -> Option<f64> {
    match webidl::argument::<webidl::UnrestrictedDouble>(
        scope,
        args,
        index,
        webidl::Context::argument(prefix, (index + 1) as usize),
    ) {
        Ok(value) => Some(f64::from(value)),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn canvas_context_boolean_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) -> Option<bool> {
    match webidl::convert::<webidl::Boolean>(scope, value, webidl::Context::member(owner, property))
    {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn canvas_context_image_smoothing_quality_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<ImageSmoothingQuality> {
    match webidl::convert::<webidl::EnumValue<ImageSmoothingQuality>>(
        scope,
        value,
        webidl::Context::member("CanvasRenderingContext2D", "imageSmoothingQuality"),
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

pub(crate) fn canvas_context_global_alpha_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "globalAlpha getter") {
        return;
    }
    let value = context_number_slot(scope, args.this(), CANVAS_CONTEXT_GLOBAL_ALPHA_SLOT)
        .unwrap_or(DEFAULT_GLOBAL_ALPHA);
    rv.set(v8::Number::new(scope, value).into());
}

pub(crate) fn canvas_context_global_alpha_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "globalAlpha setter") {
        return;
    }
    canvas_context_global_alpha_assign(scope, args.this(), args.get(0));
    rv.set_undefined();
}

fn canvas_context_global_alpha_assign<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    // Per HTML spec: globalAlpha is an `unrestricted double`. The WebIDL
    // conversion itself must throw TypeError on values that aren't
    // convertible (e.g. Symbol) — only AFTER coercion succeeds do we
    // silently ignore non-finite / out-of-range numbers per the canvas
    // setter semantics.
    let unrestricted = match webidl::convert::<webidl::UnrestrictedDouble>(
        scope,
        value,
        webidl::Context::member("CanvasRenderingContext2D", "globalAlpha"),
    ) {
        Ok(value) => value,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let raw = f64::from(unrestricted);
    if !raw.is_finite() || !(0.0..=1.0).contains(&raw) {
        return;
    }
    set_context_number_slot(scope, holder, CANVAS_CONTEXT_GLOBAL_ALPHA_SLOT, raw);
}

pub(crate) fn canvas_context_global_composite_operation_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "globalCompositeOperation getter") {
        return;
    }
    let value = context_string_slot(
        scope,
        args.this(),
        CANVAS_CONTEXT_GLOBAL_COMPOSITE_OPERATION_SLOT,
    )
    .unwrap_or_else(|| DEFAULT_GLOBAL_COMPOSITE_OPERATION.to_owned());
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8::String::empty(scope))
            .into(),
    );
}

pub(crate) fn canvas_context_global_composite_operation_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "globalCompositeOperation setter") {
        return;
    }
    canvas_context_global_composite_operation_assign(scope, args.this(), args.get(0));
    rv.set_undefined();
}

fn canvas_context_global_composite_operation_assign<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    // Per HTML spec: setting an unrecognised value for globalCompositeOperation
    // (legacy aliases like `darker`/`highlight`, capitalised variants,
    // unknown strings) must leave the property unchanged. Only the canonical
    // lowercase tokens in the shared native compositor are accepted.
    let Some(string) = canvas_context_dom_string_value(
        scope,
        value,
        "CanvasRenderingContext2D",
        "globalCompositeOperation",
    ) else {
        return;
    };
    let Some(canonical) = canvas_composite_operation_canonical(&string) else {
        return;
    };
    set_context_string_slot(
        scope,
        holder,
        CANVAS_CONTEXT_GLOBAL_COMPOSITE_OPERATION_SLOT,
        canonical,
    );
}

fn context_string_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<String> {
    let object = moli_webapi_declare::web_api_object_target(scope, object).unwrap_or(object);
    get_private_value(scope, object, slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn context_bool_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<bool> {
    let object = moli_webapi_declare::web_api_object_target(scope, object).unwrap_or(object);
    get_private_value(scope, object, slot).map(|value| value.boolean_value(scope))
}

fn context_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<f64> {
    let object = moli_webapi_declare::web_api_object_target(scope, object).unwrap_or(object);
    get_private_value(scope, object, slot).and_then(|value| value.number_value(scope))
}

fn set_context_string_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: &str,
) {
    let object = moli_webapi_declare::web_api_object_target(scope, object).unwrap_or(object);
    if let Some(value) = v8_string(scope, value) {
        set_private_value(scope, object, slot, value.into());
    }
}

fn set_context_bool_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: bool,
) {
    let object = moli_webapi_declare::web_api_object_target(scope, object).unwrap_or(object);
    let value = v8::Boolean::new(scope, value);
    set_private_value(scope, object, slot, value.into());
}

fn set_context_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: f64,
) {
    let object = moli_webapi_declare::web_api_object_target(scope, object).unwrap_or(object);
    let value = v8::Number::new(scope, value);
    set_private_value(scope, object, slot, value.into());
}

pub(crate) fn canvas_context_fill_rect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated fillRect binding validates the receiver");
    let Some(parsed) = webidl::parse_args::<CanvasContextFillRectArgs>(scope, &args) else {
        return;
    };
    if ![parsed.x, parsed.y, parsed.width, parsed.height]
        .into_iter()
        .all(f64::is_finite)
        || parsed.width == 0.0
        || parsed.height == 0.0
    {
        return;
    }
    let Some(canvas) = canvas_owner_from_context(scope, context) else {
        return;
    };
    let path_state = canvas_path_state(scope, context);
    let state = path_state.borrow();
    if state.inverse_transform().is_none() {
        return;
    }
    let transform = state.transform();
    drop(state);
    // Rectangles use local coordinates and must leave the default path untouched.
    let mut rectangle = super::path::Canvas2dPathState::default();
    rectangle.rect(parsed.x, parsed.y, parsed.width, parsed.height);
    let fragment = PaintFragment::Fill {
        fill_rule: moli_layout::PaintFillRule::NonZero,
        shape: PaintShape::Path(rectangle.paint_path()),
        brush: PaintBrush::Solid(PaintColor::WHITE),
        transform,
    };
    let Some(color) = context_fill_paint(scope, context) else {
        return;
    };
    rasterize_canvas_fragment(scope, context, canvas, fragment, color);
}

pub(crate) fn canvas_context_clear_rect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated clearRect receiver");
    let Some(parsed) = webidl::parse_args::<CanvasContextClearRectArgs>(scope, &args) else {
        return;
    };
    if ![parsed.x, parsed.y, parsed.width, parsed.height]
        .into_iter()
        .all(f64::is_finite)
        || parsed.width == 0.0
        || parsed.height == 0.0
    {
        return;
    }
    let Some(canvas) = canvas_owner_from_context(scope, context) else {
        return;
    };
    let transform = canvas_path_state(scope, context).borrow().transform();
    let mut rectangle = super::path::Canvas2dPathState::default();
    rectangle.rect(parsed.x, parsed.y, parsed.width, parsed.height);
    let fragment = PaintFragment::Fill {
        fill_rule: moli_layout::PaintFillRule::NonZero,
        shape: PaintShape::Path(rectangle.paint_path()),
        brush: PaintBrush::Solid(PaintColor::WHITE),
        transform,
    };
    let opaque = !super::settings::Canvas2dSettings::for_context(scope, context).alpha;
    super::clipping::draw_with_clip(scope, context, canvas, |image, clip| {
        let width = image.pixels.width();
        let height = image.pixels.height();
        if width == 0 || height == 0 {
            return Ok(());
        }
        let mut snapshot = PaintSnapshot::new(
            PaintViewport::new(width, height, 1.0),
            PaintColor::new(0.0, 0.0, 0.0, 0.0),
        );
        snapshot.push_fragment(fragment);
        let raster = moli_paint::raster_snapshot(&snapshot)?;
        image
            .pixels
            .clear_coverage(&raster.rgba, clip, opaque)
            .expect("validated clear and clipping dimensions");
        Ok(())
    });
}

pub(crate) fn canvas_context_rect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated CanvasPath binding validates the receiver");
    let Some(parsed) = webidl::parse_args::<CanvasContextRectPathArgs>(scope, &args) else {
        return;
    };
    let path_state = canvas_path_state(scope, receiver);
    with_path_state(&path_state, |state| {
        state.rect(parsed.x, parsed.y, parsed.width, parsed.height);
    });
}

pub(crate) fn canvas_context_begin_path_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "beginPath") {
        return;
    }
    let path_state = canvas_path_state(scope, args.this());
    with_path_state(&path_state, |state| state.begin_path());
}

pub(crate) fn canvas_context_close_path_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated CanvasPath binding validates the receiver");
    let path_state = canvas_path_state(scope, receiver);
    with_path_state(&path_state, |state| state.close_path());
}

pub(crate) fn canvas_context_move_to_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated CanvasPath binding validates the receiver");
    let Some(parsed) = webidl::parse_args::<CanvasContextMoveToArgs>(scope, &args) else {
        return;
    };
    let path_state = canvas_path_state(scope, receiver);
    with_path_state(&path_state, |state| state.move_to(parsed.x, parsed.y));
}

pub(crate) fn canvas_context_line_to_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated CanvasPath binding validates the receiver");
    let Some(parsed) = webidl::parse_args::<CanvasContextLineToArgs>(scope, &args) else {
        return;
    };
    let path_state = canvas_path_state(scope, receiver);
    with_path_state(&path_state, |state| state.line_to(parsed.x, parsed.y));
}

pub(crate) fn canvas_context_quadratic_curve_to_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated CanvasPath binding validates the receiver");
    let Some(parsed) = webidl::parse_args::<CanvasContextQuadraticCurveToArgs>(scope, &args) else {
        return;
    };
    let path_state = canvas_path_state(scope, receiver);
    with_path_state(&path_state, |state| {
        state.quadratic_curve_to(parsed.cpx, parsed.cpy, parsed.x, parsed.y);
    });
}

pub(crate) fn canvas_context_bezier_curve_to_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated CanvasPath binding validates the receiver");
    let Some(parsed) = webidl::parse_args::<CanvasContextBezierCurveToArgs>(scope, &args) else {
        return;
    };
    let path_state = canvas_path_state(scope, receiver);
    with_path_state(&path_state, |state| {
        state.bezier_curve_to(
            parsed.cp1x,
            parsed.cp1y,
            parsed.cp2x,
            parsed.cp2y,
            parsed.x,
            parsed.y,
        );
    });
}

pub(crate) fn canvas_context_arc_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated CanvasPath binding validates the receiver");
    let Some(parsed) = webidl::parse_args::<CanvasContextArcArgs>(scope, &args) else {
        return;
    };
    if ![
        parsed.x,
        parsed.y,
        parsed.radius,
        parsed.start_angle,
        parsed.end_angle,
    ]
    .into_iter()
    .all(f64::is_finite)
    {
        return;
    }
    if parsed.radius < 0.0 {
        webidl::throw_index_size_error(scope);
        rv.set_undefined();
        return;
    }
    let path_state = canvas_path_state(scope, receiver);
    with_path_state(&path_state, |state| {
        state.arc(
            parsed.x,
            parsed.y,
            parsed.radius,
            parsed.start_angle,
            parsed.end_angle,
            parsed.counterclockwise,
        );
    });
    rv.set_undefined();
}

pub(crate) fn canvas_context_arc_to_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated CanvasPath binding validates the receiver");
    let Some(parsed) = webidl::parse_args::<CanvasContextArcToArgs>(scope, &args) else {
        return;
    };
    if ![parsed.x1, parsed.y1, parsed.x2, parsed.y2, parsed.radius]
        .into_iter()
        .all(f64::is_finite)
    {
        return;
    }
    if parsed.radius < 0.0 {
        webidl::throw_index_size_error(scope);
        rv.set_undefined();
        return;
    }
    let path_state = canvas_path_state(scope, receiver);
    with_path_state(&path_state, |state| {
        state.arc_to(parsed.x1, parsed.y1, parsed.x2, parsed.y2, parsed.radius);
    });
    rv.set_undefined();
}

pub(crate) fn canvas_context_ellipse_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated CanvasPath binding validates the receiver");
    let Some(parsed) = webidl::parse_args::<CanvasContextEllipseArgs>(scope, &args) else {
        return;
    };
    if ![
        parsed.x,
        parsed.y,
        parsed.radius_x,
        parsed.radius_y,
        parsed.rotation,
        parsed.start_angle,
        parsed.end_angle,
    ]
    .into_iter()
    .all(f64::is_finite)
    {
        return;
    }
    if parsed.radius_x < 0.0 || parsed.radius_y < 0.0 {
        webidl::throw_index_size_error(scope);
        rv.set_undefined();
        return;
    }
    let path_state = canvas_path_state(scope, receiver);
    with_path_state(&path_state, |state| {
        state.ellipse(
            parsed.x,
            parsed.y,
            parsed.radius_x,
            parsed.radius_y,
            parsed.rotation,
            parsed.start_angle,
            parsed.end_angle,
            parsed.counterclockwise,
        );
    });
    rv.set_undefined();
}

pub(crate) fn canvas_context_translate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "translate") {
        return;
    }
    let prefix = "CanvasRenderingContext2D.translate";
    let Some(x) = canvas_required_unrestricted_double_arg(scope, &args, 0, prefix) else {
        return;
    };
    let Some(y) = canvas_required_unrestricted_double_arg(scope, &args, 1, prefix) else {
        return;
    };
    let path_state = canvas_path_state(scope, args.this());
    with_path_state(&path_state, |state| state.translate(x, y));
}

pub(crate) fn canvas_context_scale_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "scale") {
        return;
    }
    let prefix = "CanvasRenderingContext2D.scale";
    let Some(x) = canvas_required_unrestricted_double_arg(scope, &args, 0, prefix) else {
        return;
    };
    let Some(y) = canvas_required_unrestricted_double_arg(scope, &args, 1, prefix) else {
        return;
    };
    let path_state = canvas_path_state(scope, args.this());
    with_path_state(&path_state, |state| state.scale(x, y));
}

pub(crate) fn canvas_context_rotate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "rotate") {
        return;
    }
    let prefix = "CanvasRenderingContext2D.rotate";
    let Some(angle) = canvas_required_unrestricted_double_arg(scope, &args, 0, prefix) else {
        return;
    };
    let path_state = canvas_path_state(scope, args.this());
    with_path_state(&path_state, |state| state.rotate(angle));
}

pub(crate) fn canvas_context_transform_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "transform") {
        return;
    }
    let Some([a, b, c, d, e, f]) =
        super::transform::transform_arguments(scope, &args, "CanvasRenderingContext2D.transform")
    else {
        return;
    };
    let path_state = canvas_path_state(scope, args.this());
    with_path_state(&path_state, |state| {
        state.concatenate_transform(a, b, c, d, e, f);
    });
}

pub(crate) fn canvas_context_set_transform_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "setTransform") {
        return;
    }
    let Some([a, b, c, d, e, f]) = super::transform::set_transform_arguments(scope, &args) else {
        return;
    };
    let path_state = canvas_path_state(scope, args.this());
    with_path_state(&path_state, |state| {
        state.set_transform(a, b, c, d, e, f);
    });
}

pub(crate) fn canvas_context_reset_transform_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "resetTransform") {
        return;
    }
    let path_state = canvas_path_state(scope, args.this());
    with_path_state(&path_state, |state| state.reset_transform());
}

pub(crate) fn canvas_context_fill_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated CanvasDrawPath binding validates the receiver");
    let Some((explicit_path, rule)) = super::path2d::fill_arguments(scope, &args) else {
        return;
    };
    let Some(canvas) = canvas_owner_from_context(scope, context) else {
        return;
    };
    let path_state = canvas_path_state(scope, explicit_path.unwrap_or(context));
    let transform = canvas_path_state(scope, context).borrow().transform();
    let fragment = with_path_state(&path_state, |state| {
        if state.is_empty() || state.inverse_transform().is_none() {
            return None;
        }
        Some(PaintFragment::Fill {
            fill_rule: rule.paint_rule(),
            shape: PaintShape::Path(state.paint_path()),
            brush: PaintBrush::Solid(PaintColor::WHITE),
            transform: if explicit_path.is_some() {
                transform
            } else {
                PaintTransform2D::IDENTITY
            },
        })
    });
    if let Some(fragment) = fragment {
        let Some(color) = context_fill_paint(scope, context) else {
            return;
        };
        rasterize_canvas_fragment(scope, context, canvas, fragment, color);
    }
    rv.set_undefined();
}

pub(crate) fn canvas_context_stroke_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated CanvasDrawPath binding validates the receiver");
    let Some(parsed) = webidl::parse_args::<super::path2d::StrokeArgs>(scope, &args) else {
        return;
    };
    let explicit_path = parsed.path.map(|object| {
        moli_webapi_declare::web_api_object_target(scope, object)
            .expect("validated Path2D argument")
    });
    let Some(canvas) = canvas_owner_from_context(scope, context) else {
        return;
    };
    let path_state = canvas_path_state(scope, explicit_path.unwrap_or(context));
    let transform = canvas_path_state(scope, context).borrow().transform();
    let fragment = with_path_state(&path_state, |state| {
        if state.is_empty() {
            return None;
        }
        Some(PaintFragment::Stroke(context_stroke(
            scope,
            context,
            if explicit_path.is_some() {
                state.paint_path()
            } else {
                state.stroke_path()?
            },
            transform,
        )))
    });
    if let Some(fragment) = fragment {
        let Some(color) = context_style_paint(scope, context, CANVAS_CONTEXT_STROKE_STYLE_SLOT)
        else {
            return;
        };
        rasterize_canvas_fragment(scope, context, canvas, fragment, color);
    }
    rv.set_undefined();
}

pub(crate) fn canvas_context_stroke_rect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "strokeRect") {
        return;
    }
    let Some(canvas) = canvas_owner_from_context(scope, args.this()) else {
        return;
    };
    let prefix = "CanvasRenderingContext2D.strokeRect";
    let Some(x) = canvas_required_unrestricted_double_arg(scope, &args, 0, prefix) else {
        rv.set_undefined();
        return;
    };
    let Some(y) = canvas_required_unrestricted_double_arg(scope, &args, 1, prefix) else {
        rv.set_undefined();
        return;
    };
    let Some(width) = canvas_required_unrestricted_double_arg(scope, &args, 2, prefix) else {
        rv.set_undefined();
        return;
    };
    let Some(height) = canvas_required_unrestricted_double_arg(scope, &args, 3, prefix) else {
        rv.set_undefined();
        return;
    };
    let path_state = canvas_path_state(scope, args.this());
    // strokeRect must not alter the current default path.
    let path = with_path_state(&path_state, |state| {
        let mut rect_path = super::path::Canvas2dPathState::default();
        rect_path.rect(x, y, width, height);
        state.inverse_transform()?;
        Some((rect_path.paint_path(), state.transform()))
    });
    let Some((path, transform)) = path else {
        return;
    };
    let stroke = context_stroke(scope, args.this(), path, transform);
    let Some(color) = context_style_paint(scope, args.this(), CANVAS_CONTEXT_STROKE_STYLE_SLOT)
    else {
        return;
    };
    rasterize_canvas_fragment(
        scope,
        args.this(),
        canvas,
        PaintFragment::Stroke(stroke),
        color,
    );
    rv.set_undefined();
}

pub(crate) fn canvas_context_line_width_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "lineWidth getter") {
        return;
    }
    let value = context_number_slot(scope, args.this(), CANVAS_CONTEXT_LINE_WIDTH_SLOT)
        .unwrap_or(DEFAULT_LINE_WIDTH);
    rv.set(v8::Number::new(scope, value).into());
}

pub(crate) fn canvas_context_line_width_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "lineWidth setter") {
        return;
    }
    canvas_context_positive_number_assign(
        scope,
        args.this(),
        args.get(0),
        CANVAS_CONTEXT_LINE_WIDTH_SLOT,
    );
    rv.set_undefined();
}

pub(crate) fn canvas_context_miter_limit_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "miterLimit getter") {
        return;
    }
    let value = context_number_slot(scope, args.this(), CANVAS_CONTEXT_MITER_LIMIT_SLOT)
        .unwrap_or(DEFAULT_MITER_LIMIT);
    rv.set(v8::Number::new(scope, value).into());
}

pub(crate) fn canvas_context_miter_limit_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "miterLimit setter") {
        return;
    }
    canvas_context_positive_number_assign(
        scope,
        args.this(),
        args.get(0),
        CANVAS_CONTEXT_MITER_LIMIT_SLOT,
    );
    rv.set_undefined();
}

pub(crate) fn canvas_context_line_dash_offset_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "lineDashOffset getter") {
        return;
    }
    let value = context_number_slot(scope, args.this(), CANVAS_CONTEXT_LINE_DASH_OFFSET_SLOT)
        .unwrap_or(DEFAULT_LINE_DASH_OFFSET);
    rv.set(v8::Number::new(scope, value).into());
}

pub(crate) fn canvas_context_line_dash_offset_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "lineDashOffset setter") {
        return;
    }
    // A non-finite value is ignored per the canvas setter semantics.
    let value = webidl::convert::<webidl::UnrestrictedDouble>(
        scope,
        args.get(0),
        webidl::Context::member("CanvasRenderingContext2D", "lineDashOffset"),
    );
    if let Ok(value) = value {
        let value = f64::from(value);
        if value.is_finite() {
            set_context_number_slot(
                scope,
                args.this(),
                CANVAS_CONTEXT_LINE_DASH_OFFSET_SLOT,
                value,
            );
        }
    }
    rv.set_undefined();
}

pub(crate) fn canvas_context_line_cap_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "lineCap getter") {
        return;
    }
    let value = context_string_slot(scope, args.this(), CANVAS_CONTEXT_LINE_CAP_SLOT)
        .unwrap_or_else(|| DEFAULT_LINE_CAP.to_owned());
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8::String::empty(scope))
            .into(),
    );
}

pub(crate) fn canvas_context_line_cap_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "lineCap setter") {
        return;
    }
    canvas_context_enum_string_assign(
        scope,
        args.this(),
        args.get(0),
        CANVAS_CONTEXT_LINE_CAP_SLOT,
        &["butt", "round", "square"],
    );
    rv.set_undefined();
}

pub(crate) fn canvas_context_line_join_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "lineJoin getter") {
        return;
    }
    let value = context_string_slot(scope, args.this(), CANVAS_CONTEXT_LINE_JOIN_SLOT)
        .unwrap_or_else(|| DEFAULT_LINE_JOIN.to_owned());
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8::String::empty(scope))
            .into(),
    );
}

pub(crate) fn canvas_context_line_join_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "lineJoin setter") {
        return;
    }
    canvas_context_enum_string_assign(
        scope,
        args.this(),
        args.get(0),
        CANVAS_CONTEXT_LINE_JOIN_SLOT,
        &["miter", "round", "bevel"],
    );
    rv.set_undefined();
}

fn canvas_context_positive_number_assign<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    slot: &'static str,
) {
    let value = webidl::convert::<webidl::UnrestrictedDouble>(
        scope,
        value,
        webidl::Context::member("CanvasRenderingContext2D", slot),
    );
    if let Ok(value) = value {
        let value = f64::from(value);
        if value.is_finite() && value > 0.0 {
            set_context_number_slot(scope, holder, slot, value);
        }
    }
}

fn canvas_context_enum_string_assign<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    slot: &'static str,
    accepted: &[&'static str],
) {
    let Some(string) =
        canvas_context_dom_string_value(scope, value, "CanvasRenderingContext2D", slot)
    else {
        return;
    };
    if accepted.contains(&string.as_str()) {
        set_context_string_slot(scope, holder, slot, &string);
    }
}

fn with_path_state<T>(
    state: &std::cell::RefCell<super::path::Canvas2dPathState>,
    update: impl FnOnce(&mut super::path::Canvas2dPathState) -> T,
) -> T {
    update(&mut state.borrow_mut())
}

fn context_fill_paint<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> Option<super::style_state::CanvasPaint> {
    context_style_paint(scope, context, CANVAS_CONTEXT_FILL_STYLE_SLOT)
}

fn context_global_alpha<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> f64 {
    context_number_slot(scope, context, CANVAS_CONTEXT_GLOBAL_ALPHA_SLOT)
        .unwrap_or(DEFAULT_GLOBAL_ALPHA)
}

fn context_style_paint<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<super::style_state::CanvasPaint> {
    let context = moli_webapi_declare::web_api_object_target(scope, context)
        .expect("validated Canvas paint receiver");
    let alpha = context_global_alpha(scope, context) as f32;
    super::style_state::context_paint(scope, context, slot, alpha)
}

fn context_stroke<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    path: moli_layout::PaintPath,
    transform: PaintTransform2D,
) -> PaintStroke {
    let style = context_stroke_style(scope, context);
    let cap = match style.start_cap {
        kurbo::Cap::Round => PaintLineCap::Round,
        kurbo::Cap::Square => PaintLineCap::Square,
        kurbo::Cap::Butt => PaintLineCap::Butt,
    };
    PaintStroke {
        path,
        color: PaintColor::WHITE,
        width: style.width as f32,
        join: match style.join {
            kurbo::Join::Round => PaintLineJoin::Round,
            kurbo::Join::Bevel => PaintLineJoin::Bevel,
            kurbo::Join::Miter => PaintLineJoin::Miter,
        },
        start_cap: cap,
        end_cap: cap,
        miter_limit: style.miter_limit as f32,
        dash_pattern: style
            .dash_pattern
            .iter()
            .map(|value| *value as f32)
            .collect(),
        dash_offset: style.dash_offset as f32,
        transform,
    }
}

pub(super) fn context_stroke_style<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> kurbo::Stroke {
    let join = match context_string_slot(scope, context, CANVAS_CONTEXT_LINE_JOIN_SLOT).as_deref() {
        Some("round") => kurbo::Join::Round,
        Some("bevel") => kurbo::Join::Bevel,
        _ => kurbo::Join::Miter,
    };
    let cap = match context_string_slot(scope, context, CANVAS_CONTEXT_LINE_CAP_SLOT).as_deref() {
        Some("round") => kurbo::Cap::Round,
        Some("square") => kurbo::Cap::Square,
        _ => kurbo::Cap::Butt,
    };
    let dashes = context_line_dash(scope, context);
    kurbo::Stroke {
        width: context_number_slot(scope, context, CANVAS_CONTEXT_LINE_WIDTH_SLOT)
            .unwrap_or(DEFAULT_LINE_WIDTH),
        join,
        start_cap: cap,
        end_cap: cap,
        miter_limit: context_number_slot(scope, context, CANVAS_CONTEXT_MITER_LIMIT_SLOT)
            .unwrap_or(DEFAULT_MITER_LIMIT),
        // HTML treats an all-zero dash list as a solid stroke. Keep the
        // original private list intact for getLineDash and save/restore.
        dash_pattern: if dashes.iter().all(|dash| *dash == 0.0) {
            Default::default()
        } else {
            dashes.into_iter().collect()
        },
        dash_offset: context_number_slot(scope, context, CANVAS_CONTEXT_LINE_DASH_OFFSET_SLOT)
            .unwrap_or(DEFAULT_LINE_DASH_OFFSET),
    }
}

fn context_line_dash<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> Vec<f64> {
    let Some(array) = get_private_value(scope, context, CANVAS_CONTEXT_LINE_DASH_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return Vec::new();
    };
    (0..array.length())
        .filter_map(|index| array.get_index(scope, index))
        .filter_map(|value| value.number_value(scope))
        .collect()
}

fn rasterize_canvas_fragment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    canvas: v8::Local<'s, v8::Object>,
    fragment: PaintFragment,
    paint: super::style_state::CanvasPaint,
) {
    super::clipping::draw_with_clip(scope, context, canvas, |image, clip| {
        let width = image.pixels.width();
        let height = image.pixels.height();
        if width == 0 || height == 0 {
            return Ok(());
        }
        let mut snapshot = PaintSnapshot::new(
            PaintViewport::new(width, height, 1.0),
            PaintColor::new(0.0, 0.0, 0.0, 0.0),
        );
        snapshot.push_fragment(fragment);
        if let Ok(raster) = moli_paint::raster_snapshot(&snapshot)
            && raster.width == width
            && raster.height == height
        {
            image
                .pixels
                .composite_source(
                    &raster.rgba,
                    clip,
                    moli_canvas::CompositeOperation::SourceOver,
                    &paint,
                )
                .expect("validated raster and clipping dimensions");
        }
        Ok(())
    });
}

pub(crate) fn canvas_context_fill_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(canvas) = canvas_owner_from_context(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<CanvasContextFillTextArgs>(scope, &args) else {
        return;
    };
    draw_canvas_context_text(
        scope,
        args.this(),
        canvas,
        &parsed.text,
        parsed.x,
        parsed.y,
        CANVAS_CONTEXT_FILL_STYLE_SLOT,
    );
}

pub(crate) fn canvas_context_stroke_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(canvas) = canvas_owner_from_context(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<CanvasContextStrokeTextArgs>(scope, &args) else {
        return;
    };
    draw_canvas_context_text(
        scope,
        args.this(),
        canvas,
        &parsed.text,
        parsed.x,
        parsed.y,
        CANVAS_CONTEXT_STROKE_STYLE_SLOT,
    );
}

fn draw_canvas_context_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    canvas: v8::Local<'s, v8::Object>,
    text: &str,
    x: f64,
    y: f64,
    slot: &'static str,
) {
    let font = context_string_slot(scope, context, CANVAS_CONTEXT_FONT_SLOT)
        .unwrap_or_else(|| DEFAULT_FONT.to_owned());
    let Some(color) = context_style_paint(scope, context, slot) else {
        return;
    };
    super::clipping::draw_with_clip(scope, context, canvas, |image, clip| {
        let width = image.pixels.width();
        let height = image.pixels.height();
        let Some(length) = moli_canvas::byte_len(width, height) else {
            return Ok(());
        };
        let mut coverage = vec![0; length];
        draw_text(&mut coverage, width, height, text, x, y, &font, [255; 4]);
        image
            .pixels
            .composite_source(
                &coverage,
                clip,
                moli_canvas::CompositeOperation::SourceOver,
                &color,
            )
            .expect("validated text and clipping dimensions");
        Ok(())
    });
}

pub(crate) fn canvas_context_set_line_dash_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if canvas_owner_from_context(scope, args.this()).is_none() {
        throw_type_error(
            scope,
            "CanvasRenderingContext2D.setLineDash: Illegal invocation",
        );
        return;
    }
    let sequence = match webidl::argument::<webidl::Sequence<webidl::UnrestrictedDouble>>(
        scope,
        &args,
        0,
        webidl::Context::argument("CanvasRenderingContext2D.setLineDash", 1),
    ) {
        Ok(sequence) => sequence,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let mut segments = sequence
        .0
        .into_iter()
        .map(|segment| segment.0)
        .collect::<Vec<_>>();
    if segments
        .iter()
        .any(|segment| !segment.is_finite() || *segment < 0.0)
    {
        return;
    }
    if segments.len() % 2 == 1 {
        let repeated = segments.clone();
        segments.extend(repeated);
    }
    let values = segments
        .into_iter()
        .map(|segment| v8::Number::new(scope, segment).into())
        .collect::<Vec<_>>();
    let line_dash = v8::Array::new_with_elements(scope, &values);
    set_private_value(
        scope,
        args.this(),
        CANVAS_CONTEXT_LINE_DASH_SLOT,
        line_dash.into(),
    );
}

pub(crate) fn canvas_context_get_line_dash_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if canvas_owner_from_context(scope, args.this()).is_none() {
        throw_type_error(
            scope,
            "CanvasRenderingContext2D.getLineDash: Illegal invocation",
        );
        return;
    }
    let Some(line_dash) = get_private_value(scope, args.this(), CANVAS_CONTEXT_LINE_DASH_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    let values = (0..line_dash.length())
        .filter_map(|index| line_dash.get_index(scope, index))
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &values).into());
}

pub(crate) fn canvas_context_measure_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "measureText") {
        return;
    }
    let Some(parsed) = webidl::parse_args::<CanvasContextMeasureTextArgs>(scope, &args) else {
        return;
    };
    let font = context_string_slot(scope, args.this(), CANVAS_CONTEXT_FONT_SLOT)
        .unwrap_or_else(|| DEFAULT_FONT.to_owned());
    let declaration = CanvasTextMetricsDeclaration {
        width: measure_text_width(&parsed.text, &font),
    };
    let relevant_context = canvas_context_relevant_context(scope, args.this())
        .unwrap_or_else(|| scope.get_current_context());
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    let Ok(metrics) = declaration.bind(target_scope) else {
        return;
    };
    rv.set(metrics.into());
}

fn canvas_context_relevant_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Context>> {
    // Adoption changes ownerDocument, but the context keeps its original realm.
    context.get_creation_context(scope).or_else(|| {
        canvas_owner_from_context(scope, context)
            .and_then(|canvas| crate::native_bridge::node_relevant_context(scope, canvas))
    })
}

mod draw_image;
pub(crate) use draw_image::install_canvas_draw_image_bindings;

mod pixel_manipulation;
pub(crate) use pixel_manipulation::install_canvas_pixel_bindings;
