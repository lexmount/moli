pub(crate) use moli_webapi::canvas_context_values::CanvasContextKind;

use super::super::blob::build_blob_object;
use super::super::native_bridge::element;
use super::super::util::{throw_type_error, v8_string};
use super::shared::global_constructor_object;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

pub(crate) use moli_webapi::canvas_context_values::{
    CANVAS_CONTEXT_FILL_STYLE_SLOT, CANVAS_CONTEXT_FONT_SLOT, CANVAS_CONTEXT_GLOBAL_ALPHA_SLOT,
    CANVAS_CONTEXT_GLOBAL_COMPOSITE_OPERATION_SLOT, CANVAS_CONTEXT_IMAGE_SMOOTHING_ENABLED_SLOT,
    CANVAS_CONTEXT_IMAGE_SMOOTHING_QUALITY_SLOT, CANVAS_CONTEXT_LINE_CAP_SLOT,
    CANVAS_CONTEXT_LINE_DASH_OFFSET_SLOT, CANVAS_CONTEXT_LINE_JOIN_SLOT,
    CANVAS_CONTEXT_LINE_WIDTH_SLOT, CANVAS_CONTEXT_MITER_LIMIT_SLOT,
    CANVAS_CONTEXT_STROKE_STYLE_SLOT, DEFAULT_GLOBAL_ALPHA, DEFAULT_GLOBAL_COMPOSITE_OPERATION,
    DEFAULT_LINE_CAP, DEFAULT_LINE_DASH_OFFSET, DEFAULT_LINE_JOIN, DEFAULT_LINE_WIDTH,
    DEFAULT_MITER_LIMIT, DEFAULT_STROKE_STYLE, OFFSCREEN_CANVAS_HEIGHT_SLOT,
    OFFSCREEN_CANVAS_WIDTH_SLOT,
};

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
#[webapi(interface = web_api_interfaces::HTMLCanvasElement)]
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

mod backing_store;
mod bitmap_renderer;
pub(crate) use bitmap_renderer::build_bitmap_renderer_context;
mod blob_serialization;
pub(crate) use blob_serialization::{
    CanvasBlobCallbackTask, CanvasBlobCallbackTaskEffect, CanvasBlobEncodeJob,
    canvas_to_blob_callback,
};
mod constructors;
mod context2d;
mod helpers;
mod image_bitmap;
mod objects;
mod offscreen;
mod path;
mod path2d;
mod state;
mod transform;
use moli_webapi::webgl;

pub(crate) use path2d::{install_canvas_path_bindings, path2d_constructor_callback};
pub(crate) use transform::install_canvas_transform_bindings;

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
    canvas_context_close_path_callback, canvas_context_create_image_data_callback,
    canvas_context_create_linear_gradient_callback, canvas_context_draw_image_callback,
    canvas_context_ellipse_callback, canvas_context_fill_callback,
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
    install_canvas_rect_bindings, is_canvas_context_receiver,
};
pub(crate) use image_bitmap::{
    BitmapRejection, BitmapTaskResult, settle_bitmap_task_result,
    window_create_image_bitmap_callback,
};
pub(crate) use objects::{build_canvas_rendering_context_2d_object, build_offscreen_canvas_object};
pub(crate) use offscreen::{
    offscreen_canvas_convert_to_blob_callback, offscreen_canvas_get_context_callback,
    offscreen_canvas_transfer_to_image_bitmap_callback,
};
pub(crate) use webgl::{
    build_webgl_context_object, build_webgl2_context_object,
    webgl_debug_renderer_info_constructor_callback, webgl_lose_context_constructor_callback,
    webgl_rendering_context_constructor_callback,
};

pub(super) fn install_canvas_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    webgl::install_template_bindings(scope, template, interface_name);
    match interface_name {
        "CanvasGradient" => moli_webapi::canvas_gradient::install(scope, template),
        "HTMLCanvasElement" => {
            let prototype = template.prototype_template(scope);
            HtmlCanvasElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "OffscreenCanvas" => {
            offscreen::install_offscreen_canvas_template_bindings(scope, template);
        }
        "ImageBitmap" => {
            image_bitmap::install_image_bitmap_template_bindings(scope, template);
        }
        "ImageBitmapRenderingContext" => bitmap_renderer::install(scope, template),
        "TextMetrics" => {
            moli_webapi::text_metrics::install_text_metrics_template_bindings(scope, template);
        }
        _ => {}
    }
}
