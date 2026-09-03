use super::{
    build_dom_point_object, build_dom_rect_object,
    geometry_runtime::{
        build_svg_point_object_with_values, dom_point_clone_data, dom_point_init_from_object,
        set_svg_point_read_only,
    },
    optional_dom_point_init_arg,
    selection::{selection_clear, selection_dispatch_change, selection_has_range},
    selection_value_for_window,
};
use crate::{
    native_bridge::throw_dom_exception,
    util::{callback_data_item, get_private_value, set_private_value, v8_string, v8str},
    webidl,
};
use moli_svg::{
    self as svg_geometry, SvgGeometryBox, SvgGeometryElement, SvgGeometryPoint, SvgGeometrySegment,
    SvgLength, SvgLengthUnit, SvgMatrixComponents, SvgTransform, SvgTransformKind,
};

mod bindings;
mod builders;
mod callbacks;

const SVG_GRAPHICS_TRANSFORM_SLOT: &str = "__moliSvgGraphicsTransform";
const SVG_GRAPHICS_REQUIRED_EXTENSIONS_SLOT: &str = "__moliSvgGraphicsRequiredExtensions";
const SVG_GRAPHICS_SYSTEM_LANGUAGE_SLOT: &str = "__moliSvgGraphicsSystemLanguage";
const SVG_PATTERN_TRANSFORM_SLOT: &str = "__moliSvgPatternTransform";
const SVG_GRADIENT_TRANSFORM_SLOT: &str = "__moliSvgGradientTransform";
const SVG_GEOMETRY_PATH_LENGTH_SLOT: &str = "__moliSvgGeometryPathLength";
const SVG_ELEMENT_CLASS_NAME_SLOT: &str = "__moliSvgElementClassName";
const SVG_URI_HREF_SLOT: &str = "__moliSvgUriHref";
const SVG_ANIMATED_STRING_BASE_VAL_SLOT: &str = "__moliSvgAnimatedStringBaseVal";
const SVG_ANIMATED_STRING_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedStringAnimVal";
const SVG_ANIMATED_STRING_OWNER_ELEMENT_SLOT: &str = "__moliSvgAnimatedStringOwnerElement";
const SVG_ANIMATED_STRING_OWNER_ATTRIBUTE_SLOT: &str = "__moliSvgAnimatedStringOwnerAttribute";
const SVG_ANIMATED_BOOLEAN_BASE_VAL_SLOT: &str = "__moliSvgAnimatedBooleanBaseVal";
const SVG_ANIMATED_BOOLEAN_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedBooleanAnimVal";
const SVG_ANIMATED_BOOLEAN_OWNER_ELEMENT_SLOT: &str = "__moliSvgAnimatedBooleanOwnerElement";
const SVG_ANIMATED_BOOLEAN_OWNER_ATTRIBUTE_SLOT: &str = "__moliSvgAnimatedBooleanOwnerAttribute";
const SVG_ANIMATED_BOOLEAN_INITIAL_VALUE_SLOT: &str = "__moliSvgAnimatedBooleanInitialValue";
const SVG_FE_CONVOLVE_MATRIX_PRESERVE_ALPHA_SLOT: &str = "__moliSvgFeConvolveMatrixPreserveAlpha";
const SVG_ANIMATED_LENGTH_BASE_VAL_SLOT: &str = "__moliSvgAnimatedLengthBaseVal";
const SVG_ANIMATED_LENGTH_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedLengthAnimVal";
const SVG_MARKER_ORIENT_ANGLE_SLOT: &str = "__moliSvgMarkerOrientAngle";
const SVG_MARKER_UNITS_SLOT: &str = "__moliSvgMarkerUnits";
const SVG_MARKER_ORIENT_TYPE_SLOT: &str = "__moliSvgMarkerOrientType";
const SVG_ANIMATED_ANGLE_BASE_VAL_SLOT: &str = "__moliSvgAnimatedAngleBaseVal";
const SVG_ANIMATED_ANGLE_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedAngleAnimVal";
const SVG_FIT_VIEW_BOX_SLOT: &str = "__moliSvgFitViewBox";
const SVG_FIT_PRESERVE_ASPECT_RATIO_SLOT: &str = "__moliSvgFitPreserveAspectRatio";
const SVG_ANIMATED_RECT_BASE_VAL_SLOT: &str = "__moliSvgAnimatedRectBaseVal";
const SVG_ANIMATED_RECT_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedRectAnimVal";
const SVG_ANIMATED_PRESERVE_ASPECT_RATIO_BASE_VAL_SLOT: &str =
    "__moliSvgAnimatedPreserveAspectRatioBaseVal";
const SVG_ANIMATED_PRESERVE_ASPECT_RATIO_ANIM_VAL_SLOT: &str =
    "__moliSvgAnimatedPreserveAspectRatioAnimVal";
const SVG_PRESERVE_ASPECT_RATIO_ALIGN_SLOT: &str = "__moliSvgPreserveAspectRatioAlign";
const SVG_PRESERVE_ASPECT_RATIO_MEET_OR_SLICE_SLOT: &str =
    "__moliSvgPreserveAspectRatioMeetOrSlice";
const SVG_PRESERVE_ASPECT_RATIO_OWNER_ELEMENT_SLOT: &str =
    "__moliSvgPreserveAspectRatioOwnerElement";
const SVG_PRESERVE_ASPECT_RATIO_READ_ONLY_SLOT: &str = "__moliSvgPreserveAspectRatioReadOnly";
const SVG_ANGLE_OWNER_ELEMENT_SLOT: &str = "__moliSvgAngleOwnerElement";
const SVG_ANGLE_OWNER_ATTRIBUTE_SLOT: &str = "__moliSvgAngleOwnerAttribute";
const SVG_ANGLE_UNIT_TYPE_SLOT: &str = "__moliSvgAngleUnitType";
const SVG_ANGLE_VALUE_SLOT: &str = "__moliSvgAngleValue";
const SVG_ANGLE_VALUE_IN_SPECIFIED_UNITS_SLOT: &str = "__moliSvgAngleValueInSpecifiedUnits";
const SVG_ANGLE_VALUE_AS_STRING_SLOT: &str = "__moliSvgAngleValueAsString";
const SVG_ANGLE_READ_ONLY_SLOT: &str = "__moliSvgAngleReadOnly";
const SVG_ANIMATED_LENGTH_LIST_BASE_VAL_SLOT: &str = "__moliSvgAnimatedLengthListBaseVal";
const SVG_ANIMATED_LENGTH_LIST_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedLengthListAnimVal";
const SVG_LENGTH_LIST_ITEMS_SLOT: &str = "__moliSvgLengthListItems";
const SVG_LENGTH_OWNER_ELEMENT_SLOT: &str = "__moliSvgLengthOwnerElement";
const SVG_LENGTH_OWNER_ATTRIBUTE_SLOT: &str = "__moliSvgLengthOwnerAttribute";
const SVG_LENGTH_UNIT_TYPE_SLOT: &str = "__moliSvgLengthUnitType";
const SVG_LENGTH_UNIT_SUFFIX_SLOT: &str = "__moliSvgLengthUnitSuffix";
const SVG_LENGTH_VALUE_SLOT: &str = "__moliSvgLengthValue";
const SVG_LENGTH_VALUE_IN_SPECIFIED_UNITS_SLOT: &str = "__moliSvgLengthValueInSpecifiedUnits";
const SVG_LENGTH_VALUE_AS_STRING_SLOT: &str = "__moliSvgLengthValueAsString";
const SVG_NUMBER_VALUE_SLOT: &str = "__moliSvgNumberValue";
const SVG_ANIMATED_NUMBER_OWNER_ELEMENT_SLOT: &str = "__moliSvgAnimatedNumberOwnerElement";
const SVG_ANIMATED_NUMBER_OWNER_ATTRIBUTE_SLOT: &str = "__moliSvgAnimatedNumberOwnerAttribute";
const SVG_ANIMATED_NUMBER_PROPERTY_INDEX_SLOT: &str = "__moliSvgAnimatedNumberPropertyIndex";
const SVG_ANIMATED_NUMBER_BASE_VAL_SLOT: &str = "__moliSvgAnimatedNumberBaseVal";
const SVG_ANIMATED_NUMBER_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedNumberAnimVal";
const SVG_ANIMATED_INTEGER_BASE_VAL_SLOT: &str = "__moliSvgAnimatedIntegerBaseVal";
const SVG_ANIMATED_INTEGER_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedIntegerAnimVal";
const SVG_ANIMATED_INTEGER_OWNER_ELEMENT_SLOT: &str = "__moliSvgAnimatedIntegerOwnerElement";
const SVG_ANIMATED_INTEGER_PROPERTY_INDEX_SLOT: &str = "__moliSvgAnimatedIntegerPropertyIndex";
const SVG_ANIMATED_NUMBER_LIST_BASE_VAL_SLOT: &str = "__moliSvgAnimatedNumberListBaseVal";
const SVG_ANIMATED_NUMBER_LIST_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedNumberListAnimVal";
const SVG_NUMBER_LIST_ITEMS_SLOT: &str = "__moliSvgNumberListItems";
const SVG_POINT_LIST_ITEMS_SLOT: &str = "__moliSvgPointListItems";
const SVG_POINTS_SLOT: &str = "__moliSvgPoints";
const SVG_ANIMATED_POINTS_SLOT: &str = "__moliSvgAnimatedPoints";
const SVG_STRING_LIST_ITEMS_SLOT: &str = "__moliSvgStringListItems";
const SVG_STRING_LIST_OWNER_ELEMENT_SLOT: &str = "__moliSvgStringListOwnerElement";
const SVG_STRING_LIST_OWNER_ATTRIBUTE_SLOT: &str = "__moliSvgStringListOwnerAttribute";
const SVG_STRING_LIST_SYNCED_ATTRIBUTE_VALUE_SLOT: &str = "__moliSvgStringListSyncedAttributeValue";
const SVG_STRING_LIST_SYNCED_ATTRIBUTE_PRESENT_SLOT: &str =
    "__moliSvgStringListSyncedAttributePresent";
const SVG_VALUE_LIST_OWNER_ELEMENT_SLOT: &str = "__moliSvgValueListOwnerElement";
const SVG_VALUE_LIST_OWNER_ATTRIBUTE_SLOT: &str = "__moliSvgValueListOwnerAttribute";
const SVG_VALUE_LIST_ITEM_OWNER_LIST_SLOT: &str = "__moliSvgValueListItemOwnerList";
const SVG_VALUE_LIST_READ_ONLY_SLOT: &str = "__moliSvgValueListReadOnly";
const SVG_VALUE_LIST_SYNCED_ATTRIBUTE_VALUE_SLOT: &str = "__moliSvgValueListSyncedAttributeValue";
const SVG_ANIMATED_ENUMERATION_BASE_VAL_SLOT: &str = "__moliSvgAnimatedEnumerationBaseVal";
const SVG_ANIMATED_ENUMERATION_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedEnumerationAnimVal";
const SVG_ANIMATED_ENUMERATION_OWNER_ELEMENT_SLOT: &str =
    "__moliSvgAnimatedEnumerationOwnerElement";
const SVG_ANIMATED_ENUMERATION_PROPERTY_INDEX_SLOT: &str =
    "__moliSvgAnimatedEnumerationPropertyIndex";
const SVG_CLIP_PATH_UNITS_SLOT: &str = "__moliSvgClipPathUnits";
const SVG_FILTER_UNITS_SLOT: &str = "__moliSvgFilterUnits";
const SVG_PRIMITIVE_UNITS_SLOT: &str = "__moliSvgPrimitiveUnits";
const SVG_GRADIENT_UNITS_SLOT: &str = "__moliSvgGradientUnits";
const SVG_GRADIENT_SPREAD_METHOD_SLOT: &str = "__moliSvgGradientSpreadMethod";
const SVG_MASK_UNITS_SLOT: &str = "__moliSvgMaskUnits";
const SVG_MASK_CONTENT_UNITS_SLOT: &str = "__moliSvgMaskContentUnits";
const SVG_PATTERN_UNITS_SLOT: &str = "__moliSvgPatternUnits";
const SVG_PATTERN_CONTENT_UNITS_SLOT: &str = "__moliSvgPatternContentUnits";
const SVG_COMPONENT_TRANSFER_TYPE_SLOT: &str = "__moliSvgComponentTransferType";
const SVG_FE_BLEND_MODE_SLOT: &str = "__moliSvgFeBlendMode";
const SVG_FE_COLOR_MATRIX_TYPE_SLOT: &str = "__moliSvgFeColorMatrixType";
const SVG_FE_COMPOSITE_OPERATOR_SLOT: &str = "__moliSvgFeCompositeOperator";
const SVG_FE_CONVOLVE_MATRIX_EDGE_MODE_SLOT: &str = "__moliSvgFeConvolveMatrixEdgeMode";
const SVG_FE_CONVOLVE_MATRIX_ORDER_X_SLOT: &str = "__moliSvgFeConvolveMatrixOrderX";
const SVG_FE_CONVOLVE_MATRIX_ORDER_Y_SLOT: &str = "__moliSvgFeConvolveMatrixOrderY";
const SVG_FE_CONVOLVE_MATRIX_TARGET_X_SLOT: &str = "__moliSvgFeConvolveMatrixTargetX";
const SVG_FE_CONVOLVE_MATRIX_TARGET_Y_SLOT: &str = "__moliSvgFeConvolveMatrixTargetY";
const SVG_FE_DISPLACEMENT_MAP_X_CHANNEL_SLOT: &str = "__moliSvgFeDisplacementMapXChannel";
const SVG_FE_DISPLACEMENT_MAP_Y_CHANNEL_SLOT: &str = "__moliSvgFeDisplacementMapYChannel";
const SVG_FE_MORPHOLOGY_OPERATOR_SLOT: &str = "__moliSvgFeMorphologyOperator";
const SVG_FE_TURBULENCE_STITCH_TILES_SLOT: &str = "__moliSvgFeTurbulenceStitchTiles";
const SVG_FE_TURBULENCE_TYPE_SLOT: &str = "__moliSvgFeTurbulenceType";
const SVG_FE_TURBULENCE_NUM_OCTAVES_SLOT: &str = "__moliSvgFeTurbulenceNumOctaves";
const SVG_TEXT_PATH_METHOD_SLOT: &str = "__moliSvgTextPathMethod";
const SVG_TEXT_PATH_SPACING_SLOT: &str = "__moliSvgTextPathSpacing";
const SVG_TEXT_PATH_SIDE_SLOT: &str = "__moliSvgTextPathSide";
const SVG_ANIMATED_TRANSFORM_LIST_BASE_VAL_SLOT: &str = "__moliSvgAnimatedTransformListBaseVal";
const SVG_ANIMATED_TRANSFORM_LIST_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedTransformListAnimVal";
const SVG_TRANSFORM_LIST_ITEMS_SLOT: &str = "__moliSvgTransformListItems";
const SVG_TRANSFORM_LIST_OWNER_ELEMENT_SLOT: &str = "__moliSvgTransformListOwnerElement";
const SVG_TRANSFORM_LIST_OWNER_ATTRIBUTE_SLOT: &str = "__moliSvgTransformListOwnerAttribute";
const SVG_TRANSFORM_LIST_ITEM_OWNER_LIST_SLOT: &str = "__moliSvgTransformListItemOwnerList";
const SVG_TRANSFORM_LIST_SYNCED_ATTRIBUTE_VALUE_SLOT: &str =
    "__moliSvgTransformListSyncedAttributeValue";
const SVG_TRANSFORM_TYPE_SLOT: &str = "__moliSvgTransformType";
const SVG_TRANSFORM_ANGLE_SLOT: &str = "__moliSvgTransformAngle";
const SVG_TRANSFORM_MATRIX_SLOT: &str = "__moliSvgTransformMatrix";
const SVG_MATRIX_A_SLOT: &str = "__moliSvgMatrixA";
const SVG_MATRIX_B_SLOT: &str = "__moliSvgMatrixB";
const SVG_MATRIX_C_SLOT: &str = "__moliSvgMatrixC";
const SVG_MATRIX_D_SLOT: &str = "__moliSvgMatrixD";
const SVG_MATRIX_E_SLOT: &str = "__moliSvgMatrixE";
const SVG_MATRIX_F_SLOT: &str = "__moliSvgMatrixF";
const SVG_TEXT_CONTENT_TEXT_LENGTH_SLOT: &str = "__moliSvgTextContentTextLength";
const SVG_TEXT_CONTENT_LENGTH_ADJUST_SLOT: &str = "__moliSvgTextContentLengthAdjust";
const SVG_TEXT_POSITIONING_X_SLOT: &str = "__moliSvgTextPositioningX";
const SVG_TEXT_POSITIONING_Y_SLOT: &str = "__moliSvgTextPositioningY";
const SVG_TEXT_POSITIONING_DX_SLOT: &str = "__moliSvgTextPositioningDx";
const SVG_TEXT_POSITIONING_DY_SLOT: &str = "__moliSvgTextPositioningDy";
const SVG_TEXT_POSITIONING_ROTATE_SLOT: &str = "__moliSvgTextPositioningRotate";

const SVG_TEST_STRING_LIST_ATTRIBUTES: &[(&str, &str)] = &[
    ("requiredExtensions", SVG_GRAPHICS_REQUIRED_EXTENSIONS_SLOT),
    ("systemLanguage", SVG_GRAPHICS_SYSTEM_LANGUAGE_SLOT),
];

#[derive(Clone, Copy)]
enum SvgListKind {
    Length,
    Number,
    Point,
}

#[derive(Clone, Copy)]
enum SvgAnimatedEnumerationKind {
    Keywords(&'static [(&'static str, u32)]),
    MarkerOrient,
}

#[derive(Clone, Copy)]
struct SvgAnimatedEnumerationProperty {
    index: usize,
    attribute: &'static str,
    cache_slot: &'static str,
    initial_value: u32,
    kind: SvgAnimatedEnumerationKind,
}

#[derive(Clone, Copy)]
enum SvgAnimatedIntegerComponent {
    Scalar,
    PairFirst,
    PairSecondOrFirst,
}

#[derive(Clone, Copy)]
struct SvgAnimatedIntegerProperty {
    index: usize,
    interface: &'static str,
    local_name: &'static str,
    name: &'static str,
    attribute: &'static str,
    cache_slot: &'static str,
    initial_value: i32,
    component: SvgAnimatedIntegerComponent,
}

#[derive(Clone, Copy)]
enum SvgAnimatedNumberComponent {
    Scalar,
    PairFirst,
    PairSecondOrFirst,
    NumberOrPercentage,
}

#[derive(Clone, Copy)]
struct SvgAnimatedNumberProperty {
    index: usize,
    interface: &'static str,
    name: &'static str,
    attribute: &'static str,
    initial_value: f64,
    component: SvgAnimatedNumberComponent,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG list item")]
struct SvgListItemArgs<'s> {
    #[webidl(required)]
    item: v8::Local<'s, v8::Value>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG list index")]
struct SvgListIndexArgs {
    #[webidl(required)]
    index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG list item/index")]
struct SvgListItemIndexArgs<'s> {
    #[webidl(required)]
    item: v8::Local<'s, v8::Value>,
    #[webidl(required)]
    index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG matrix")]
struct SvgMatrixArg<'s> {
    #[webidl(required)]
    matrix: v8::Local<'s, v8::Value>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG transform translate")]
struct SvgTransformTranslateArgs {
    #[webidl(required, converter = "double")]
    tx: f64,
    #[webidl(required, converter = "double")]
    ty: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG transform scale")]
struct SvgTransformScaleArgs {
    #[webidl(required, converter = "double")]
    sx: f64,
    #[webidl(required, converter = "double")]
    sy: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG transform rotate")]
struct SvgTransformRotateArgs {
    #[webidl(required, converter = "double")]
    angle: f64,
    #[webidl(required, converter = "double")]
    cx: f64,
    #[webidl(required, converter = "double")]
    cy: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG angle")]
struct SvgAngleArg {
    #[webidl(required, converter = "double")]
    angle: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG matrix translate")]
struct SvgMatrixTranslateArgs {
    #[webidl(required, converter = "double")]
    x: f64,
    #[webidl(required, converter = "double")]
    y: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG matrix scale")]
struct SvgMatrixScaleArg {
    #[webidl(required, converter = "double")]
    scale_factor: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG matrix scaleNonUniform")]
struct SvgMatrixScaleNonUniformArgs {
    #[webidl(required, converter = "double")]
    scale_factor_x: f64,
    #[webidl(required, converter = "double")]
    scale_factor_y: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG matrix rotateFromVector")]
struct SvgMatrixRotateFromVectorArgs {
    #[webidl(required, converter = "double")]
    x: f64,
    #[webidl(required, converter = "double")]
    y: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG length newValueSpecifiedUnits")]
struct SvgLengthNewValueSpecifiedUnitsArgs {
    #[webidl(required, converter = "unsigned_short")]
    unit_type: u16,
    #[webidl(required, converter = "double")]
    value: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG length convertToSpecifiedUnits")]
struct SvgLengthConvertToSpecifiedUnitsArgs {
    #[webidl(required, converter = "unsigned_short")]
    unit_type: u16,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG angle newValueSpecifiedUnits")]
struct SvgAngleNewValueSpecifiedUnitsArgs {
    #[webidl(required, converter = "unsigned_short")]
    unit_type: u16,
    #[webidl(required, converter = "double")]
    value: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG angle convertToSpecifiedUnits")]
struct SvgAngleConvertToSpecifiedUnitsArgs {
    #[webidl(required, converter = "unsigned_short")]
    unit_type: u16,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVGGeometryElement.getPointAtLength")]
struct SvgGeometryPointAtLengthArgs {
    #[webidl(required, converter = "double")]
    distance: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVGTextContentElement character index")]
struct SvgTextCharacterIndexArgs {
    #[webidl(required, converter = "unsigned_long")]
    charnum: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVGTextContentElement substring")]
struct SvgTextSubstringArgs {
    #[webidl(required, converter = "unsigned_long")]
    charnum: u32,
    #[webidl(required, converter = "unsigned_long")]
    nchars: u32,
}

const SVG_LENGTH_TYPE_UNKNOWN: u32 = 0;
const SVG_LENGTH_TYPE_NUMBER: u32 = 1;
const SVG_LENGTH_TYPE_PERCENTAGE: u32 = 2;
const SVG_LENGTH_TYPE_EMS: u32 = 3;
const SVG_LENGTH_TYPE_EXS: u32 = 4;
const SVG_LENGTH_TYPE_PX: u32 = 5;
const SVG_LENGTH_TYPE_CM: u32 = 6;
const SVG_LENGTH_TYPE_MM: u32 = 7;
const SVG_LENGTH_TYPE_IN: u32 = 8;
const SVG_LENGTH_TYPE_PT: u32 = 9;
const SVG_LENGTH_TYPE_PC: u32 = 10;

const SVG_ANGLE_TYPE_UNKNOWN: u32 = 0;
const SVG_ANGLE_TYPE_UNSPECIFIED: u32 = 1;
const SVG_ANGLE_TYPE_DEG: u32 = 2;
const SVG_ANGLE_TYPE_RAD: u32 = 3;
const SVG_ANGLE_TYPE_GRAD: u32 = 4;

const SVG_PRESERVE_ASPECT_RATIO_UNKNOWN: u32 = 0;
const SVG_PRESERVE_ASPECT_RATIO_NONE: u32 = 1;
const SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MIN: u32 = 2;
const SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MIN: u32 = 3;
const SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MIN: u32 = 4;
const SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MID: u32 = 5;
const SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MID: u32 = 6;
const SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MID: u32 = 7;
const SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MAX: u32 = 8;
const SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MAX: u32 = 9;
const SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MAX: u32 = 10;

const SVG_MEET_OR_SLICE_UNKNOWN: u32 = 0;
const SVG_MEET_OR_SLICE_MEET: u32 = 1;
const SVG_MEET_OR_SLICE_SLICE: u32 = 2;

const SVG_MARKER_UNITS_UNKNOWN: u32 = 0;
const SVG_MARKER_UNITS_USER_SPACE_ON_USE: u32 = 1;
const SVG_MARKER_UNITS_STROKE_WIDTH: u32 = 2;

const SVG_MARKER_ORIENT_UNKNOWN: u32 = 0;
const SVG_MARKER_ORIENT_AUTO: u32 = 1;
const SVG_MARKER_ORIENT_ANGLE: u32 = 2;
const SVG_MARKER_ORIENT_AUTO_START_REVERSE: u32 = 3;

const SVG_TRANSFORM_TYPE_UNKNOWN: u32 = 0;
const SVG_TRANSFORM_TYPE_MATRIX: u32 = 1;
const SVG_TRANSFORM_TYPE_TRANSLATE: u32 = 2;
const SVG_TRANSFORM_TYPE_SCALE: u32 = 3;
const SVG_TRANSFORM_TYPE_ROTATE: u32 = 4;
const SVG_TRANSFORM_TYPE_SKEWX: u32 = 5;
const SVG_TRANSFORM_TYPE_SKEWY: u32 = 6;

const SVG_LENGTH_ADJUST_UNKNOWN: u32 = 0;
const SVG_LENGTH_ADJUST_SPACING: u32 = 1;
const SVG_LENGTH_ADJUST_SPACING_AND_GLYPHS: u32 = 2;

const SVG_UNIT_TYPE_UNKNOWN: u32 = 0;
const SVG_UNIT_TYPE_USER_SPACE_ON_USE: u32 = 1;
const SVG_UNIT_TYPE_OBJECT_BOUNDING_BOX: u32 = 2;

const SVG_SPREAD_METHOD_UNKNOWN: u32 = 0;
const SVG_SPREAD_METHOD_PAD: u32 = 1;
const SVG_SPREAD_METHOD_REFLECT: u32 = 2;
const SVG_SPREAD_METHOD_REPEAT: u32 = 3;

const SVG_COMPONENT_TRANSFER_TYPE_UNKNOWN: u32 = 0;
const SVG_COMPONENT_TRANSFER_TYPE_IDENTITY: u32 = 1;
const SVG_COMPONENT_TRANSFER_TYPE_TABLE: u32 = 2;
const SVG_COMPONENT_TRANSFER_TYPE_DISCRETE: u32 = 3;
const SVG_COMPONENT_TRANSFER_TYPE_LINEAR: u32 = 4;
const SVG_COMPONENT_TRANSFER_TYPE_GAMMA: u32 = 5;

const SVG_FE_BLEND_MODE_UNKNOWN: u32 = 0;
const SVG_FE_BLEND_MODE_NORMAL: u32 = 1;
const SVG_FE_BLEND_MODE_MULTIPLY: u32 = 2;
const SVG_FE_BLEND_MODE_SCREEN: u32 = 3;
const SVG_FE_BLEND_MODE_DARKEN: u32 = 4;
const SVG_FE_BLEND_MODE_LIGHTEN: u32 = 5;
const SVG_FE_BLEND_MODE_OVERLAY: u32 = 6;
const SVG_FE_BLEND_MODE_COLOR_DODGE: u32 = 7;
const SVG_FE_BLEND_MODE_COLOR_BURN: u32 = 8;
const SVG_FE_BLEND_MODE_HARD_LIGHT: u32 = 9;
const SVG_FE_BLEND_MODE_SOFT_LIGHT: u32 = 10;
const SVG_FE_BLEND_MODE_DIFFERENCE: u32 = 11;
const SVG_FE_BLEND_MODE_EXCLUSION: u32 = 12;
const SVG_FE_BLEND_MODE_HUE: u32 = 13;
const SVG_FE_BLEND_MODE_SATURATION: u32 = 14;
const SVG_FE_BLEND_MODE_COLOR: u32 = 15;
const SVG_FE_BLEND_MODE_LUMINOSITY: u32 = 16;

const SVG_FE_COLOR_MATRIX_TYPE_UNKNOWN: u32 = 0;
const SVG_FE_COLOR_MATRIX_TYPE_MATRIX: u32 = 1;
const SVG_FE_COLOR_MATRIX_TYPE_SATURATE: u32 = 2;
const SVG_FE_COLOR_MATRIX_TYPE_HUE_ROTATE: u32 = 3;
const SVG_FE_COLOR_MATRIX_TYPE_LUMINANCE_TO_ALPHA: u32 = 4;

const SVG_FE_COMPOSITE_OPERATOR_UNKNOWN: u32 = 0;
const SVG_FE_COMPOSITE_OPERATOR_OVER: u32 = 1;
const SVG_FE_COMPOSITE_OPERATOR_IN: u32 = 2;
const SVG_FE_COMPOSITE_OPERATOR_OUT: u32 = 3;
const SVG_FE_COMPOSITE_OPERATOR_ATOP: u32 = 4;
const SVG_FE_COMPOSITE_OPERATOR_XOR: u32 = 5;
const SVG_FE_COMPOSITE_OPERATOR_LIGHTER: u32 = 6;
const SVG_FE_COMPOSITE_OPERATOR_ARITHMETIC: u32 = 7;

const SVG_EDGE_MODE_UNKNOWN: u32 = 0;
const SVG_EDGE_MODE_DUPLICATE: u32 = 1;
const SVG_EDGE_MODE_WRAP: u32 = 2;
const SVG_EDGE_MODE_NONE: u32 = 3;

const SVG_CHANNEL_UNKNOWN: u32 = 0;
const SVG_CHANNEL_R: u32 = 1;
const SVG_CHANNEL_G: u32 = 2;
const SVG_CHANNEL_B: u32 = 3;
const SVG_CHANNEL_A: u32 = 4;

const SVG_MORPHOLOGY_OPERATOR_UNKNOWN: u32 = 0;
const SVG_MORPHOLOGY_OPERATOR_ERODE: u32 = 1;
const SVG_MORPHOLOGY_OPERATOR_DILATE: u32 = 2;

const SVG_TURBULENCE_TYPE_UNKNOWN: u32 = 0;
const SVG_TURBULENCE_TYPE_FRACTAL_NOISE: u32 = 1;
const SVG_TURBULENCE_TYPE_TURBULENCE: u32 = 2;

const SVG_STITCH_TYPE_UNKNOWN: u32 = 0;
const SVG_STITCH_TYPE_STITCH: u32 = 1;
const SVG_STITCH_TYPE_NO_STITCH: u32 = 2;

const SVG_TEXT_PATH_METHOD_TYPE_UNKNOWN: u32 = 0;
const SVG_TEXT_PATH_METHOD_TYPE_ALIGN: u32 = 1;
const SVG_TEXT_PATH_METHOD_TYPE_STRETCH: u32 = 2;

const SVG_TEXT_PATH_SPACING_TYPE_UNKNOWN: u32 = 0;
const SVG_TEXT_PATH_SPACING_TYPE_AUTO: u32 = 1;
const SVG_TEXT_PATH_SPACING_TYPE_EXACT: u32 = 2;

const SVG_TEXT_PATH_SIDE_TYPE_UNKNOWN: u32 = 0;
const SVG_TEXT_PATH_SIDE_TYPE_LEFT: u32 = 1;
const SVG_TEXT_PATH_SIDE_TYPE_RIGHT: u32 = 2;

const SVG_UNIT_TYPE_VALUES: &[(&str, u32)] = &[
    ("userSpaceOnUse", SVG_UNIT_TYPE_USER_SPACE_ON_USE),
    ("objectBoundingBox", SVG_UNIT_TYPE_OBJECT_BOUNDING_BOX),
];
const SVG_SPREAD_METHOD_VALUES: &[(&str, u32)] = &[
    ("pad", SVG_SPREAD_METHOD_PAD),
    ("reflect", SVG_SPREAD_METHOD_REFLECT),
    ("repeat", SVG_SPREAD_METHOD_REPEAT),
];
const SVG_LENGTH_ADJUST_VALUES: &[(&str, u32)] = &[
    ("spacing", SVG_LENGTH_ADJUST_SPACING),
    ("spacingAndGlyphs", SVG_LENGTH_ADJUST_SPACING_AND_GLYPHS),
];
const SVG_MARKER_UNITS_VALUES: &[(&str, u32)] = &[
    ("userSpaceOnUse", SVG_MARKER_UNITS_USER_SPACE_ON_USE),
    ("strokeWidth", SVG_MARKER_UNITS_STROKE_WIDTH),
];
const SVG_COMPONENT_TRANSFER_TYPE_VALUES: &[(&str, u32)] = &[
    ("identity", SVG_COMPONENT_TRANSFER_TYPE_IDENTITY),
    ("table", SVG_COMPONENT_TRANSFER_TYPE_TABLE),
    ("discrete", SVG_COMPONENT_TRANSFER_TYPE_DISCRETE),
    ("linear", SVG_COMPONENT_TRANSFER_TYPE_LINEAR),
    ("gamma", SVG_COMPONENT_TRANSFER_TYPE_GAMMA),
];
const SVG_FE_BLEND_MODE_VALUES: &[(&str, u32)] = &[
    ("normal", SVG_FE_BLEND_MODE_NORMAL),
    ("multiply", SVG_FE_BLEND_MODE_MULTIPLY),
    ("screen", SVG_FE_BLEND_MODE_SCREEN),
    ("darken", SVG_FE_BLEND_MODE_DARKEN),
    ("lighten", SVG_FE_BLEND_MODE_LIGHTEN),
    ("overlay", SVG_FE_BLEND_MODE_OVERLAY),
    ("color-dodge", SVG_FE_BLEND_MODE_COLOR_DODGE),
    ("color-burn", SVG_FE_BLEND_MODE_COLOR_BURN),
    ("hard-light", SVG_FE_BLEND_MODE_HARD_LIGHT),
    ("soft-light", SVG_FE_BLEND_MODE_SOFT_LIGHT),
    ("difference", SVG_FE_BLEND_MODE_DIFFERENCE),
    ("exclusion", SVG_FE_BLEND_MODE_EXCLUSION),
    ("hue", SVG_FE_BLEND_MODE_HUE),
    ("saturation", SVG_FE_BLEND_MODE_SATURATION),
    ("color", SVG_FE_BLEND_MODE_COLOR),
    ("luminosity", SVG_FE_BLEND_MODE_LUMINOSITY),
];
const SVG_FE_COLOR_MATRIX_TYPE_VALUES: &[(&str, u32)] = &[
    ("matrix", SVG_FE_COLOR_MATRIX_TYPE_MATRIX),
    ("saturate", SVG_FE_COLOR_MATRIX_TYPE_SATURATE),
    ("hueRotate", SVG_FE_COLOR_MATRIX_TYPE_HUE_ROTATE),
    (
        "luminanceToAlpha",
        SVG_FE_COLOR_MATRIX_TYPE_LUMINANCE_TO_ALPHA,
    ),
];
const SVG_FE_COMPOSITE_OPERATOR_VALUES: &[(&str, u32)] = &[
    ("over", SVG_FE_COMPOSITE_OPERATOR_OVER),
    ("in", SVG_FE_COMPOSITE_OPERATOR_IN),
    ("out", SVG_FE_COMPOSITE_OPERATOR_OUT),
    ("atop", SVG_FE_COMPOSITE_OPERATOR_ATOP),
    ("xor", SVG_FE_COMPOSITE_OPERATOR_XOR),
    ("lighter", SVG_FE_COMPOSITE_OPERATOR_LIGHTER),
    ("arithmetic", SVG_FE_COMPOSITE_OPERATOR_ARITHMETIC),
];
const SVG_EDGE_MODE_VALUES: &[(&str, u32)] = &[
    ("duplicate", SVG_EDGE_MODE_DUPLICATE),
    ("wrap", SVG_EDGE_MODE_WRAP),
    ("none", SVG_EDGE_MODE_NONE),
];
const SVG_CHANNEL_VALUES: &[(&str, u32)] = &[
    ("R", SVG_CHANNEL_R),
    ("G", SVG_CHANNEL_G),
    ("B", SVG_CHANNEL_B),
    ("A", SVG_CHANNEL_A),
];
const SVG_MORPHOLOGY_OPERATOR_VALUES: &[(&str, u32)] = &[
    ("erode", SVG_MORPHOLOGY_OPERATOR_ERODE),
    ("dilate", SVG_MORPHOLOGY_OPERATOR_DILATE),
];
const SVG_TURBULENCE_TYPE_VALUES: &[(&str, u32)] = &[
    ("fractalNoise", SVG_TURBULENCE_TYPE_FRACTAL_NOISE),
    ("turbulence", SVG_TURBULENCE_TYPE_TURBULENCE),
];
const SVG_STITCH_TYPE_VALUES: &[(&str, u32)] = &[
    ("stitch", SVG_STITCH_TYPE_STITCH),
    ("noStitch", SVG_STITCH_TYPE_NO_STITCH),
];
const SVG_TEXT_PATH_METHOD_TYPE_VALUES: &[(&str, u32)] = &[
    ("align", SVG_TEXT_PATH_METHOD_TYPE_ALIGN),
    ("stretch", SVG_TEXT_PATH_METHOD_TYPE_STRETCH),
];
const SVG_TEXT_PATH_SPACING_TYPE_VALUES: &[(&str, u32)] = &[
    ("auto", SVG_TEXT_PATH_SPACING_TYPE_AUTO),
    ("exact", SVG_TEXT_PATH_SPACING_TYPE_EXACT),
];
const SVG_TEXT_PATH_SIDE_TYPE_VALUES: &[(&str, u32)] = &[
    ("left", SVG_TEXT_PATH_SIDE_TYPE_LEFT),
    ("right", SVG_TEXT_PATH_SIDE_TYPE_RIGHT),
];

const SVG_ANIMATED_ENUMERATION_PROPERTIES: &[SvgAnimatedEnumerationProperty] = &[
    SvgAnimatedEnumerationProperty {
        index: 0,
        attribute: "clipPathUnits",
        cache_slot: SVG_CLIP_PATH_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_USER_SPACE_ON_USE,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_UNIT_TYPE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 1,
        attribute: "filterUnits",
        cache_slot: SVG_FILTER_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_OBJECT_BOUNDING_BOX,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_UNIT_TYPE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 2,
        attribute: "primitiveUnits",
        cache_slot: SVG_PRIMITIVE_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_USER_SPACE_ON_USE,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_UNIT_TYPE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 3,
        attribute: "gradientUnits",
        cache_slot: SVG_GRADIENT_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_OBJECT_BOUNDING_BOX,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_UNIT_TYPE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 4,
        attribute: "spreadMethod",
        cache_slot: SVG_GRADIENT_SPREAD_METHOD_SLOT,
        initial_value: SVG_SPREAD_METHOD_PAD,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_SPREAD_METHOD_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 5,
        attribute: "maskUnits",
        cache_slot: SVG_MASK_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_OBJECT_BOUNDING_BOX,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_UNIT_TYPE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 6,
        attribute: "maskContentUnits",
        cache_slot: SVG_MASK_CONTENT_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_USER_SPACE_ON_USE,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_UNIT_TYPE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 7,
        attribute: "patternUnits",
        cache_slot: SVG_PATTERN_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_OBJECT_BOUNDING_BOX,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_UNIT_TYPE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 8,
        attribute: "patternContentUnits",
        cache_slot: SVG_PATTERN_CONTENT_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_USER_SPACE_ON_USE,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_UNIT_TYPE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 9,
        attribute: "lengthAdjust",
        cache_slot: SVG_TEXT_CONTENT_LENGTH_ADJUST_SLOT,
        initial_value: SVG_LENGTH_ADJUST_SPACING,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_LENGTH_ADJUST_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 10,
        attribute: "markerUnits",
        cache_slot: SVG_MARKER_UNITS_SLOT,
        initial_value: SVG_MARKER_UNITS_STROKE_WIDTH,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_MARKER_UNITS_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 11,
        attribute: "orient",
        cache_slot: SVG_MARKER_ORIENT_TYPE_SLOT,
        initial_value: SVG_MARKER_ORIENT_ANGLE,
        kind: SvgAnimatedEnumerationKind::MarkerOrient,
    },
    SvgAnimatedEnumerationProperty {
        index: 12,
        attribute: "type",
        cache_slot: SVG_COMPONENT_TRANSFER_TYPE_SLOT,
        initial_value: SVG_COMPONENT_TRANSFER_TYPE_IDENTITY,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_COMPONENT_TRANSFER_TYPE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 13,
        attribute: "mode",
        cache_slot: SVG_FE_BLEND_MODE_SLOT,
        initial_value: SVG_FE_BLEND_MODE_NORMAL,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_FE_BLEND_MODE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 14,
        attribute: "type",
        cache_slot: SVG_FE_COLOR_MATRIX_TYPE_SLOT,
        initial_value: SVG_FE_COLOR_MATRIX_TYPE_MATRIX,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_FE_COLOR_MATRIX_TYPE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 15,
        attribute: "operator",
        cache_slot: SVG_FE_COMPOSITE_OPERATOR_SLOT,
        initial_value: SVG_FE_COMPOSITE_OPERATOR_OVER,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_FE_COMPOSITE_OPERATOR_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 16,
        attribute: "edgeMode",
        cache_slot: SVG_FE_CONVOLVE_MATRIX_EDGE_MODE_SLOT,
        initial_value: SVG_EDGE_MODE_DUPLICATE,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_EDGE_MODE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 17,
        attribute: "xChannelSelector",
        cache_slot: SVG_FE_DISPLACEMENT_MAP_X_CHANNEL_SLOT,
        initial_value: SVG_CHANNEL_A,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_CHANNEL_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 18,
        attribute: "yChannelSelector",
        cache_slot: SVG_FE_DISPLACEMENT_MAP_Y_CHANNEL_SLOT,
        initial_value: SVG_CHANNEL_A,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_CHANNEL_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 19,
        attribute: "operator",
        cache_slot: SVG_FE_MORPHOLOGY_OPERATOR_SLOT,
        initial_value: SVG_MORPHOLOGY_OPERATOR_ERODE,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_MORPHOLOGY_OPERATOR_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 20,
        attribute: "stitchTiles",
        cache_slot: SVG_FE_TURBULENCE_STITCH_TILES_SLOT,
        initial_value: SVG_STITCH_TYPE_NO_STITCH,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_STITCH_TYPE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 21,
        attribute: "type",
        cache_slot: SVG_FE_TURBULENCE_TYPE_SLOT,
        initial_value: SVG_TURBULENCE_TYPE_TURBULENCE,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_TURBULENCE_TYPE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 22,
        attribute: "method",
        cache_slot: SVG_TEXT_PATH_METHOD_SLOT,
        initial_value: SVG_TEXT_PATH_METHOD_TYPE_ALIGN,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_TEXT_PATH_METHOD_TYPE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 23,
        attribute: "spacing",
        cache_slot: SVG_TEXT_PATH_SPACING_SLOT,
        initial_value: SVG_TEXT_PATH_SPACING_TYPE_EXACT,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_TEXT_PATH_SPACING_TYPE_VALUES),
    },
    SvgAnimatedEnumerationProperty {
        index: 24,
        attribute: "side",
        cache_slot: SVG_TEXT_PATH_SIDE_SLOT,
        initial_value: SVG_TEXT_PATH_SIDE_TYPE_LEFT,
        kind: SvgAnimatedEnumerationKind::Keywords(SVG_TEXT_PATH_SIDE_TYPE_VALUES),
    },
];

const fn svg_animated_number_property(
    index: usize,
    interface: &'static str,
    name: &'static str,
    attribute: &'static str,
    initial_value: f64,
    component: SvgAnimatedNumberComponent,
) -> SvgAnimatedNumberProperty {
    SvgAnimatedNumberProperty {
        index,
        interface,
        name,
        attribute,
        initial_value,
        component,
    }
}

const SVG_ANIMATED_NUMBER_PROPERTIES: &[SvgAnimatedNumberProperty] = &[
    svg_animated_number_property(
        0,
        "SVGComponentTransferFunctionElement",
        "slope",
        "slope",
        1.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        1,
        "SVGComponentTransferFunctionElement",
        "intercept",
        "intercept",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        2,
        "SVGComponentTransferFunctionElement",
        "amplitude",
        "amplitude",
        1.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        3,
        "SVGComponentTransferFunctionElement",
        "exponent",
        "exponent",
        1.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        4,
        "SVGComponentTransferFunctionElement",
        "offset",
        "offset",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        5,
        "SVGFECompositeElement",
        "k1",
        "k1",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        6,
        "SVGFECompositeElement",
        "k2",
        "k2",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        7,
        "SVGFECompositeElement",
        "k3",
        "k3",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        8,
        "SVGFECompositeElement",
        "k4",
        "k4",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        9,
        "SVGFEConvolveMatrixElement",
        "divisor",
        "divisor",
        1.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        10,
        "SVGFEConvolveMatrixElement",
        "bias",
        "bias",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        11,
        "SVGFEConvolveMatrixElement",
        "kernelUnitLengthX",
        "kernelUnitLength",
        0.0,
        SvgAnimatedNumberComponent::PairFirst,
    ),
    svg_animated_number_property(
        12,
        "SVGFEConvolveMatrixElement",
        "kernelUnitLengthY",
        "kernelUnitLength",
        0.0,
        SvgAnimatedNumberComponent::PairSecondOrFirst,
    ),
    svg_animated_number_property(
        13,
        "SVGFEDiffuseLightingElement",
        "surfaceScale",
        "surfaceScale",
        1.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        14,
        "SVGFEDiffuseLightingElement",
        "diffuseConstant",
        "diffuseConstant",
        1.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        15,
        "SVGFEDiffuseLightingElement",
        "kernelUnitLengthX",
        "kernelUnitLength",
        0.0,
        SvgAnimatedNumberComponent::PairFirst,
    ),
    svg_animated_number_property(
        16,
        "SVGFEDiffuseLightingElement",
        "kernelUnitLengthY",
        "kernelUnitLength",
        0.0,
        SvgAnimatedNumberComponent::PairSecondOrFirst,
    ),
    svg_animated_number_property(
        17,
        "SVGFEDisplacementMapElement",
        "scale",
        "scale",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        18,
        "SVGFEDistantLightElement",
        "azimuth",
        "azimuth",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        19,
        "SVGFEDistantLightElement",
        "elevation",
        "elevation",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        20,
        "SVGFEDropShadowElement",
        "dx",
        "dx",
        2.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        21,
        "SVGFEDropShadowElement",
        "dy",
        "dy",
        2.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        22,
        "SVGFEDropShadowElement",
        "stdDeviationX",
        "stdDeviation",
        2.0,
        SvgAnimatedNumberComponent::PairFirst,
    ),
    svg_animated_number_property(
        23,
        "SVGFEDropShadowElement",
        "stdDeviationY",
        "stdDeviation",
        2.0,
        SvgAnimatedNumberComponent::PairSecondOrFirst,
    ),
    svg_animated_number_property(
        24,
        "SVGFEGaussianBlurElement",
        "stdDeviationX",
        "stdDeviation",
        0.0,
        SvgAnimatedNumberComponent::PairFirst,
    ),
    svg_animated_number_property(
        25,
        "SVGFEGaussianBlurElement",
        "stdDeviationY",
        "stdDeviation",
        0.0,
        SvgAnimatedNumberComponent::PairSecondOrFirst,
    ),
    svg_animated_number_property(
        26,
        "SVGFEMorphologyElement",
        "radiusX",
        "radius",
        0.0,
        SvgAnimatedNumberComponent::PairFirst,
    ),
    svg_animated_number_property(
        27,
        "SVGFEMorphologyElement",
        "radiusY",
        "radius",
        0.0,
        SvgAnimatedNumberComponent::PairSecondOrFirst,
    ),
    svg_animated_number_property(
        28,
        "SVGFEOffsetElement",
        "dx",
        "dx",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        29,
        "SVGFEOffsetElement",
        "dy",
        "dy",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        30,
        "SVGFEPointLightElement",
        "x",
        "x",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        31,
        "SVGFEPointLightElement",
        "y",
        "y",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        32,
        "SVGFEPointLightElement",
        "z",
        "z",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        33,
        "SVGFESpecularLightingElement",
        "surfaceScale",
        "surfaceScale",
        1.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        34,
        "SVGFESpecularLightingElement",
        "specularConstant",
        "specularConstant",
        1.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        35,
        "SVGFESpecularLightingElement",
        "specularExponent",
        "specularExponent",
        1.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        36,
        "SVGFESpecularLightingElement",
        "kernelUnitLengthX",
        "kernelUnitLength",
        0.0,
        SvgAnimatedNumberComponent::PairFirst,
    ),
    svg_animated_number_property(
        37,
        "SVGFESpecularLightingElement",
        "kernelUnitLengthY",
        "kernelUnitLength",
        0.0,
        SvgAnimatedNumberComponent::PairSecondOrFirst,
    ),
    svg_animated_number_property(
        38,
        "SVGFESpotLightElement",
        "x",
        "x",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        39,
        "SVGFESpotLightElement",
        "y",
        "y",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        40,
        "SVGFESpotLightElement",
        "z",
        "z",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        41,
        "SVGFESpotLightElement",
        "pointsAtX",
        "pointsAtX",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        42,
        "SVGFESpotLightElement",
        "pointsAtY",
        "pointsAtY",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        43,
        "SVGFESpotLightElement",
        "pointsAtZ",
        "pointsAtZ",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        44,
        "SVGFESpotLightElement",
        "specularExponent",
        "specularExponent",
        1.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        45,
        "SVGFESpotLightElement",
        "limitingConeAngle",
        "limitingConeAngle",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        46,
        "SVGFETurbulenceElement",
        "baseFrequencyX",
        "baseFrequency",
        0.0,
        SvgAnimatedNumberComponent::PairFirst,
    ),
    svg_animated_number_property(
        47,
        "SVGFETurbulenceElement",
        "baseFrequencyY",
        "baseFrequency",
        0.0,
        SvgAnimatedNumberComponent::PairSecondOrFirst,
    ),
    svg_animated_number_property(
        48,
        "SVGFETurbulenceElement",
        "seed",
        "seed",
        0.0,
        SvgAnimatedNumberComponent::Scalar,
    ),
    svg_animated_number_property(
        49,
        "SVGStopElement",
        "offset",
        "offset",
        0.0,
        SvgAnimatedNumberComponent::NumberOrPercentage,
    ),
];

const SVG_ANIMATED_INTEGER_PROPERTIES: &[SvgAnimatedIntegerProperty] = &[
    SvgAnimatedIntegerProperty {
        index: 0,
        interface: "SVGFEConvolveMatrixElement",
        local_name: "feConvolveMatrix",
        name: "orderX",
        attribute: "order",
        cache_slot: SVG_FE_CONVOLVE_MATRIX_ORDER_X_SLOT,
        initial_value: 3,
        component: SvgAnimatedIntegerComponent::PairFirst,
    },
    SvgAnimatedIntegerProperty {
        index: 1,
        interface: "SVGFEConvolveMatrixElement",
        local_name: "feConvolveMatrix",
        name: "orderY",
        attribute: "order",
        cache_slot: SVG_FE_CONVOLVE_MATRIX_ORDER_Y_SLOT,
        initial_value: 3,
        component: SvgAnimatedIntegerComponent::PairSecondOrFirst,
    },
    SvgAnimatedIntegerProperty {
        index: 2,
        interface: "SVGFEConvolveMatrixElement",
        local_name: "feConvolveMatrix",
        name: "targetX",
        attribute: "targetX",
        cache_slot: SVG_FE_CONVOLVE_MATRIX_TARGET_X_SLOT,
        initial_value: 0,
        component: SvgAnimatedIntegerComponent::Scalar,
    },
    SvgAnimatedIntegerProperty {
        index: 3,
        interface: "SVGFEConvolveMatrixElement",
        local_name: "feConvolveMatrix",
        name: "targetY",
        attribute: "targetY",
        cache_slot: SVG_FE_CONVOLVE_MATRIX_TARGET_Y_SLOT,
        initial_value: 0,
        component: SvgAnimatedIntegerComponent::Scalar,
    },
    SvgAnimatedIntegerProperty {
        index: 4,
        interface: "SVGFETurbulenceElement",
        local_name: "feTurbulence",
        name: "numOctaves",
        attribute: "numOctaves",
        cache_slot: SVG_FE_TURBULENCE_NUM_OCTAVES_SLOT,
        initial_value: 1,
        component: SvgAnimatedIntegerComponent::Scalar,
    },
];

pub(in crate::context_bootstrap) fn install_svg_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    bindings::install_svg_element_accessor_bindings(scope, template, name);
    bindings::install_svg_enumeration_constant_bindings(scope, template, name);
    match name {
        "SVGLength" => bindings::install_svg_length_bindings(scope, template),
        "SVGAngle" => bindings::install_svg_angle_bindings(scope, template),
        "SVGNumber" => bindings::install_svg_number_bindings(scope, template),
        "SVGAnimatedString" => bindings::install_svg_animated_string_bindings(scope, template),
        "SVGAnimatedBoolean" => bindings::install_svg_animated_boolean_bindings(scope, template),
        "SVGAnimatedLength" => bindings::install_svg_animated_length_bindings(scope, template),
        "SVGAnimatedAngle" => bindings::install_svg_animated_angle_bindings(scope, template),
        "SVGAnimatedRect" => bindings::install_svg_animated_rect_bindings(scope, template),
        "SVGPreserveAspectRatio" => {
            bindings::install_svg_preserve_aspect_ratio_bindings(scope, template)
        }
        "SVGAnimatedPreserveAspectRatio" => {
            bindings::install_svg_animated_preserve_aspect_ratio_bindings(scope, template)
        }
        "SVGLengthList" => bindings::install_svg_length_list_bindings(scope, template),
        "SVGAnimatedLengthList" => {
            bindings::install_svg_animated_length_list_bindings(scope, template)
        }
        "SVGAnimatedNumber" => bindings::install_svg_animated_number_bindings(scope, template),
        "SVGAnimatedInteger" => bindings::install_svg_animated_integer_bindings(scope, template),
        "SVGNumberList" => bindings::install_svg_number_list_bindings(scope, template),
        "SVGPointList" => bindings::install_svg_point_list_bindings(scope, template),
        "SVGStringList" => bindings::install_svg_string_list_bindings(scope, template),
        "SVGAnimatedNumberList" => {
            bindings::install_svg_animated_number_list_bindings(scope, template)
        }
        "SVGAnimatedEnumeration" => {
            bindings::install_svg_animated_enumeration_bindings(scope, template)
        }
        "SVGUnitTypes" => bindings::install_svg_unit_types_bindings(scope, template),
        "SVGAnimatedTransformList" => {
            bindings::install_svg_animated_transform_list_bindings(scope, template)
        }
        "SVGTransformList" => bindings::install_svg_transform_list_bindings(scope, template),
        "SVGTransform" => bindings::install_svg_transform_bindings(scope, template),
        "SVGMatrix" => bindings::install_svg_matrix_bindings(scope, template),
        "SVGGraphicsElement" => bindings::install_svg_graphics_element_bindings(scope, template),
        "SVGGeometryElement" => bindings::install_svg_geometry_element_bindings(scope, template),
        "SVGTextContentElement" => {
            bindings::install_svg_text_content_element_bindings(scope, template)
        }
        "SVGGradientElement" => bindings::install_svg_gradient_element_bindings(scope, template),
        "SVGMarkerElement" => bindings::install_svg_marker_element_bindings(scope, template),
        "SVGSVGElement" => bindings::install_svg_svg_element_bindings(scope, template),
        _ => {}
    }
}

pub(in crate::context_bootstrap) fn reflect_svg_point_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    point: v8::Local<'s, v8::Object>,
) {
    builders::reflect_svg_value_list_item_to_owner_list(scope, point, SvgListKind::Point);
}

pub(in crate::context_bootstrap) fn sync_svg_view_box_rect_from_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rect: v8::Local<'s, v8::Object>,
) {
    builders::sync_svg_view_box_rect_from_owner(scope, rect);
}

pub(in crate::context_bootstrap) fn reflect_svg_view_box_rect_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rect: v8::Local<'s, v8::Object>,
) {
    builders::reflect_svg_view_box_rect_mutation(scope, rect);
}
