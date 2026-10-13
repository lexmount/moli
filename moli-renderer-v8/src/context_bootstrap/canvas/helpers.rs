use super::style_state::CanvasStyleColor;
use super::*;
use crate::native_bridge::{
    element::{StyleMode, style_property_value},
    node_runtime_and_handle_from_object_or_detached,
};
use crate::webidl;
use moli_canvas::DEFAULT_FONT;
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
    fill_style: CanvasStyleColor => CANVAS_CONTEXT_FILL_STYLE_SLOT = CanvasStyleColor::black();
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
    stroke_style: CanvasStyleColor => CANVAS_CONTEXT_STROKE_STYLE_SLOT = CanvasStyleColor::black();
    line_dash: Vec<f64> => CANVAS_CONTEXT_LINE_DASH_SLOT = Vec::new();
    shadow_blur: f64 => CANVAS_CONTEXT_SHADOW_BLUR_SLOT = 0.0;
    shadow_color: CanvasStyleColor => CANVAS_CONTEXT_SHADOW_COLOR_SLOT = CanvasStyleColor::transparent();
    shadow_offset_x: f64 => CANVAS_CONTEXT_SHADOW_OFFSET_X_SLOT = 0.0;
    shadow_offset_y: f64 => CANVAS_CONTEXT_SHADOW_OFFSET_Y_SLOT = 0.0;
    text_align: &'static str => CANVAS_CONTEXT_TEXT_ALIGN_SLOT = "start";
    text_baseline: &'static str => CANVAS_CONTEXT_TEXT_BASELINE_SLOT = "alphabetic";
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

pub(super) fn canonical_canvas_color<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    raw: &str,
) -> Option<CanvasStyleColor> {
    // Resolve currentcolor after DOMString conversion. Author code can change
    // the canvas's document, connection or inherited color during conversion.
    let current = super::backing_store::canvas_owner_from_context(scope, context)
        .and_then(|canvas| node_runtime_and_handle_from_object_or_detached(scope, canvas).ok())
        .and_then(|(runtime, handle)| {
            let runtime = unsafe { &*runtime };
            if !runtime.node_is_connected_for_web_api(handle) {
                return None;
            }
            Some(style_property_value(
                runtime,
                handle,
                StyleMode::Computed,
                "color",
            ))
        });
    let color = moli_css_parse::parse_css_color_for_canvas(raw, current.as_deref())?;
    Some(CanvasStyleColor::new(
        color,
        super::settings::Canvas2dSettings::for_context(scope, context).color_space,
    ))
}

pub(super) fn init_canvas_like_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    CanvasLikeContextObjectDeclaration::new()
        .initialize(scope, object)
        .expect("canvas context declaration should initialize object");
}
