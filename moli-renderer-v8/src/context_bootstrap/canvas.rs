pub(crate) use moli_webapi::canvas_context_values::CanvasContextKind;

use super::super::native_bridge::element;
use super::super::util::{throw_type_error, v8_string};
use super::shared::global_constructor_object;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

pub(crate) use moli_webapi::canvas_context_values::{
    CANVAS_CONTEXT_SHADOW_BLUR_SLOT, CANVAS_CONTEXT_SHADOW_COLOR_SLOT, CANVAS_CONTEXT_SHADOW_OFFSET_X_SLOT, CANVAS_CONTEXT_SHADOW_OFFSET_Y_SLOT, CANVAS_CONTEXT_TEXT_ALIGN_SLOT, CANVAS_CONTEXT_TEXT_BASELINE_SLOT,
    CANVAS_CONTEXT_FILL_STYLE_SLOT, CANVAS_CONTEXT_FONT_SLOT, CANVAS_CONTEXT_GLOBAL_ALPHA_SLOT,
    CANVAS_CONTEXT_GLOBAL_COMPOSITE_OPERATION_SLOT, CANVAS_CONTEXT_IMAGE_SMOOTHING_ENABLED_SLOT,
    CANVAS_CONTEXT_IMAGE_SMOOTHING_QUALITY_SLOT, CANVAS_CONTEXT_LINE_CAP_SLOT,
    CANVAS_CONTEXT_LINE_DASH_OFFSET_SLOT, CANVAS_CONTEXT_LINE_DASH_SLOT, CANVAS_CONTEXT_LINE_JOIN_SLOT,
    CANVAS_CONTEXT_LINE_WIDTH_SLOT, CANVAS_CONTEXT_MITER_LIMIT_SLOT,
    CANVAS_CONTEXT_STROKE_STYLE_SLOT, DEFAULT_GLOBAL_ALPHA, DEFAULT_GLOBAL_COMPOSITE_OPERATION,
    DEFAULT_LINE_CAP, DEFAULT_LINE_DASH_OFFSET, DEFAULT_LINE_JOIN, DEFAULT_LINE_WIDTH,
    DEFAULT_MITER_LIMIT, OFFSCREEN_CANVAS_HEIGHT_SLOT,
    OFFSCREEN_CANVAS_WIDTH_SLOT,
};

pub(crate) fn canvas_composite_operation_canonical(value: &str) -> Option<&'static str> {
    moli_canvas::CompositeOperation::parse(value).map(moli_canvas::CompositeOperation::as_str)
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
pub(super) mod pixels;
mod settings;
pub(crate) use settings::{Canvas2dSettings, parse_canvas_2d_settings};
mod style_state;
pub(crate) use style_state::install_canvas_style_state_bindings;
mod bitmap_renderer;
pub(crate) use bitmap_renderer::build_bitmap_renderer_context;
mod blob_serialization;
pub(crate) use blob_serialization::{
    CanvasBlobCompletion, CanvasBlobEncodeJob, CanvasBlobFile, CanvasBlobPromise, CanvasBlobTask,
    CanvasBlobTaskEffect, canvas_to_blob_callback,
};
mod clipping;
mod gradient;
pub(crate) use gradient::install_canvas_gradient_bindings;
mod pattern;
pub(crate) use pattern::install_canvas_pattern_bindings;
mod constructors;
mod context2d;
mod drawing;
mod drawing_state;
mod helpers;
mod image_bitmap;
mod image_paint;
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
    canvas_context_close_path_callback, canvas_context_ellipse_callback,
    canvas_context_fill_callback, canvas_context_fill_text_callback,
    canvas_context_font_getter_callback, canvas_context_font_setter_callback,
    canvas_context_get_line_dash_callback, canvas_context_global_alpha_getter_callback,
    canvas_context_global_alpha_setter_callback,
    canvas_context_global_composite_operation_getter_callback,
    canvas_context_global_composite_operation_setter_callback,
    canvas_context_image_smoothing_enabled_getter_callback,
    canvas_context_image_smoothing_enabled_setter_callback,
    canvas_context_image_smoothing_quality_getter_callback,
    canvas_context_image_smoothing_quality_setter_callback,
    canvas_context_line_cap_getter_callback, canvas_context_line_cap_setter_callback,
    canvas_context_line_dash_offset_getter_callback,
    canvas_context_line_dash_offset_setter_callback, canvas_context_line_join_getter_callback,
    canvas_context_line_join_setter_callback, canvas_context_line_to_callback,
    canvas_context_line_width_getter_callback, canvas_context_line_width_setter_callback,
    canvas_context_measure_text_callback, canvas_context_miter_limit_getter_callback,
    canvas_context_miter_limit_setter_callback, canvas_context_move_to_callback,
    canvas_context_quadratic_curve_to_callback, canvas_context_rect_callback,
    canvas_context_reset_transform_callback, canvas_context_rotate_callback,
    canvas_context_scale_callback, canvas_context_set_line_dash_callback,
    canvas_context_set_transform_callback, canvas_context_stroke_callback,
    canvas_context_stroke_rect_callback,
     canvas_context_stroke_text_callback,
    canvas_context_transform_callback, canvas_context_translate_callback,
    install_canvas_draw_image_bindings, install_canvas_pixel_bindings,
    install_canvas_rect_bindings, is_canvas_context_receiver,
};
pub(crate) use drawing_state::install_canvas_drawing_state_bindings;
pub(crate) use image_bitmap::{
    BitmapRejection, BitmapTaskResult, ImageBitmapClonePayload,
    build_image_bitmap_from_clone_payload, create_image_bitmap_callback, detach_image_bitmap,
    image_bitmap_clone_payload_from_object, settle_bitmap_task_result,
    window_create_image_bitmap_callback,
};
pub(crate) use objects::{build_canvas_rendering_context_2d_object, build_offscreen_canvas_object};
pub(crate) use offscreen::{
    offscreen_canvas_convert_to_blob_callback, offscreen_canvas_get_context_callback,
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

mod image_source;
pub(in crate::context_bootstrap) use image_source::{CanvasImageSource, image_source_pixels};
