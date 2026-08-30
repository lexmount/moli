use super::{
    build_dom_point_object, build_dom_rect_object, optional_dom_point_init_arg,
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
mod rect;

const SVG_GRAPHICS_TRANSFORM_SLOT: &str = "__moliSvgGraphicsTransform";
const SVG_PATTERN_TRANSFORM_SLOT: &str = "__moliSvgPatternTransform";
const SVG_GRADIENT_TRANSFORM_SLOT: &str = "__moliSvgGradientTransform";
const SVG_GEOMETRY_PATH_LENGTH_SLOT: &str = "__moliSvgGeometryPathLength";
const SVG_ELEMENT_CLASS_NAME_SLOT: &str = "__moliSvgElementClassName";
const SVG_URI_HREF_SLOT: &str = "__moliSvgUriHref";
const SVG_ANIMATED_STRING_BASE_VAL_SLOT: &str = "__moliSvgAnimatedStringBaseVal";
const SVG_ANIMATED_STRING_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedStringAnimVal";
const SVG_ANIMATED_STRING_OWNER_ELEMENT_SLOT: &str = "__moliSvgAnimatedStringOwnerElement";
const SVG_ANIMATED_STRING_OWNER_ATTRIBUTE_SLOT: &str = "__moliSvgAnimatedStringOwnerAttribute";
const SVG_ANIMATED_LENGTH_BASE_VAL_SLOT: &str = "__moliSvgAnimatedLengthBaseVal";
const SVG_ANIMATED_LENGTH_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedLengthAnimVal";
const SVG_ANIMATED_LENGTH_LIST_BASE_VAL_SLOT: &str = "__moliSvgAnimatedLengthListBaseVal";
const SVG_ANIMATED_LENGTH_LIST_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedLengthListAnimVal";
const SVG_LENGTH_LIST_ITEMS_SLOT: &str = "__moliSvgLengthListItems";
const SVG_LENGTH_OWNER_ELEMENT_SLOT: &str = "__moliSvgLengthOwnerElement";
const SVG_LENGTH_OWNER_ATTRIBUTE_SLOT: &str = "__moliSvgLengthOwnerAttribute";
const SVG_LENGTH_UNIT_TYPE_SLOT: &str = "__moliSvgLengthUnitType";
const SVG_LENGTH_VALUE_SLOT: &str = "__moliSvgLengthValue";
const SVG_LENGTH_VALUE_AS_STRING_SLOT: &str = "__moliSvgLengthValueAsString";
const SVG_NUMBER_VALUE_SLOT: &str = "__moliSvgNumberValue";
const SVG_ANIMATED_NUMBER_OWNER_ELEMENT_SLOT: &str = "__moliSvgAnimatedNumberOwnerElement";
const SVG_ANIMATED_NUMBER_OWNER_ATTRIBUTE_SLOT: &str = "__moliSvgAnimatedNumberOwnerAttribute";
const SVG_ANIMATED_NUMBER_BASE_VAL_SLOT: &str = "__moliSvgAnimatedNumberBaseVal";
const SVG_ANIMATED_NUMBER_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedNumberAnimVal";
const SVG_ANIMATED_NUMBER_LIST_BASE_VAL_SLOT: &str = "__moliSvgAnimatedNumberListBaseVal";
const SVG_ANIMATED_NUMBER_LIST_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedNumberListAnimVal";
const SVG_NUMBER_LIST_ITEMS_SLOT: &str = "__moliSvgNumberListItems";
const SVG_VALUE_LIST_OWNER_ELEMENT_SLOT: &str = "__moliSvgValueListOwnerElement";
const SVG_VALUE_LIST_OWNER_ATTRIBUTE_SLOT: &str = "__moliSvgValueListOwnerAttribute";
const SVG_VALUE_LIST_ITEM_OWNER_LIST_SLOT: &str = "__moliSvgValueListItemOwnerList";
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

#[derive(Clone, Copy)]
enum SvgListKind {
    Length,
    Number,
}

#[derive(Clone, Copy)]
enum SvgAnimatedEnumerationKind {
    UnitType,
    SpreadMethod,
    LengthAdjust,
}

#[derive(Clone, Copy)]
struct SvgAnimatedEnumerationProperty {
    index: usize,
    attribute: &'static str,
    cache_slot: &'static str,
    initial_value: u32,
    kind: SvgAnimatedEnumerationKind,
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
    #[webidl(required)]
    value: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVG length convertToSpecifiedUnits")]
struct SvgLengthConvertToSpecifiedUnitsArgs {
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

const SVG_ANIMATED_ENUMERATION_PROPERTIES: &[SvgAnimatedEnumerationProperty] = &[
    SvgAnimatedEnumerationProperty {
        index: 0,
        attribute: "clipPathUnits",
        cache_slot: SVG_CLIP_PATH_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_USER_SPACE_ON_USE,
        kind: SvgAnimatedEnumerationKind::UnitType,
    },
    SvgAnimatedEnumerationProperty {
        index: 1,
        attribute: "filterUnits",
        cache_slot: SVG_FILTER_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_OBJECT_BOUNDING_BOX,
        kind: SvgAnimatedEnumerationKind::UnitType,
    },
    SvgAnimatedEnumerationProperty {
        index: 2,
        attribute: "primitiveUnits",
        cache_slot: SVG_PRIMITIVE_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_USER_SPACE_ON_USE,
        kind: SvgAnimatedEnumerationKind::UnitType,
    },
    SvgAnimatedEnumerationProperty {
        index: 3,
        attribute: "gradientUnits",
        cache_slot: SVG_GRADIENT_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_OBJECT_BOUNDING_BOX,
        kind: SvgAnimatedEnumerationKind::UnitType,
    },
    SvgAnimatedEnumerationProperty {
        index: 4,
        attribute: "spreadMethod",
        cache_slot: SVG_GRADIENT_SPREAD_METHOD_SLOT,
        initial_value: SVG_SPREAD_METHOD_PAD,
        kind: SvgAnimatedEnumerationKind::SpreadMethod,
    },
    SvgAnimatedEnumerationProperty {
        index: 5,
        attribute: "maskUnits",
        cache_slot: SVG_MASK_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_OBJECT_BOUNDING_BOX,
        kind: SvgAnimatedEnumerationKind::UnitType,
    },
    SvgAnimatedEnumerationProperty {
        index: 6,
        attribute: "maskContentUnits",
        cache_slot: SVG_MASK_CONTENT_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_USER_SPACE_ON_USE,
        kind: SvgAnimatedEnumerationKind::UnitType,
    },
    SvgAnimatedEnumerationProperty {
        index: 7,
        attribute: "patternUnits",
        cache_slot: SVG_PATTERN_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_OBJECT_BOUNDING_BOX,
        kind: SvgAnimatedEnumerationKind::UnitType,
    },
    SvgAnimatedEnumerationProperty {
        index: 8,
        attribute: "patternContentUnits",
        cache_slot: SVG_PATTERN_CONTENT_UNITS_SLOT,
        initial_value: SVG_UNIT_TYPE_USER_SPACE_ON_USE,
        kind: SvgAnimatedEnumerationKind::UnitType,
    },
    SvgAnimatedEnumerationProperty {
        index: 9,
        attribute: "lengthAdjust",
        cache_slot: SVG_TEXT_CONTENT_LENGTH_ADJUST_SLOT,
        initial_value: SVG_LENGTH_ADJUST_SPACING,
        kind: SvgAnimatedEnumerationKind::LengthAdjust,
    },
];

pub(in crate::context_bootstrap) fn install_svg_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    bindings::install_svg_element_accessor_bindings(scope, template, name);
    match name {
        "SVGLength" => bindings::install_svg_length_bindings(scope, template),
        "SVGNumber" => bindings::install_svg_number_bindings(scope, template),
        "SVGRect" => rect::install_bindings(scope, template),
        "SVGAnimatedString" => bindings::install_svg_animated_string_bindings(scope, template),
        "SVGAnimatedLength" => bindings::install_svg_animated_length_bindings(scope, template),
        "SVGLengthList" => bindings::install_svg_length_list_bindings(scope, template),
        "SVGAnimatedLengthList" => {
            bindings::install_svg_animated_length_list_bindings(scope, template)
        }
        "SVGAnimatedNumber" => bindings::install_svg_animated_number_bindings(scope, template),
        "SVGNumberList" => bindings::install_svg_number_list_bindings(scope, template),
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
        "SVGSVGElement" => bindings::install_svg_svg_element_bindings(scope, template),
        _ => {}
    }
}
