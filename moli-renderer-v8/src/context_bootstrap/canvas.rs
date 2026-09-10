use super::super::blob::build_blob_object;
use super::super::native_bridge::element;
use super::super::util::{throw_type_error, v8_string};
use super::shared::{global_constructor_object, global_constructor_prototype};
use crate::webidl;
use moli_webapi_declare::WebApiFunctionTemplate;
use std::str::FromStr;

const OFFSCREEN_CANVAS_WIDTH_SLOT: &str = "__moliOffscreenCanvasWidth";
const OFFSCREEN_CANVAS_HEIGHT_SLOT: &str = "__moliOffscreenCanvasHeight";
const CANVAS_CONTEXT_FILL_STYLE_SLOT: &str = "__moliCanvasContextFillStyle";
const CANVAS_CONTEXT_FONT_SLOT: &str = "__moliCanvasContextFont";
const CANVAS_CONTEXT_IMAGE_SMOOTHING_ENABLED_SLOT: &str =
    "__moliCanvasContextImageSmoothingEnabled";
const CANVAS_CONTEXT_IMAGE_SMOOTHING_QUALITY_SLOT: &str =
    "__moliCanvasContextImageSmoothingQuality";
const CANVAS_CONTEXT_GLOBAL_ALPHA_SLOT: &str = "__moliCanvasContextGlobalAlpha";
const CANVAS_CONTEXT_GLOBAL_COMPOSITE_OPERATION_SLOT: &str =
    "__moliCanvasContextGlobalCompositeOperation";
const CANVAS_CONTEXT_LINE_WIDTH_SLOT: &str = "__moliCanvasContextLineWidth";
const CANVAS_CONTEXT_LINE_CAP_SLOT: &str = "__moliCanvasContextLineCap";
const CANVAS_CONTEXT_LINE_JOIN_SLOT: &str = "__moliCanvasContextLineJoin";
const CANVAS_CONTEXT_MITER_LIMIT_SLOT: &str = "__moliCanvasContextMiterLimit";
const CANVAS_CONTEXT_LINE_DASH_OFFSET_SLOT: &str = "__moliCanvasContextLineDashOffset";
const CANVAS_CONTEXT_STROKE_STYLE_SLOT: &str = "__moliCanvasContextStrokeStyle";

pub(crate) const DEFAULT_GLOBAL_ALPHA: f64 = 1.0;
pub(crate) const DEFAULT_GLOBAL_COMPOSITE_OPERATION: &str = "source-over";
pub(crate) const DEFAULT_LINE_WIDTH: f64 = 1.0;
pub(crate) const DEFAULT_LINE_CAP: &str = "butt";
pub(crate) const DEFAULT_LINE_JOIN: &str = "miter";
pub(crate) const DEFAULT_MITER_LIMIT: f64 = 10.0;
pub(crate) const DEFAULT_LINE_DASH_OFFSET: f64 = 0.0;
pub(crate) const DEFAULT_STROKE_STYLE: &str = "#000000";

/// Composite operations recognised by the HTML Canvas 2D spec.
///
/// Setters that receive any other value (including legacy aliases such as
/// `darker`, `clear`, `highlight`, capitalised variants, and unknown strings)
/// must silently leave the current value unchanged per the spec.
pub(crate) const VALID_GLOBAL_COMPOSITE_OPERATIONS: &[&str] = &[
    "source-over",
    "source-in",
    "source-out",
    "source-atop",
    "destination-over",
    "destination-in",
    "destination-out",
    "destination-atop",
    "lighter",
    "copy",
    "xor",
    "multiply",
    "screen",
    "overlay",
    "darken",
    "lighten",
    "color-dodge",
    "color-burn",
    "hard-light",
    "soft-light",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "color",
    "luminosity",
    "plus-darker",
    "plus-lighter",
];

pub(crate) fn canvas_composite_operation_canonical(value: &str) -> Option<&'static str> {
    VALID_GLOBAL_COMPOSITE_OPERATIONS
        .iter()
        .copied()
        .find(|candidate| *candidate == value)
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLCanvasElement", receiver = crate::native_bridge::receivers::html_canvas_element)]
struct HtmlCanvasElementPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        enumerable,
        getter = element::html_canvas_width_getter_callback,
        setter = element::html_canvas_width_setter_callback
    )]
    width: (),

    #[webapi(
        accessor_property,
        enumerable,
        getter = element::html_canvas_height_getter_callback,
        setter = element::html_canvas_height_setter_callback
    )]
    height: (),
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, strum::EnumString, strum::IntoStaticStr, webidl::WebIdlEnum,
)]
#[webidl(name = "OffscreenRenderingContextType", parse_with = Self::parse)]
#[strum(serialize_all = "lowercase")]
pub(crate) enum CanvasContextKind {
    #[strum(serialize = "2d")]
    TwoD,
    WebGl,
    #[strum(serialize = "webgl2")]
    WebGl2,
    BitmapRenderer,
    WebGpu,
}

impl CanvasContextKind {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    pub(crate) fn label(self) -> &'static str {
        self.into()
    }
}

#[cfg(test)]
mod canvas_context_kind_tests {
    use super::CanvasContextKind;

    #[test]
    fn canvas_context_kind_parses_supported_context_ids() {
        assert_eq!(
            CanvasContextKind::parse("2d"),
            Some(CanvasContextKind::TwoD)
        );
        assert_eq!(
            CanvasContextKind::parse("webgl"),
            Some(CanvasContextKind::WebGl)
        );
        assert_eq!(
            CanvasContextKind::parse("webgl2"),
            Some(CanvasContextKind::WebGl2)
        );
        assert_eq!(CanvasContextKind::parse("WebGL"), None);
        assert_eq!(
            CanvasContextKind::parse("bitmaprenderer"),
            Some(CanvasContextKind::BitmapRenderer)
        );
    }
}

mod backing_store;
mod constructors;
mod context2d;
mod helpers;
mod image_bitmap;
mod objects;
mod offscreen;
mod path;
mod state;
mod transform;
mod webgl;
mod webgl_creation;

pub(crate) use backing_store::{
    attach_canvas_like_context_object, canvas_like_to_data_url,
    reset_html_canvas_backing_store_for_dimension_assignment,
};
pub(crate) use constructors::{
    canvas_rendering_context_2d_constructor_callback, offscreen_canvas_constructor_callback,
    offscreen_canvas_rendering_context_2d_constructor_callback,
};
pub(crate) use context2d::{
    canvas_context_arc_callback, canvas_context_arc_to_callback,
    canvas_context_begin_path_callback, canvas_context_bezier_curve_to_callback,
    canvas_context_clear_rect_callback, canvas_context_close_path_callback,
    canvas_context_create_image_data_callback, canvas_context_create_linear_gradient_callback,
    canvas_context_draw_image_callback, canvas_context_ellipse_callback,
    canvas_context_fill_callback, canvas_context_fill_rect_callback,
    canvas_context_fill_style_getter_callback, canvas_context_fill_style_setter_callback,
    canvas_context_fill_text_callback, canvas_context_font_getter_callback,
    canvas_context_font_setter_callback, canvas_context_get_image_data_callback,
    canvas_context_get_line_dash_callback, canvas_context_global_alpha_getter_callback,
    canvas_context_global_alpha_setter_callback,
    canvas_context_global_composite_operation_getter_callback,
    canvas_context_global_composite_operation_setter_callback,
    canvas_context_image_smoothing_enabled_getter_callback,
    canvas_context_image_smoothing_enabled_setter_callback,
    canvas_context_image_smoothing_quality_getter_callback,
    canvas_context_image_smoothing_quality_setter_callback,
    canvas_context_is_point_in_path_callback, canvas_context_line_cap_getter_callback,
    canvas_context_line_cap_setter_callback, canvas_context_line_dash_offset_getter_callback,
    canvas_context_line_dash_offset_setter_callback, canvas_context_line_join_getter_callback,
    canvas_context_line_join_setter_callback, canvas_context_line_to_callback,
    canvas_context_line_width_getter_callback, canvas_context_line_width_setter_callback,
    canvas_context_measure_text_callback, canvas_context_miter_limit_getter_callback,
    canvas_context_miter_limit_setter_callback, canvas_context_move_to_callback,
    canvas_context_noop_callback, canvas_context_put_image_data_callback,
    canvas_context_quadratic_curve_to_callback, canvas_context_rect_callback,
    canvas_context_reset_transform_callback, canvas_context_rotate_callback,
    canvas_context_scale_callback, canvas_context_set_line_dash_callback,
    canvas_context_set_transform_callback, canvas_context_stroke_callback,
    canvas_context_stroke_rect_callback, canvas_context_stroke_style_getter_callback,
    canvas_context_stroke_style_setter_callback, canvas_context_stroke_text_callback,
    canvas_context_transform_callback, canvas_context_translate_callback,
    canvas_gradient_add_color_stop_callback,
};
pub(crate) use image_bitmap::window_create_image_bitmap_callback;
pub(crate) use objects::{build_canvas_rendering_context_2d_object, build_offscreen_canvas_object};
pub(crate) use offscreen::{
    offscreen_canvas_convert_to_blob_callback, offscreen_canvas_get_context_callback,
};
pub(crate) use webgl::{WEBGL_CONSTANTS, WEBGL2_CONSTANTS, webgl_unavailable_receiver_callback};
pub(crate) use webgl_creation::{
    WEBGL_BACKEND_UNAVAILABLE, WEBGL_CONTEXT_TYPE_CONFLICT, dispatch_webgl_creation_error,
    webgl_context_event_constructor_callback,
};

pub(super) fn install_canvas_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    match interface_name {
        "HTMLCanvasElement" => {
            let prototype = template.prototype_template(scope);
            HtmlCanvasElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "OffscreenCanvas" => {
            offscreen::install_offscreen_canvas_template_bindings(scope, template);
        }
        "WebGLContextEvent" => {
            webgl_creation::install_webgl_context_event_template(scope, template);
        }
        "ImageBitmap" => {
            image_bitmap::install_image_bitmap_template_bindings(scope, template);
        }
        "TextMetrics" => {
            context2d::install_text_metrics_template_bindings(scope, template);
        }
        _ => {}
    }
}
