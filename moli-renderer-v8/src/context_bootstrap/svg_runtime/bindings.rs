use super::callbacks::*;
use super::*;
use crate::util::callback_data_index_value;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGLength", enumerable)]
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
#[webapi(name = "SVGAngle", enumerable)]
struct SvgAngleTemplateMethodsDeclaration {
    #[webapi(constant = "SVG_ANGLETYPE_UNKNOWN", value = SVG_ANGLE_TYPE_UNKNOWN)]
    angle_type_unknown: (),

    #[webapi(
        constant = "SVG_ANGLETYPE_UNSPECIFIED",
        value = SVG_ANGLE_TYPE_UNSPECIFIED
    )]
    angle_type_unspecified: (),

    #[webapi(constant = "SVG_ANGLETYPE_DEG", value = SVG_ANGLE_TYPE_DEG)]
    angle_type_deg: (),

    #[webapi(constant = "SVG_ANGLETYPE_RAD", value = SVG_ANGLE_TYPE_RAD)]
    angle_type_rad: (),

    #[webapi(constant = "SVG_ANGLETYPE_GRAD", value = SVG_ANGLE_TYPE_GRAD)]
    angle_type_grad: (),

    #[webapi(
        method = "newValueSpecifiedUnits",
        length = 2,
        callback = svg_angle_new_value_specified_units_callback
    )]
    new_value_specified_units: (),

    #[webapi(
        method = "convertToSpecifiedUnits",
        length = 1,
        callback = svg_angle_convert_to_specified_units_callback
    )]
    convert_to_specified_units: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGLengthList", enumerable)]
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
#[webapi(name = "SVGNumberList", enumerable)]
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
#[webapi(name = "SVGPointList", enumerable)]
struct SvgPointListTemplateMethodsDeclaration {
    #[webapi(method = "clear", length = 0, callback = svg_point_list_clear_callback)]
    clear: (),

    #[webapi(
        method = "initialize",
        length = 1,
        callback = svg_point_list_initialize_callback
    )]
    initialize: (),

    #[webapi(method = "getItem", length = 1, callback = svg_point_list_get_item_callback)]
    get_item: (),

    #[webapi(
        method = "insertItemBefore",
        length = 2,
        callback = svg_point_list_insert_item_before_callback
    )]
    insert_item_before: (),

    #[webapi(
        method = "replaceItem",
        length = 2,
        callback = svg_point_list_replace_item_callback
    )]
    replace_item: (),

    #[webapi(
        method = "removeItem",
        length = 1,
        callback = svg_point_list_remove_item_callback
    )]
    remove_item: (),

    #[webapi(
        method = "appendItem",
        length = 1,
        callback = svg_point_list_append_item_callback
    )]
    append_item: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGStringList", enumerable)]
struct SvgStringListTemplateMethodsDeclaration {
    #[webapi(method = "clear", length = 0, callback = svg_string_list_clear_callback)]
    clear: (),

    #[webapi(
        method = "initialize",
        length = 1,
        callback = svg_string_list_initialize_callback
    )]
    initialize: (),

    #[webapi(method = "getItem", length = 1, callback = svg_string_list_get_item_callback)]
    get_item: (),

    #[webapi(
        method = "insertItemBefore",
        length = 2,
        callback = svg_string_list_insert_item_before_callback
    )]
    insert_item_before: (),

    #[webapi(
        method = "replaceItem",
        length = 2,
        callback = svg_string_list_replace_item_callback
    )]
    replace_item: (),

    #[webapi(
        method = "removeItem",
        length = 1,
        callback = svg_string_list_remove_item_callback
    )]
    remove_item: (),

    #[webapi(
        method = "appendItem",
        length = 1,
        callback = svg_string_list_append_item_callback
    )]
    append_item: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGTransformList", enumerable)]
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
        length = 0,
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
#[webapi(name = "SVGTransform", enumerable)]
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

    #[webapi(method = "setMatrix", length = 0, callback = svg_transform_set_matrix_callback)]
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
#[webapi(name = "SVGMatrix", enumerable)]
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
#[webapi(name = "SVGGraphicsElement", enumerable)]
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
#[webapi(name = "SVGGeometryElement", enumerable)]
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
#[webapi(name = "SVGTextContentElement", enumerable)]
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
#[webapi(name = "SVGUnitTypes", enumerable)]
struct SvgUnitTypesTemplateConstantsDeclaration {
    #[webapi(constant = "SVG_UNIT_TYPE_UNKNOWN", value = SVG_UNIT_TYPE_UNKNOWN)]
    unknown: (),

    #[webapi(
        constant = "SVG_UNIT_TYPE_USERSPACEONUSE",
        value = SVG_UNIT_TYPE_USER_SPACE_ON_USE
    )]
    user_space_on_use: (),

    #[webapi(
        constant = "SVG_UNIT_TYPE_OBJECTBOUNDINGBOX",
        value = SVG_UNIT_TYPE_OBJECT_BOUNDING_BOX
    )]
    object_bounding_box: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGGradientElement", enumerable)]
struct SvgGradientElementTemplateConstantsDeclaration {
    #[webapi(
        constant = "SVG_SPREADMETHOD_UNKNOWN",
        value = SVG_SPREAD_METHOD_UNKNOWN
    )]
    spread_method_unknown: (),

    #[webapi(
        constant = "SVG_SPREADMETHOD_PAD",
        value = SVG_SPREAD_METHOD_PAD
    )]
    spread_method_pad: (),

    #[webapi(
        constant = "SVG_SPREADMETHOD_REFLECT",
        value = SVG_SPREAD_METHOD_REFLECT
    )]
    spread_method_reflect: (),

    #[webapi(
        constant = "SVG_SPREADMETHOD_REPEAT",
        value = SVG_SPREAD_METHOD_REPEAT
    )]
    spread_method_repeat: (),
}

macro_rules! define_svg_enumeration_constants {
    ($declaration:ident, $interface:literal, {
        $($field:ident => $constant:literal = $value:expr),+ $(,)?
    }) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(name = $interface, enumerable)]
        struct $declaration {
            $(
                #[webapi(constant = $constant, value = $value)]
                $field: (),
            )+
        }
    };
}

define_svg_enumeration_constants!(
    SvgPreserveAspectRatioTemplateConstantsDeclaration,
    "SVGPreserveAspectRatio",
    {
        preserve_aspect_ratio_unknown => "SVG_PRESERVEASPECTRATIO_UNKNOWN" = SVG_PRESERVE_ASPECT_RATIO_UNKNOWN,
        preserve_aspect_ratio_none => "SVG_PRESERVEASPECTRATIO_NONE" = SVG_PRESERVE_ASPECT_RATIO_NONE,
        preserve_aspect_ratio_x_min_y_min => "SVG_PRESERVEASPECTRATIO_XMINYMIN" = SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MIN,
        preserve_aspect_ratio_x_mid_y_min => "SVG_PRESERVEASPECTRATIO_XMIDYMIN" = SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MIN,
        preserve_aspect_ratio_x_max_y_min => "SVG_PRESERVEASPECTRATIO_XMAXYMIN" = SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MIN,
        preserve_aspect_ratio_x_min_y_mid => "SVG_PRESERVEASPECTRATIO_XMINYMID" = SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MID,
        preserve_aspect_ratio_x_mid_y_mid => "SVG_PRESERVEASPECTRATIO_XMIDYMID" = SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MID,
        preserve_aspect_ratio_x_max_y_mid => "SVG_PRESERVEASPECTRATIO_XMAXYMID" = SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MID,
        preserve_aspect_ratio_x_min_y_max => "SVG_PRESERVEASPECTRATIO_XMINYMAX" = SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MAX,
        preserve_aspect_ratio_x_mid_y_max => "SVG_PRESERVEASPECTRATIO_XMIDYMAX" = SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MAX,
        preserve_aspect_ratio_x_max_y_max => "SVG_PRESERVEASPECTRATIO_XMAXYMAX" = SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MAX,
        meet_or_slice_unknown => "SVG_MEETORSLICE_UNKNOWN" = SVG_MEET_OR_SLICE_UNKNOWN,
        meet_or_slice_meet => "SVG_MEETORSLICE_MEET" = SVG_MEET_OR_SLICE_MEET,
        meet_or_slice_slice => "SVG_MEETORSLICE_SLICE" = SVG_MEET_OR_SLICE_SLICE,
    }
);

define_svg_enumeration_constants!(
    SvgComponentTransferFunctionElementTemplateConstantsDeclaration,
    "SVGComponentTransferFunctionElement",
    {
        unknown => "SVG_FECOMPONENTTRANSFER_TYPE_UNKNOWN" = SVG_COMPONENT_TRANSFER_TYPE_UNKNOWN,
        identity => "SVG_FECOMPONENTTRANSFER_TYPE_IDENTITY" = SVG_COMPONENT_TRANSFER_TYPE_IDENTITY,
        table => "SVG_FECOMPONENTTRANSFER_TYPE_TABLE" = SVG_COMPONENT_TRANSFER_TYPE_TABLE,
        discrete => "SVG_FECOMPONENTTRANSFER_TYPE_DISCRETE" = SVG_COMPONENT_TRANSFER_TYPE_DISCRETE,
        linear => "SVG_FECOMPONENTTRANSFER_TYPE_LINEAR" = SVG_COMPONENT_TRANSFER_TYPE_LINEAR,
        gamma => "SVG_FECOMPONENTTRANSFER_TYPE_GAMMA" = SVG_COMPONENT_TRANSFER_TYPE_GAMMA,
    }
);

define_svg_enumeration_constants!(
    SvgFeBlendElementTemplateConstantsDeclaration,
    "SVGFEBlendElement",
    {
        unknown => "SVG_FEBLEND_MODE_UNKNOWN" = SVG_FE_BLEND_MODE_UNKNOWN,
        normal => "SVG_FEBLEND_MODE_NORMAL" = SVG_FE_BLEND_MODE_NORMAL,
        multiply => "SVG_FEBLEND_MODE_MULTIPLY" = SVG_FE_BLEND_MODE_MULTIPLY,
        screen => "SVG_FEBLEND_MODE_SCREEN" = SVG_FE_BLEND_MODE_SCREEN,
        darken => "SVG_FEBLEND_MODE_DARKEN" = SVG_FE_BLEND_MODE_DARKEN,
        lighten => "SVG_FEBLEND_MODE_LIGHTEN" = SVG_FE_BLEND_MODE_LIGHTEN,
        overlay => "SVG_FEBLEND_MODE_OVERLAY" = SVG_FE_BLEND_MODE_OVERLAY,
        color_dodge => "SVG_FEBLEND_MODE_COLOR_DODGE" = SVG_FE_BLEND_MODE_COLOR_DODGE,
        color_burn => "SVG_FEBLEND_MODE_COLOR_BURN" = SVG_FE_BLEND_MODE_COLOR_BURN,
        hard_light => "SVG_FEBLEND_MODE_HARD_LIGHT" = SVG_FE_BLEND_MODE_HARD_LIGHT,
        soft_light => "SVG_FEBLEND_MODE_SOFT_LIGHT" = SVG_FE_BLEND_MODE_SOFT_LIGHT,
        difference => "SVG_FEBLEND_MODE_DIFFERENCE" = SVG_FE_BLEND_MODE_DIFFERENCE,
        exclusion => "SVG_FEBLEND_MODE_EXCLUSION" = SVG_FE_BLEND_MODE_EXCLUSION,
        hue => "SVG_FEBLEND_MODE_HUE" = SVG_FE_BLEND_MODE_HUE,
        saturation => "SVG_FEBLEND_MODE_SATURATION" = SVG_FE_BLEND_MODE_SATURATION,
        color => "SVG_FEBLEND_MODE_COLOR" = SVG_FE_BLEND_MODE_COLOR,
        luminosity => "SVG_FEBLEND_MODE_LUMINOSITY" = SVG_FE_BLEND_MODE_LUMINOSITY,
    }
);

define_svg_enumeration_constants!(
    SvgFeColorMatrixElementTemplateConstantsDeclaration,
    "SVGFEColorMatrixElement",
    {
        unknown => "SVG_FECOLORMATRIX_TYPE_UNKNOWN" = SVG_FE_COLOR_MATRIX_TYPE_UNKNOWN,
        matrix => "SVG_FECOLORMATRIX_TYPE_MATRIX" = SVG_FE_COLOR_MATRIX_TYPE_MATRIX,
        saturate => "SVG_FECOLORMATRIX_TYPE_SATURATE" = SVG_FE_COLOR_MATRIX_TYPE_SATURATE,
        hue_rotate => "SVG_FECOLORMATRIX_TYPE_HUEROTATE" = SVG_FE_COLOR_MATRIX_TYPE_HUE_ROTATE,
        luminance_to_alpha => "SVG_FECOLORMATRIX_TYPE_LUMINANCETOALPHA" = SVG_FE_COLOR_MATRIX_TYPE_LUMINANCE_TO_ALPHA,
    }
);

define_svg_enumeration_constants!(
    SvgFeCompositeElementTemplateConstantsDeclaration,
    "SVGFECompositeElement",
    {
        unknown => "SVG_FECOMPOSITE_OPERATOR_UNKNOWN" = SVG_FE_COMPOSITE_OPERATOR_UNKNOWN,
        over => "SVG_FECOMPOSITE_OPERATOR_OVER" = SVG_FE_COMPOSITE_OPERATOR_OVER,
        input => "SVG_FECOMPOSITE_OPERATOR_IN" = SVG_FE_COMPOSITE_OPERATOR_IN,
        out => "SVG_FECOMPOSITE_OPERATOR_OUT" = SVG_FE_COMPOSITE_OPERATOR_OUT,
        atop => "SVG_FECOMPOSITE_OPERATOR_ATOP" = SVG_FE_COMPOSITE_OPERATOR_ATOP,
        xor => "SVG_FECOMPOSITE_OPERATOR_XOR" = SVG_FE_COMPOSITE_OPERATOR_XOR,
        lighter => "SVG_FECOMPOSITE_OPERATOR_LIGHTER" = SVG_FE_COMPOSITE_OPERATOR_LIGHTER,
        arithmetic => "SVG_FECOMPOSITE_OPERATOR_ARITHMETIC" = SVG_FE_COMPOSITE_OPERATOR_ARITHMETIC,
    }
);

define_svg_enumeration_constants!(
    SvgFeConvolveMatrixElementTemplateConstantsDeclaration,
    "SVGFEConvolveMatrixElement",
    {
        unknown => "SVG_EDGEMODE_UNKNOWN" = SVG_EDGE_MODE_UNKNOWN,
        duplicate => "SVG_EDGEMODE_DUPLICATE" = SVG_EDGE_MODE_DUPLICATE,
        wrap => "SVG_EDGEMODE_WRAP" = SVG_EDGE_MODE_WRAP,
        none => "SVG_EDGEMODE_NONE" = SVG_EDGE_MODE_NONE,
    }
);

define_svg_enumeration_constants!(
    SvgFeDisplacementMapElementTemplateConstantsDeclaration,
    "SVGFEDisplacementMapElement",
    {
        unknown => "SVG_CHANNEL_UNKNOWN" = SVG_CHANNEL_UNKNOWN,
        red => "SVG_CHANNEL_R" = SVG_CHANNEL_R,
        green => "SVG_CHANNEL_G" = SVG_CHANNEL_G,
        blue => "SVG_CHANNEL_B" = SVG_CHANNEL_B,
        alpha => "SVG_CHANNEL_A" = SVG_CHANNEL_A,
    }
);

define_svg_enumeration_constants!(
    SvgFeMorphologyElementTemplateConstantsDeclaration,
    "SVGFEMorphologyElement",
    {
        unknown => "SVG_MORPHOLOGY_OPERATOR_UNKNOWN" = SVG_MORPHOLOGY_OPERATOR_UNKNOWN,
        erode => "SVG_MORPHOLOGY_OPERATOR_ERODE" = SVG_MORPHOLOGY_OPERATOR_ERODE,
        dilate => "SVG_MORPHOLOGY_OPERATOR_DILATE" = SVG_MORPHOLOGY_OPERATOR_DILATE,
    }
);

define_svg_enumeration_constants!(
    SvgFeTurbulenceElementTemplateConstantsDeclaration,
    "SVGFETurbulenceElement",
    {
        turbulence_unknown => "SVG_TURBULENCE_TYPE_UNKNOWN" = SVG_TURBULENCE_TYPE_UNKNOWN,
        fractal_noise => "SVG_TURBULENCE_TYPE_FRACTALNOISE" = SVG_TURBULENCE_TYPE_FRACTAL_NOISE,
        turbulence => "SVG_TURBULENCE_TYPE_TURBULENCE" = SVG_TURBULENCE_TYPE_TURBULENCE,
        stitch_unknown => "SVG_STITCHTYPE_UNKNOWN" = SVG_STITCH_TYPE_UNKNOWN,
        stitch => "SVG_STITCHTYPE_STITCH" = SVG_STITCH_TYPE_STITCH,
        no_stitch => "SVG_STITCHTYPE_NOSTITCH" = SVG_STITCH_TYPE_NO_STITCH,
    }
);

define_svg_enumeration_constants!(
    SvgTextPathElementTemplateConstantsDeclaration,
    "SVGTextPathElement",
    {
        method_unknown => "TEXTPATH_METHODTYPE_UNKNOWN" = SVG_TEXT_PATH_METHOD_TYPE_UNKNOWN,
        method_align => "TEXTPATH_METHODTYPE_ALIGN" = SVG_TEXT_PATH_METHOD_TYPE_ALIGN,
        method_stretch => "TEXTPATH_METHODTYPE_STRETCH" = SVG_TEXT_PATH_METHOD_TYPE_STRETCH,
        spacing_unknown => "TEXTPATH_SPACINGTYPE_UNKNOWN" = SVG_TEXT_PATH_SPACING_TYPE_UNKNOWN,
        spacing_auto => "TEXTPATH_SPACINGTYPE_AUTO" = SVG_TEXT_PATH_SPACING_TYPE_AUTO,
        spacing_exact => "TEXTPATH_SPACINGTYPE_EXACT" = SVG_TEXT_PATH_SPACING_TYPE_EXACT,
        side_unknown => "TEXTPATH_SIDETYPE_UNKNOWN" = SVG_TEXT_PATH_SIDE_TYPE_UNKNOWN,
        side_left => "TEXTPATH_SIDETYPE_LEFT" = SVG_TEXT_PATH_SIDE_TYPE_LEFT,
        side_right => "TEXTPATH_SIDETYPE_RIGHT" = SVG_TEXT_PATH_SIDE_TYPE_RIGHT,
    }
);

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGMarkerElement", enumerable)]
struct SvgMarkerElementTemplateMethodsDeclaration {
    #[webapi(
        constant = "SVG_MARKERUNITS_UNKNOWN",
        value = SVG_MARKER_UNITS_UNKNOWN
    )]
    marker_units_unknown: (),

    #[webapi(
        constant = "SVG_MARKERUNITS_USERSPACEONUSE",
        value = SVG_MARKER_UNITS_USER_SPACE_ON_USE
    )]
    marker_units_user_space_on_use: (),

    #[webapi(
        constant = "SVG_MARKERUNITS_STROKEWIDTH",
        value = SVG_MARKER_UNITS_STROKE_WIDTH
    )]
    marker_units_stroke_width: (),

    #[webapi(
        constant = "SVG_MARKER_ORIENT_UNKNOWN",
        value = SVG_MARKER_ORIENT_UNKNOWN
    )]
    marker_orient_unknown: (),

    #[webapi(
        constant = "SVG_MARKER_ORIENT_AUTO",
        value = SVG_MARKER_ORIENT_AUTO
    )]
    marker_orient_auto: (),

    #[webapi(
        constant = "SVG_MARKER_ORIENT_ANGLE",
        value = SVG_MARKER_ORIENT_ANGLE
    )]
    marker_orient_angle: (),

    #[webapi(
        constant = "SVG_MARKER_ORIENT_AUTO_START_REVERSE",
        value = SVG_MARKER_ORIENT_AUTO_START_REVERSE
    )]
    marker_orient_auto_start_reverse: (),

    #[webapi(
        method = "setOrientToAuto",
        length = 0,
        callback = svg_marker_set_orient_to_auto_callback
    )]
    set_orient_to_auto: (),

    #[webapi(
        method = "setOrientToAngle",
        length = 1,
        callback = svg_marker_set_orient_to_angle_callback
    )]
    set_orient_to_angle: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGSVGElement", enumerable)]
struct SvgSvgElementTemplateMethodsDeclaration {
    #[webapi(
        method = "deselectAll",
        length = 0,
        callback = svg_svg_element_deselect_all_callback
    )]
    deselect_all: (),

    #[webapi(
        method = "createSVGNumber",
        length = 0,
        callback = svg_svg_element_create_number_callback
    )]
    create_svg_number: (),

    #[webapi(
        method = "createSVGLength",
        length = 0,
        callback = svg_svg_element_create_length_callback
    )]
    create_svg_length: (),

    #[webapi(
        method = "createSVGAngle",
        length = 0,
        callback = svg_svg_element_create_angle_callback
    )]
    create_svg_angle: (),

    #[webapi(
        method = "createSVGPoint",
        length = 0,
        callback = svg_svg_element_create_point_callback
    )]
    create_svg_point: (),

    #[webapi(
        method = "createSVGMatrix",
        length = 0,
        callback = svg_svg_element_create_matrix_callback
    )]
    create_svg_matrix: (),

    #[webapi(
        method = "createSVGRect",
        length = 0,
        callback = svg_svg_element_create_rect_callback
    )]
    create_svg_rect: (),

    #[webapi(
        method = "createSVGTransform",
        length = 0,
        callback = svg_svg_element_create_transform_callback
    )]
    create_svg_transform: (),

    #[webapi(
        method = "createSVGTransformFromMatrix",
        length = 0,
        callback = svg_svg_element_create_transform_from_matrix_callback
    )]
    create_svg_transform_from_matrix: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGLength", enumerable)]
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
#[webapi(name = "SVGAngle", enumerable)]
struct SvgAngleTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "unitType",
        getter = svg_angle_getter,
        data = callback_data_index_value(scope, 0)
    )]
    unit_type: (),

    #[webapi(
        accessor_property = "value",
        getter = svg_angle_getter,
        setter = svg_angle_setter,
        data = callback_data_index_value(scope, 1)
    )]
    value: (),

    #[webapi(
        accessor_property = "valueInSpecifiedUnits",
        getter = svg_angle_getter,
        setter = svg_angle_setter,
        data = callback_data_index_value(scope, 2)
    )]
    value_in_specified_units: (),

    #[webapi(
        accessor_property = "valueAsString",
        getter = svg_angle_getter,
        setter = svg_angle_setter,
        data = callback_data_index_value(scope, 3)
    )]
    value_as_string: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGAnimatedString", enumerable)]
struct SvgAnimatedStringTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "baseVal",
        getter = svg_animated_string_getter,
        setter = svg_animated_string_setter,
        data = callback_data_index_value(scope, 0)
    )]
    base_val: (),

    #[webapi(
        accessor_property = "animVal",
        getter = svg_animated_string_getter,
        data = callback_data_index_value(scope, 1)
    )]
    anim_val: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGAnimatedBoolean", enumerable)]
struct SvgAnimatedBooleanTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "baseVal",
        getter = svg_animated_boolean_getter,
        setter = svg_animated_boolean_setter,
        data = callback_data_index_value(scope, 0)
    )]
    base_val: (),

    #[webapi(
        accessor_property = "animVal",
        getter = svg_animated_boolean_getter,
        data = callback_data_index_value(scope, 1)
    )]
    anim_val: (),
}

#[allow(dead_code)]
#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGAnimatedLength", enumerable)]
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
#[webapi(name = "SVGAnimatedAngle", enumerable)]
struct SvgAnimatedAngleTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "baseVal",
        getter = svg_animated_angle_getter,
        data = callback_data_index_value(scope, 0)
    )]
    base_val: (),

    #[webapi(
        accessor_property = "animVal",
        getter = svg_animated_angle_getter,
        data = callback_data_index_value(scope, 1)
    )]
    anim_val: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGAnimatedRect", enumerable)]
struct SvgAnimatedRectTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "baseVal",
        getter = svg_animated_rect_getter,
        data = callback_data_index_value(scope, 0)
    )]
    base_val: (),

    #[webapi(
        accessor_property = "animVal",
        getter = svg_animated_rect_getter,
        data = callback_data_index_value(scope, 1)
    )]
    anim_val: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGPreserveAspectRatio", enumerable)]
struct SvgPreserveAspectRatioTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "align",
        getter = svg_preserve_aspect_ratio_getter,
        setter = svg_preserve_aspect_ratio_setter,
        data = callback_data_index_value(scope, 0)
    )]
    align: (),

    #[webapi(
        accessor_property = "meetOrSlice",
        getter = svg_preserve_aspect_ratio_getter,
        setter = svg_preserve_aspect_ratio_setter,
        data = callback_data_index_value(scope, 1)
    )]
    meet_or_slice: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGAnimatedPreserveAspectRatio", enumerable)]
struct SvgAnimatedPreserveAspectRatioTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "baseVal",
        getter = svg_animated_preserve_aspect_ratio_getter,
        data = callback_data_index_value(scope, 0)
    )]
    base_val: (),

    #[webapi(
        accessor_property = "animVal",
        getter = svg_animated_preserve_aspect_ratio_getter,
        data = callback_data_index_value(scope, 1)
    )]
    anim_val: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGNumber", enumerable)]
struct SvgNumberTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "value",
        getter = svg_number_getter,
        setter = svg_number_setter
    )]
    value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGAnimatedLengthList", enumerable)]
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
#[webapi(name = "SVGAnimatedNumber", enumerable)]
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
#[webapi(name = "SVGAnimatedInteger", enumerable)]
struct SvgAnimatedIntegerTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "baseVal",
        getter = svg_animated_integer_getter,
        setter = svg_animated_integer_setter,
        data = callback_data_index_value(scope, 0)
    )]
    base_val: (),

    #[webapi(
        accessor_property = "animVal",
        getter = svg_animated_integer_getter,
        data = callback_data_index_value(scope, 1)
    )]
    anim_val: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGAnimatedNumberList", enumerable)]
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
#[webapi(name = "SVGAnimatedEnumeration", enumerable)]
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
#[webapi(name = "SVGLengthList", enumerable)]
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
#[webapi(name = "SVGNumberList", enumerable)]
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
#[webapi(name = "SVGPointList", enumerable)]
struct SvgPointListTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "length", getter = svg_point_list_length_getter)]
    length: (),

    #[webapi(
        accessor_property = "numberOfItems",
        getter = svg_point_list_length_getter
    )]
    number_of_items: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGStringList", enumerable)]
struct SvgStringListTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "length", getter = svg_string_list_length_getter)]
    length: (),

    #[webapi(
        accessor_property = "numberOfItems",
        getter = svg_string_list_length_getter
    )]
    number_of_items: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGAnimatedTransformList", enumerable)]
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
#[webapi(name = "SVGTransformList", enumerable)]
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
#[webapi(name = "SVGTransform", enumerable)]
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
#[webapi(name = "SVGMatrix", enumerable)]
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

macro_rules! define_svg_animated_number_accessors {
    (
        $declaration:ident,
        $interface:literal,
        $(($field:ident, $name:literal, $index:literal)),+ $(,)?
    ) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(name = $interface, enumerable)]
        struct $declaration {
            $(
                #[webapi(
                    accessor_property = $name,
                    getter = svg_element_animated_number_getter,
                    data = callback_data_index_value(scope, $index)
                )]
                $field: (),
            )+
        }
    };
}

define_svg_animated_number_accessors!(
    SvgComponentTransferFunctionAnimatedNumberAccessorsDeclaration,
    "SVGComponentTransferFunctionElement",
    (slope, "slope", 0),
    (intercept, "intercept", 1),
    (amplitude, "amplitude", 2),
    (exponent, "exponent", 3),
    (offset, "offset", 4),
);

define_svg_animated_number_accessors!(
    SvgFeCompositeAnimatedNumberAccessorsDeclaration,
    "SVGFECompositeElement",
    (k1, "k1", 5),
    (k2, "k2", 6),
    (k3, "k3", 7),
    (k4, "k4", 8),
);

define_svg_animated_number_accessors!(
    SvgFeConvolveMatrixAnimatedNumberAccessorsDeclaration,
    "SVGFEConvolveMatrixElement",
    (divisor, "divisor", 9),
    (bias, "bias", 10),
    (kernel_unit_length_x, "kernelUnitLengthX", 11),
    (kernel_unit_length_y, "kernelUnitLengthY", 12),
);

define_svg_animated_number_accessors!(
    SvgFeDiffuseLightingAnimatedNumberAccessorsDeclaration,
    "SVGFEDiffuseLightingElement",
    (surface_scale, "surfaceScale", 13),
    (diffuse_constant, "diffuseConstant", 14),
    (kernel_unit_length_x, "kernelUnitLengthX", 15),
    (kernel_unit_length_y, "kernelUnitLengthY", 16),
);

define_svg_animated_number_accessors!(
    SvgFeDisplacementMapAnimatedNumberAccessorsDeclaration,
    "SVGFEDisplacementMapElement",
    (scale, "scale", 17),
);

define_svg_animated_number_accessors!(
    SvgFeDistantLightAnimatedNumberAccessorsDeclaration,
    "SVGFEDistantLightElement",
    (azimuth, "azimuth", 18),
    (elevation, "elevation", 19),
);

define_svg_animated_number_accessors!(
    SvgFeDropShadowAnimatedNumberAccessorsDeclaration,
    "SVGFEDropShadowElement",
    (dx, "dx", 20),
    (dy, "dy", 21),
    (std_deviation_x, "stdDeviationX", 22),
    (std_deviation_y, "stdDeviationY", 23),
);

define_svg_animated_number_accessors!(
    SvgFeGaussianBlurAnimatedNumberAccessorsDeclaration,
    "SVGFEGaussianBlurElement",
    (std_deviation_x, "stdDeviationX", 24),
    (std_deviation_y, "stdDeviationY", 25),
);

define_svg_animated_number_accessors!(
    SvgFeMorphologyAnimatedNumberAccessorsDeclaration,
    "SVGFEMorphologyElement",
    (radius_x, "radiusX", 26),
    (radius_y, "radiusY", 27),
);

define_svg_animated_number_accessors!(
    SvgFeOffsetAnimatedNumberAccessorsDeclaration,
    "SVGFEOffsetElement",
    (dx, "dx", 28),
    (dy, "dy", 29),
);

define_svg_animated_number_accessors!(
    SvgFePointLightAnimatedNumberAccessorsDeclaration,
    "SVGFEPointLightElement",
    (x, "x", 30),
    (y, "y", 31),
    (z, "z", 32),
);

define_svg_animated_number_accessors!(
    SvgFeSpecularLightingAnimatedNumberAccessorsDeclaration,
    "SVGFESpecularLightingElement",
    (surface_scale, "surfaceScale", 33),
    (specular_constant, "specularConstant", 34),
    (specular_exponent, "specularExponent", 35),
    (kernel_unit_length_x, "kernelUnitLengthX", 36),
    (kernel_unit_length_y, "kernelUnitLengthY", 37),
);

define_svg_animated_number_accessors!(
    SvgFeSpotLightAnimatedNumberAccessorsDeclaration,
    "SVGFESpotLightElement",
    (x, "x", 38),
    (y, "y", 39),
    (z, "z", 40),
    (points_at_x, "pointsAtX", 41),
    (points_at_y, "pointsAtY", 42),
    (points_at_z, "pointsAtZ", 43),
    (specular_exponent, "specularExponent", 44),
    (limiting_cone_angle, "limitingConeAngle", 45),
);

define_svg_animated_number_accessors!(
    SvgFeTurbulenceAnimatedNumberAccessorsDeclaration,
    "SVGFETurbulenceElement",
    (base_frequency_x, "baseFrequencyX", 46),
    (base_frequency_y, "baseFrequencyY", 47),
    (seed, "seed", 48),
);

define_svg_animated_number_accessors!(
    SvgStopAnimatedNumberAccessorsDeclaration,
    "SVGStopElement",
    (offset, "offset", 49),
);

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGGraphicsElement", enumerable)]
struct SvgGraphicsElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "transform", getter = svg_graphics_transform_getter)]
    transform: (),

    #[webapi(
        accessor_property = "requiredExtensions",
        getter = svg_graphics_test_string_list_getter,
        data = callback_data_index_value(scope, 0)
    )]
    required_extensions: (),

    #[webapi(
        accessor_property = "systemLanguage",
        getter = svg_graphics_test_string_list_getter,
        data = callback_data_index_value(scope, 1)
    )]
    system_language: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGGeometryElement", enumerable)]
struct SvgGeometryElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "pathLength", getter = svg_geometry_path_length_getter)]
    path_length: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGAnimatedPoints", enumerable)]
struct SvgAnimatedPointsPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property = "points",
        getter = svg_animated_points_getter,
        data = callback_data_index_value(scope, 0)
    )]
    points: (),

    #[webapi(
        accessor_property = "animatedPoints",
        getter = svg_animated_points_getter,
        data = callback_data_index_value(scope, 1)
    )]
    animated_points: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGFitToViewBox", enumerable)]
struct SvgFitToViewBoxPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property = "viewBox",
        getter = svg_fit_to_view_box_getter,
        data = callback_data_index_value(scope, 0)
    )]
    view_box: (),

    #[webapi(
        accessor_property = "preserveAspectRatio",
        getter = svg_fit_to_view_box_getter,
        data = callback_data_index_value(scope, 1)
    )]
    preserve_aspect_ratio: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGImageElement", enumerable)]
struct SvgImagePreserveAspectRatioPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property = "preserveAspectRatio",
        getter = svg_fit_to_view_box_getter,
        data = callback_data_index_value(scope, 1)
    )]
    preserve_aspect_ratio: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGTextContentElement", enumerable)]
struct SvgTextContentElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "textLength", getter = svg_text_content_text_length_getter)]
    text_length: (),

    #[webapi(
        accessor_property = "lengthAdjust",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 9)
    )]
    length_adjust: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGTextPositioningElement", enumerable)]
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
#[webapi(name = "SVGPatternElement", enumerable)]
struct SvgPatternElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "x", getter = svg_pattern_animated_length_getter, data = callback_data_index_value(scope, 0))]
    x: (),

    #[webapi(accessor_property = "y", getter = svg_pattern_animated_length_getter, data = callback_data_index_value(scope, 1))]
    y: (),

    #[webapi(accessor_property = "width", getter = svg_pattern_animated_length_getter, data = callback_data_index_value(scope, 2))]
    width: (),

    #[webapi(accessor_property = "height", getter = svg_pattern_animated_length_getter, data = callback_data_index_value(scope, 3))]
    height: (),

    #[webapi(
        accessor_property = "patternUnits",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 7)
    )]
    pattern_units: (),

    #[webapi(
        accessor_property = "patternContentUnits",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 8)
    )]
    pattern_content_units: (),

    #[webapi(accessor_property = "patternTransform", getter = svg_pattern_transform_getter)]
    pattern_transform: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGGradientElement", enumerable)]
struct SvgGradientElementPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property = "gradientUnits",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 3)
    )]
    gradient_units: (),

    #[webapi(
        accessor_property = "spreadMethod",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 4)
    )]
    spread_method: (),

    #[webapi(accessor_property = "gradientTransform", getter = svg_gradient_transform_getter)]
    gradient_transform: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGLinearGradientElement", enumerable)]
struct SvgLinearGradientElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "x1", getter = svg_linear_gradient_animated_length_getter, data = callback_data_index_value(scope, 0))]
    x1: (),

    #[webapi(accessor_property = "y1", getter = svg_linear_gradient_animated_length_getter, data = callback_data_index_value(scope, 1))]
    y1: (),

    #[webapi(accessor_property = "x2", getter = svg_linear_gradient_animated_length_getter, data = callback_data_index_value(scope, 2))]
    x2: (),

    #[webapi(accessor_property = "y2", getter = svg_linear_gradient_animated_length_getter, data = callback_data_index_value(scope, 3))]
    y2: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGRadialGradientElement", enumerable)]
struct SvgRadialGradientElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "cx", getter = svg_radial_gradient_animated_length_getter, data = callback_data_index_value(scope, 0))]
    cx: (),

    #[webapi(accessor_property = "cy", getter = svg_radial_gradient_animated_length_getter, data = callback_data_index_value(scope, 1))]
    cy: (),

    #[webapi(accessor_property = "r", getter = svg_radial_gradient_animated_length_getter, data = callback_data_index_value(scope, 2))]
    r: (),

    #[webapi(accessor_property = "fx", getter = svg_radial_gradient_animated_length_getter, data = callback_data_index_value(scope, 3))]
    fx: (),

    #[webapi(accessor_property = "fy", getter = svg_radial_gradient_animated_length_getter, data = callback_data_index_value(scope, 4))]
    fy: (),

    #[webapi(accessor_property = "fr", getter = svg_radial_gradient_animated_length_getter, data = callback_data_index_value(scope, 5))]
    fr: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGSVGElement", enumerable)]
struct SvgSvgElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "x", getter = svg_svg_animated_length_getter, data = callback_data_index_value(scope, 0))]
    x: (),

    #[webapi(accessor_property = "y", getter = svg_svg_animated_length_getter, data = callback_data_index_value(scope, 1))]
    y: (),

    #[webapi(accessor_property = "width", getter = svg_svg_animated_length_getter, data = callback_data_index_value(scope, 2))]
    width: (),

    #[webapi(accessor_property = "height", getter = svg_svg_animated_length_getter, data = callback_data_index_value(scope, 3))]
    height: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGClipPathElement", enumerable)]
struct SvgClipPathElementPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property = "clipPathUnits",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 0)
    )]
    clip_path_units: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGFilterElement", enumerable)]
struct SvgFilterElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "x", getter = svg_filter_animated_length_getter, data = callback_data_index_value(scope, 0))]
    x: (),

    #[webapi(accessor_property = "y", getter = svg_filter_animated_length_getter, data = callback_data_index_value(scope, 1))]
    y: (),

    #[webapi(accessor_property = "width", getter = svg_filter_animated_length_getter, data = callback_data_index_value(scope, 2))]
    width: (),

    #[webapi(accessor_property = "height", getter = svg_filter_animated_length_getter, data = callback_data_index_value(scope, 3))]
    height: (),

    #[webapi(
        accessor_property = "filterUnits",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 1)
    )]
    filter_units: (),

    #[webapi(
        accessor_property = "primitiveUnits",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 2)
    )]
    primitive_units: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGFilterPrimitiveStandardAttributes", enumerable)]
struct SvgFilterPrimitiveStandardAttributesPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "x", getter = svg_filter_primitive_animated_length_getter, data = callback_data_index_value(scope, 0))]
    x: (),

    #[webapi(accessor_property = "y", getter = svg_filter_primitive_animated_length_getter, data = callback_data_index_value(scope, 1))]
    y: (),

    #[webapi(accessor_property = "width", getter = svg_filter_primitive_animated_length_getter, data = callback_data_index_value(scope, 2))]
    width: (),

    #[webapi(accessor_property = "height", getter = svg_filter_primitive_animated_length_getter, data = callback_data_index_value(scope, 3))]
    height: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGComponentTransferFunctionElement", enumerable)]
struct SvgComponentTransferFunctionElementPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property = "type",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 12)
    )]
    transfer_type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGFEBlendElement", enumerable)]
struct SvgFeBlendElementPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property = "mode",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 13)
    )]
    mode: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGFEColorMatrixElement", enumerable)]
struct SvgFeColorMatrixElementPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property = "type",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 14)
    )]
    color_matrix_type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGFECompositeElement", enumerable)]
struct SvgFeCompositeElementPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property = "operator",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 15)
    )]
    operator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGFEConvolveMatrixElement", enumerable)]
struct SvgFeConvolveMatrixElementPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property = "orderX",
        getter = svg_element_animated_integer_getter,
        data = callback_data_index_value(scope, 0)
    )]
    order_x: (),

    #[webapi(
        accessor_property = "orderY",
        getter = svg_element_animated_integer_getter,
        data = callback_data_index_value(scope, 1)
    )]
    order_y: (),

    #[webapi(
        accessor_property = "targetX",
        getter = svg_element_animated_integer_getter,
        data = callback_data_index_value(scope, 2)
    )]
    target_x: (),

    #[webapi(
        accessor_property = "targetY",
        getter = svg_element_animated_integer_getter,
        data = callback_data_index_value(scope, 3)
    )]
    target_y: (),

    #[webapi(
        accessor_property = "edgeMode",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 16)
    )]
    edge_mode: (),

    #[webapi(
        accessor_property = "preserveAlpha",
        getter = svg_fe_convolve_matrix_preserve_alpha_getter
    )]
    preserve_alpha: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGFEDisplacementMapElement", enumerable)]
struct SvgFeDisplacementMapElementPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property = "xChannelSelector",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 17)
    )]
    x_channel_selector: (),

    #[webapi(
        accessor_property = "yChannelSelector",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 18)
    )]
    y_channel_selector: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGFEMorphologyElement", enumerable)]
struct SvgFeMorphologyElementPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property = "operator",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 19)
    )]
    operator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGFETurbulenceElement", enumerable)]
struct SvgFeTurbulenceElementPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property = "numOctaves",
        getter = svg_element_animated_integer_getter,
        data = callback_data_index_value(scope, 4)
    )]
    num_octaves: (),

    #[webapi(
        accessor_property = "stitchTiles",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 20)
    )]
    stitch_tiles: (),

    #[webapi(
        accessor_property = "type",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 21)
    )]
    turbulence_type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGTextPathElement", enumerable)]
struct SvgTextPathElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "startOffset", getter = svg_text_path_animated_length_getter, data = callback_data_index_value(scope, 0))]
    start_offset: (),

    #[webapi(
        accessor_property = "method",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 22)
    )]
    method: (),

    #[webapi(
        accessor_property = "spacing",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 23)
    )]
    spacing: (),

    #[webapi(
        accessor_property = "side",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 24)
    )]
    side: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGMaskElement", enumerable)]
struct SvgMaskElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "x", getter = svg_mask_animated_length_getter, data = callback_data_index_value(scope, 0))]
    x: (),

    #[webapi(accessor_property = "y", getter = svg_mask_animated_length_getter, data = callback_data_index_value(scope, 1))]
    y: (),

    #[webapi(accessor_property = "width", getter = svg_mask_animated_length_getter, data = callback_data_index_value(scope, 2))]
    width: (),

    #[webapi(accessor_property = "height", getter = svg_mask_animated_length_getter, data = callback_data_index_value(scope, 3))]
    height: (),

    #[webapi(
        accessor_property = "maskUnits",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 5)
    )]
    mask_units: (),

    #[webapi(
        accessor_property = "maskContentUnits",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 6)
    )]
    mask_content_units: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGRectElement", enumerable)]
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
#[webapi(name = "SVGCircleElement", enumerable)]
struct SvgCircleElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "cx", getter = svg_circle_animated_length_getter, data = callback_data_index_value(scope, 0))]
    cx: (),

    #[webapi(accessor_property = "cy", getter = svg_circle_animated_length_getter, data = callback_data_index_value(scope, 1))]
    cy: (),

    #[webapi(accessor_property = "r", getter = svg_circle_animated_length_getter, data = callback_data_index_value(scope, 2))]
    r: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGEllipseElement", enumerable)]
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
#[webapi(name = "SVGLineElement", enumerable)]
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

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGMarkerElement", enumerable)]
struct SvgMarkerElementPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "refX", getter = svg_marker_animated_length_getter, data = callback_data_index_value(scope, 0))]
    ref_x: (),

    #[webapi(accessor_property = "refY", getter = svg_marker_animated_length_getter, data = callback_data_index_value(scope, 1))]
    ref_y: (),

    #[webapi(accessor_property = "markerWidth", getter = svg_marker_animated_length_getter, data = callback_data_index_value(scope, 2))]
    marker_width: (),

    #[webapi(accessor_property = "markerHeight", getter = svg_marker_animated_length_getter, data = callback_data_index_value(scope, 3))]
    marker_height: (),

    #[webapi(
        accessor_property = "markerUnits",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 10)
    )]
    marker_units: (),

    #[webapi(
        accessor_property = "orientType",
        getter = svg_element_animated_enumeration_getter,
        data = callback_data_index_value(scope, 11)
    )]
    orient_type: (),

    #[webapi(
        accessor_property = "orientAngle",
        getter = svg_marker_orient_angle_getter
    )]
    orient_angle: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGElement", enumerable)]
struct SvgElementPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property = "className",
        getter = svg_element_class_name_getter
    )]
    class_name: (),

    #[webapi(
        accessor_property = "ownerSVGElement",
        getter = svg_element_owner_svg_element_getter
    )]
    owner_svg_element: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGURIReference", enumerable)]
struct SvgUriReferencePrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "href", getter = svg_uri_href_getter)]
    href: (),
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

pub(super) fn install_svg_angle_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAngleTemplateMethodsDeclaration::initialize_template(scope, template);
    SvgAngleTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
    SvgAngleTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_animated_length_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAnimatedLengthTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_animated_angle_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAnimatedAngleTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_animated_rect_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAnimatedRectTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_preserve_aspect_ratio_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgPreserveAspectRatioTemplateConstantsDeclaration::initialize_template(scope, template);
    SvgPreserveAspectRatioTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
    SvgPreserveAspectRatioTemplateConstantsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_animated_preserve_aspect_ratio_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAnimatedPreserveAspectRatioTemplateAccessorsDeclaration::initialize_prototype_template(
        scope, proto,
    );
}

pub(super) fn install_svg_number_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgNumberTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_animated_string_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAnimatedStringTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_animated_boolean_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAnimatedBooleanTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
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

pub(super) fn install_svg_animated_integer_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAnimatedIntegerTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
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

pub(super) fn install_svg_point_list_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    install_svg_value_list_bindings(scope, template, SvgListKind::Point);
}

pub(super) fn install_svg_string_list_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgStringListTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
    SvgStringListTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_animated_enumeration_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgAnimatedEnumerationTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_unit_types_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    SvgUnitTypesTemplateConstantsDeclaration::initialize_template(scope, template);
    SvgUnitTypesTemplateConstantsDeclaration::initialize_prototype_template(scope, prototype);
}

pub(super) fn install_svg_gradient_element_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    SvgGradientElementTemplateConstantsDeclaration::initialize_template(scope, template);
    SvgGradientElementTemplateConstantsDeclaration::initialize_prototype_template(scope, prototype);
}

pub(super) fn install_svg_enumeration_constant_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    macro_rules! install_constants {
        ($declaration:ty) => {{
            <$declaration>::initialize_template(scope, template);
            <$declaration>::initialize_prototype_template(scope, prototype);
        }};
    }
    match interface_name {
        "SVGComponentTransferFunctionElement" => {
            install_constants!(SvgComponentTransferFunctionElementTemplateConstantsDeclaration);
        }
        "SVGFEBlendElement" => {
            install_constants!(SvgFeBlendElementTemplateConstantsDeclaration);
        }
        "SVGFEColorMatrixElement" => {
            install_constants!(SvgFeColorMatrixElementTemplateConstantsDeclaration);
        }
        "SVGFECompositeElement" => {
            install_constants!(SvgFeCompositeElementTemplateConstantsDeclaration);
        }
        "SVGFEConvolveMatrixElement" => {
            install_constants!(SvgFeConvolveMatrixElementTemplateConstantsDeclaration);
        }
        "SVGFEDisplacementMapElement" => {
            install_constants!(SvgFeDisplacementMapElementTemplateConstantsDeclaration);
        }
        "SVGFEMorphologyElement" => {
            install_constants!(SvgFeMorphologyElementTemplateConstantsDeclaration);
        }
        "SVGFETurbulenceElement" => {
            install_constants!(SvgFeTurbulenceElementTemplateConstantsDeclaration);
        }
        "SVGTextPathElement" => {
            install_constants!(SvgTextPathElementTemplateConstantsDeclaration);
        }
        _ => {}
    }
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
        SvgListKind::Point => {
            SvgPointListTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
            SvgPointListTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
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

pub(super) fn install_svg_marker_element_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgMarkerElementTemplateMethodsDeclaration::initialize_template(scope, template);
    SvgMarkerElementTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_svg_svg_element_bindings(
    scope: &mut v8::PinScope<'_, '_, ()>,
    template: v8::Local<'_, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    SvgSvgElementTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

fn install_svg_animated_number_element_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface_name: &str,
) {
    macro_rules! install_accessors {
        ($declaration:ty) => {
            <$declaration>::initialize_prototype_template(scope, prototype)
        };
    }
    match interface_name {
        "SVGComponentTransferFunctionElement" => {
            install_accessors!(SvgComponentTransferFunctionAnimatedNumberAccessorsDeclaration);
        }
        "SVGFECompositeElement" => {
            install_accessors!(SvgFeCompositeAnimatedNumberAccessorsDeclaration);
        }
        "SVGFEConvolveMatrixElement" => {
            install_accessors!(SvgFeConvolveMatrixAnimatedNumberAccessorsDeclaration);
        }
        "SVGFEDiffuseLightingElement" => {
            install_accessors!(SvgFeDiffuseLightingAnimatedNumberAccessorsDeclaration);
        }
        "SVGFEDisplacementMapElement" => {
            install_accessors!(SvgFeDisplacementMapAnimatedNumberAccessorsDeclaration);
        }
        "SVGFEDistantLightElement" => {
            install_accessors!(SvgFeDistantLightAnimatedNumberAccessorsDeclaration);
        }
        "SVGFEDropShadowElement" => {
            install_accessors!(SvgFeDropShadowAnimatedNumberAccessorsDeclaration);
        }
        "SVGFEGaussianBlurElement" => {
            install_accessors!(SvgFeGaussianBlurAnimatedNumberAccessorsDeclaration);
        }
        "SVGFEMorphologyElement" => {
            install_accessors!(SvgFeMorphologyAnimatedNumberAccessorsDeclaration);
        }
        "SVGFEOffsetElement" => {
            install_accessors!(SvgFeOffsetAnimatedNumberAccessorsDeclaration);
        }
        "SVGFEPointLightElement" => {
            install_accessors!(SvgFePointLightAnimatedNumberAccessorsDeclaration);
        }
        "SVGFESpecularLightingElement" => {
            install_accessors!(SvgFeSpecularLightingAnimatedNumberAccessorsDeclaration);
        }
        "SVGFESpotLightElement" => {
            install_accessors!(SvgFeSpotLightAnimatedNumberAccessorsDeclaration);
        }
        "SVGFETurbulenceElement" => {
            install_accessors!(SvgFeTurbulenceAnimatedNumberAccessorsDeclaration);
        }
        "SVGStopElement" => {
            install_accessors!(SvgStopAnimatedNumberAccessorsDeclaration);
        }
        _ => {}
    }
}

pub(super) fn install_svg_element_accessor_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    install_svg_animated_number_element_bindings(scope, prototype, interface_name);
    if matches!(
        interface_name,
        "SVGSVGElement"
            | "SVGSymbolElement"
            | "SVGMarkerElement"
            | "SVGPatternElement"
            | "SVGViewElement"
    ) {
        SvgFitToViewBoxPrototypeAccessorsDeclaration::initialize_prototype_template(
            scope, prototype,
        );
    }
    if matches!(
        interface_name,
        "SVGFEBlendElement"
            | "SVGFEColorMatrixElement"
            | "SVGFECompositeElement"
            | "SVGFEConvolveMatrixElement"
            | "SVGFEDiffuseLightingElement"
            | "SVGFEDisplacementMapElement"
            | "SVGFEDropShadowElement"
            | "SVGFEGaussianBlurElement"
            | "SVGFEMorphologyElement"
            | "SVGFEOffsetElement"
            | "SVGFESpecularLightingElement"
            | "SVGFETurbulenceElement"
    ) {
        SvgFilterPrimitiveStandardAttributesPrototypeAccessorsDeclaration::initialize_prototype_template(
            scope, prototype,
        );
    }
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
        "SVGClipPathElement" => {
            SvgClipPathElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGFilterElement" => {
            SvgFilterElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGComponentTransferFunctionElement" => {
            SvgComponentTransferFunctionElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGFEBlendElement" => {
            SvgFeBlendElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGFEColorMatrixElement" => {
            SvgFeColorMatrixElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGFECompositeElement" => {
            SvgFeCompositeElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGFEConvolveMatrixElement" => {
            SvgFeConvolveMatrixElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGFEDisplacementMapElement" => {
            SvgFeDisplacementMapElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGFEMorphologyElement" => {
            SvgFeMorphologyElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGFETurbulenceElement" => {
            SvgFeTurbulenceElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGMaskElement" => {
            SvgMaskElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGTextPositioningElement" => {
            SvgTextPositioningElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGTextPathElement" => {
            SvgTextPathElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            SvgUriReferencePrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGPatternElement" => {
            SvgPatternElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            SvgUriReferencePrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGGradientElement" => {
            SvgGradientElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGLinearGradientElement" => {
            SvgLinearGradientElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            SvgUriReferencePrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGRadialGradientElement" => {
            SvgRadialGradientElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            SvgUriReferencePrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGSVGElement" => {
            SvgSvgElementPrototypeAccessorsDeclaration::initialize_prototype_template(
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
        "SVGPolygonElement" | "SVGPolylineElement" => {
            SvgAnimatedPointsPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGMarkerElement" => {
            SvgMarkerElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGElement" => {
            SvgElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGAElement" | "SVGScriptElement" => {
            SvgUriReferencePrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SVGImageElement" | "SVGUseElement" => {
            SvgGraphicsBoxElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            SvgUriReferencePrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            if interface_name == "SVGImageElement" {
                SvgImagePreserveAspectRatioPrototypeAccessorsDeclaration::initialize_prototype_template(
                    scope, prototype,
                );
            }
        }
        "SVGForeignObjectElement" => {
            SvgGraphicsBoxElementPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}
