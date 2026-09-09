use super::callbacks::*;
use super::text::*;
use super::*;
use crate::util::callback_data_index_value;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGLength, enumerable)]
struct SvgLengthTemplateMethodsDeclaration {
    #[webapi(constant = "SVG_LENGTHTYPE_UNKNOWN", value = SVG_LENGTH_TYPE_UNKNOWN)]
    length_type_unknown: (),

    #[webapi(constant = "SVG_LENGTHTYPE_NUMBER", value = SVG_LENGTH_TYPE_NUMBER)]
    length_type_number: (),

    #[webapi(
        constant = "SVG_LENGTHTYPE_PERCENTAGE",
        value = SVG_LENGTH_TYPE_PERCENTAGE
    )]
    length_type_percentage: (),

    #[webapi(constant = "SVG_LENGTHTYPE_EMS", value = SVG_LENGTH_TYPE_EMS)]
    length_type_ems: (),

    #[webapi(constant = "SVG_LENGTHTYPE_EXS", value = SVG_LENGTH_TYPE_EXS)]
    length_type_exs: (),

    #[webapi(constant = "SVG_LENGTHTYPE_PX", value = SVG_LENGTH_TYPE_PX)]
    length_type_px: (),

    #[webapi(constant = "SVG_LENGTHTYPE_CM", value = SVG_LENGTH_TYPE_CM)]
    length_type_cm: (),

    #[webapi(constant = "SVG_LENGTHTYPE_MM", value = SVG_LENGTH_TYPE_MM)]
    length_type_mm: (),

    #[webapi(constant = "SVG_LENGTHTYPE_IN", value = SVG_LENGTH_TYPE_IN)]
    length_type_in: (),

    #[webapi(constant = "SVG_LENGTHTYPE_PT", value = SVG_LENGTH_TYPE_PT)]
    length_type_pt: (),

    #[webapi(constant = "SVG_LENGTHTYPE_PC", value = SVG_LENGTH_TYPE_PC)]
    length_type_pc: (),

    #[webapi(
        method = "newValueSpecifiedUnits",
        length = 2,
        callback = svg_length_new_value_specified_units_callback
    )]
    new_value_specified_units: (),

    #[webapi(
        method = "convertToSpecifiedUnits",
        length = 1,
        callback = svg_length_convert_to_specified_units_callback
    )]
    convert_to_specified_units: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGLengthList, enumerable)]
struct SvgLengthListTemplateMethodsDeclaration {
    #[webapi(method = "clear", length = 0, callback = svg_length_list_clear_callback)]
    clear: (),

    #[webapi(
        method = "initialize",
        length = 1,
        callback = svg_length_list_initialize_callback
    )]
    initialize: (),

    #[webapi(method = "getItem", length = 1, callback = svg_length_list_get_item_callback)]
    get_item: (),

    #[webapi(
        method = "insertItemBefore",
        length = 2,
        callback = svg_length_list_insert_item_before_callback
    )]
    insert_item_before: (),

    #[webapi(
        method = "replaceItem",
        length = 2,
        callback = svg_length_list_replace_item_callback
    )]
    replace_item: (),

    #[webapi(
        method = "removeItem",
        length = 1,
        callback = svg_length_list_remove_item_callback
    )]
    remove_item: (),

    #[webapi(
        method = "appendItem",
        length = 1,
        callback = svg_length_list_append_item_callback
    )]
    append_item: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGNumberList, enumerable)]
struct SvgNumberListTemplateMethodsDeclaration {
    #[webapi(method = "clear", length = 0, callback = svg_number_list_clear_callback)]
    clear: (),

    #[webapi(
        method = "initialize",
        length = 1,
        callback = svg_number_list_initialize_callback
    )]
    initialize: (),

    #[webapi(method = "getItem", length = 1, callback = svg_number_list_get_item_callback)]
    get_item: (),

    #[webapi(
        method = "insertItemBefore",
        length = 2,
        callback = svg_number_list_insert_item_before_callback
    )]
    insert_item_before: (),

    #[webapi(
        method = "replaceItem",
        length = 2,
        callback = svg_number_list_replace_item_callback
    )]
    replace_item: (),

    #[webapi(
        method = "removeItem",
        length = 1,
        callback = svg_number_list_remove_item_callback
    )]
    remove_item: (),

    #[webapi(
        method = "appendItem",
        length = 1,
        callback = svg_number_list_append_item_callback
    )]
    append_item: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGTransformList, enumerable)]
struct SvgTransformListTemplateMethodsDeclaration {
    #[webapi(method = "clear", length = 0, callback = svg_transform_list_clear_callback)]
    clear: (),

    #[webapi(
        method = "initialize",
        length = 1,
        callback = svg_transform_list_initialize_callback
    )]
    initialize: (),

    #[webapi(method = "getItem", length = 1, callback = svg_transform_list_get_item_callback)]
    get_item: (),

    #[webapi(
        method = "insertItemBefore",
        length = 2,
        callback = svg_transform_list_insert_item_before_callback
    )]
    insert_item_before: (),

    #[webapi(
        method = "replaceItem",
        length = 2,
        callback = svg_transform_list_replace_item_callback
    )]
    replace_item: (),

    #[webapi(
        method = "removeItem",
        length = 1,
        callback = svg_transform_list_remove_item_callback
    )]
    remove_item: (),

    #[webapi(
        method = "appendItem",
        length = 1,
        callback = svg_transform_list_append_item_callback
    )]
    append_item: (),

    #[webapi(
        method = "createSVGTransformFromMatrix",
        length = 1,
        callback = svg_transform_list_create_transform_from_matrix_callback
    )]
    create_svg_transform_from_matrix: (),

    #[webapi(
        method = "consolidate",
        length = 0,
        callback = svg_transform_list_consolidate_callback
    )]
    consolidate: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGTransform, enumerable)]
struct SvgTransformTemplateMethodsDeclaration {
    #[webapi(
        constant = "SVG_TRANSFORM_UNKNOWN",
        value = SVG_TRANSFORM_TYPE_UNKNOWN
    )]
    transform_unknown: (),

    #[webapi(constant = "SVG_TRANSFORM_MATRIX", value = SVG_TRANSFORM_TYPE_MATRIX)]
    transform_matrix: (),

    #[webapi(
        constant = "SVG_TRANSFORM_TRANSLATE",
        value = SVG_TRANSFORM_TYPE_TRANSLATE
    )]
    transform_translate: (),

    #[webapi(constant = "SVG_TRANSFORM_SCALE", value = SVG_TRANSFORM_TYPE_SCALE)]
    transform_scale: (),

    #[webapi(constant = "SVG_TRANSFORM_ROTATE", value = SVG_TRANSFORM_TYPE_ROTATE)]
    transform_rotate: (),

    #[webapi(constant = "SVG_TRANSFORM_SKEWX", value = SVG_TRANSFORM_TYPE_SKEWX)]
    transform_skew_x: (),

    #[webapi(constant = "SVG_TRANSFORM_SKEWY", value = SVG_TRANSFORM_TYPE_SKEWY)]
    transform_skew_y: (),

    #[webapi(method = "setMatrix", length = 1, callback = svg_transform_set_matrix_callback)]
    set_matrix: (),

    #[webapi(
        method = "setTranslate",
        length = 2,
        callback = svg_transform_set_translate_callback
    )]
    set_translate: (),

    #[webapi(method = "setScale", length = 2, callback = svg_transform_set_scale_callback)]
    set_scale: (),

    #[webapi(method = "setRotate", length = 3, callback = svg_transform_set_rotate_callback)]
    set_rotate: (),

    #[webapi(method = "setSkewX", length = 1, callback = svg_transform_set_skew_x_callback)]
    set_skew_x: (),

    #[webapi(method = "setSkewY", length = 1, callback = svg_transform_set_skew_y_callback)]
    set_skew_y: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGMatrix, enumerable)]
struct SvgMatrixTemplateMethodsDeclaration {
    #[webapi(method = "multiply", length = 1, callback = svg_matrix_multiply_callback)]
    multiply: (),

    #[webapi(method = "inverse", length = 0, callback = svg_matrix_inverse_callback)]
    inverse: (),

    #[webapi(method = "translate", length = 2, callback = svg_matrix_translate_callback)]
    translate: (),

    #[webapi(method = "scale", length = 1, callback = svg_matrix_scale_callback)]
    scale: (),

    #[webapi(
        method = "scaleNonUniform",
        length = 2,
        callback = svg_matrix_scale_non_uniform_callback
    )]
    scale_non_uniform: (),

    #[webapi(method = "rotate", length = 1, callback = svg_matrix_rotate_callback)]
    rotate: (),

    #[webapi(
        method = "rotateFromVector",
        length = 2,
        callback = svg_matrix_rotate_from_vector_callback
    )]
    rotate_from_vector: (),

    #[webapi(method = "flipX", length = 0, callback = svg_matrix_flip_x_callback)]
    flip_x: (),

    #[webapi(method = "flipY", length = 0, callback = svg_matrix_flip_y_callback)]
    flip_y: (),

    #[webapi(method = "skewX", length = 1, callback = svg_matrix_skew_x_callback)]
    skew_x: (),

    #[webapi(method = "skewY", length = 1, callback = svg_matrix_skew_y_callback)]
    skew_y: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGGraphicsElement, enumerable)]
struct SvgGraphicsElementTemplateMethodsDeclaration {
    #[webapi(method = "getBBox", length = 0, callback = svg_graphics_get_bbox_callback)]
    get_bbox: (),

    #[webapi(method = "getCTM", length = 0, callback = svg_graphics_get_ctm_callback)]
    get_ctm: (),

    #[webapi(
        method = "getScreenCTM",
        length = 0,
        callback = svg_graphics_get_screen_ctm_callback
    )]
    get_screen_ctm: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGGeometryElement, enumerable)]
struct SvgGeometryElementTemplateMethodsDeclaration {
    #[webapi(
        method = "isPointInFill",
        length = 0,
        callback = svg_geometry_is_point_in_fill_callback
    )]
    is_point_in_fill: (),

    #[webapi(
        method = "isPointInStroke",
        length = 0,
        callback = svg_geometry_is_point_in_stroke_callback
    )]
    is_point_in_stroke: (),

    #[webapi(
        method = "getTotalLength",
        length = 0,
        callback = svg_geometry_get_total_length_callback
    )]
    get_total_length: (),

    #[webapi(
        method = "getPointAtLength",
        length = 1,
        callback = svg_geometry_get_point_at_length_callback
    )]
    get_point_at_length: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGTextContentElement, receiver = super::text::is_text_receiver, enumerable)]
struct SvgTextContentElementTemplateMethodsDeclaration {
    #[webapi(constant = "LENGTHADJUST_UNKNOWN", value = SVG_LENGTH_ADJUST_UNKNOWN)]
    length_adjust_unknown: (),

    #[webapi(constant = "LENGTHADJUST_SPACING", value = SVG_LENGTH_ADJUST_SPACING)]
    length_adjust_spacing: (),

    #[webapi(
        constant = "LENGTHADJUST_SPACINGANDGLYPHS",
        value = SVG_LENGTH_ADJUST_SPACING_AND_GLYPHS
    )]
    length_adjust_spacing_and_glyphs: (),

    #[webapi(
        method = "getNumberOfChars",
        length = 0,
        callback = svg_text_content_get_number_of_chars_callback
    )]
    get_number_of_chars: (),

    #[webapi(
        method = "getComputedTextLength",
        length = 0,
        callback = svg_text_content_get_computed_text_length_callback
    )]
    get_computed_text_length: (),

    #[webapi(
        method = "getSubStringLength",
        length = 2,
        callback = svg_text_content_get_substring_length_callback
    )]
    get_sub_string_length: (),

    #[webapi(
        method = "getStartPositionOfChar",
        length = 1,
        callback = svg_text_content_get_start_position_of_char_callback
    )]
    get_start_position_of_char: (),

    #[webapi(
        method = "getEndPositionOfChar",
        length = 1,
        callback = svg_text_content_get_end_position_of_char_callback
    )]
    get_end_position_of_char: (),

    #[webapi(
        method = "getExtentOfChar",
        length = 1,
        callback = svg_text_content_get_extent_of_char_callback
    )]
    get_extent_of_char: (),

    #[webapi(
        method = "getRotationOfChar",
        length = 1,
        callback = svg_text_content_get_rotation_of_char_callback
    )]
    get_rotation_of_char: (),

    #[webapi(
        method = "getCharNumAtPosition",
        length = 1,
        callback = svg_text_content_get_char_num_at_position_callback
    )]
    get_char_num_at_position: (),

    #[webapi(
        method = "selectSubString",
        length = 2,
        callback = svg_text_content_select_substring_callback
    )]
    select_sub_string: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGSVGElement, enumerable)]
struct SvgSvgElementTemplateMethodsDeclaration {
    #[webapi(method = "createSVGPoint", length = 0, callback = super::point::create,
        receiver = super::point::is_svg_root)]
    create_svg_point: (),

    #[webapi(
        method = "createSVGRect",
        length = 0,
        callback = super::rect::create_svg_rect
    )]
    create_svg_rect: (),

    #[webapi(
        method = "createSVGMatrix",
        length = 0,
        callback = svg_svg_element_create_matrix_callback
    )]
    create_svg_matrix: (),

    #[webapi(
        method = "createSVGTransform",
        length = 0,
        callback = svg_svg_element_create_transform_callback
    )]
    create_svg_transform: (),

    #[webapi(
        method = "createSVGTransformFromMatrix",
        length = 1,
        callback = svg_svg_element_create_transform_from_matrix_callback
    )]
    create_svg_transform_from_matrix: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGLength, enumerable)]
struct SvgLengthTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "unitType",
        getter = svg_length_getter,
        data = callback_data_index_value(scope, 0)
    )]
    unit_type: (),

    #[webapi(
        accessor_property = "value",
        getter = svg_length_getter,
        setter = svg_length_setter,
        data = callback_data_index_value(scope, 1)
    )]
    value: (),

    #[webapi(
        accessor_property = "valueInSpecifiedUnits",
        getter = svg_length_getter,
        setter = svg_length_setter,
        data = callback_data_index_value(scope, 2)
    )]
    value_in_specified_units: (),

    #[webapi(
        accessor_property = "valueAsString",
        getter = svg_length_getter,
        setter = svg_length_setter,
        data = callback_data_index_value(scope, 3)
    )]
    value_as_string: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGAnimatedLength, enumerable)]
struct SvgAnimatedLengthTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "baseVal",
        getter = svg_animated_length_getter,
        data = callback_data_index_value(scope, 0)
    )]
    base_val: (),

    #[webapi(
        accessor_property = "animVal",
        getter = svg_animated_length_getter,
        data = callback_data_index_value(scope, 1)
    )]
    anim_val: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGNumber, enumerable)]
struct SvgNumberTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "value",
        getter = svg_number_getter,
        setter = svg_number_setter
    )]
    value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGAnimatedLengthList, enumerable)]
struct SvgAnimatedLengthListTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "baseVal",
        getter = svg_animated_length_list_getter,
        data = callback_data_index_value(scope, 0)
    )]
    base_val: (),

    #[webapi(
        accessor_property = "animVal",
        getter = svg_animated_length_list_getter,
        data = callback_data_index_value(scope, 1)
    )]
    anim_val: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGAnimatedNumber, enumerable)]
struct SvgAnimatedNumberTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "baseVal",
        getter = svg_animated_number_getter,
        setter = svg_animated_number_setter,
        data = callback_data_index_value(scope, 0)
    )]
    base_val: (),

    #[webapi(
        accessor_property = "animVal",
        getter = svg_animated_number_getter,
        data = callback_data_index_value(scope, 1)
    )]
    anim_val: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGAnimatedNumberList, enumerable)]
struct SvgAnimatedNumberListTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "baseVal",
        getter = svg_animated_number_list_getter,
        data = callback_data_index_value(scope, 0)
    )]
    base_val: (),

    #[webapi(
        accessor_property = "animVal",
        getter = svg_animated_number_list_getter,
        data = callback_data_index_value(scope, 1)
    )]
    anim_val: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGAnimatedEnumeration, enumerable)]
struct SvgAnimatedEnumerationTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "baseVal",
        getter = svg_animated_enumeration_getter,
        setter = svg_animated_enumeration_setter,
        data = callback_data_index_value(scope, 0)
    )]
    base_val: (),

    #[webapi(
        accessor_property = "animVal",
        getter = svg_animated_enumeration_getter,
        data = callback_data_index_value(scope, 1)
    )]
    anim_val: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGLengthList, enumerable)]
struct SvgLengthListTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "length", getter = svg_length_list_length_getter)]
    length: (),

    #[webapi(
        accessor_property = "numberOfItems",
        getter = svg_length_list_length_getter
    )]
    number_of_items: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGNumberList, enumerable)]
struct SvgNumberListTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "length", getter = svg_number_list_length_getter)]
    length: (),

    #[webapi(
        accessor_property = "numberOfItems",
        getter = svg_number_list_length_getter
    )]
    number_of_items: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGAnimatedTransformList, enumerable)]
struct SvgAnimatedTransformListTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "baseVal",
        getter = svg_animated_transform_list_getter,
        data = callback_data_index_value(scope, 0)
    )]
    base_val: (),

    #[webapi(
        accessor_property = "animVal",
        getter = svg_animated_transform_list_getter,
        data = callback_data_index_value(scope, 1)
    )]
    anim_val: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGTransformList, enumerable)]
struct SvgTransformListTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "length", getter = svg_transform_list_length_getter)]
    length: (),

    #[webapi(
        accessor_property = "numberOfItems",
        getter = svg_transform_list_length_getter
    )]
    number_of_items: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGTransform, enumerable)]
struct SvgTransformTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "type",
        getter = svg_transform_getter,
        data = callback_data_index_value(scope, 0)
    )]
    type_: (),

    #[webapi(
        accessor_property = "matrix",
        getter = svg_transform_getter,
        data = callback_data_index_value(scope, 1)
    )]
    matrix: (),

    #[webapi(
        accessor_property = "angle",
        getter = svg_transform_getter,
        data = callback_data_index_value(scope, 2)
    )]
    angle: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGMatrix, enumerable)]
struct SvgMatrixTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "a",
        getter = svg_matrix_getter,
        setter = svg_matrix_setter,
        data = callback_data_index_value(scope, 0)
    )]
    a: (),

    #[webapi(
        accessor_property = "b",
        getter = svg_matrix_getter,
        setter = svg_matrix_setter,
        data = callback_data_index_value(scope, 1)
    )]
    b: (),

    #[webapi(
        accessor_property = "c",
        getter = svg_matrix_getter,
        setter = svg_matrix_setter,
        data = callback_data_index_value(scope, 2)
    )]
    c: (),

    #[webapi(
        accessor_property = "d",
        getter = svg_matrix_getter,
        setter = svg_matrix_setter,
        data = callback_data_index_value(scope, 3)
    )]
    d: (),

    #[webapi(
        accessor_property = "e",
        getter = svg_matrix_getter,
        setter = svg_matrix_setter,
        data = callback_data_index_value(scope, 4)
    )]
    e: (),

    #[webapi(
        accessor_property = "f",
        getter = svg_matrix_getter,
        setter = svg_matrix_setter,
        data = callback_data_index_value(scope, 5)
    )]
    f: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGGraphicsElement, enumerable)]
struct SvgGraphicsElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "transform", getter = svg_graphics_transform_getter)]
    transform: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGGeometryElement, enumerable)]
struct SvgGeometryElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "pathLength", getter = svg_geometry_path_length_getter)]
    path_length: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGTextContentElement, receiver = super::text::is_text_receiver, enumerable)]
struct SvgTextContentElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "textLength", getter = svg_text_content_text_length_getter)]
    text_length: (),

    #[webapi(accessor_property = "lengthAdjust", getter = svg_text_content_length_adjust_getter)]
    length_adjust: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGTextPositioningElement, enumerable)]
struct SvgTextPositioningElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "x", getter = svg_text_positioning_list_getter, data = callback_data_index_value(scope, 0))]
    x: (),

    #[webapi(accessor_property = "y", getter = svg_text_positioning_list_getter, data = callback_data_index_value(scope, 1))]
    y: (),

    #[webapi(accessor_property = "dx", getter = svg_text_positioning_list_getter, data = callback_data_index_value(scope, 2))]
    dx: (),

    #[webapi(accessor_property = "dy", getter = svg_text_positioning_list_getter, data = callback_data_index_value(scope, 3))]
    dy: (),

    #[webapi(accessor_property = "rotate", getter = svg_text_positioning_list_getter, data = callback_data_index_value(scope, 4))]
    rotate: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGPatternElement, enumerable)]
struct SvgPatternElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "patternTransform", getter = svg_pattern_transform_getter)]
    pattern_transform: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGGradientElement, enumerable)]
struct SvgGradientElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "gradientTransform", getter = svg_gradient_transform_getter)]
    gradient_transform: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGRectElement, enumerable)]
struct SvgRectElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "x", getter = svg_rect_animated_length_getter, data = callback_data_index_value(scope, 0))]
    x: (),

    #[webapi(accessor_property = "y", getter = svg_rect_animated_length_getter, data = callback_data_index_value(scope, 1))]
    y: (),

    #[webapi(accessor_property = "width", getter = svg_rect_animated_length_getter, data = callback_data_index_value(scope, 2))]
    width: (),

    #[webapi(accessor_property = "height", getter = svg_rect_animated_length_getter, data = callback_data_index_value(scope, 3))]
    height: (),

    #[webapi(accessor_property = "rx", getter = svg_rect_animated_length_getter, data = callback_data_index_value(scope, 4))]
    rx: (),

    #[webapi(accessor_property = "ry", getter = svg_rect_animated_length_getter, data = callback_data_index_value(scope, 5))]
    ry: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGCircleElement, enumerable)]
struct SvgCircleElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "cx", getter = svg_circle_animated_length_getter, data = callback_data_index_value(scope, 0))]
    cx: (),

    #[webapi(accessor_property = "cy", getter = svg_circle_animated_length_getter, data = callback_data_index_value(scope, 1))]
    cy: (),

    #[webapi(accessor_property = "r", getter = svg_circle_animated_length_getter, data = callback_data_index_value(scope, 2))]
    r: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGEllipseElement, enumerable)]
struct SvgEllipseElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "cx", getter = svg_ellipse_animated_length_getter, data = callback_data_index_value(scope, 0))]
    cx: (),

    #[webapi(accessor_property = "cy", getter = svg_ellipse_animated_length_getter, data = callback_data_index_value(scope, 1))]
    cy: (),

    #[webapi(accessor_property = "rx", getter = svg_ellipse_animated_length_getter, data = callback_data_index_value(scope, 2))]
    rx: (),

    #[webapi(accessor_property = "ry", getter = svg_ellipse_animated_length_getter, data = callback_data_index_value(scope, 3))]
    ry: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGLineElement, enumerable)]
struct SvgLineElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "x1", getter = svg_line_animated_length_getter, data = callback_data_index_value(scope, 0))]
    x1: (),

    #[webapi(accessor_property = "y1", getter = svg_line_animated_length_getter, data = callback_data_index_value(scope, 1))]
    y1: (),

    #[webapi(accessor_property = "x2", getter = svg_line_animated_length_getter, data = callback_data_index_value(scope, 2))]
    x2: (),

    #[webapi(accessor_property = "y2", getter = svg_line_animated_length_getter, data = callback_data_index_value(scope, 3))]
    y2: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGGraphicsBoxElement", enumerable)]
struct SvgGraphicsBoxElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "x", getter = svg_box_animated_length_getter, data = callback_data_index_value(scope, 0))]
    x: (),

    #[webapi(accessor_property = "y", getter = svg_box_animated_length_getter, data = callback_data_index_value(scope, 1))]
    y: (),

    #[webapi(accessor_property = "width", getter = svg_box_animated_length_getter, data = callback_data_index_value(scope, 2))]
    width: (),

    #[webapi(accessor_property = "height", getter = svg_box_animated_length_getter, data = callback_data_index_value(scope, 3))]
    height: (),
}

pub(super) fn install_svg_length_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgLengthTemplateMethodsDeclaration::initialize_template(scope, template);
    SvgLengthTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
    SvgLengthTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_animated_length_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAnimatedLengthTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_number_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgNumberTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_animated_length_list_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAnimatedLengthListTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_length_list_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    install_svg_value_list_bindings(scope, template, SvgListKind::Length);
}

pub(super) fn install_svg_animated_number_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAnimatedNumberTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_animated_number_list_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAnimatedNumberListTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_number_list_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    install_svg_value_list_bindings(scope, template, SvgListKind::Number);
}

pub(super) fn install_svg_animated_enumeration_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAnimatedEnumerationTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_value_list_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    kind: SvgListKind,
) {
    let proto = template.prototype_template(scope);
    match kind {
        SvgListKind::Length => {
            SvgLengthListTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
            SvgLengthListTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        SvgListKind::Number => {
            SvgNumberListTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
            SvgNumberListTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
    }
}

pub(super) fn install_svg_animated_transform_list_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAnimatedTransformListTemplateAccessorsDeclaration::initialize_prototype_template(
        scope, proto,
    );
}

pub(super) fn install_svg_transform_list_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgTransformListTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
    SvgTransformListTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_transform_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgTransformTemplateMethodsDeclaration::initialize_template(scope, template);
    SvgTransformTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
    SvgTransformTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_matrix_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgMatrixTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
    SvgMatrixTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_graphics_element_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgGraphicsElementTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_geometry_element_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgGeometryElementTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_text_content_element_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgTextContentElementTemplateMethodsDeclaration::initialize_template(scope, template);
    SvgTextContentElementTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_svg_element_bindings(
    scope: &mut v8::PinScope<'_, '_, ()>,
    template: v8::Local<'_, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgSvgElementTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_element_accessor_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "SVGGraphicsElement" => {
            SvgGraphicsElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGGeometryElement" => {
            SvgGeometryElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGTextContentElement" => {
            SvgTextContentElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGTextPositioningElement" => {
            SvgTextPositioningElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGPatternElement" => {
            SvgPatternElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGGradientElement" => {
            SvgGradientElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGRectElement" => {
            SvgRectElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGCircleElement" => {
            SvgCircleElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGEllipseElement" => {
            SvgEllipseElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGLineElement" => {
            SvgLineElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGImageElement" | "SVGUseElement" | "SVGForeignObjectElement" => {
            SvgGraphicsBoxElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}
