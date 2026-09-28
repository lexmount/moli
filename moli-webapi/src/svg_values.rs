//! SVG declaration values and the state populated by their factories.

pub mod constants;
pub mod matrix;

use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, set_private_value};
use moli_webapi_declare::WebApiObject;

pub const SVG_ANGLE_READ_ONLY_SLOT: &str = "__moliSvgAngleReadOnly";
pub const SVG_ANGLE_UNIT_TYPE_SLOT: &str = "__moliSvgAngleUnitType";
pub const SVG_ANGLE_VALUE_AS_STRING_SLOT: &str = "__moliSvgAngleValueAsString";
pub const SVG_ANGLE_VALUE_IN_SPECIFIED_UNITS_SLOT: &str = "__moliSvgAngleValueInSpecifiedUnits";
pub const SVG_ANGLE_VALUE_SLOT: &str = "__moliSvgAngleValue";
pub const SVG_ANIMATED_ANGLE_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedAngleAnimVal";
pub const SVG_ANIMATED_ANGLE_BASE_VAL_SLOT: &str = "__moliSvgAnimatedAngleBaseVal";
pub const SVG_ANIMATED_BOOLEAN_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedBooleanAnimVal";
pub const SVG_ANIMATED_BOOLEAN_BASE_VAL_SLOT: &str = "__moliSvgAnimatedBooleanBaseVal";
pub const SVG_ANIMATED_ENUMERATION_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedEnumerationAnimVal";
pub const SVG_ANIMATED_ENUMERATION_BASE_VAL_SLOT: &str = "__moliSvgAnimatedEnumerationBaseVal";
pub const SVG_ANIMATED_INTEGER_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedIntegerAnimVal";
pub const SVG_ANIMATED_INTEGER_BASE_VAL_SLOT: &str = "__moliSvgAnimatedIntegerBaseVal";
pub const SVG_ANIMATED_LENGTH_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedLengthAnimVal";
pub const SVG_ANIMATED_LENGTH_BASE_VAL_SLOT: &str = "__moliSvgAnimatedLengthBaseVal";
pub const SVG_ANIMATED_LENGTH_LIST_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedLengthListAnimVal";
pub const SVG_ANIMATED_LENGTH_LIST_BASE_VAL_SLOT: &str = "__moliSvgAnimatedLengthListBaseVal";
pub const SVG_ANIMATED_NUMBER_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedNumberAnimVal";
pub const SVG_ANIMATED_NUMBER_BASE_VAL_SLOT: &str = "__moliSvgAnimatedNumberBaseVal";
pub const SVG_ANIMATED_NUMBER_LIST_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedNumberListAnimVal";
pub const SVG_ANIMATED_NUMBER_LIST_BASE_VAL_SLOT: &str = "__moliSvgAnimatedNumberListBaseVal";
pub const SVG_ANIMATED_PRESERVE_ASPECT_RATIO_ANIM_VAL_SLOT: &str =
    "__moliSvgAnimatedPreserveAspectRatioAnimVal";
pub const SVG_ANIMATED_PRESERVE_ASPECT_RATIO_BASE_VAL_SLOT: &str =
    "__moliSvgAnimatedPreserveAspectRatioBaseVal";
pub const SVG_ANIMATED_RECT_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedRectAnimVal";
pub const SVG_ANIMATED_RECT_BASE_VAL_SLOT: &str = "__moliSvgAnimatedRectBaseVal";
pub const SVG_ANIMATED_STRING_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedStringAnimVal";
pub const SVG_ANIMATED_STRING_BASE_VAL_SLOT: &str = "__moliSvgAnimatedStringBaseVal";
pub const SVG_ANIMATED_TRANSFORM_LIST_ANIM_VAL_SLOT: &str = "__moliSvgAnimatedTransformListAnimVal";
pub const SVG_ANIMATED_TRANSFORM_LIST_BASE_VAL_SLOT: &str = "__moliSvgAnimatedTransformListBaseVal";
pub const SVG_LENGTH_UNIT_SUFFIX_SLOT: &str = "__moliSvgLengthUnitSuffix";
pub const SVG_LENGTH_UNIT_TYPE_SLOT: &str = "__moliSvgLengthUnitType";
pub const SVG_LENGTH_VALUE_AS_STRING_SLOT: &str = "__moliSvgLengthValueAsString";
pub const SVG_LENGTH_VALUE_IN_SPECIFIED_UNITS_SLOT: &str = "__moliSvgLengthValueInSpecifiedUnits";
pub const SVG_LENGTH_VALUE_SLOT: &str = "__moliSvgLengthValue";
pub const SVG_NUMBER_VALUE_SLOT: &str = "__moliSvgNumberValue";
pub const SVG_PRESERVE_ASPECT_RATIO_ALIGN_SLOT: &str = "__moliSvgPreserveAspectRatioAlign";
pub const SVG_PRESERVE_ASPECT_RATIO_MEET_OR_SLICE_SLOT: &str =
    "__moliSvgPreserveAspectRatioMeetOrSlice";
pub const SVG_PRESERVE_ASPECT_RATIO_OWNER_ELEMENT_SLOT: &str =
    "__moliSvgPreserveAspectRatioOwnerElement";
pub const SVG_PRESERVE_ASPECT_RATIO_READ_ONLY_SLOT: &str = "__moliSvgPreserveAspectRatioReadOnly";
pub const SVG_STRING_LIST_ITEMS_SLOT: &str = "__moliSvgStringListItems";

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::SVGAnimatedString,
    own_to_string_tag = "SVGAnimatedString"
)]
pub struct SvgAnimatedStringObjectDeclaration<'s> {
    #[webapi(slot = SVG_ANIMATED_STRING_BASE_VAL_SLOT)]
    base_val: v8::Local<'s, v8::String>,
    #[webapi(slot = SVG_ANIMATED_STRING_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'s, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::SVGAnimatedBoolean,
    own_to_string_tag = "SVGAnimatedBoolean"
)]
pub struct SvgAnimatedBooleanObjectDeclaration {
    #[webapi(slot = SVG_ANIMATED_BOOLEAN_BASE_VAL_SLOT)]
    base_val: bool,
    #[webapi(slot = SVG_ANIMATED_BOOLEAN_ANIM_VAL_SLOT)]
    anim_val: bool,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::SVGAnimatedNumber,
    own_to_string_tag = "SVGAnimatedNumber"
)]
struct SvgAnimatedNumberObjectDeclaration {
    #[webapi(slot = SVG_ANIMATED_NUMBER_BASE_VAL_SLOT)]
    base_val: f64,
    #[webapi(slot = SVG_ANIMATED_NUMBER_ANIM_VAL_SLOT)]
    anim_val: f64,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::SVGAnimatedInteger,
    own_to_string_tag = "SVGAnimatedInteger"
)]
struct SvgAnimatedIntegerObjectDeclaration {
    #[webapi(slot = SVG_ANIMATED_INTEGER_BASE_VAL_SLOT)]
    base_val: i32,
    #[webapi(slot = SVG_ANIMATED_INTEGER_ANIM_VAL_SLOT)]
    anim_val: i32,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::SVGNumber, own_to_string_tag = "SVGNumber")]
struct SvgNumberObjectDeclaration {
    #[webapi(slot = SVG_NUMBER_VALUE_SLOT)]
    value: f64,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::SVGAnimatedEnumeration,
    own_to_string_tag = "SVGAnimatedEnumeration"
)]
struct SvgAnimatedEnumerationObjectDeclaration {
    #[webapi(slot = SVG_ANIMATED_ENUMERATION_BASE_VAL_SLOT)]
    base_val: u32,
    #[webapi(slot = SVG_ANIMATED_ENUMERATION_ANIM_VAL_SLOT)]
    anim_val: u32,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::SVGAnimatedLength,
    fallback_to_string_tag = "SVGAnimatedLength"
)]
pub struct SvgAnimatedLengthObjectDeclaration<'scope> {
    #[webapi(slot = SVG_ANIMATED_LENGTH_BASE_VAL_SLOT)]
    base_val: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_ANIMATED_LENGTH_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::SVGAnimatedAngle,
    fallback_to_string_tag = "SVGAnimatedAngle"
)]
pub struct SvgAnimatedAngleObjectDeclaration<'scope> {
    #[webapi(slot = SVG_ANIMATED_ANGLE_BASE_VAL_SLOT)]
    base_val: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_ANIMATED_ANGLE_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::SVGAnimatedRect,
    fallback_to_string_tag = "SVGAnimatedRect"
)]
pub struct SvgAnimatedRectObjectDeclaration<'scope> {
    #[webapi(slot = SVG_ANIMATED_RECT_BASE_VAL_SLOT)]
    base_val: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_ANIMATED_RECT_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::SVGPreserveAspectRatio,
    fallback_to_string_tag = "SVGPreserveAspectRatio"
)]
struct SvgPreserveAspectRatioObjectDeclaration<'scope> {
    #[webapi(slot = SVG_PRESERVE_ASPECT_RATIO_ALIGN_SLOT)]
    align: u32,
    #[webapi(slot = SVG_PRESERVE_ASPECT_RATIO_MEET_OR_SLICE_SLOT)]
    meet_or_slice: u32,
    #[webapi(slot = SVG_PRESERVE_ASPECT_RATIO_OWNER_ELEMENT_SLOT)]
    owner: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_PRESERVE_ASPECT_RATIO_READ_ONLY_SLOT)]
    read_only: bool,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::SVGAnimatedPreserveAspectRatio,
    fallback_to_string_tag = "SVGAnimatedPreserveAspectRatio"
)]
struct SvgAnimatedPreserveAspectRatioObjectDeclaration<'scope> {
    #[webapi(slot = SVG_ANIMATED_PRESERVE_ASPECT_RATIO_BASE_VAL_SLOT)]
    base_val: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_ANIMATED_PRESERVE_ASPECT_RATIO_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::SVGLength, fallback_to_string_tag = "SVGLength")]
pub struct SvgLengthObjectDeclaration {
    #[webapi(slot = SVG_LENGTH_UNIT_TYPE_SLOT)]
    unit_type: u32,
    #[webapi(slot = SVG_LENGTH_UNIT_SUFFIX_SLOT)]
    unit_suffix: String,
    #[webapi(slot = SVG_LENGTH_VALUE_SLOT)]
    value: f64,
    #[webapi(slot = SVG_LENGTH_VALUE_IN_SPECIFIED_UNITS_SLOT)]
    value_in_specified_units: f64,
    #[webapi(slot = SVG_LENGTH_VALUE_AS_STRING_SLOT)]
    value_as_string: String,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::SVGAngle, fallback_to_string_tag = "SVGAngle")]
pub struct SvgAngleObjectDeclaration {
    #[webapi(slot = SVG_ANGLE_UNIT_TYPE_SLOT)]
    unit_type: u32,
    #[webapi(slot = SVG_ANGLE_VALUE_SLOT)]
    value: f64,
    #[webapi(slot = SVG_ANGLE_VALUE_IN_SPECIFIED_UNITS_SLOT)]
    value_in_specified_units: f64,
    #[webapi(slot = SVG_ANGLE_VALUE_AS_STRING_SLOT)]
    value_as_string: String,
    #[webapi(slot = SVG_ANGLE_READ_ONLY_SLOT)]
    read_only: bool,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::SVGAnimatedLengthList,
    own_to_string_tag = "SVGAnimatedLengthList"
)]
pub struct SvgAnimatedLengthListObjectDeclaration<'scope> {
    #[webapi(slot = SVG_ANIMATED_LENGTH_LIST_BASE_VAL_SLOT)]
    base_val: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_ANIMATED_LENGTH_LIST_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::SVGAnimatedNumberList,
    own_to_string_tag = "SVGAnimatedNumberList"
)]
pub struct SvgAnimatedNumberListObjectDeclaration<'scope> {
    #[webapi(slot = SVG_ANIMATED_NUMBER_LIST_BASE_VAL_SLOT)]
    base_val: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_ANIMATED_NUMBER_LIST_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::SVGAnimatedTransformList,
    own_to_string_tag = "SVGAnimatedTransformList"
)]
pub struct SvgAnimatedTransformListObjectDeclaration<'scope> {
    #[webapi(slot = SVG_ANIMATED_TRANSFORM_LIST_BASE_VAL_SLOT)]
    base_val: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_ANIMATED_TRANSFORM_LIST_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::SVGStringList, own_to_string_tag = "SVGStringList")]
pub struct SvgStringListObjectDeclaration<'scope> {
    #[webapi(slot = SVG_STRING_LIST_ITEMS_SLOT)]
    items: Vec<v8::Local<'scope, v8::Value>>,
}

pub const SVG_RECT_X_SLOT: &str = "__moliSvgRectX";
const Y: &str = "__moliSvgRectY";
const WIDTH: &str = "__moliSvgRectWidth";
const HEIGHT: &str = "__moliSvgRectHeight";
pub const SVG_RECT_FIELD_SLOTS: &[(&str, &str)] = &[
    ("x", SVG_RECT_X_SLOT),
    ("y", Y),
    ("width", WIDTH),
    ("height", HEIGHT),
];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::SVGRect)]
struct SvgRectObjectDeclaration {
    #[webapi(slot = SVG_RECT_X_SLOT)]
    x: f64,
    #[webapi(slot = Y)]
    y: f64,
    #[webapi(slot = WIDTH)]
    width: f64,
    #[webapi(slot = HEIGHT)]
    height: f64,
}

pub fn build_svg_animated_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: f64,
) -> v8::Local<'s, v8::Object> {
    SvgAnimatedNumberObjectDeclaration::new(value, value)
        .bind(scope)
        .expect("SVGAnimatedNumber declaration should bind")
}

pub fn build_svg_animated_integer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: i32,
) -> v8::Local<'s, v8::Object> {
    SvgAnimatedIntegerObjectDeclaration::new(value, value)
        .bind(scope)
        .expect("SVGAnimatedInteger declaration should bind")
}

pub fn build_svg_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: f64,
) -> v8::Local<'s, v8::Object> {
    SvgNumberObjectDeclaration::new(value)
        .bind(scope)
        .expect("SVGNumber declaration should bind")
}

pub fn build_svg_animated_enumeration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: u32,
) -> v8::Local<'s, v8::Object> {
    SvgAnimatedEnumerationObjectDeclaration::new(value, value)
        .bind(scope)
        .expect("SVGAnimatedEnumeration declaration should bind")
}

pub fn new_svg_animated_preserve_aspect_ratio<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    align: u32,
    meet_or_slice: u32,
) -> v8::Local<'s, v8::Object> {
    let base_val = SvgPreserveAspectRatioObjectDeclaration::new(align, meet_or_slice, owner, false)
        .bind(scope)
        .expect("SVGPreserveAspectRatio base value declaration should bind");
    let anim_val = SvgPreserveAspectRatioObjectDeclaration::new(align, meet_or_slice, owner, true)
        .bind(scope)
        .expect("SVGPreserveAspectRatio animated value declaration should bind");
    SvgAnimatedPreserveAspectRatioObjectDeclaration::new(base_val, anim_val)
        .bind(scope)
        .expect("SVGAnimatedPreserveAspectRatio declaration should bind")
}

pub fn new_svg_rect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: [f64; 4],
) -> v8::Local<'s, v8::Object> {
    let [x, y, width, height] = values.map(|value| f64::from(value as f32));
    SvgRectObjectDeclaration::new(x, y, width, height)
        .bind(scope)
        .expect("SVGRect declaration should bind")
}

pub fn svg_view_box_rect_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rect: v8::Local<'s, v8::Object>,
) -> [f64; 4] {
    [SVG_RECT_X_SLOT, Y, WIDTH, HEIGHT].map(|slot| {
        get_private_value(scope, rect, slot)
            .and_then(|value| v8::Local::<v8::Number>::try_from(value).ok())
            .map_or(0.0, |value| value.value())
    })
}

pub fn set_svg_view_box_rect_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rect: v8::Local<'s, v8::Object>,
    values: [f64; 4],
) {
    for (slot, value) in [SVG_RECT_X_SLOT, Y, WIDTH, HEIGHT].into_iter().zip(values) {
        set_private_value(
            scope,
            rect,
            slot,
            v8::Number::new(scope, f64::from(value as f32)).into(),
        );
    }
}
