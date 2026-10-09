use super::*;
use crate::webidl;
use moli_canvas::{DEFAULT_FILL_STYLE, DEFAULT_FONT};
use moli_webapi_declare::WebApiObject;

// Initialization, reset and save/restore use the same list of implemented
// drawing attributes. New native attributes must join this declaration.
macro_rules! canvas_drawing_slots {
    ($($name:ident: $ty:ty => $slot:ident = $default:expr;)*) => {
        #[derive(WebApiObject)]
        #[webapi(fragment, prototype = "CanvasRenderingContext2D")]
        struct CanvasLikeContextObjectDeclaration {
            $(#[webapi(slot = $slot, constructor_default = $default)]
            $name: $ty,)*
        }

        pub(super) const CANVAS_DRAWING_SLOTS: &[&str] = &[$($slot),*];
    };
}

canvas_drawing_slots! {
    fill_style: &'static str => CANVAS_CONTEXT_FILL_STYLE_SLOT = DEFAULT_FILL_STYLE;
    font: &'static str => CANVAS_CONTEXT_FONT_SLOT = DEFAULT_FONT;
    image_smoothing_enabled: bool => CANVAS_CONTEXT_IMAGE_SMOOTHING_ENABLED_SLOT = true;
    image_smoothing_quality: &'static str => CANVAS_CONTEXT_IMAGE_SMOOTHING_QUALITY_SLOT = "low";
    global_alpha: f64 => CANVAS_CONTEXT_GLOBAL_ALPHA_SLOT = super::DEFAULT_GLOBAL_ALPHA;
    global_composite_operation: &'static str => CANVAS_CONTEXT_GLOBAL_COMPOSITE_OPERATION_SLOT = super::DEFAULT_GLOBAL_COMPOSITE_OPERATION;
    line_width: f64 => CANVAS_CONTEXT_LINE_WIDTH_SLOT = super::DEFAULT_LINE_WIDTH;
    line_cap: &'static str => CANVAS_CONTEXT_LINE_CAP_SLOT = super::DEFAULT_LINE_CAP;
    line_join: &'static str => CANVAS_CONTEXT_LINE_JOIN_SLOT = super::DEFAULT_LINE_JOIN;
    miter_limit: f64 => CANVAS_CONTEXT_MITER_LIMIT_SLOT = super::DEFAULT_MITER_LIMIT;
    line_dash_offset: f64 => CANVAS_CONTEXT_LINE_DASH_OFFSET_SLOT = super::DEFAULT_LINE_DASH_OFFSET;
    stroke_style: &'static str => CANVAS_CONTEXT_STROKE_STYLE_SLOT = super::DEFAULT_STROKE_STYLE;
    line_dash: Vec<f64> => CANVAS_CONTEXT_LINE_DASH_SLOT = Vec::new();
}

pub(super) fn canvas_unrestricted_double_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    prefix: &'static str,
) -> Option<f64> {
    webidl::argument::<webidl::UnrestrictedDouble>(
        scope,
        args,
        index,
        webidl::Context::argument(prefix, (index + 1) as usize),
    )
    .ok()
    .map(f64::from)
}

pub(super) fn canonical_canvas_fill_style(raw: &str) -> Option<String> {
    moli_canvas::canonicalize_fill_style(raw)
}

pub(super) fn init_canvas_like_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    CanvasLikeContextObjectDeclaration::new()
        .initialize(scope, object)
        .expect("canvas context declaration should initialize object");
}
