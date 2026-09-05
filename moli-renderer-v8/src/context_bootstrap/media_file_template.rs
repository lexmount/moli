use super::canvas::{
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
    canvas_gradient_add_color_stop_callback, install_canvas_template_bindings,
    offscreen_canvas_convert_to_blob_callback, offscreen_canvas_get_context_callback,
    webgl_boolean_callback, webgl_check_framebuffer_status_callback, webgl_create_buffer_callback,
    webgl_create_framebuffer_callback, webgl_create_program_callback,
    webgl_create_renderbuffer_callback, webgl_create_shader_callback,
    webgl_get_attrib_location_callback, webgl_get_context_attributes_callback,
    webgl_get_extension_callback, webgl_get_parameter_callback, webgl_get_shader_info_log_callback,
    webgl_get_shader_precision_format_callback, webgl_get_supported_extensions_callback,
    webgl_is_context_lost_callback, webgl_lose_context_noop_callback, webgl_noop_callback,
    webgl_uniform_location_callback, webgl_zero_callback, webgl2_color_space_getter_callback,
    webgl2_color_space_setter_callback, webgl2_get_extension_callback,
    webgl2_get_internalformat_parameter_callback, webgl2_get_parameter_callback,
    webgl2_get_supported_extensions_callback,
};
use super::file_api::{
    file_list_item_callback, file_reader_abort_callback, file_reader_add_event_listener_callback,
    file_reader_read_as_array_buffer_callback, file_reader_read_as_binary_string_callback,
    file_reader_read_as_data_url_callback, file_reader_read_as_text_callback,
    file_reader_remove_event_listener_callback,
};
use crate::{blob, util::callback_data_index_value, xml_serializer};
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Blob", enumerable)]
struct BlobTemplateMethodsDeclaration {
    #[webapi(method, length = 0, callback = blob::blob_text_callback)]
    text: (),

    #[webapi(method, length = 0, callback = blob::blob_array_buffer_callback)]
    array_buffer: (),

    #[webapi(method, length = 0, callback = blob::blob_bytes_callback)]
    bytes: (),

    #[webapi(method, length = 0, callback = blob::blob_stream_callback)]
    stream: (),

    #[webapi(method, length = 0, callback = blob::blob_slice_callback)]
    slice: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "OffscreenCanvas", enumerable)]
struct OffscreenCanvasTemplateMethodsDeclaration {
    #[webapi(method, length = 1, callback = offscreen_canvas_get_context_callback)]
    get_context: (),

    #[webapi(method, length = 0, callback = offscreen_canvas_convert_to_blob_callback)]
    convert_to_blob: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FileList", enumerable)]
struct FileListTemplateMethodsDeclaration {
    #[webapi(method, length = 1, callback = file_list_item_callback)]
    item: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator",
    )]
    iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FileReader", enumerable)]
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

    #[webapi(method, length = 2, callback = file_reader_add_event_listener_callback)]
    add_event_listener: (),

    #[webapi(
        method,
        length = 2,
        callback = file_reader_remove_event_listener_callback
    )]
    remove_event_listener: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "XMLSerializer", enumerable)]
struct XmlSerializerTemplateMethodsDeclaration {
    #[webapi(
        method,
        length = 1,
        callback = xml_serializer::xml_serializer_serialize_to_string_callback
    )]
    serialize_to_string: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "CanvasRenderingContext2D", enumerable)]
struct CanvasRenderingContext2dTemplateDeclaration {
    #[webapi(
        accessor_property = "fillStyle",
        getter = canvas_context_fill_style_getter_callback,
        setter = canvas_context_fill_style_setter_callback
    )]
    fill_style: (),

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
        accessor_property = "strokeStyle",
        getter = canvas_context_stroke_style_getter_callback,
        setter = canvas_context_stroke_style_setter_callback
    )]
    stroke_style: (),

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

    #[webapi(method = "fillRect", length = 4, callback = canvas_context_fill_rect_callback)]
    fill_rect: (),

    #[webapi(
        method = "clearRect",
        length = 4,
        callback = canvas_context_clear_rect_callback
    )]
    clear_rect: (),

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

    #[webapi(method = "save", length = 0, callback = canvas_context_noop_callback)]
    save: (),

    #[webapi(method = "restore", length = 0, callback = canvas_context_noop_callback)]
    restore: (),

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

    #[webapi(method = "closePath", length = 0, callback = canvas_context_close_path_callback)]
    close_path: (),

    #[webapi(method = "moveTo", length = 2, callback = canvas_context_move_to_callback)]
    move_to: (),

    #[webapi(method = "lineTo", length = 2, callback = canvas_context_line_to_callback)]
    line_to: (),

    #[webapi(
        method = "quadraticCurveTo",
        length = 4,
        callback = canvas_context_quadratic_curve_to_callback
    )]
    quadratic_curve_to: (),

    #[webapi(
        method = "bezierCurveTo",
        length = 6,
        callback = canvas_context_bezier_curve_to_callback
    )]
    bezier_curve_to: (),

    #[webapi(method = "arcTo", length = 5, callback = canvas_context_arc_to_callback)]
    arc_to: (),

    #[webapi(method = "arc", length = 5, callback = canvas_context_arc_callback)]
    arc: (),

    #[webapi(method = "ellipse", length = 7, callback = canvas_context_ellipse_callback)]
    ellipse: (),

    #[webapi(method = "fill", length = 1, callback = canvas_context_fill_callback)]
    fill: (),

    #[webapi(method = "stroke", length = 0, callback = canvas_context_stroke_callback)]
    stroke: (),

    #[webapi(method = "clip", length = 0, callback = canvas_context_noop_callback)]
    clip: (),

    #[webapi(method = "rect", length = 4, callback = canvas_context_rect_callback)]
    rect: (),

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

    #[webapi(method = "drawImage", length = 3, callback = canvas_context_draw_image_callback)]
    draw_image: (),

    #[webapi(method = "measureText", length = 1, callback = canvas_context_measure_text_callback)]
    measure_text: (),

    #[webapi(
        method = "createLinearGradient",
        length = 4,
        callback = canvas_context_create_linear_gradient_callback
    )]
    create_linear_gradient: (),

    #[webapi(
        method = "createImageData",
        length = 2,
        callback = canvas_context_create_image_data_callback
    )]
    create_image_data: (),

    #[webapi(
        method = "putImageData",
        length = 3,
        callback = canvas_context_put_image_data_callback
    )]
    put_image_data: (),

    #[webapi(
        method = "getImageData",
        length = 4,
        callback = canvas_context_get_image_data_callback
    )]
    get_image_data: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "CanvasGradient", enumerable)]
struct CanvasGradientTemplateMethodsDeclaration {
    #[webapi(
        method = "addColorStop",
        length = 2,
        callback = canvas_gradient_add_color_stop_callback
    )]
    add_color_stop: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "WebGLRenderingContext", enumerable)]
struct WebGlRenderingContextTemplateMethodsDeclaration {
    #[webapi(method = "clearColor", length = 4, callback = webgl_noop_callback)]
    clear_color: (),

    #[webapi(method = "enable", length = 1, callback = webgl_noop_callback)]
    enable: (),

    #[webapi(method = "depthFunc", length = 1, callback = webgl_noop_callback)]
    depth_func: (),

    #[webapi(method = "clear", length = 1, callback = webgl_noop_callback)]
    clear: (),

    #[webapi(method = "createBuffer", length = 0, callback = webgl_create_buffer_callback)]
    create_buffer: (),

    #[webapi(method = "bindBuffer", length = 2, callback = webgl_noop_callback)]
    bind_buffer: (),

    #[webapi(method = "bufferData", length = 3, callback = webgl_noop_callback)]
    buffer_data: (),

    #[webapi(method = "createProgram", length = 0, callback = webgl_create_program_callback)]
    create_program: (),

    #[webapi(method = "deleteProgram", length = 1, callback = webgl_noop_callback)]
    delete_program: (),

    #[webapi(method = "createShader", length = 1, callback = webgl_create_shader_callback)]
    create_shader: (),

    #[webapi(method = "shaderSource", length = 2, callback = webgl_noop_callback)]
    shader_source: (),

    #[webapi(method = "compileShader", length = 1, callback = webgl_noop_callback)]
    compile_shader: (),

    #[webapi(method = "deleteShader", length = 1, callback = webgl_noop_callback)]
    delete_shader: (),

    #[webapi(method = "attachShader", length = 2, callback = webgl_noop_callback)]
    attach_shader: (),

    #[webapi(method = "linkProgram", length = 1, callback = webgl_noop_callback)]
    link_program: (),

    #[webapi(method = "useProgram", length = 1, callback = webgl_noop_callback)]
    use_program: (),

    #[webapi(
        method = "getAttribLocation",
        length = 2,
        callback = webgl_get_attrib_location_callback
    )]
    get_attrib_location: (),

    #[webapi(
        method = "getUniformLocation",
        length = 2,
        callback = webgl_uniform_location_callback
    )]
    get_uniform_location: (),

    #[webapi(
        method = "enableVertexAttribArray",
        length = 1,
        callback = webgl_noop_callback
    )]
    enable_vertex_attrib_array: (),

    #[webapi(
        method = "vertexAttribPointer",
        length = 6,
        callback = webgl_noop_callback
    )]
    vertex_attrib_pointer: (),

    #[webapi(method = "uniform2f", length = 3, callback = webgl_noop_callback)]
    uniform2f: (),

    #[webapi(method = "uniform1f", length = 2, callback = webgl_noop_callback)]
    uniform1f: (),

    #[webapi(method = "uniform2fv", length = 2, callback = webgl_noop_callback)]
    uniform2fv: (),

    #[webapi(method = "drawArrays", length = 3, callback = webgl_noop_callback)]
    draw_arrays: (),

    #[webapi(method = "getError", length = 0, callback = webgl_zero_callback)]
    get_error: (),

    #[webapi(method = "getShaderParameter", length = 2, callback = webgl_boolean_callback)]
    get_shader_parameter: (),

    #[webapi(
        method = "getProgramParameter",
        length = 2,
        callback = webgl_boolean_callback
    )]
    get_program_parameter: (),

    #[webapi(
        method = "getShaderInfoLog",
        length = 1,
        callback = webgl_get_shader_info_log_callback
    )]
    get_shader_info_log: (),

    #[webapi(
        method = "getProgramInfoLog",
        length = 1,
        callback = webgl_get_shader_info_log_callback
    )]
    get_program_info_log: (),

    #[webapi(
        method = "getSupportedExtensions",
        length = 0,
        callback = webgl_get_supported_extensions_callback
    )]
    get_supported_extensions: (),

    #[webapi(method = "getExtension", length = 1, callback = webgl_get_extension_callback)]
    get_extension: (),

    #[webapi(method = "getParameter", length = 1, callback = webgl_get_parameter_callback)]
    get_parameter: (),

    #[webapi(
        method = "getContextAttributes",
        length = 0,
        callback = webgl_get_context_attributes_callback
    )]
    get_context_attributes: (),

    #[webapi(method = "isContextLost", length = 0, callback = webgl_is_context_lost_callback)]
    is_context_lost: (),

    #[webapi(
        method = "getShaderPrecisionFormat",
        length = 2,
        callback = webgl_get_shader_precision_format_callback
    )]
    get_shader_precision_format: (),

    #[webapi(method = "readPixels", length = 7, callback = webgl_noop_callback)]
    read_pixels: (),

    #[webapi(
        method = "createFramebuffer",
        length = 0,
        callback = webgl_create_framebuffer_callback
    )]
    create_framebuffer: (),

    #[webapi(method = "bindFramebuffer", length = 2, callback = webgl_noop_callback)]
    bind_framebuffer: (),

    #[webapi(
        method = "createRenderbuffer",
        length = 0,
        callback = webgl_create_renderbuffer_callback
    )]
    create_renderbuffer: (),

    #[webapi(method = "bindRenderbuffer", length = 2, callback = webgl_noop_callback)]
    bind_renderbuffer: (),

    #[webapi(method = "renderbufferStorage", length = 4, callback = webgl_noop_callback)]
    renderbuffer_storage: (),

    #[webapi(
        method = "framebufferRenderbuffer",
        length = 4,
        callback = webgl_noop_callback
    )]
    framebuffer_renderbuffer: (),

    #[webapi(
        method = "checkFramebufferStatus",
        length = 1,
        callback = webgl_check_framebuffer_status_callback
    )]
    check_framebuffer_status: (),

    #[webapi(method = "deleteRenderbuffer", length = 1, callback = webgl_noop_callback)]
    delete_renderbuffer: (),

    #[webapi(method = "deleteFramebuffer", length = 1, callback = webgl_noop_callback)]
    delete_framebuffer: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "WebGL2RenderingContext", enumerable)]
struct WebGl2RenderingContextTemplateDeclaration {
    #[webapi(
        method = "getSupportedExtensions",
        length = 0,
        callback = webgl2_get_supported_extensions_callback
    )]
    get_supported_extensions: (),

    #[webapi(method = "getExtension", length = 1, callback = webgl2_get_extension_callback)]
    get_extension: (),

    #[webapi(method = "getParameter", length = 1, callback = webgl2_get_parameter_callback)]
    get_parameter: (),

    #[webapi(
        method = "getInternalformatParameter",
        length = 3,
        callback = webgl2_get_internalformat_parameter_callback
    )]
    get_internalformat_parameter: (),

    #[webapi(
        accessor_property,
        enumerable,
        getter = webgl2_color_space_getter_callback,
        setter = webgl2_color_space_setter_callback,
        data = callback_data_index_value(scope, 0)
    )]
    drawing_buffer_color_space: (),

    #[webapi(
        accessor_property,
        enumerable,
        getter = webgl2_color_space_getter_callback,
        setter = webgl2_color_space_setter_callback,
        data = callback_data_index_value(scope, 1)
    )]
    unpack_color_space: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "WEBGL_lose_context", enumerable)]
struct WebGlLoseContextTemplateMethodsDeclaration {
    #[webapi(method = "loseContext", length = 0, callback = webgl_lose_context_noop_callback)]
    lose_context: (),

    #[webapi(method = "restoreContext", length = 0, callback = webgl_lose_context_noop_callback)]
    restore_context: (),
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
            BlobTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "ImageData" => {}
        "CanvasGradient" => {
            let proto = template.prototype_template(scope);
            CanvasGradientTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "OffscreenCanvas" => {
            let proto = template.prototype_template(scope);
            OffscreenCanvasTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "CanvasRenderingContext2D" | "OffscreenCanvasRenderingContext2D" => {
            let proto = template.prototype_template(scope);
            CanvasRenderingContext2dTemplateDeclaration::initialize_prototype_template(
                scope, proto,
            );
        }
        "WebGLRenderingContext" => {
            let proto = template.prototype_template(scope);
            WebGlRenderingContextTemplateMethodsDeclaration::initialize_prototype_template(
                scope, proto,
            );
        }
        "WebGL2RenderingContext" => {
            let proto = template.prototype_template(scope);
            WebGlRenderingContextTemplateMethodsDeclaration::initialize_prototype_template(
                scope, proto,
            );
            WebGl2RenderingContextTemplateDeclaration::initialize_prototype_template(scope, proto);
        }
        "WEBGL_lose_context" => {
            let proto = template.prototype_template(scope);
            WebGlLoseContextTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "FileList" => {
            let proto = template.prototype_template(scope);
            FileListTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
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
