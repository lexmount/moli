use super::canvas::{
    canvas_context_begin_path_callback, canvas_context_create_linear_gradient_callback,
    canvas_context_fill_text_callback, canvas_context_font_getter_callback,
    canvas_context_font_setter_callback, canvas_context_get_line_dash_callback,
    canvas_context_global_alpha_getter_callback, canvas_context_global_alpha_setter_callback,
    canvas_context_global_composite_operation_getter_callback,
    canvas_context_global_composite_operation_setter_callback,
    canvas_context_image_smoothing_enabled_getter_callback,
    canvas_context_image_smoothing_enabled_setter_callback,
    canvas_context_image_smoothing_quality_getter_callback,
    canvas_context_image_smoothing_quality_setter_callback,
    canvas_context_is_point_in_path_callback, canvas_context_line_cap_getter_callback,
    canvas_context_line_cap_setter_callback, canvas_context_line_dash_offset_getter_callback,
    canvas_context_line_dash_offset_setter_callback, canvas_context_line_join_getter_callback,
    canvas_context_line_join_setter_callback, canvas_context_line_width_getter_callback,
    canvas_context_line_width_setter_callback, canvas_context_measure_text_callback,
    canvas_context_miter_limit_getter_callback, canvas_context_miter_limit_setter_callback,
    canvas_context_noop_callback,
    canvas_context_reset_transform_callback, canvas_context_rotate_callback,
    canvas_context_scale_callback, canvas_context_set_line_dash_callback,
    canvas_context_set_transform_callback,
    canvas_context_stroke_rect_callback,
     canvas_context_stroke_text_callback,
    canvas_context_transform_callback, canvas_context_translate_callback,
    install_canvas_draw_image_bindings, install_canvas_pixel_bindings, install_canvas_drawing_state_bindings, install_canvas_path_bindings, install_canvas_rect_bindings, install_canvas_style_state_bindings, install_canvas_template_bindings,
    install_canvas_transform_bindings, offscreen_canvas_convert_to_blob_callback,
    offscreen_canvas_get_context_callback,
};
use super::file_api::{
    file_reader_abort_callback, file_reader_read_as_array_buffer_callback,
    file_reader_read_as_binary_string_callback, file_reader_read_as_data_url_callback,
    file_reader_read_as_text_callback,
};
use crate::web_api_interfaces;
use crate::xml_serializer;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::OffscreenCanvas, enumerable)]
struct OffscreenCanvasTemplateMethodsDeclaration {
    #[webapi(method, length = 1, callback = offscreen_canvas_get_context_callback)]
    get_context: (),

    #[webapi(method, length = 0, returns_promise, callback = offscreen_canvas_convert_to_blob_callback)]
    convert_to_blob: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::FileReader, enumerable)]
struct FileReaderTemplateMethodsDeclaration {
    #[webapi(method, length = 1, callback = file_reader_read_as_text_callback)]
    read_as_text: (),

    #[webapi(method = "readAsDataURL", length = 1, callback = file_reader_read_as_data_url_callback)]
    read_as_data_url: (),

    #[webapi(
        method,
        length = 1,
        callback = file_reader_read_as_array_buffer_callback
    )]
    read_as_array_buffer: (),

    #[webapi(
        method,
        length = 1,
        callback = file_reader_read_as_binary_string_callback
    )]
    read_as_binary_string: (),

    #[webapi(method, length = 0, callback = file_reader_abort_callback)]
    abort: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::XMLSerializer, enumerable)]
struct XmlSerializerTemplateMethodsDeclaration {
    #[webapi(
        method,
        length = 1,
        callback = xml_serializer::xml_serializer_serialize_to_string_callback
    )]
    serialize_to_string: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CanvasRenderingContext2D, enumerable, receiver = super::canvas::is_canvas_context_receiver)]
struct CanvasRenderingContext2dTemplateDeclaration {
    #[webapi(
        accessor_property = "font",
        getter = canvas_context_font_getter_callback,
        setter = canvas_context_font_setter_callback
    )]
    font: (),

    #[webapi(
        accessor_property = "imageSmoothingEnabled",
        getter = canvas_context_image_smoothing_enabled_getter_callback,
        setter = canvas_context_image_smoothing_enabled_setter_callback
    )]
    image_smoothing_enabled: (),

    #[webapi(
        accessor_property = "imageSmoothingQuality",
        getter = canvas_context_image_smoothing_quality_getter_callback,
        setter = canvas_context_image_smoothing_quality_setter_callback
    )]
    image_smoothing_quality: (),

    #[webapi(
        accessor_property = "globalAlpha",
        getter = canvas_context_global_alpha_getter_callback,
        setter = canvas_context_global_alpha_setter_callback
    )]
    global_alpha: (),

    #[webapi(
        accessor_property = "globalCompositeOperation",
        getter = canvas_context_global_composite_operation_getter_callback,
        setter = canvas_context_global_composite_operation_setter_callback
    )]
    global_composite_operation: (),

    #[webapi(
        accessor_property = "lineWidth",
        getter = canvas_context_line_width_getter_callback,
        setter = canvas_context_line_width_setter_callback
    )]
    line_width: (),

    #[webapi(
        accessor_property = "lineCap",
        getter = canvas_context_line_cap_getter_callback,
        setter = canvas_context_line_cap_setter_callback
    )]
    line_cap: (),

    #[webapi(
        accessor_property = "lineJoin",
        getter = canvas_context_line_join_getter_callback,
        setter = canvas_context_line_join_setter_callback
    )]
    line_join: (),

    #[webapi(
        accessor_property = "miterLimit",
        getter = canvas_context_miter_limit_getter_callback,
        setter = canvas_context_miter_limit_setter_callback
    )]
    miter_limit: (),

    #[webapi(
        accessor_property = "lineDashOffset",
        getter = canvas_context_line_dash_offset_getter_callback,
        setter = canvas_context_line_dash_offset_setter_callback
    )]
    line_dash_offset: (),

    #[webapi(
        method = "setLineDash",
        length = 1,
        callback = canvas_context_set_line_dash_callback
    )]
    set_line_dash: (),

    #[webapi(
        method = "getLineDash",
        length = 0,
        callback = canvas_context_get_line_dash_callback
    )]
    get_line_dash: (),

    #[webapi(
        method = "strokeRect",
        length = 4,
        callback = canvas_context_stroke_rect_callback
    )]
    stroke_rect: (),

    #[webapi(method = "fillText", length = 3, callback = canvas_context_fill_text_callback)]
    fill_text: (),

    #[webapi(
        method = "strokeText",
        length = 3,
        callback = canvas_context_stroke_text_callback
    )]
    stroke_text: (),

    #[webapi(method = "scale", length = 2, callback = canvas_context_scale_callback)]
    scale: (),

    #[webapi(method = "translate", length = 2, callback = canvas_context_translate_callback)]
    translate: (),

    #[webapi(method = "rotate", length = 1, callback = canvas_context_rotate_callback)]
    rotate: (),

    #[webapi(method = "transform", length = 6, callback = canvas_context_transform_callback)]
    transform: (),

    #[webapi(
        method = "setTransform",
        length = 0,
        callback = canvas_context_set_transform_callback
    )]
    set_transform: (),

    #[webapi(
        method = "resetTransform",
        length = 0,
        callback = canvas_context_reset_transform_callback
    )]
    reset_transform: (),

    #[webapi(method = "beginPath", length = 0, callback = canvas_context_begin_path_callback)]
    begin_path: (),

    #[webapi(method = "clip", length = 0, callback = canvas_context_noop_callback)]
    clip: (),

    #[webapi(
        method = "isPointInPath",
        length = 2,
        callback = canvas_context_is_point_in_path_callback
    )]
    is_point_in_path: (),

    #[webapi(
        method = "isPointInStroke",
        length = 2,
        callback = canvas_context_is_point_in_path_callback
    )]
    is_point_in_stroke: (),

    #[webapi(method = "measureText", length = 1, callback = canvas_context_measure_text_callback)]
    measure_text: (),

    #[webapi(
        method = "createLinearGradient",
        length = 4,
        callback = canvas_context_create_linear_gradient_callback
    )]
    create_linear_gradient: (),
}

pub(super) fn install_media_file_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    spec_name: &str,
) {
    install_canvas_template_bindings(scope, template, spec_name);
    match spec_name {
        "Blob" => {
            let proto = template.prototype_template(scope);
            moli_webapi::blob::install_blob_methods(scope, proto);
        }
        "ImageData" => {}
        "OffscreenCanvas" => {
            let proto = template.prototype_template(scope);
            OffscreenCanvasTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "CanvasRenderingContext2D" | "OffscreenCanvasRenderingContext2D" => {
            let proto = template.prototype_template(scope);
            CanvasRenderingContext2dTemplateDeclaration::initialize_prototype_template(
                scope, proto,
            );
            install_canvas_transform_bindings(scope, proto, spec_name);
            install_canvas_drawing_state_bindings(scope, proto, spec_name);
            install_canvas_style_state_bindings(scope, proto, spec_name);
            install_canvas_pixel_bindings(scope, proto, spec_name);
            install_canvas_rect_bindings(scope, proto, spec_name);
            install_canvas_path_bindings(scope, proto, spec_name);
            install_canvas_draw_image_bindings(scope, proto, spec_name);
        }
        "Path2D" => {
            install_canvas_path_bindings(scope, template.prototype_template(scope), spec_name);
        }
        "FileReader" => {
            let proto = template.prototype_template(scope);
            FileReaderTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "XMLSerializer" => {
            let proto = template.prototype_template(scope);
            XmlSerializerTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        _ => {}
    }
}
