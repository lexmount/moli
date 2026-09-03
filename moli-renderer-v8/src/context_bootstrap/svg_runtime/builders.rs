use super::callbacks::*;
use super::*;
use crate::util::serialize_v8_iter_array;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    interface = "SVGAnimatedString",
    own_to_string_tag = "SVGAnimatedString"
)]
struct SvgAnimatedStringObjectDeclaration {
    #[webapi(slot = SVG_ANIMATED_STRING_BASE_VAL_SLOT)]
    base_val: String,
    #[webapi(slot = SVG_ANIMATED_STRING_ANIM_VAL_SLOT)]
    anim_val: String,
}

#[derive(WebApiObject)]
#[webapi(
    interface = "SVGAnimatedBoolean",
    own_to_string_tag = "SVGAnimatedBoolean"
)]
struct SvgAnimatedBooleanObjectDeclaration {
    #[webapi(slot = SVG_ANIMATED_BOOLEAN_BASE_VAL_SLOT)]
    base_val: bool,
    #[webapi(slot = SVG_ANIMATED_BOOLEAN_ANIM_VAL_SLOT)]
    anim_val: bool,
}

#[derive(WebApiObject)]
#[webapi(
    interface = "SVGAnimatedNumber",
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
    interface = "SVGAnimatedInteger",
    own_to_string_tag = "SVGAnimatedInteger"
)]
struct SvgAnimatedIntegerObjectDeclaration {
    #[webapi(slot = SVG_ANIMATED_INTEGER_BASE_VAL_SLOT)]
    base_val: i32,
    #[webapi(slot = SVG_ANIMATED_INTEGER_ANIM_VAL_SLOT)]
    anim_val: i32,
}

#[derive(WebApiObject)]
#[webapi(interface = "SVGNumber", own_to_string_tag = "SVGNumber")]
struct SvgNumberObjectDeclaration {
    #[webapi(slot = SVG_NUMBER_VALUE_SLOT)]
    value: f64,
}

#[derive(WebApiObject)]
#[webapi(
    interface = "SVGAnimatedEnumeration",
    own_to_string_tag = "SVGAnimatedEnumeration"
)]
struct SvgAnimatedEnumerationObjectDeclaration {
    #[webapi(slot = SVG_ANIMATED_ENUMERATION_BASE_VAL_SLOT)]
    base_val: u32,
    #[webapi(slot = SVG_ANIMATED_ENUMERATION_ANIM_VAL_SLOT)]
    anim_val: u32,
}

#[derive(WebApiObject)]
#[webapi(interface = "SVGMatrix", own_to_string_tag = "SVGMatrix")]
struct SvgMatrixObjectDeclaration {
    #[webapi(slot = SVG_MATRIX_A_SLOT)]
    a: f64,
    #[webapi(slot = SVG_MATRIX_B_SLOT)]
    b: f64,
    #[webapi(slot = SVG_MATRIX_C_SLOT)]
    c: f64,
    #[webapi(slot = SVG_MATRIX_D_SLOT)]
    d: f64,
    #[webapi(slot = SVG_MATRIX_E_SLOT)]
    e: f64,
    #[webapi(slot = SVG_MATRIX_F_SLOT)]
    f: f64,
    #[webapi(method, callback = svg_matrix_multiply_callback, length = 1)]
    multiply: (),
    #[webapi(method, callback = svg_matrix_inverse_callback, length = 0)]
    inverse: (),
    #[webapi(method, callback = svg_matrix_translate_callback, length = 2)]
    translate: (),
    #[webapi(method, callback = svg_matrix_scale_callback, length = 1)]
    scale: (),
    #[webapi(
        method,
        callback = svg_matrix_scale_non_uniform_callback,
        length = 2
    )]
    scale_non_uniform: (),
    #[webapi(method, callback = svg_matrix_rotate_callback, length = 1)]
    rotate: (),
    #[webapi(
        method,
        callback = svg_matrix_rotate_from_vector_callback,
        length = 2
    )]
    rotate_from_vector: (),
    #[webapi(method, callback = svg_matrix_flip_x_callback, length = 0)]
    flip_x: (),
    #[webapi(method, callback = svg_matrix_flip_y_callback, length = 0)]
    flip_y: (),
    #[webapi(method, callback = svg_matrix_skew_x_callback, length = 1)]
    skew_x: (),
    #[webapi(method, callback = svg_matrix_skew_y_callback, length = 1)]
    skew_y: (),
}

#[derive(WebApiObject)]
#[webapi(
    interface = "SVGAnimatedLength",
    fallback_to_string_tag = "SVGAnimatedLength"
)]
struct SvgAnimatedLengthObjectDeclaration<'scope> {
    #[webapi(slot = SVG_ANIMATED_LENGTH_BASE_VAL_SLOT)]
    base_val: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_ANIMATED_LENGTH_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = "SVGAnimatedAngle",
    fallback_to_string_tag = "SVGAnimatedAngle"
)]
struct SvgAnimatedAngleObjectDeclaration<'scope> {
    #[webapi(slot = SVG_ANIMATED_ANGLE_BASE_VAL_SLOT)]
    base_val: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_ANIMATED_ANGLE_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = "SVGAnimatedRect",
    fallback_to_string_tag = "SVGAnimatedRect"
)]
struct SvgAnimatedRectObjectDeclaration<'scope> {
    #[webapi(slot = SVG_ANIMATED_RECT_BASE_VAL_SLOT)]
    base_val: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_ANIMATED_RECT_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = "SVGPreserveAspectRatio",
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
    interface = "SVGAnimatedPreserveAspectRatio",
    fallback_to_string_tag = "SVGAnimatedPreserveAspectRatio"
)]
struct SvgAnimatedPreserveAspectRatioObjectDeclaration<'scope> {
    #[webapi(slot = SVG_ANIMATED_PRESERVE_ASPECT_RATIO_BASE_VAL_SLOT)]
    base_val: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_ANIMATED_PRESERVE_ASPECT_RATIO_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "SVGLength", fallback_to_string_tag = "SVGLength")]
struct SvgLengthObjectDeclaration {
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
#[webapi(interface = "SVGAngle", fallback_to_string_tag = "SVGAngle")]
struct SvgAngleObjectDeclaration {
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
    interface = "SVGAnimatedLengthList",
    own_to_string_tag = "SVGAnimatedLengthList"
)]
struct SvgAnimatedLengthListObjectDeclaration<'scope> {
    #[webapi(slot = SVG_ANIMATED_LENGTH_LIST_BASE_VAL_SLOT)]
    base_val: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_ANIMATED_LENGTH_LIST_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = "SVGAnimatedNumberList",
    own_to_string_tag = "SVGAnimatedNumberList"
)]
struct SvgAnimatedNumberListObjectDeclaration<'scope> {
    #[webapi(slot = SVG_ANIMATED_NUMBER_LIST_BASE_VAL_SLOT)]
    base_val: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_ANIMATED_NUMBER_LIST_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = "SVGAnimatedTransformList",
    own_to_string_tag = "SVGAnimatedTransformList"
)]
struct SvgAnimatedTransformListObjectDeclaration<'scope> {
    #[webapi(slot = SVG_ANIMATED_TRANSFORM_LIST_BASE_VAL_SLOT)]
    base_val: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SVG_ANIMATED_TRANSFORM_LIST_ANIM_VAL_SLOT)]
    anim_val: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "SVGLengthList", own_to_string_tag = "SVGLengthList")]
struct SvgLengthListObjectDeclaration<'scope> {
    #[webapi(slot = SVG_LENGTH_LIST_ITEMS_SLOT)]
    items: Vec<v8::Local<'scope, v8::Value>>,
    #[webapi(slot = SVG_VALUE_LIST_READ_ONLY_SLOT)]
    read_only: bool,
    #[webapi(method, callback = svg_length_list_clear_callback, length = 0)]
    clear: (),
    #[webapi(method, callback = svg_length_list_initialize_callback, length = 1)]
    initialize: (),
    #[webapi(method, callback = svg_length_list_get_item_callback, length = 1)]
    get_item: (),
    #[webapi(
        method,
        callback = svg_length_list_insert_item_before_callback,
        length = 2
    )]
    insert_item_before: (),
    #[webapi(method, callback = svg_length_list_replace_item_callback, length = 2)]
    replace_item: (),
    #[webapi(method, callback = svg_length_list_remove_item_callback, length = 1)]
    remove_item: (),
    #[webapi(method, callback = svg_length_list_append_item_callback, length = 1)]
    append_item: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "SVGNumberList", own_to_string_tag = "SVGNumberList")]
struct SvgNumberListObjectDeclaration<'scope> {
    #[webapi(slot = SVG_NUMBER_LIST_ITEMS_SLOT)]
    items: Vec<v8::Local<'scope, v8::Value>>,
    #[webapi(slot = SVG_VALUE_LIST_READ_ONLY_SLOT)]
    read_only: bool,
    #[webapi(method, callback = svg_number_list_clear_callback, length = 0)]
    clear: (),
    #[webapi(method, callback = svg_number_list_initialize_callback, length = 1)]
    initialize: (),
    #[webapi(method, callback = svg_number_list_get_item_callback, length = 1)]
    get_item: (),
    #[webapi(
        method,
        callback = svg_number_list_insert_item_before_callback,
        length = 2
    )]
    insert_item_before: (),
    #[webapi(method, callback = svg_number_list_replace_item_callback, length = 2)]
    replace_item: (),
    #[webapi(method, callback = svg_number_list_remove_item_callback, length = 1)]
    remove_item: (),
    #[webapi(method, callback = svg_number_list_append_item_callback, length = 1)]
    append_item: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "SVGPointList", own_to_string_tag = "SVGPointList")]
struct SvgPointListObjectDeclaration<'scope> {
    #[webapi(slot = SVG_POINT_LIST_ITEMS_SLOT)]
    items: Vec<v8::Local<'scope, v8::Value>>,
    #[webapi(slot = SVG_VALUE_LIST_READ_ONLY_SLOT)]
    read_only: bool,
    #[webapi(method, callback = svg_point_list_clear_callback, length = 0)]
    clear: (),
    #[webapi(method, callback = svg_point_list_initialize_callback, length = 1)]
    initialize: (),
    #[webapi(method, callback = svg_point_list_get_item_callback, length = 1)]
    get_item: (),
    #[webapi(
        method,
        callback = svg_point_list_insert_item_before_callback,
        length = 2
    )]
    insert_item_before: (),
    #[webapi(method, callback = svg_point_list_replace_item_callback, length = 2)]
    replace_item: (),
    #[webapi(method, callback = svg_point_list_remove_item_callback, length = 1)]
    remove_item: (),
    #[webapi(method, callback = svg_point_list_append_item_callback, length = 1)]
    append_item: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "SVGStringList", own_to_string_tag = "SVGStringList")]
struct SvgStringListObjectDeclaration<'scope> {
    #[webapi(slot = SVG_STRING_LIST_ITEMS_SLOT)]
    items: Vec<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(interface = "SVGTransformList", own_to_string_tag = "SVGTransformList")]
struct SvgTransformListObjectDeclaration<'scope> {
    #[webapi(slot = SVG_TRANSFORM_LIST_ITEMS_SLOT)]
    items: Vec<v8::Local<'scope, v8::Value>>,
    #[webapi(method, callback = svg_transform_list_clear_callback, length = 0)]
    clear: (),
    #[webapi(method, callback = svg_transform_list_initialize_callback, length = 1)]
    initialize: (),
    #[webapi(method, callback = svg_transform_list_get_item_callback, length = 1)]
    get_item: (),
    #[webapi(
        method,
        callback = svg_transform_list_insert_item_before_callback,
        length = 2
    )]
    insert_item_before: (),
    #[webapi(
        method,
        callback = svg_transform_list_replace_item_callback,
        length = 2
    )]
    replace_item: (),
    #[webapi(method, callback = svg_transform_list_remove_item_callback, length = 1)]
    remove_item: (),
    #[webapi(method, callback = svg_transform_list_append_item_callback, length = 1)]
    append_item: (),
    #[webapi(
        method = "createSVGTransformFromMatrix",
        callback = svg_transform_list_create_transform_from_matrix_callback,
        length = 0
    )]
    create_svg_transform_from_matrix: (),
    #[webapi(method, callback = svg_transform_list_consolidate_callback, length = 0)]
    consolidate: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "SVGTransform", own_to_string_tag = "SVGTransform")]
struct SvgTransformObjectDeclaration<'scope> {
    #[webapi(slot = SVG_TRANSFORM_TYPE_SLOT)]
    transform_type: u32,
    #[webapi(slot = SVG_TRANSFORM_ANGLE_SLOT)]
    angle: f64,
    #[webapi(slot = SVG_TRANSFORM_MATRIX_SLOT)]
    matrix: v8::Local<'scope, v8::Object>,
    #[webapi(method, callback = svg_transform_set_matrix_callback, length = 0)]
    set_matrix: (),
    #[webapi(method, callback = svg_transform_set_translate_callback, length = 2)]
    set_translate: (),
    #[webapi(method, callback = svg_transform_set_scale_callback, length = 2)]
    set_scale: (),
    #[webapi(method, callback = svg_transform_set_rotate_callback, length = 3)]
    set_rotate: (),
    #[webapi(method, callback = svg_transform_set_skew_x_callback, length = 1)]
    set_skew_x: (),
    #[webapi(method, callback = svg_transform_set_skew_y_callback, length = 1)]
    set_skew_y: (),
}

pub(super) fn build_svg_animated_string_for_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) -> v8::Local<'s, v8::Object> {
    let value = svg_owner_attribute_value(scope, owner, attribute).unwrap_or_default();
    let object = SvgAnimatedStringObjectDeclaration::new(value.clone(), value)
        .bind(scope)
        .expect("SVGAnimatedString declaration should bind");
    set_svg_animated_string_owner_attribute(scope, object, owner, attribute);
    object
}

pub(super) fn build_svg_animated_boolean_for_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
    initial_value: bool,
) -> v8::Local<'s, v8::Object> {
    let object = SvgAnimatedBooleanObjectDeclaration::new(initial_value, initial_value)
        .bind(scope)
        .expect("SVGAnimatedBoolean declaration should bind");
    set_private_value(
        scope,
        object,
        SVG_ANIMATED_BOOLEAN_OWNER_ELEMENT_SLOT,
        owner.into(),
    );
    set_private_value(
        scope,
        object,
        SVG_ANIMATED_BOOLEAN_OWNER_ATTRIBUTE_SLOT,
        v8_string(scope, attribute)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
    set_private_value(
        scope,
        object,
        SVG_ANIMATED_BOOLEAN_INITIAL_VALUE_SLOT,
        v8::Boolean::new(scope, initial_value).into(),
    );
    sync_svg_animated_boolean_from_owner_attribute(scope, object);
    object
}

pub(super) fn sync_svg_animated_boolean_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = get_private_value(scope, animated, SVG_ANIMATED_BOOLEAN_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(attribute) =
        get_private_value(scope, animated, SVG_ANIMATED_BOOLEAN_OWNER_ATTRIBUTE_SLOT)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .filter(|value| !value.is_empty())
    else {
        return;
    };
    let initial_value = get_private_value(scope, animated, SVG_ANIMATED_BOOLEAN_INITIAL_VALUE_SLOT)
        .is_some_and(|value| value.is_true());
    let value = match svg_owner_attribute_value(scope, owner, &attribute).as_deref() {
        Some("true") => true,
        Some("false") => false,
        _ => initial_value,
    };
    set_svg_animated_boolean_values(scope, animated, value);
}

pub(super) fn set_svg_animated_boolean_values(
    scope: &mut v8::PinScope<'_, '_>,
    animated: v8::Local<'_, v8::Object>,
    value: bool,
) {
    let value = v8::Boolean::new(scope, value);
    set_private_value(
        scope,
        animated,
        SVG_ANIMATED_BOOLEAN_BASE_VAL_SLOT,
        value.into(),
    );
    set_private_value(
        scope,
        animated,
        SVG_ANIMATED_BOOLEAN_ANIM_VAL_SLOT,
        value.into(),
    );
}

pub(super) fn reflect_svg_animated_boolean_to_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = get_private_value(scope, animated, SVG_ANIMATED_BOOLEAN_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(attribute) =
        get_private_value(scope, animated, SVG_ANIMATED_BOOLEAN_OWNER_ATTRIBUTE_SLOT)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .filter(|value| !value.is_empty())
    else {
        return;
    };
    let value = get_private_value(scope, animated, SVG_ANIMATED_BOOLEAN_BASE_VAL_SLOT)
        .is_some_and(|value| value.is_true());
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, owner)
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_attribute(
        scope,
        runtime_ptr,
        handle,
        &attribute,
        if value { "true" } else { "false" },
    );
}

pub(super) fn build_svg_animated_length_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let base_val = build_svg_length_list(scope, false);
    let anim_val = build_svg_length_list(scope, true);
    SvgAnimatedLengthListObjectDeclaration::new(base_val, anim_val)
        .bind(scope)
        .expect("SVGAnimatedLengthList declaration should bind")
}

pub(super) fn build_svg_animated_number_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let base_val = build_svg_number_list(scope, false);
    let anim_val = build_svg_number_list(scope, true);
    SvgAnimatedNumberListObjectDeclaration::new(base_val, anim_val)
        .bind(scope)
        .expect("SVGAnimatedNumberList declaration should bind")
}

pub(super) fn build_svg_animated_value_list_for_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
    kind: SvgListKind,
) -> v8::Local<'s, v8::Value> {
    let object = match kind {
        SvgListKind::Length => build_svg_animated_length_list(scope),
        SvgListKind::Number => build_svg_animated_number_list(scope),
        SvgListKind::Point => unreachable!("SVGPointList is not an animated wrapper"),
    };
    sync_svg_animated_value_list_from_owner_attribute(scope, object, owner, attribute, kind);
    if let Some(base_val) = svg_animated_value_list_member(scope, object, "baseVal", kind) {
        set_svg_value_list_owner_attribute(scope, base_val, owner, attribute);
    }
    object.into()
}

pub(super) fn build_svg_animated_transform_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let base_val = build_svg_transform_list(scope);
    let anim_val = build_svg_transform_list(scope);
    SvgAnimatedTransformListObjectDeclaration::new(base_val, anim_val)
        .bind(scope)
        .expect("SVGAnimatedTransformList declaration should bind")
}

pub(super) fn build_svg_animated_transform_list_for_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) -> v8::Local<'s, v8::Object> {
    let object = build_svg_animated_transform_list(scope);
    sync_svg_animated_transform_list_from_owner_attribute(scope, object, owner, attribute);
    if let Some(base_val) = svg_animated_transform_list_member(scope, object, "baseVal") {
        set_svg_transform_list_owner_attribute(scope, base_val, owner, attribute);
    }
    object
}

pub(super) fn build_svg_animated_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: f64,
) -> v8::Local<'s, v8::Object> {
    SvgAnimatedNumberObjectDeclaration::new(value, value)
        .bind(scope)
        .expect("SVGAnimatedNumber declaration should bind")
}

pub(super) fn build_svg_animated_number_for_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) -> v8::Local<'s, v8::Object> {
    let object = build_svg_animated_number(scope, 0.0);
    set_svg_animated_number_owner_attribute(scope, object, owner, attribute);
    sync_svg_animated_number_from_owner_attribute(scope, object, owner, attribute);
    object
}

pub(super) fn build_svg_animated_number_for_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    property: SvgAnimatedNumberProperty,
) -> v8::Local<'s, v8::Object> {
    let object = build_svg_animated_number(scope, property.initial_value);
    set_svg_animated_number_owner_attribute(scope, object, owner, property.attribute);
    let index = u32::try_from(property.index).expect("SVG number property index exceeds u32");
    let index = v8::Integer::new_from_unsigned(scope, index);
    set_private_value(
        scope,
        object,
        SVG_ANIMATED_NUMBER_PROPERTY_INDEX_SLOT,
        index.into(),
    );
    sync_svg_animated_number_from_property(scope, object);
    object
}

pub(super) fn svg_animated_number_property_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
) -> Option<SvgAnimatedNumberProperty> {
    let index = get_private_value(scope, animated, SVG_ANIMATED_NUMBER_PROPERTY_INDEX_SLOT)?
        .uint32_value(scope)?;
    SVG_ANIMATED_NUMBER_PROPERTIES.get(index as usize).copied()
}

pub(super) fn svg_animated_number_cache_slot(property: SvgAnimatedNumberProperty) -> String {
    format!("__moliSvgAnimatedNumberProperty{}", property.index)
}

pub(super) fn build_svg_animated_integer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: i32,
) -> v8::Local<'s, v8::Object> {
    SvgAnimatedIntegerObjectDeclaration::new(value, value)
        .bind(scope)
        .expect("SVGAnimatedInteger declaration should bind")
}

pub(super) fn build_svg_animated_integer_for_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    property: SvgAnimatedIntegerProperty,
) -> v8::Local<'s, v8::Object> {
    let object = build_svg_animated_integer(scope, property.initial_value);
    set_private_value(
        scope,
        object,
        SVG_ANIMATED_INTEGER_OWNER_ELEMENT_SLOT,
        owner.into(),
    );
    let index = u32::try_from(property.index).expect("SVG integer property index exceeds u32");
    let index = v8::Integer::new_from_unsigned(scope, index);
    set_private_value(
        scope,
        object,
        SVG_ANIMATED_INTEGER_PROPERTY_INDEX_SLOT,
        index.into(),
    );
    sync_svg_animated_integer_from_owner_attribute(scope, object);
    object
}

pub(super) fn svg_animated_integer_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
) -> Option<SvgAnimatedIntegerProperty> {
    let index = get_private_value(scope, animated, SVG_ANIMATED_INTEGER_PROPERTY_INDEX_SLOT)?
        .uint32_value(scope)?;
    SVG_ANIMATED_INTEGER_PROPERTIES.get(index as usize).copied()
}

pub(super) fn sync_svg_animated_integer_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
) {
    let Some(property) = svg_animated_integer_property(scope, animated) else {
        return;
    };
    let Some(owner) = get_private_value(scope, animated, SVG_ANIMATED_INTEGER_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let value = svg_owner_attribute_value(scope, owner, property.attribute)
        .as_deref()
        .and_then(|raw| parse_svg_animated_integer(property, raw))
        .unwrap_or(property.initial_value);
    set_svg_animated_integer_values(scope, animated, value);
}

pub(super) fn reflect_svg_animated_integer_to_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
) {
    let Some(property) = svg_animated_integer_property(scope, animated) else {
        return;
    };
    let Some(owner) = get_private_value(scope, animated, SVG_ANIMATED_INTEGER_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let value = get_private_value(scope, animated, SVG_ANIMATED_INTEGER_BASE_VAL_SLOT)
        .and_then(|value| value.int32_value(scope))
        .unwrap_or(property.initial_value);
    let serialized = match property.component {
        SvgAnimatedIntegerComponent::Scalar => value.to_string(),
        SvgAnimatedIntegerComponent::PairFirst | SvgAnimatedIntegerComponent::PairSecondOrFirst => {
            let (mut first, mut second) =
                svg_owner_attribute_value(scope, owner, property.attribute)
                    .as_deref()
                    .and_then(parse_svg_integer_pair)
                    .unwrap_or((property.initial_value, property.initial_value));
            match property.component {
                SvgAnimatedIntegerComponent::PairFirst => first = value,
                SvgAnimatedIntegerComponent::PairSecondOrFirst => second = value,
                SvgAnimatedIntegerComponent::Scalar => unreachable!(),
            }
            format!("{first} {second}")
        }
    };
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, owner)
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_attribute(scope, runtime_ptr, handle, property.attribute, &serialized);
}

pub(super) fn set_svg_animated_integer_values(
    scope: &mut v8::PinScope<'_, '_>,
    animated: v8::Local<'_, v8::Object>,
    value: i32,
) {
    let value = v8::Integer::new(scope, value);
    set_private_value(
        scope,
        animated,
        SVG_ANIMATED_INTEGER_BASE_VAL_SLOT,
        value.into(),
    );
    set_private_value(
        scope,
        animated,
        SVG_ANIMATED_INTEGER_ANIM_VAL_SLOT,
        value.into(),
    );
}

pub(super) fn parse_svg_animated_integer(
    property: SvgAnimatedIntegerProperty,
    value: &str,
) -> Option<i32> {
    match property.component {
        SvgAnimatedIntegerComponent::Scalar => parse_svg_integer(value),
        SvgAnimatedIntegerComponent::PairFirst => {
            parse_svg_integer_pair(value).map(|(first, _)| first)
        }
        SvgAnimatedIntegerComponent::PairSecondOrFirst => {
            parse_svg_integer_pair(value).map(|(_, second)| second)
        }
    }
}

fn parse_svg_integer(value: &str) -> Option<i32> {
    value.trim().parse().ok()
}

fn parse_svg_integer_pair(value: &str) -> Option<(i32, i32)> {
    let value = value.trim();
    if let Some((first, second)) = value.split_once(',') {
        if second.contains(',') {
            return None;
        }
        return Some((parse_svg_integer(first)?, parse_svg_integer(second)?));
    }
    let mut values = value.split_ascii_whitespace();
    let first = parse_svg_integer(values.next()?)?;
    let second = match values.next() {
        Some(value) => parse_svg_integer(value)?,
        None => first,
    };
    values.next().is_none().then_some((first, second))
}

pub(super) fn build_svg_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: f64,
) -> v8::Local<'s, v8::Object> {
    SvgNumberObjectDeclaration::new(value)
        .bind(scope)
        .expect("SVGNumber declaration should bind")
}

pub(super) fn build_svg_animated_enumeration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: u32,
) -> v8::Local<'s, v8::Object> {
    SvgAnimatedEnumerationObjectDeclaration::new(value, value)
        .bind(scope)
        .expect("SVGAnimatedEnumeration declaration should bind")
}

pub(super) fn build_svg_animated_enumeration_for_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    property: SvgAnimatedEnumerationProperty,
) -> v8::Local<'s, v8::Object> {
    let object = build_svg_animated_enumeration(scope, property.initial_value);
    set_private_value(
        scope,
        object,
        SVG_ANIMATED_ENUMERATION_OWNER_ELEMENT_SLOT,
        owner.into(),
    );
    let index = u32::try_from(property.index).expect("SVG enumeration property index exceeds u32");
    let index = v8::Integer::new_from_unsigned(scope, index);
    set_private_value(
        scope,
        object,
        SVG_ANIMATED_ENUMERATION_PROPERTY_INDEX_SLOT,
        index.into(),
    );
    sync_svg_animated_enumeration_from_owner_attribute(scope, object);
    object
}

pub(super) fn svg_animated_enumeration_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
) -> Option<SvgAnimatedEnumerationProperty> {
    let index = get_private_value(
        scope,
        animated,
        SVG_ANIMATED_ENUMERATION_PROPERTY_INDEX_SLOT,
    )?
    .uint32_value(scope)?;
    SVG_ANIMATED_ENUMERATION_PROPERTIES
        .get(index as usize)
        .copied()
}

pub(super) fn sync_svg_animated_enumeration_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
) {
    let Some(property) = svg_animated_enumeration_property(scope, animated) else {
        return;
    };
    let Some(owner) =
        get_private_value(scope, animated, SVG_ANIMATED_ENUMERATION_OWNER_ELEMENT_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let value = svg_owner_attribute_value(scope, owner, property.attribute)
        .as_deref()
        .and_then(|raw| parse_svg_animated_enumeration(property.kind, raw))
        .unwrap_or(property.initial_value);
    set_svg_animated_enumeration_values(scope, animated, value);
}

pub(super) fn reflect_svg_animated_enumeration_to_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
) {
    let Some(property) = svg_animated_enumeration_property(scope, animated) else {
        return;
    };
    let Some(owner) =
        get_private_value(scope, animated, SVG_ANIMATED_ENUMERATION_OWNER_ELEMENT_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(value) =
        svg_length_number_slot(scope, animated, SVG_ANIMATED_ENUMERATION_BASE_VAL_SLOT)
            .map(|value| value as u32)
            .and_then(|value| serialize_svg_animated_enumeration(property.kind, value))
    else {
        return;
    };
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, owner)
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_attribute(scope, runtime_ptr, handle, property.attribute, value);
}

pub(super) fn set_svg_animated_enumeration_values(
    scope: &mut v8::PinScope<'_, '_>,
    animated: v8::Local<'_, v8::Object>,
    value: u32,
) {
    let value = v8::Integer::new_from_unsigned(scope, value);
    set_private_value(
        scope,
        animated,
        SVG_ANIMATED_ENUMERATION_BASE_VAL_SLOT,
        value.into(),
    );
    set_private_value(
        scope,
        animated,
        SVG_ANIMATED_ENUMERATION_ANIM_VAL_SLOT,
        value.into(),
    );
}

pub(super) fn parse_svg_animated_enumeration(
    kind: SvgAnimatedEnumerationKind,
    value: &str,
) -> Option<u32> {
    match kind {
        SvgAnimatedEnumerationKind::Keywords(values) => values
            .iter()
            .find_map(|(keyword, enumerated)| (*keyword == value).then_some(*enumerated)),
        SvgAnimatedEnumerationKind::MarkerOrient => match value {
            "auto" => Some(SVG_MARKER_ORIENT_AUTO),
            "auto-start-reverse" => Some(SVG_MARKER_ORIENT_AUTO_START_REVERSE),
            value if parse_svg_angle_value(value).is_some() => Some(SVG_MARKER_ORIENT_ANGLE),
            _ => None,
        },
    }
}

pub(super) fn serialize_svg_animated_enumeration(
    kind: SvgAnimatedEnumerationKind,
    value: u32,
) -> Option<&'static str> {
    match kind {
        SvgAnimatedEnumerationKind::Keywords(values) => values
            .iter()
            .find_map(|(keyword, enumerated)| (*enumerated == value).then_some(*keyword)),
        SvgAnimatedEnumerationKind::MarkerOrient => match value {
            SVG_MARKER_ORIENT_AUTO => Some("auto"),
            SVG_MARKER_ORIENT_ANGLE => Some("0"),
            SVG_MARKER_ORIENT_AUTO_START_REVERSE => Some("auto-start-reverse"),
            _ => None,
        },
    }
}

pub(super) fn build_svg_length_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    read_only: bool,
) -> v8::Local<'s, v8::Object> {
    let template = v8::ObjectTemplate::new(scope);
    configure_svg_value_list_indexed_property_handler(template);
    let object = template
        .new_instance(scope)
        .expect("SVGLengthList object template should instantiate");
    SvgLengthListObjectDeclaration::new(Vec::new(), read_only)
        .bind_into(scope, object)
        .expect("SVGLengthList declaration should bind");
    object
}

pub(super) fn build_svg_number_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    read_only: bool,
) -> v8::Local<'s, v8::Object> {
    let template = v8::ObjectTemplate::new(scope);
    configure_svg_value_list_indexed_property_handler(template);
    let object = template
        .new_instance(scope)
        .expect("SVGNumberList object template should instantiate");
    SvgNumberListObjectDeclaration::new(Vec::new(), read_only)
        .bind_into(scope, object)
        .expect("SVGNumberList declaration should bind");
    object
}

pub(super) fn build_svg_point_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    read_only: bool,
) -> v8::Local<'s, v8::Object> {
    let template = v8::ObjectTemplate::new(scope);
    configure_svg_value_list_indexed_property_handler(template);
    let object = template
        .new_instance(scope)
        .expect("SVGPointList object template should instantiate");
    SvgPointListObjectDeclaration::new(Vec::new(), read_only)
        .bind_into(scope, object)
        .expect("SVGPointList declaration should bind");
    object
}

pub(super) fn build_svg_point_list_for_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    read_only: bool,
) -> v8::Local<'s, v8::Object> {
    let list = build_svg_point_list(scope, read_only);
    set_svg_value_list_owner_attribute(scope, list, owner, "points");
    sync_svg_value_list_from_owner_attribute(scope, list, SvgListKind::Point);
    list
}

pub(super) fn build_svg_string_list_for_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) -> v8::Local<'s, v8::Object> {
    let template = v8::ObjectTemplate::new(scope);
    configure_svg_string_list_indexed_property_handler(template);
    let object = template
        .new_instance(scope)
        .expect("SVGStringList object template should instantiate");
    SvgStringListObjectDeclaration::new(Vec::new())
        .bind_into(scope, object)
        .expect("SVGStringList declaration should bind");
    set_private_value(
        scope,
        object,
        SVG_STRING_LIST_OWNER_ELEMENT_SLOT,
        owner.into(),
    );
    set_private_value(
        scope,
        object,
        SVG_STRING_LIST_OWNER_ATTRIBUTE_SLOT,
        v8_string(scope, attribute)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
    sync_svg_string_list_from_owner_attribute(scope, object);
    object
}

pub(super) fn svg_string_list_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, list, SVG_STRING_LIST_ITEMS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(super) fn set_svg_string_list_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    items: v8::Local<'s, v8::Array>,
) {
    set_private_value(scope, list, SVG_STRING_LIST_ITEMS_SLOT, items.into());
}

pub(super) fn sync_svg_string_list_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) {
    let Some((owner, attribute)) = svg_string_list_owner_attribute(scope, list) else {
        return;
    };
    let raw = svg_owner_attribute_value(scope, owner, &attribute);
    let synced_value = get_private_value(scope, list, SVG_STRING_LIST_SYNCED_ATTRIBUTE_VALUE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    let synced_present =
        get_private_value(scope, list, SVG_STRING_LIST_SYNCED_ATTRIBUTE_PRESENT_SLOT)
            .map(|value| value.is_true());
    if synced_present == Some(raw.is_some())
        && synced_value.as_deref() == Some(raw.as_deref().unwrap_or_default())
    {
        return;
    }

    let values = parse_svg_string_list_attribute(&attribute, raw.as_deref());
    let length = i32::try_from(values.len()).unwrap_or(i32::MAX);
    let items = v8::Array::new(scope, length);
    for (index, value) in values.into_iter().enumerate() {
        let Ok(index) = u32::try_from(index) else {
            break;
        };
        if let Some(value) = v8_string(scope, &value) {
            let _ = items.set_index(scope, index, value.into());
        }
    }
    set_svg_string_list_items(scope, list, items);
    set_svg_string_list_synced_attribute(scope, list, raw.as_deref());
}

pub(super) fn reflect_svg_string_list_to_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) {
    let Some((owner, attribute)) = svg_string_list_owner_attribute(scope, list) else {
        return;
    };
    let value = serialize_svg_string_list(scope, list, &attribute);
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, owner)
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_attribute(scope, runtime_ptr, handle, &attribute, &value);
    set_svg_string_list_synced_attribute(scope, list, Some(&value));
}

fn svg_string_list_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, String)> {
    let owner = get_private_value(scope, list, SVG_STRING_LIST_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let attribute = get_private_value(scope, list, SVG_STRING_LIST_OWNER_ATTRIBUTE_SLOT)?
        .to_string(scope)?
        .to_rust_string_lossy(scope);
    (!attribute.is_empty()).then_some((owner, attribute))
}

fn set_svg_string_list_synced_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    raw: Option<&str>,
) {
    set_private_value(
        scope,
        list,
        SVG_STRING_LIST_SYNCED_ATTRIBUTE_VALUE_SLOT,
        v8_string(scope, raw.unwrap_or_default())
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
    set_private_value(
        scope,
        list,
        SVG_STRING_LIST_SYNCED_ATTRIBUTE_PRESENT_SLOT,
        v8::Boolean::new(scope, raw.is_some()).into(),
    );
}

fn parse_svg_string_list_attribute(attribute: &str, raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    if attribute == "systemLanguage" {
        return raw
            .split(',')
            .map(|token| token.trim_matches(is_html_ascii_whitespace).to_owned())
            .collect();
    }
    raw.split(is_html_ascii_whitespace)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

fn serialize_svg_string_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    attribute: &str,
) -> String {
    let Some(items) = svg_string_list_items(scope, list) else {
        return String::new();
    };
    let values = (0..items.length())
        .filter_map(|index| items.get_index(scope, index))
        .filter_map(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .collect::<Vec<_>>();
    values.join(if attribute == "systemLanguage" {
        ","
    } else {
        " "
    })
}

fn is_html_ascii_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}' | '\u{000A}' | '\u{000C}' | '\u{000D}' | '\u{0020}'
    )
}

pub(super) fn build_svg_transform_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    SvgTransformListObjectDeclaration::new(Vec::new())
        .bind(scope)
        .expect("SVGTransformList declaration should bind")
}

pub(super) fn build_svg_transform<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transform: SvgTransform,
) -> v8::Local<'s, v8::Object> {
    let matrix = build_svg_matrix(scope, transform.matrix);
    SvgTransformObjectDeclaration::new(
        svg_transform_type_for_kind(transform.kind),
        transform.angle,
        matrix,
    )
    .bind(scope)
    .expect("SVGTransform declaration should bind")
}

pub(super) fn build_svg_matrix<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    components: SvgMatrixComponents,
) -> v8::Local<'s, v8::Object> {
    super::super::ensure_intrinsic_interface_prototype(scope, "SVGMatrix")
        .expect("SVGMatrix compatibility prototype should materialize");
    SvgMatrixObjectDeclaration::new(
        components.a,
        components.b,
        components.c,
        components.d,
        components.e,
        components.f,
    )
    .bind(scope)
    .expect("SVGMatrix declaration should bind")
}

pub(super) fn build_svg_animated_length_for_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
    initial_value: &str,
) -> v8::Local<'s, v8::Object> {
    let parsed = svg_animated_length_attribute_value(scope, owner, attribute, initial_value);
    let base_val = build_svg_length_from_parsed(scope, parsed);
    set_svg_length_owner_attribute(scope, base_val, owner, attribute);
    let anim_val = build_svg_length_from_parsed(scope, parsed);
    SvgAnimatedLengthObjectDeclaration::new(base_val, anim_val)
        .bind(scope)
        .expect("SVGAnimatedLength declaration should bind")
}

pub(super) fn build_svg_animated_angle_for_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) -> v8::Local<'s, v8::Object> {
    let parsed = svg_owner_attribute_value(scope, owner, attribute)
        .as_deref()
        .and_then(parse_svg_orient_angle_value)
        .unwrap_or_default();
    let base_val = build_svg_angle_from_parsed(scope, &parsed, false);
    set_svg_angle_owner_attribute(scope, base_val, owner, attribute);
    let anim_val = build_svg_angle_from_parsed(scope, &parsed, true);
    set_svg_angle_owner_attribute(scope, anim_val, owner, attribute);
    SvgAnimatedAngleObjectDeclaration::new(base_val, anim_val)
        .bind(scope)
        .expect("SVGAnimatedAngle declaration should bind")
}

pub(super) fn build_svg_animated_rect_for_view_box<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let values = svg_view_box_attribute_value(scope, owner);
    let base_val = super::super::dom_rect::build_svg_view_box_rect(scope, owner, values, false);
    let anim_val = super::super::dom_rect::build_svg_view_box_rect(scope, owner, values, true);
    SvgAnimatedRectObjectDeclaration::new(base_val, anim_val)
        .bind(scope)
        .expect("SVGAnimatedRect declaration should bind")
}

pub(super) fn build_svg_animated_preserve_aspect_ratio<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let [align, meet_or_slice] = svg_preserve_aspect_ratio_attribute_value(scope, owner);
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

pub(super) fn build_svg_angle<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    build_svg_angle_from_parsed(scope, &SvgParsedAngle::default(), false)
}

pub(super) fn build_svg_angle_from_parsed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: &SvgParsedAngle,
    read_only: bool,
) -> v8::Local<'s, v8::Object> {
    SvgAngleObjectDeclaration::new(
        parsed.unit_type,
        parsed.value,
        parsed.value_in_specified_units,
        parsed.value_as_string.clone(),
        read_only,
    )
    .bind(scope)
    .expect("SVGAngle declaration should bind")
}

pub(super) fn build_svg_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: f64,
) -> v8::Local<'s, v8::Object> {
    build_svg_length_from_parsed(
        scope,
        SvgParsedLength {
            value,
            unit_type: SVG_LENGTH_TYPE_NUMBER,
            unit: SvgLengthUnit::Number,
            raw: None,
        },
    )
}

pub(super) fn build_svg_length_from_parsed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: SvgParsedLength,
) -> v8::Local<'s, v8::Object> {
    let value_as_string = parsed
        .raw
        .map(str::to_owned)
        .or_else(|| Some(SvgLength::new(parsed.value, parsed.unit).serialize()))
        .unwrap_or_else(|| "0".to_owned());
    let value =
        resolve_svg_length_without_context(parsed.value, parsed.unit).unwrap_or(parsed.value);
    SvgLengthObjectDeclaration::new(
        parsed.unit_type,
        parsed.unit.suffix().to_owned(),
        value,
        parsed.value,
        value_as_string,
    )
    .bind(scope)
    .expect("SVGLength declaration should bind")
}

pub(super) fn build_dom_point_like<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    x: f64,
    y: f64,
) -> v8::Local<'s, v8::Object> {
    build_dom_point_object(scope, x, y, 0.0, 1.0)
}

pub(super) fn build_dom_rect_like<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> v8::Local<'s, v8::Object> {
    build_dom_rect_object(scope, x, y, width, height)
}

pub(super) fn build_zero_dom_rect_like<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    build_dom_rect_like(scope, 0.0, 0.0, 0.0, 0.0)
}

pub(super) fn svg_geometry_segments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> Vec<SvgGeometrySegment> {
    let Some(geometry) = svg_geometry_element(scope, element) else {
        return Vec::new();
    };
    svg_geometry::segments_for_element(geometry)
}

pub(super) fn svg_geometry_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> Option<SvgGeometryElement> {
    let (runtime_ptr, handle) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, element).ok()?;
    svg_geometry_element_for_handle(unsafe { &*runtime_ptr }, handle)
}

pub(super) fn svg_geometry_element_for_handle(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> Option<SvgGeometryElement> {
    let local_name = runtime
        .dom_host()
        .node(handle)?
        .local_name()?
        .to_ascii_lowercase();
    match local_name.as_str() {
        "circle" => Some(SvgGeometryElement::Circle {
            cx: svg_geometry_length_attribute_for_handle(runtime, handle, "cx"),
            cy: svg_geometry_length_attribute_for_handle(runtime, handle, "cy"),
            r: svg_geometry_length_attribute_for_handle(runtime, handle, "r"),
        }),
        "ellipse" => Some(SvgGeometryElement::Ellipse {
            cx: svg_geometry_length_attribute_for_handle(runtime, handle, "cx"),
            cy: svg_geometry_length_attribute_for_handle(runtime, handle, "cy"),
            rx: svg_geometry_length_attribute_for_handle(runtime, handle, "rx"),
            ry: svg_geometry_length_attribute_for_handle(runtime, handle, "ry"),
        }),
        "line" => Some(SvgGeometryElement::Line {
            x1: svg_geometry_length_attribute_for_handle(runtime, handle, "x1"),
            y1: svg_geometry_length_attribute_for_handle(runtime, handle, "y1"),
            x2: svg_geometry_length_attribute_for_handle(runtime, handle, "x2"),
            y2: svg_geometry_length_attribute_for_handle(runtime, handle, "y2"),
        }),
        "path" => Some(SvgGeometryElement::Path {
            d: svg_computed_path_data(runtime, handle)
                .or_else(|| runtime.dom_host().get_attribute(handle, "d"))
                .unwrap_or_default(),
        }),
        "polygon" => Some(SvgGeometryElement::Polygon {
            points: runtime
                .dom_host()
                .get_attribute(handle, "points")
                .unwrap_or_default(),
        }),
        "polyline" => Some(SvgGeometryElement::Polyline {
            points: runtime
                .dom_host()
                .get_attribute(handle, "points")
                .unwrap_or_default(),
        }),
        "rect" => Some(SvgGeometryElement::Rect {
            x: svg_geometry_length_attribute_for_handle(runtime, handle, "x"),
            y: svg_geometry_length_attribute_for_handle(runtime, handle, "y"),
            width: svg_geometry_length_attribute_for_handle(runtime, handle, "width"),
            height: svg_geometry_length_attribute_for_handle(runtime, handle, "height"),
            rx: svg_geometry_rect_radius_attribute_for_handle(runtime, handle, "rx", "ry"),
            ry: svg_geometry_rect_radius_attribute_for_handle(runtime, handle, "ry", "rx"),
        }),
        _ => None,
    }
}

fn svg_computed_path_data(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> Option<String> {
    let computed =
        crate::native_bridge::element::computed_style_property_for_handle(runtime, handle, "d");
    let value = computed.trim();
    let inner = value.strip_prefix("path(")?.strip_suffix(')')?.trim();
    let quote = inner.chars().next()?;
    if !matches!(quote, '\'' | '"') || !inner.ends_with(quote) {
        return None;
    }
    Some(inner[quote.len_utf8()..inner.len() - quote.len_utf8()].to_owned())
}

fn svg_geometry_length_attribute_for_handle(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    attribute: &str,
) -> f64 {
    runtime
        .dom_host()
        .get_attribute(handle, attribute)
        .as_deref()
        .and_then(parse_svg_length_value)
        .filter(|parsed| parsed.value.is_finite())
        .map(|parsed| parsed.value)
        .unwrap_or(0.0)
}

fn svg_geometry_rect_radius_attribute_for_handle(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    attribute: &str,
    fallback_attribute: &str,
) -> f64 {
    svg_geometry_optional_length_attribute_for_handle(runtime, handle, attribute)
        .or_else(|| {
            svg_geometry_optional_length_attribute_for_handle(runtime, handle, fallback_attribute)
        })
        .unwrap_or(0.0)
}

fn svg_geometry_optional_length_attribute_for_handle(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    attribute: &str,
) -> Option<f64> {
    runtime
        .dom_host()
        .get_attribute(handle, attribute)
        .as_deref()
        .and_then(parse_svg_length_value)
        .filter(|parsed| parsed.value.is_finite())
        .map(|parsed| parsed.value)
}

pub(super) fn svg_graphics_bounding_box<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> Option<SvgGeometryBox> {
    let (runtime_ptr, handle) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, element).ok()?;
    let runtime = unsafe { &*runtime_ptr };
    let node = runtime.dom_host().node(handle)?;
    if !node.is_connected() || svg_bbox_has_non_rendered_ancestor(runtime, handle) {
        return None;
    }
    svg_bbox_for_handle(
        runtime,
        handle,
        SvgMatrixComponents::identity(),
        false,
        &mut Vec::new(),
    )
}

fn svg_bbox_for_handle(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    transform: SvgMatrixComponents,
    referenced: bool,
    reference_stack: &mut Vec<crate::document_runtime::DomHandle>,
) -> Option<SvgGeometryBox> {
    let local_name = runtime.dom_host().node(handle)?.local_name()?;
    if svg_element_has_display_none(runtime, handle)
        || (!referenced && svg_local_name_is_non_rendered_container(local_name))
    {
        return None;
    }

    if let Some(geometry) = svg_geometry_element_for_handle(runtime, handle) {
        return svg_geometry::bounding_box_for_transformed_element(&geometry, transform);
    }

    match local_name {
        "text" => None,
        "tspan" => svg_text_bounding_box(runtime, handle, transform),
        "image" => {
            let (width, height) = svg_image_dimensions(runtime, handle);
            let geometry = SvgGeometryElement::Rect {
                x: svg_geometry_length_attribute_for_handle(runtime, handle, "x"),
                y: svg_geometry_length_attribute_for_handle(runtime, handle, "y"),
                width,
                height,
                rx: 0.0,
                ry: 0.0,
            };
            svg_geometry::bounding_box_for_transformed_element(&geometry, transform)
        }
        "foreignObject" => {
            let geometry = SvgGeometryElement::Rect {
                x: svg_geometry_length_attribute_for_handle(runtime, handle, "x"),
                y: svg_geometry_length_attribute_for_handle(runtime, handle, "y"),
                width: svg_geometry_length_attribute_for_handle(runtime, handle, "width"),
                height: svg_geometry_length_attribute_for_handle(runtime, handle, "height"),
                rx: 0.0,
                ry: 0.0,
            };
            svg_geometry::bounding_box_for_transformed_element(&geometry, transform)
        }
        "use" => svg_use_bounding_box(runtime, handle, transform, reference_stack),
        _ => {
            let mut result: Option<SvgGeometryBox> = None;
            for child in runtime.dom_host().child_handles(handle) {
                let Some(child_name) = runtime
                    .dom_host()
                    .node(child)
                    .and_then(|node| node.local_name())
                else {
                    continue;
                };
                let child_transform = transform.multiply(svg_transform_for_handle(runtime, child));
                let child_transform = if child_name == "svg" {
                    child_transform.multiply(SvgMatrixComponents::translate(
                        svg_geometry_length_attribute_for_handle(runtime, child, "x"),
                        svg_geometry_length_attribute_for_handle(runtime, child, "y"),
                    ))
                } else {
                    child_transform
                };
                if let Some(child_box) = svg_bbox_for_handle(
                    runtime,
                    child,
                    child_transform,
                    referenced,
                    reference_stack,
                ) {
                    result = Some(result.map_or(child_box, |current| current.union(child_box)));
                }
            }
            result
        }
    }
}

fn svg_image_dimensions(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> (f64, f64) {
    let width =
        svg_geometry_optional_length_attribute_for_handle(runtime, handle, "width").unwrap_or(0.0);
    let height =
        svg_geometry_optional_length_attribute_for_handle(runtime, handle, "height").unwrap_or(0.0);
    let Some((intrinsic_width, intrinsic_height)) = runtime.image_resource_intrinsic_size(handle)
    else {
        return (width, height);
    };
    let intrinsic_width = intrinsic_width as f64;
    let intrinsic_height = intrinsic_height as f64;
    if intrinsic_width <= 0.0 || intrinsic_height <= 0.0 {
        return (width, height);
    }
    match (width.is_sign_negative(), height.is_sign_negative()) {
        (true, false) if height > 0.0 => (height * intrinsic_width / intrinsic_height, height),
        (false, true) if width > 0.0 => (width, width * intrinsic_height / intrinsic_width),
        _ => (width, height),
    }
}

fn svg_text_bounding_box(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    transform: SvgMatrixComponents,
) -> Option<SvgGeometryBox> {
    let text_root = svg_text_root(runtime, handle)?;
    let text = runtime.dom_host().text_content(handle)?;
    let character_count = svg_rendered_text_character_count(&text);
    if character_count == 0 {
        return None;
    }

    let font_size = crate::native_bridge::element::computed_style_property_for_handle(
        runtime,
        handle,
        "font-size",
    );
    let font_size = parse_svg_length_value(font_size.trim())
        .map(|length| length.value)
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(16.0);
    let font_family = crate::native_bridge::element::computed_style_property_for_handle(
        runtime,
        handle,
        "font-family",
    );
    let is_ahem = font_family.split(',').any(|family| {
        family
            .trim_matches([' ', '\'', '"'])
            .eq_ignore_ascii_case("Ahem")
    });
    let glyph_advance = if is_ahem { font_size } else { font_size * 0.6 };

    let explicit_x = svg_geometry_optional_length_attribute_for_handle(runtime, handle, "x");
    let preceding_characters = if handle == text_root || explicit_x.is_some() {
        0
    } else {
        svg_text_characters_before(runtime, text_root, handle).unwrap_or(0)
    };
    let x = explicit_x
        .or_else(|| svg_inherited_text_position(runtime, handle, text_root, "x"))
        .unwrap_or(0.0)
        + svg_geometry_optional_length_attribute_for_handle(runtime, handle, "dx")
            .or_else(|| svg_inherited_text_position(runtime, handle, text_root, "dx"))
            .unwrap_or(0.0)
        + preceding_characters as f64 * glyph_advance;
    let baseline = svg_geometry_optional_length_attribute_for_handle(runtime, handle, "y")
        .or_else(|| svg_inherited_text_position(runtime, handle, text_root, "y"))
        .unwrap_or(0.0)
        + svg_geometry_optional_length_attribute_for_handle(runtime, handle, "dy")
            .or_else(|| svg_inherited_text_position(runtime, handle, text_root, "dy"))
            .unwrap_or(0.0);

    let geometry = SvgGeometryElement::Rect {
        x,
        y: baseline - font_size * 0.8,
        width: character_count as f64 * glyph_advance,
        height: font_size,
        rx: 0.0,
        ry: 0.0,
    };
    svg_geometry::bounding_box_for_transformed_element(&geometry, transform)
}

fn svg_text_root(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> Option<crate::document_runtime::DomHandle> {
    let mut current = Some(handle);
    while let Some(candidate) = current {
        let node = runtime.dom_host().node(candidate)?;
        if node.local_name() == Some("text") {
            return Some(candidate);
        }
        current = node.parent_node_id();
    }
    None
}

fn svg_inherited_text_position(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    text_root: crate::document_runtime::DomHandle,
    attribute: &str,
) -> Option<f64> {
    let mut current = runtime.dom_host().node(handle)?.parent_node_id();
    while let Some(candidate) = current {
        if let Some(value) =
            svg_geometry_optional_length_attribute_for_handle(runtime, candidate, attribute)
        {
            return Some(value);
        }
        if candidate == text_root {
            break;
        }
        current = runtime.dom_host().node(candidate)?.parent_node_id();
    }
    None
}

fn svg_text_characters_before(
    runtime: &crate::native_bridge::JsContextHost,
    root: crate::document_runtime::DomHandle,
    target: crate::document_runtime::DomHandle,
) -> Option<usize> {
    fn visit(
        runtime: &crate::native_bridge::JsContextHost,
        current: crate::document_runtime::DomHandle,
        target: crate::document_runtime::DomHandle,
        count: &mut usize,
    ) -> bool {
        for child in runtime.dom_host().child_handles(current) {
            if child == target {
                return true;
            }
            let Some(node) = runtime.dom_host().node(child) else {
                continue;
            };
            if let Some(value) = node.data_value()
                && node.is_text()
            {
                *count += svg_rendered_text_character_count(value);
                continue;
            }
            if visit(runtime, child, target, count) {
                return true;
            }
        }
        false
    }

    let mut count = 0;
    visit(runtime, root, target, &mut count).then_some(count)
}

fn svg_rendered_text_character_count(text: &str) -> usize {
    text.split_whitespace()
        .map(|chunk| chunk.chars().count())
        .sum()
}

fn svg_use_bounding_box(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    transform: SvgMatrixComponents,
    reference_stack: &mut Vec<crate::document_runtime::DomHandle>,
) -> Option<SvgGeometryBox> {
    if reference_stack.contains(&handle) {
        return None;
    }
    let href = runtime
        .dom_host()
        .get_attribute(handle, "href")
        .or_else(|| runtime.dom_host().get_attribute(handle, "xlink:href"))?;
    let target = runtime.get_element_by_id(href.trim().strip_prefix('#')?)?;
    reference_stack.push(handle);
    let translated = transform.multiply(SvgMatrixComponents::translate(
        svg_geometry_length_attribute_for_handle(runtime, handle, "x"),
        svg_geometry_length_attribute_for_handle(runtime, handle, "y"),
    ));
    let target_transform = translated.multiply(svg_transform_for_handle(runtime, target));
    let result = svg_bbox_for_handle(runtime, target, target_transform, true, reference_stack);
    reference_stack.pop();
    result
}

fn svg_transform_for_handle(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> SvgMatrixComponents {
    runtime
        .dom_host()
        .get_attribute(handle, "transform")
        .as_deref()
        .and_then(svg_geometry::parse_transform_attribute)
        .and_then(|transforms| {
            svg_geometry::consolidate_transform_matrices(
                transforms.into_iter().map(|transform| transform.matrix),
            )
        })
        .unwrap_or_else(SvgMatrixComponents::identity)
}

fn svg_bbox_has_non_rendered_ancestor(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> bool {
    let mut current = Some(handle);
    while let Some(candidate) = current {
        let Some(node) = runtime.dom_host().node(candidate) else {
            return true;
        };
        if node
            .local_name()
            .is_some_and(svg_local_name_is_non_rendered_container)
            || svg_element_has_display_none(runtime, candidate)
        {
            return true;
        }
        current = node.parent_node_id();
    }
    false
}

fn svg_local_name_is_non_rendered_container(local_name: &str) -> bool {
    matches!(
        local_name,
        "clipPath" | "defs" | "marker" | "mask" | "pattern" | "symbol"
    )
}

fn svg_element_has_display_none(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> bool {
    crate::native_bridge::element::computed_style_property_for_handle(runtime, handle, "display")
        .trim()
        .eq_ignore_ascii_case("none")
}

pub(super) fn svg_fill_allows_paint<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> bool {
    svg_owner_attribute_value(scope, element, "fill")
        .is_none_or(|fill| !fill.trim().eq_ignore_ascii_case("none"))
}

pub(super) fn svg_transform_value_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Local::<v8::Object>::try_from(value).ok();
    if let Some(object) = object
        && get_private_value(scope, object, SVG_TRANSFORM_MATRIX_SLOT).is_some()
    {
        return Some(object);
    }
    webidl::throw_type_error(scope, "Argument 1 can not be converted to SVGTransform");
    None
}

pub(super) fn svg_list_item_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    items: v8::Local<'s, v8::Array>,
    index: u32,
) -> Option<v8::Local<'s, v8::Value>> {
    if index >= items.length() {
        webidl::throw_index_size_error(scope);
        return None;
    }
    match items.get_index(scope, index) {
        Some(value) => Some(value),
        None => {
            webidl::throw_index_size_error(scope);
            None
        }
    }
}

pub(super) fn svg_matrix_value_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Local::<v8::Object>::try_from(value).ok();
    if let Some(object) = object
        && get_private_value(scope, object, SVG_MATRIX_A_SLOT).is_some()
    {
        return Some(object);
    }
    webidl::throw_type_error(scope, "Argument 1 can not be converted to SVGMatrix");
    None
}

pub(super) fn svg_value_list_item_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    kind: SvgListKind,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Local::<v8::Object>::try_from(value).ok();
    let valid = object.is_some_and(|object| match kind {
        SvgListKind::Length => get_private_value(scope, object, SVG_LENGTH_VALUE_SLOT).is_some(),
        SvgListKind::Number => get_private_value(scope, object, SVG_NUMBER_VALUE_SLOT).is_some(),
        SvgListKind::Point => {
            dom_point_clone_data(scope, object).is_some_and(|(mutable, _)| mutable)
        }
    });
    if valid {
        return object;
    }
    let interface = match kind {
        SvgListKind::Length => "SVGLength",
        SvgListKind::Number => "SVGNumber",
        SvgListKind::Point => "DOMPoint",
    };
    webidl::throw_type_error(
        scope,
        &format!("Argument 1 can not be converted to {interface}"),
    );
    None
}

pub(super) fn svg_value_list_is_read_only<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, object, SVG_VALUE_LIST_READ_ONLY_SLOT)
        .is_some_and(|value| value.is_true())
}

pub(super) fn svg_value_list_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    kind: SvgListKind,
) -> v8::Local<'s, v8::Array> {
    let slot = match kind {
        SvgListKind::Length => SVG_LENGTH_LIST_ITEMS_SLOT,
        SvgListKind::Number => SVG_NUMBER_LIST_ITEMS_SLOT,
        SvgListKind::Point => SVG_POINT_LIST_ITEMS_SLOT,
    };
    get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .unwrap_or_else(|| v8::Array::new(scope, 0))
}

pub(super) fn set_svg_value_list_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    items: v8::Local<'s, v8::Array>,
    kind: SvgListKind,
) {
    let slot = match kind {
        SvgListKind::Length => SVG_LENGTH_LIST_ITEMS_SLOT,
        SvgListKind::Number => SVG_NUMBER_LIST_ITEMS_SLOT,
        SvgListKind::Point => SVG_POINT_LIST_ITEMS_SLOT,
    };
    if let Some(current) = get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        detach_svg_value_list_items(scope, current);
    }
    if matches!(kind, SvgListKind::Point) && svg_value_list_is_read_only(scope, object) {
        for index in 0..items.length() {
            if let Some(point) = items
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            {
                set_svg_point_read_only(scope, point, true);
            }
        }
    }
    attach_svg_value_list_items(scope, object, items);
    set_private_value(scope, object, slot, items.into());
}

pub(super) fn attach_svg_value_list_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    items: v8::Local<'s, v8::Array>,
) {
    for index in 0..items.length() {
        if let Some(item) = items
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            set_svg_value_list_item_owner_list(scope, item, list);
        }
    }
}

pub(super) fn detach_svg_value_list_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    items: v8::Local<'s, v8::Array>,
) {
    for index in 0..items.length() {
        if let Some(item) = items
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            clear_svg_value_list_item_owner_list(scope, item);
        }
    }
}

pub(super) fn set_svg_value_list_item_owner_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
    list: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        item,
        SVG_VALUE_LIST_ITEM_OWNER_LIST_SLOT,
        list.into(),
    );
}

pub(super) fn clear_svg_value_list_item_owner_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        item,
        SVG_VALUE_LIST_ITEM_OWNER_LIST_SLOT,
        v8::undefined(scope).into(),
    );
}

pub(super) fn set_svg_value_list_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) {
    set_private_value(scope, list, SVG_VALUE_LIST_OWNER_ELEMENT_SLOT, owner.into());
    set_private_value(
        scope,
        list,
        SVG_VALUE_LIST_OWNER_ATTRIBUTE_SLOT,
        v8_string(scope, attribute)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

pub(super) fn svg_animated_value_list_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
    name: &str,
    kind: SvgListKind,
) -> Option<v8::Local<'s, v8::Object>> {
    let slot = match (kind, name) {
        (SvgListKind::Length, "baseVal") => SVG_ANIMATED_LENGTH_LIST_BASE_VAL_SLOT,
        (SvgListKind::Length, "animVal") => SVG_ANIMATED_LENGTH_LIST_ANIM_VAL_SLOT,
        (SvgListKind::Number, "baseVal") => SVG_ANIMATED_NUMBER_LIST_BASE_VAL_SLOT,
        (SvgListKind::Number, "animVal") => SVG_ANIMATED_NUMBER_LIST_ANIM_VAL_SLOT,
        _ => return None,
    };
    get_private_value(scope, animated, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn sync_svg_animated_value_list_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
    kind: SvgListKind,
) {
    let raw = svg_owner_attribute_value(scope, owner, attribute);
    let raw_value = raw.clone().unwrap_or_default();
    if let Some(base_val) = svg_animated_value_list_member(scope, animated, "baseVal", kind) {
        set_svg_value_list_owner_attribute(scope, base_val, owner, attribute);
        sync_svg_value_list_from_owner_attribute(scope, base_val, kind);
    }
    if let Some(anim_val) = svg_animated_value_list_member(scope, animated, "animVal", kind)
        && svg_value_list_synced_attribute_value(scope, anim_val).as_deref()
            != Some(raw_value.as_str())
    {
        let anim_items = build_svg_value_list_items_from_attribute(scope, raw.as_deref(), kind);
        set_svg_value_list_items(scope, anim_val, anim_items, kind);
        set_svg_value_list_synced_attribute_value(scope, anim_val, &raw_value);
    }
}

pub(super) fn sync_svg_value_list_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    kind: SvgListKind,
) {
    let Some(owner) = get_private_value(scope, list, SVG_VALUE_LIST_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(attribute) = get_private_value(scope, list, SVG_VALUE_LIST_OWNER_ATTRIBUTE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let raw = svg_owner_attribute_value(scope, owner, &attribute);
    let raw_value = raw.clone().unwrap_or_default();
    if svg_value_list_synced_attribute_value(scope, list).as_deref() == Some(raw_value.as_str()) {
        return;
    }
    let items = build_svg_value_list_items_from_attribute(scope, raw.as_deref(), kind);
    set_svg_value_list_items(scope, list, items, kind);
    set_svg_value_list_synced_attribute_value(scope, list, &raw_value);
}

pub(super) fn build_svg_value_list_items_from_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    raw: Option<&str>,
    kind: SvgListKind,
) -> v8::Local<'s, v8::Array> {
    let values = build_svg_value_list_item_values_from_attribute(scope, raw, kind);
    serialize_v8_iter_array(scope, values).unwrap_or_else(|| v8::Array::new(scope, 0))
}

pub(super) fn build_svg_value_list_item_values_from_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    raw: Option<&str>,
    kind: SvgListKind,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    match kind {
        SvgListKind::Length => svg_geometry::parse_length_list(raw)
            .unwrap_or_default()
            .into_iter()
            .map(|parsed| {
                build_svg_length_from_parsed(scope, svg_parsed_length_from_svg_length(parsed))
            })
            .collect(),
        SvgListKind::Number => svg_geometry::parse_number_list(raw)
            .unwrap_or_default()
            .into_iter()
            .map(|value| build_svg_number(scope, value))
            .collect(),
        SvgListKind::Point => svg_geometry::parse_point_list(raw)
            .unwrap_or_default()
            .into_iter()
            .map(|(x, y)| build_svg_point_object_with_values(scope, x, y))
            .collect(),
    }
}

pub(super) fn reflect_svg_value_list_to_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    kind: SvgListKind,
) {
    let Some(owner) = get_private_value(scope, list, SVG_VALUE_LIST_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(attribute) = get_private_value(scope, list, SVG_VALUE_LIST_OWNER_ATTRIBUTE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let value = serialize_svg_value_list_items(scope, list, kind);
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, owner)
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_attribute(scope, runtime_ptr, handle, &attribute, &value);
    set_svg_value_list_synced_attribute_value(scope, list, &value);
}

pub(super) fn reflect_svg_value_list_item_to_owner_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
    kind: SvgListKind,
) {
    let Some(list) = get_private_value(scope, item, SVG_VALUE_LIST_ITEM_OWNER_LIST_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    reflect_svg_value_list_to_owner_attribute(scope, list, kind);
}

pub(super) fn svg_value_list_synced_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<String> {
    get_private_value(scope, list, SVG_VALUE_LIST_SYNCED_ATTRIBUTE_VALUE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(super) fn set_svg_value_list_synced_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    value: &str,
) {
    set_private_value(
        scope,
        list,
        SVG_VALUE_LIST_SYNCED_ATTRIBUTE_VALUE_SLOT,
        v8_string(scope, value)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

pub(super) fn serialize_svg_value_list_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    kind: SvgListKind,
) -> String {
    let items = svg_value_list_items(scope, list, kind);
    let mut values = Vec::with_capacity(items.length() as usize);
    for index in 0..items.length() {
        if let Some(value) = items
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .and_then(|item| serialize_svg_value_list_item(scope, item, kind))
        {
            values.push(value);
        }
    }
    values.join(" ")
}

pub(super) fn serialize_svg_value_list_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
    kind: SvgListKind,
) -> Option<String> {
    match kind {
        SvgListKind::Length => get_private_value(scope, item, SVG_LENGTH_VALUE_AS_STRING_SLOT)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope)),
        SvgListKind::Number => {
            let value = svg_number_slot(scope, item, SVG_NUMBER_VALUE_SLOT)?;
            Some(svg_geometry::serialize_number(value))
        }
        SvgListKind::Point => {
            let point = dom_point_init_from_object(scope, item);
            Some(format!(
                "{} {}",
                svg_geometry::serialize_number(point.x),
                svg_geometry::serialize_number(point.y)
            ))
        }
    }
}

pub(super) fn svg_transform_list_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    get_private_value(scope, object, SVG_TRANSFORM_LIST_ITEMS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .unwrap_or_else(|| v8::Array::new(scope, 0))
}

pub(super) fn set_svg_transform_list_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    items: v8::Local<'s, v8::Array>,
) {
    if let Some(current) = get_private_value(scope, object, SVG_TRANSFORM_LIST_ITEMS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        detach_svg_transform_list_items(scope, current);
    }
    attach_svg_transform_list_items(scope, object, items);
    set_private_value(scope, object, SVG_TRANSFORM_LIST_ITEMS_SLOT, items.into());
}

pub(super) fn attach_svg_transform_list_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    items: v8::Local<'s, v8::Array>,
) {
    for index in 0..items.length() {
        if let Some(item) = items
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            set_svg_transform_item_owner_list(scope, item, list);
        }
    }
}

pub(super) fn detach_svg_transform_list_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    items: v8::Local<'s, v8::Array>,
) {
    for index in 0..items.length() {
        if let Some(item) = items
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            clear_svg_transform_item_owner_list(scope, item);
        }
    }
}

pub(super) fn set_svg_transform_item_owner_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
    list: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        item,
        SVG_TRANSFORM_LIST_ITEM_OWNER_LIST_SLOT,
        list.into(),
    );
}

pub(super) fn clear_svg_transform_item_owner_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        item,
        SVG_TRANSFORM_LIST_ITEM_OWNER_LIST_SLOT,
        v8::undefined(scope).into(),
    );
}

pub(super) fn set_svg_transform_list_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) {
    set_private_value(
        scope,
        list,
        SVG_TRANSFORM_LIST_OWNER_ELEMENT_SLOT,
        owner.into(),
    );
    set_private_value(
        scope,
        list,
        SVG_TRANSFORM_LIST_OWNER_ATTRIBUTE_SLOT,
        v8_string(scope, attribute)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

pub(super) fn svg_animated_transform_list_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let slot = match name {
        "baseVal" => SVG_ANIMATED_TRANSFORM_LIST_BASE_VAL_SLOT,
        "animVal" => SVG_ANIMATED_TRANSFORM_LIST_ANIM_VAL_SLOT,
        _ => return None,
    };
    get_private_value(scope, animated, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn sync_svg_animated_transform_list_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) {
    let raw = svg_owner_attribute_value(scope, owner, attribute);
    let raw_value = raw.clone().unwrap_or_default();
    if let Some(base_val) = svg_animated_transform_list_member(scope, animated, "baseVal")
        && svg_transform_list_synced_attribute_value(scope, base_val)
            .as_deref()
            .is_some_and(|synced| synced == raw_value)
    {
        set_svg_transform_list_owner_attribute(scope, base_val, owner, attribute);
        return;
    }
    let base_items = build_svg_transform_list_items_from_attribute(scope, raw.as_deref());
    let anim_items = build_svg_transform_list_items_from_attribute(scope, raw.as_deref());
    if let Some(base_val) = svg_animated_transform_list_member(scope, animated, "baseVal") {
        set_svg_transform_list_items(scope, base_val, base_items);
        set_svg_transform_list_owner_attribute(scope, base_val, owner, attribute);
        set_svg_transform_list_synced_attribute_value(scope, base_val, &raw_value);
    }
    if let Some(anim_val) = svg_animated_transform_list_member(scope, animated, "animVal") {
        set_svg_transform_list_items(scope, anim_val, anim_items);
        set_svg_transform_list_synced_attribute_value(scope, anim_val, &raw_value);
    }
}

pub(super) fn build_svg_transform_list_items_from_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    raw: Option<&str>,
) -> v8::Local<'s, v8::Array> {
    let transforms = raw
        .and_then(svg_geometry::parse_transform_attribute)
        .map(|transforms| {
            transforms
                .into_iter()
                .map(|transform| build_svg_transform_from_parsed(scope, transform))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serialize_v8_iter_array(scope, transforms).unwrap_or_else(|| v8::Array::new(scope, 0))
}

pub(super) fn build_svg_transform_from_parsed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transform: SvgTransform,
) -> v8::Local<'s, v8::Object> {
    build_svg_transform(scope, transform)
}

pub(super) fn svg_transform_type_for_kind(kind: SvgTransformKind) -> u32 {
    match kind {
        SvgTransformKind::Matrix => SVG_TRANSFORM_TYPE_MATRIX,
        SvgTransformKind::Translate => SVG_TRANSFORM_TYPE_TRANSLATE,
        SvgTransformKind::Scale => SVG_TRANSFORM_TYPE_SCALE,
        SvgTransformKind::Rotate => SVG_TRANSFORM_TYPE_ROTATE,
        SvgTransformKind::SkewX => SVG_TRANSFORM_TYPE_SKEWX,
        SvgTransformKind::SkewY => SVG_TRANSFORM_TYPE_SKEWY,
    }
}

pub(super) fn reflect_svg_transform_list_to_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = get_private_value(scope, list, SVG_TRANSFORM_LIST_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(attribute) = get_private_value(scope, list, SVG_TRANSFORM_LIST_OWNER_ATTRIBUTE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let value = serialize_svg_transform_list_items(scope, list);
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, owner)
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_attribute(scope, runtime_ptr, handle, &attribute, &value);
    set_svg_transform_list_synced_attribute_value(scope, list, &value);
}

pub(super) fn reflect_svg_transform_item_to_owner_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    item: v8::Local<'s, v8::Object>,
) {
    let Some(list) = get_private_value(scope, item, SVG_TRANSFORM_LIST_ITEM_OWNER_LIST_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    reflect_svg_transform_list_to_owner_attribute(scope, list);
}

pub(super) fn svg_transform_list_synced_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<String> {
    get_private_value(scope, list, SVG_TRANSFORM_LIST_SYNCED_ATTRIBUTE_VALUE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(super) fn set_svg_transform_list_synced_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    value: &str,
) {
    set_private_value(
        scope,
        list,
        SVG_TRANSFORM_LIST_SYNCED_ATTRIBUTE_VALUE_SLOT,
        v8_string(scope, value)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

pub(super) fn serialize_svg_transform_list_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> String {
    let items = svg_transform_list_items(scope, list);
    let components = svg_transform_list_components(scope, items);
    svg_geometry::serialize_transform_list(&components)
}

pub(super) fn svg_transform_list_components<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    items: v8::Local<'s, v8::Array>,
) -> Vec<SvgMatrixComponents> {
    let mut components = Vec::with_capacity(items.length() as usize);
    for index in 0..items.length() {
        if let Some(value) = items
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            components.push(svg_transform_matrix_components(scope, value));
        }
    }
    components
}

pub(super) fn set_svg_transform_state(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    transform: SvgTransform,
) {
    let matrix = build_svg_matrix(scope, transform.matrix);
    set_private_value(
        scope,
        object,
        SVG_TRANSFORM_TYPE_SLOT,
        v8::Integer::new_from_unsigned(scope, svg_transform_type_for_kind(transform.kind)).into(),
    );
    set_private_value(
        scope,
        object,
        SVG_TRANSFORM_ANGLE_SLOT,
        v8::Number::new(scope, transform.angle).into(),
    );
    set_private_value(scope, object, SVG_TRANSFORM_MATRIX_SLOT, matrix.into());
}

pub(super) fn svg_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<f64> {
    get_private_value(scope, object, slot).and_then(|value| value.number_value(scope))
}

pub(super) fn svg_matrix_components<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> SvgMatrixComponents {
    SvgMatrixComponents {
        a: svg_number_slot(scope, object, SVG_MATRIX_A_SLOT).unwrap_or(1.0),
        b: svg_number_slot(scope, object, SVG_MATRIX_B_SLOT).unwrap_or(0.0),
        c: svg_number_slot(scope, object, SVG_MATRIX_C_SLOT).unwrap_or(0.0),
        d: svg_number_slot(scope, object, SVG_MATRIX_D_SLOT).unwrap_or(1.0),
        e: svg_number_slot(scope, object, SVG_MATRIX_E_SLOT).unwrap_or(0.0),
        f: svg_number_slot(scope, object, SVG_MATRIX_F_SLOT).unwrap_or(0.0),
    }
}

pub(super) fn svg_transform_matrix_components<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transform: v8::Local<'s, v8::Object>,
) -> SvgMatrixComponents {
    get_private_value(scope, transform, SVG_TRANSFORM_MATRIX_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|matrix| svg_matrix_components(scope, matrix))
        .unwrap_or_else(SvgMatrixComponents::identity)
}

pub(super) fn svg_matrix_slot(name: &str) -> Option<&'static str> {
    match name {
        "a" => Some(SVG_MATRIX_A_SLOT),
        "b" => Some(SVG_MATRIX_B_SLOT),
        "c" => Some(SVG_MATRIX_C_SLOT),
        "d" => Some(SVG_MATRIX_D_SLOT),
        "e" => Some(SVG_MATRIX_E_SLOT),
        "f" => Some(SVG_MATRIX_F_SLOT),
        _ => None,
    }
}

pub(super) fn svg_matrix_default(slot: &str) -> f64 {
    match slot {
        SVG_MATRIX_A_SLOT | SVG_MATRIX_D_SLOT => 1.0,
        _ => 0.0,
    }
}

pub(super) fn svg_animated_length_attribute_slot(name: &str) -> &'static str {
    match name {
        "x" => "__moliSvgAnimatedX",
        "y" => "__moliSvgAnimatedY",
        "width" => "__moliSvgAnimatedWidth",
        "height" => "__moliSvgAnimatedHeight",
        "cx" => "__moliSvgAnimatedCx",
        "cy" => "__moliSvgAnimatedCy",
        "r" => "__moliSvgAnimatedR",
        "rx" => "__moliSvgAnimatedRx",
        "ry" => "__moliSvgAnimatedRy",
        "x1" => "__moliSvgAnimatedX1",
        "y1" => "__moliSvgAnimatedY1",
        "x2" => "__moliSvgAnimatedX2",
        "y2" => "__moliSvgAnimatedY2",
        "fx" => "__moliSvgAnimatedFx",
        "fy" => "__moliSvgAnimatedFy",
        "fr" => "__moliSvgAnimatedFr",
        "refX" => "__moliSvgAnimatedRefX",
        "refY" => "__moliSvgAnimatedRefY",
        "markerWidth" => "__moliSvgAnimatedMarkerWidth",
        "markerHeight" => "__moliSvgAnimatedMarkerHeight",
        "textLength" => SVG_TEXT_CONTENT_TEXT_LENGTH_SLOT,
        "startOffset" => "__moliSvgAnimatedStartOffset",
        _ => "__moliSvgAnimatedUnknown",
    }
}

#[derive(Clone, Debug)]
pub(super) struct SvgParsedAngle {
    value: f64,
    value_in_specified_units: f64,
    unit_type: u32,
    value_as_string: String,
}

impl Default for SvgParsedAngle {
    fn default() -> Self {
        Self {
            value: 0.0,
            value_in_specified_units: 0.0,
            unit_type: SVG_ANGLE_TYPE_UNSPECIFIED,
            value_as_string: "0".to_owned(),
        }
    }
}

pub(super) fn parse_svg_orient_angle_value(raw: &str) -> Option<SvgParsedAngle> {
    if matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "auto" | "auto-start-reverse"
    ) {
        return Some(SvgParsedAngle::default());
    }
    parse_svg_angle_value(raw)
}

pub(super) fn parse_svg_angle_value(raw: &str) -> Option<SvgParsedAngle> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let lowercase = raw.to_ascii_lowercase();
    let (number_raw, unit_type, degrees_per_unit) = if lowercase.ends_with("deg") {
        (&raw[..raw.len() - 3], SVG_ANGLE_TYPE_DEG, 1.0)
    } else if lowercase.ends_with("grad") {
        (&raw[..raw.len() - 4], SVG_ANGLE_TYPE_GRAD, 0.9)
    } else if lowercase.ends_with("rad") {
        (
            &raw[..raw.len() - 3],
            SVG_ANGLE_TYPE_RAD,
            180.0 / std::f64::consts::PI,
        )
    } else if lowercase.ends_with("turn") {
        (&raw[..raw.len() - 4], SVG_ANGLE_TYPE_UNKNOWN, 360.0)
    } else {
        (raw, SVG_ANGLE_TYPE_UNSPECIFIED, 1.0)
    };
    let value_in_specified_units = moli_css_parse::parse_number(number_raw.trim())?;
    Some(SvgParsedAngle {
        value: value_in_specified_units * degrees_per_unit,
        value_in_specified_units,
        unit_type,
        value_as_string: raw.to_owned(),
    })
}

pub(super) fn svg_angle_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<f64> {
    get_private_value(scope, object, slot)?.number_value(scope)
}

pub(super) fn svg_angle_string_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<String> {
    get_private_value(scope, object, slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(super) fn svg_angle_is_read_only<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, object, SVG_ANGLE_READ_ONLY_SLOT).is_some_and(|value| value.is_true())
}

pub(super) fn set_svg_angle_parsed_value(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    parsed: &SvgParsedAngle,
) {
    set_private_value(
        scope,
        object,
        SVG_ANGLE_UNIT_TYPE_SLOT,
        v8::Integer::new_from_unsigned(scope, parsed.unit_type).into(),
    );
    set_private_value(
        scope,
        object,
        SVG_ANGLE_VALUE_SLOT,
        v8::Number::new(scope, parsed.value).into(),
    );
    set_private_value(
        scope,
        object,
        SVG_ANGLE_VALUE_IN_SPECIFIED_UNITS_SLOT,
        v8::Number::new(scope, parsed.value_in_specified_units).into(),
    );
    set_private_value(
        scope,
        object,
        SVG_ANGLE_VALUE_AS_STRING_SLOT,
        v8_string(scope, &parsed.value_as_string)
            .unwrap_or_else(|| v8str(scope, "0"))
            .into(),
    );
}

fn svg_angle_current_unit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> (u32, bool) {
    let unit_type = svg_angle_number_slot(scope, object, SVG_ANGLE_UNIT_TYPE_SLOT)
        .unwrap_or(SVG_ANGLE_TYPE_UNSPECIFIED as f64) as u32;
    let is_turn = unit_type == SVG_ANGLE_TYPE_UNKNOWN
        && svg_angle_string_slot(scope, object, SVG_ANGLE_VALUE_AS_STRING_SLOT)
            .is_some_and(|value| value.trim().to_ascii_lowercase().ends_with("turn"));
    (unit_type, is_turn)
}

fn svg_angle_degrees_per_unit(unit_type: u32, is_turn: bool) -> f64 {
    match unit_type {
        SVG_ANGLE_TYPE_RAD => 180.0 / std::f64::consts::PI,
        SVG_ANGLE_TYPE_GRAD => 0.9,
        SVG_ANGLE_TYPE_UNKNOWN if is_turn => 360.0,
        _ => 1.0,
    }
}

fn svg_angle_unit_suffix(unit_type: u32, is_turn: bool) -> &'static str {
    match unit_type {
        SVG_ANGLE_TYPE_DEG => "deg",
        SVG_ANGLE_TYPE_RAD => "rad",
        SVG_ANGLE_TYPE_GRAD => "grad",
        SVG_ANGLE_TYPE_UNKNOWN if is_turn => "turn",
        _ => "",
    }
}

fn svg_angle_from_specified_value(
    value_in_specified_units: f64,
    unit_type: u32,
    is_turn: bool,
) -> SvgParsedAngle {
    SvgParsedAngle {
        value: value_in_specified_units * svg_angle_degrees_per_unit(unit_type, is_turn),
        value_in_specified_units,
        unit_type,
        value_as_string: format!(
            "{}{}",
            svg_geometry::serialize_number(value_in_specified_units),
            svg_angle_unit_suffix(unit_type, is_turn)
        ),
    }
}

pub(super) fn set_svg_angle_value_degrees<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: f64,
) {
    let (unit_type, is_turn) = svg_angle_current_unit(scope, object);
    let specified = value / svg_angle_degrees_per_unit(unit_type, is_turn);
    let parsed = svg_angle_from_specified_value(specified, unit_type, is_turn);
    set_svg_angle_parsed_value(scope, object, &parsed);
}

pub(super) fn set_svg_angle_value_in_specified_units<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: f64,
) {
    let (unit_type, is_turn) = svg_angle_current_unit(scope, object);
    let parsed = svg_angle_from_specified_value(value, unit_type, is_turn);
    set_svg_angle_parsed_value(scope, object, &parsed);
}

pub(super) fn set_svg_angle_new_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    unit_type: u32,
    value: f64,
) -> bool {
    if !(SVG_ANGLE_TYPE_UNSPECIFIED..=SVG_ANGLE_TYPE_GRAD).contains(&unit_type) {
        return false;
    }
    let parsed = svg_angle_from_specified_value(value, unit_type, false);
    set_svg_angle_parsed_value(scope, object, &parsed);
    true
}

pub(super) fn convert_svg_angle_to_unit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    unit_type: u32,
) -> bool {
    if !(SVG_ANGLE_TYPE_UNSPECIFIED..=SVG_ANGLE_TYPE_GRAD).contains(&unit_type) {
        return false;
    }
    let value = svg_angle_number_slot(scope, object, SVG_ANGLE_VALUE_SLOT).unwrap_or(0.0);
    let specified = value / svg_angle_degrees_per_unit(unit_type, false);
    let parsed = svg_angle_from_specified_value(specified, unit_type, false);
    set_svg_angle_parsed_value(scope, object, &parsed);
    true
}

pub(super) fn set_svg_angle_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    angle: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) {
    set_private_value(scope, angle, SVG_ANGLE_OWNER_ELEMENT_SLOT, owner.into());
    set_private_value(
        scope,
        angle,
        SVG_ANGLE_OWNER_ATTRIBUTE_SLOT,
        v8_string(scope, attribute)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

pub(super) fn sync_svg_angle_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    angle: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = get_private_value(scope, angle, SVG_ANGLE_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(attribute) = svg_angle_string_slot(scope, angle, SVG_ANGLE_OWNER_ATTRIBUTE_SLOT)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let parsed = svg_owner_attribute_value(scope, owner, &attribute)
        .as_deref()
        .and_then(parse_svg_orient_angle_value)
        .unwrap_or_default();
    set_svg_angle_parsed_value(scope, angle, &parsed);
}

pub(super) fn sync_svg_animated_angle_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) {
    let parsed = svg_owner_attribute_value(scope, owner, attribute)
        .as_deref()
        .and_then(parse_svg_orient_angle_value)
        .unwrap_or_default();
    if let Some(base_val) = get_private_value(scope, animated, SVG_ANIMATED_ANGLE_BASE_VAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        set_svg_angle_parsed_value(scope, base_val, &parsed);
        set_svg_angle_owner_attribute(scope, base_val, owner, attribute);
    }
    if let Some(anim_val) = get_private_value(scope, animated, SVG_ANIMATED_ANGLE_ANIM_VAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        set_svg_angle_parsed_value(scope, anim_val, &parsed);
        set_svg_angle_owner_attribute(scope, anim_val, owner, attribute);
    }
}

pub(super) fn reflect_svg_angle_to_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    angle: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = get_private_value(scope, angle, SVG_ANGLE_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(attribute) = svg_angle_string_slot(scope, angle, SVG_ANGLE_OWNER_ATTRIBUTE_SLOT)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let Some(value) = svg_angle_string_slot(scope, angle, SVG_ANGLE_VALUE_AS_STRING_SLOT) else {
        return;
    };
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, owner)
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_attribute(scope, runtime_ptr, handle, &attribute, &value);
}

#[derive(Clone, Copy)]
pub(super) struct SvgParsedLength {
    value: f64,
    unit_type: u32,
    unit: SvgLengthUnit,
    raw: Option<&'static str>,
}

impl Default for SvgParsedLength {
    fn default() -> Self {
        Self {
            value: 0.0,
            unit_type: SVG_LENGTH_TYPE_NUMBER,
            unit: SvgLengthUnit::Number,
            raw: Some("0"),
        }
    }
}

pub(super) fn svg_length_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<f64> {
    let value = get_private_value(scope, object, slot)?;
    value.number_value(scope)
}

pub(super) fn set_svg_length_numeric_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: f64,
    unit_type: u32,
) {
    let value_as_string = serialize_svg_length_value(value, unit_type);
    set_svg_length_parsed_value(
        scope,
        object,
        SvgParsedLength {
            value,
            unit_type,
            unit: svg_length_unit_from_type(unit_type),
            raw: None,
        },
    );
    set_svg_length_value_string(scope, object, &value_as_string);
}

pub(super) fn set_svg_length_parsed_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    parsed: SvgParsedLength,
) {
    set_private_value(
        scope,
        object,
        SVG_LENGTH_UNIT_TYPE_SLOT,
        v8::Number::new(scope, parsed.unit_type as f64).into(),
    );
    set_private_value(
        scope,
        object,
        SVG_LENGTH_UNIT_SUFFIX_SLOT,
        v8_string(scope, parsed.unit.suffix())
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
    set_private_value(
        scope,
        object,
        SVG_LENGTH_VALUE_IN_SPECIFIED_UNITS_SLOT,
        v8::Number::new(scope, parsed.value).into(),
    );
    let raw = parsed
        .raw
        .map(str::to_owned)
        .or_else(|| Some(SvgLength::new(parsed.value, parsed.unit).serialize()))
        .unwrap_or_else(|| "0".to_owned());
    let value = resolve_svg_length_user_value_for_unit(scope, object, parsed.value, parsed.unit)
        .or_else(|| resolve_svg_length_without_context(parsed.value, parsed.unit))
        .unwrap_or(parsed.value);
    set_svg_length_value(scope, object, value, &raw);
}

pub(super) fn set_svg_length_value_in_user_units<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: f64,
) {
    let unit = svg_length_unit_slot(scope, object);
    let specified_value = svg_length_unit_factor(scope, object, unit)
        .filter(|factor| *factor != 0.0)
        .map(|factor| value / factor)
        .unwrap_or(value);
    set_svg_length_specified_value(scope, object, specified_value, value, unit);
}

pub(super) fn set_svg_length_value_in_specified_units<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    specified_value: f64,
) {
    let unit = svg_length_unit_slot(scope, object);
    let value = resolve_svg_length_user_value_for_unit(scope, object, specified_value, unit)
        .or_else(|| resolve_svg_length_without_context(specified_value, unit))
        .unwrap_or(specified_value);
    set_svg_length_specified_value(scope, object, specified_value, value, unit);
}

fn set_svg_length_specified_value(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    specified_value: f64,
    value: f64,
    unit: SvgLengthUnit,
) {
    set_private_value(
        scope,
        object,
        SVG_LENGTH_VALUE_IN_SPECIFIED_UNITS_SLOT,
        v8::Number::new(scope, specified_value).into(),
    );
    let value_as_string = SvgLength::new(specified_value, unit).serialize();
    set_svg_length_value(scope, object, value, &value_as_string);
}

pub(super) fn set_svg_length_value(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    value: f64,
    value_as_string: &str,
) {
    set_private_value(
        scope,
        object,
        SVG_LENGTH_VALUE_SLOT,
        v8::Number::new(scope, value).into(),
    );
    set_svg_length_value_string(scope, object, value_as_string);
}

pub(super) fn set_svg_length_value_string(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    value_as_string: &str,
) {
    set_private_value(
        scope,
        object,
        SVG_LENGTH_VALUE_AS_STRING_SLOT,
        v8_string(scope, value_as_string)
            .unwrap_or_else(|| v8str(scope, "0"))
            .into(),
    );
}

pub(super) fn sync_svg_animated_length_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
    initial_value: &str,
) {
    let parsed = svg_animated_length_attribute_value(scope, owner, attribute, initial_value);
    if let Some(base_val) = get_private_value(scope, animated, SVG_ANIMATED_LENGTH_BASE_VAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        set_svg_length_parsed_value(scope, base_val, parsed);
        set_svg_length_owner_attribute(scope, base_val, owner, attribute);
    }
    if let Some(anim_val) = get_private_value(scope, animated, SVG_ANIMATED_LENGTH_ANIM_VAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        set_svg_length_parsed_value(scope, anim_val, parsed);
    }
}

fn svg_animated_length_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
    initial_value: &str,
) -> SvgParsedLength {
    svg_owner_attribute_value(scope, owner, attribute)
        .as_deref()
        .and_then(parse_svg_length_value)
        .or_else(|| parse_svg_length_value(initial_value))
        .unwrap_or_default()
}

pub(super) fn sync_svg_animated_rect_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
) {
    let values = svg_view_box_attribute_value(scope, owner);
    for slot in [
        SVG_ANIMATED_RECT_BASE_VAL_SLOT,
        SVG_ANIMATED_RECT_ANIM_VAL_SLOT,
    ] {
        if let Some(rect) = get_private_value(scope, animated, slot)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            super::super::dom_rect::set_svg_view_box_rect_values(scope, rect, values);
        }
    }
}

pub(super) fn sync_svg_view_box_rect_from_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rect: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = super::super::dom_rect::svg_view_box_rect_owner(scope, rect) else {
        return;
    };
    if let Some(animated) = get_private_value(scope, owner, SVG_FIT_VIEW_BOX_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        sync_svg_animated_rect_from_owner_attribute(scope, animated, owner);
    } else {
        let values = svg_view_box_attribute_value(scope, owner);
        super::super::dom_rect::set_svg_view_box_rect_values(scope, rect, values);
    }
}

pub(super) fn reflect_svg_view_box_rect_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rect: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = super::super::dom_rect::svg_view_box_rect_owner(scope, rect) else {
        return;
    };
    let values = super::super::dom_rect::svg_view_box_rect_values(scope, rect);
    let serialized = values.map(svg_geometry::serialize_number).join(" ");
    set_svg_owner_attribute_value(scope, owner, "viewBox", &serialized);
    if let Some(animated) = get_private_value(scope, owner, SVG_FIT_VIEW_BOX_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        sync_svg_animated_rect_from_owner_attribute(scope, animated, owner);
    }
}

fn svg_view_box_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> [f64; 4] {
    svg_owner_attribute_value(scope, owner, "viewBox")
        .as_deref()
        .and_then(parse_svg_view_box_value)
        .unwrap_or([0.0; 4])
}

fn parse_svg_view_box_value(raw: &str) -> Option<[f64; 4]> {
    let values = svg_geometry::parse_number_list(raw)?;
    let [x, y, width, height] = values.as_slice() else {
        return None;
    };
    (*width >= 0.0 && *height >= 0.0).then_some([*x, *y, *width, *height])
}

pub(super) fn sync_svg_animated_preserve_aspect_ratio_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
) {
    let value = svg_preserve_aspect_ratio_attribute_value(scope, owner);
    for slot in [
        SVG_ANIMATED_PRESERVE_ASPECT_RATIO_BASE_VAL_SLOT,
        SVG_ANIMATED_PRESERVE_ASPECT_RATIO_ANIM_VAL_SLOT,
    ] {
        if let Some(aspect_ratio) = get_private_value(scope, animated, slot)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            set_svg_preserve_aspect_ratio_value(scope, aspect_ratio, value);
        }
    }
}

pub(super) fn sync_svg_preserve_aspect_ratio_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    aspect_ratio: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = get_private_value(
        scope,
        aspect_ratio,
        SVG_PRESERVE_ASPECT_RATIO_OWNER_ELEMENT_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok()) else {
        return;
    };
    let value = svg_preserve_aspect_ratio_attribute_value(scope, owner);
    set_svg_preserve_aspect_ratio_value(scope, aspect_ratio, value);
}

pub(super) fn set_svg_preserve_aspect_ratio_value(
    scope: &mut v8::PinScope<'_, '_>,
    aspect_ratio: v8::Local<'_, v8::Object>,
    [align, meet_or_slice]: [u32; 2],
) {
    set_private_value(
        scope,
        aspect_ratio,
        SVG_PRESERVE_ASPECT_RATIO_ALIGN_SLOT,
        v8::Integer::new_from_unsigned(scope, align).into(),
    );
    set_private_value(
        scope,
        aspect_ratio,
        SVG_PRESERVE_ASPECT_RATIO_MEET_OR_SLICE_SLOT,
        v8::Integer::new_from_unsigned(scope, meet_or_slice).into(),
    );
}

pub(super) fn reflect_svg_preserve_aspect_ratio_to_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    aspect_ratio: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = get_private_value(
        scope,
        aspect_ratio,
        SVG_PRESERVE_ASPECT_RATIO_OWNER_ELEMENT_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok()) else {
        return;
    };
    let align = svg_number_slot(scope, aspect_ratio, SVG_PRESERVE_ASPECT_RATIO_ALIGN_SLOT)
        .unwrap_or(SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MID as f64) as u32;
    let meet_or_slice = svg_number_slot(
        scope,
        aspect_ratio,
        SVG_PRESERVE_ASPECT_RATIO_MEET_OR_SLICE_SLOT,
    )
    .unwrap_or(SVG_MEET_OR_SLICE_MEET as f64) as u32;
    let (Some(align), Some(meet_or_slice)) = (
        serialize_svg_preserve_aspect_ratio_align(align),
        serialize_svg_meet_or_slice(meet_or_slice),
    ) else {
        return;
    };
    let serialized = format!("{align} {meet_or_slice}");
    set_svg_owner_attribute_value(scope, owner, "preserveAspectRatio", &serialized);
    if let Some(animated) = get_private_value(scope, owner, SVG_FIT_PRESERVE_ASPECT_RATIO_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        sync_svg_animated_preserve_aspect_ratio_from_owner_attribute(scope, animated, owner);
    }
}

fn svg_preserve_aspect_ratio_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> [u32; 2] {
    svg_owner_attribute_value(scope, owner, "preserveAspectRatio")
        .as_deref()
        .and_then(parse_svg_preserve_aspect_ratio_value)
        .unwrap_or([
            SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MID,
            SVG_MEET_OR_SLICE_MEET,
        ])
}

fn parse_svg_preserve_aspect_ratio_value(raw: &str) -> Option<[u32; 2]> {
    let mut tokens = raw.split_ascii_whitespace();
    let mut align = tokens.next()?;
    if align == "defer" {
        align = tokens.next()?;
    }
    let align = parse_svg_preserve_aspect_ratio_align(align)?;
    let meet_or_slice = match tokens.next() {
        None | Some("meet") => SVG_MEET_OR_SLICE_MEET,
        Some("slice") => SVG_MEET_OR_SLICE_SLICE,
        Some(_) => return None,
    };
    tokens.next().is_none().then_some([align, meet_or_slice])
}

fn parse_svg_preserve_aspect_ratio_align(value: &str) -> Option<u32> {
    match value {
        "none" => Some(SVG_PRESERVE_ASPECT_RATIO_NONE),
        "xMinYMin" => Some(SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MIN),
        "xMidYMin" => Some(SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MIN),
        "xMaxYMin" => Some(SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MIN),
        "xMinYMid" => Some(SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MID),
        "xMidYMid" => Some(SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MID),
        "xMaxYMid" => Some(SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MID),
        "xMinYMax" => Some(SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MAX),
        "xMidYMax" => Some(SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MAX),
        "xMaxYMax" => Some(SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MAX),
        _ => None,
    }
}

fn serialize_svg_preserve_aspect_ratio_align(value: u32) -> Option<&'static str> {
    match value {
        SVG_PRESERVE_ASPECT_RATIO_NONE => Some("none"),
        SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MIN => Some("xMinYMin"),
        SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MIN => Some("xMidYMin"),
        SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MIN => Some("xMaxYMin"),
        SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MID => Some("xMinYMid"),
        SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MID => Some("xMidYMid"),
        SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MID => Some("xMaxYMid"),
        SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MAX => Some("xMinYMax"),
        SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MAX => Some("xMidYMax"),
        SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MAX => Some("xMaxYMax"),
        _ => None,
    }
}

fn serialize_svg_meet_or_slice(value: u32) -> Option<&'static str> {
    match value {
        SVG_MEET_OR_SLICE_MEET => Some("meet"),
        SVG_MEET_OR_SLICE_SLICE => Some("slice"),
        _ => None,
    }
}

pub(super) fn set_svg_animated_string_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) {
    set_private_value(
        scope,
        animated,
        SVG_ANIMATED_STRING_OWNER_ELEMENT_SLOT,
        owner.into(),
    );
    set_private_value(
        scope,
        animated,
        SVG_ANIMATED_STRING_OWNER_ATTRIBUTE_SLOT,
        v8_string(scope, attribute)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

pub(super) fn sync_svg_animated_string_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = get_private_value(scope, animated, SVG_ANIMATED_STRING_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(attribute) =
        get_private_value(scope, animated, SVG_ANIMATED_STRING_OWNER_ATTRIBUTE_SLOT)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .filter(|value| !value.is_empty())
    else {
        return;
    };
    let value = svg_owner_attribute_value(scope, owner, &attribute).unwrap_or_default();
    set_svg_animated_string_values(scope, animated, &value);
}

pub(super) fn set_svg_animated_string_values(
    scope: &mut v8::PinScope<'_, '_>,
    animated: v8::Local<'_, v8::Object>,
    value: &str,
) {
    let value = v8_string(scope, value).unwrap_or_else(|| v8str(scope, ""));
    set_private_value(
        scope,
        animated,
        SVG_ANIMATED_STRING_BASE_VAL_SLOT,
        value.into(),
    );
    set_private_value(
        scope,
        animated,
        SVG_ANIMATED_STRING_ANIM_VAL_SLOT,
        value.into(),
    );
}

pub(super) fn set_svg_animated_number_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) {
    set_private_value(
        scope,
        animated,
        SVG_ANIMATED_NUMBER_OWNER_ELEMENT_SLOT,
        owner.into(),
    );
    set_private_value(
        scope,
        animated,
        SVG_ANIMATED_NUMBER_OWNER_ATTRIBUTE_SLOT,
        v8_string(scope, attribute)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

pub(super) fn sync_svg_animated_number_from_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) {
    set_svg_animated_number_owner_attribute(scope, animated, owner, attribute);
    let value = svg_owner_attribute_value(scope, owner, attribute)
        .as_deref()
        .and_then(parse_svg_number_value)
        .unwrap_or(0.0);
    set_svg_animated_number_values(scope, animated, value);
}

pub(super) fn sync_svg_animated_number_from_stored_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
) {
    if svg_animated_number_property_for_object(scope, animated).is_some() {
        sync_svg_animated_number_from_property(scope, animated);
        return;
    }
    let Some(owner) = get_private_value(scope, animated, SVG_ANIMATED_NUMBER_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(attribute) =
        get_private_value(scope, animated, SVG_ANIMATED_NUMBER_OWNER_ATTRIBUTE_SLOT)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .filter(|value| !value.is_empty())
    else {
        return;
    };
    sync_svg_animated_number_from_owner_attribute(scope, animated, owner, &attribute);
}

pub(super) fn sync_svg_animated_number_from_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
) {
    let Some(property) = svg_animated_number_property_for_object(scope, animated) else {
        return;
    };
    let Some(owner) = get_private_value(scope, animated, SVG_ANIMATED_NUMBER_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let value = svg_owner_attribute_value(scope, owner, property.attribute)
        .as_deref()
        .and_then(|raw| parse_svg_animated_number(property, raw))
        .unwrap_or(property.initial_value);
    set_svg_animated_number_values(scope, animated, value);
}

pub(super) fn set_svg_animated_number_values(
    scope: &mut v8::PinScope<'_, '_>,
    animated: v8::Local<'_, v8::Object>,
    value: f64,
) {
    let value = v8::Number::new(scope, value);
    set_private_value(
        scope,
        animated,
        SVG_ANIMATED_NUMBER_BASE_VAL_SLOT,
        value.into(),
    );
    set_private_value(
        scope,
        animated,
        SVG_ANIMATED_NUMBER_ANIM_VAL_SLOT,
        value.into(),
    );
}

pub(super) fn reflect_svg_animated_number_to_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
) {
    if svg_animated_number_property_for_object(scope, animated).is_some() {
        reflect_svg_animated_number_property_to_owner_attribute(scope, animated);
        return;
    }
    let Some(owner) = get_private_value(scope, animated, SVG_ANIMATED_NUMBER_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(attribute) =
        get_private_value(scope, animated, SVG_ANIMATED_NUMBER_OWNER_ATTRIBUTE_SLOT)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .filter(|value| !value.is_empty())
    else {
        return;
    };
    let value = svg_number_slot(scope, animated, SVG_ANIMATED_NUMBER_BASE_VAL_SLOT).unwrap_or(0.0);
    let value = svg_geometry::serialize_number(value);
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, owner)
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_attribute(scope, runtime_ptr, handle, &attribute, &value);
}

pub(super) fn reflect_svg_animated_number_property_to_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animated: v8::Local<'s, v8::Object>,
) {
    let Some(property) = svg_animated_number_property_for_object(scope, animated) else {
        return;
    };
    let Some(owner) = get_private_value(scope, animated, SVG_ANIMATED_NUMBER_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let value = svg_number_slot(scope, animated, SVG_ANIMATED_NUMBER_BASE_VAL_SLOT)
        .unwrap_or(property.initial_value);
    let serialized = match property.component {
        SvgAnimatedNumberComponent::Scalar | SvgAnimatedNumberComponent::NumberOrPercentage => {
            svg_geometry::serialize_number(value)
        }
        SvgAnimatedNumberComponent::PairFirst | SvgAnimatedNumberComponent::PairSecondOrFirst => {
            let (mut first, mut second) =
                svg_owner_attribute_value(scope, owner, property.attribute)
                    .as_deref()
                    .and_then(parse_svg_number_pair)
                    .unwrap_or((property.initial_value, property.initial_value));
            match property.component {
                SvgAnimatedNumberComponent::PairFirst => first = value,
                SvgAnimatedNumberComponent::PairSecondOrFirst => second = value,
                SvgAnimatedNumberComponent::Scalar
                | SvgAnimatedNumberComponent::NumberOrPercentage => unreachable!(),
            }
            format!(
                "{} {}",
                svg_geometry::serialize_number(first),
                svg_geometry::serialize_number(second)
            )
        }
    };
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, owner)
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_attribute(scope, runtime_ptr, handle, property.attribute, &serialized);
}

pub(super) fn parse_svg_animated_number(
    property: SvgAnimatedNumberProperty,
    value: &str,
) -> Option<f64> {
    match property.component {
        SvgAnimatedNumberComponent::Scalar => parse_svg_number_value(value),
        SvgAnimatedNumberComponent::PairFirst => {
            parse_svg_number_pair(value).map(|(first, _)| first)
        }
        SvgAnimatedNumberComponent::PairSecondOrFirst => {
            parse_svg_number_pair(value).map(|(_, second)| second)
        }
        SvgAnimatedNumberComponent::NumberOrPercentage => parse_svg_number_or_percentage(value),
    }
}

fn parse_svg_number_pair(value: &str) -> Option<(f64, f64)> {
    let values = svg_geometry::parse_number_list(value)?;
    match values.as_slice() {
        [first] => Some((*first, *first)),
        [first, second] => Some((*first, *second)),
        _ => None,
    }
}

fn parse_svg_number_or_percentage(value: &str) -> Option<f64> {
    let value = value.trim();
    match value.strip_suffix('%') {
        Some(percentage) => parse_svg_number_value(percentage).map(|value| value / 100.0),
        None => parse_svg_number_value(value),
    }
}

pub(super) fn set_svg_length_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    length: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) {
    set_private_value(scope, length, SVG_LENGTH_OWNER_ELEMENT_SLOT, owner.into());
    set_private_value(
        scope,
        length,
        SVG_LENGTH_OWNER_ATTRIBUTE_SLOT,
        v8_string(scope, attribute)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

pub(super) fn reflect_svg_length_to_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    length: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = get_private_value(scope, length, SVG_LENGTH_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(attribute) = get_private_value(scope, length, SVG_LENGTH_OWNER_ATTRIBUTE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let Some(value) = get_private_value(scope, length, SVG_LENGTH_VALUE_AS_STRING_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return;
    };
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, owner)
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_attribute(scope, runtime_ptr, handle, &attribute, &value);
}

pub(super) fn svg_owner_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) -> Option<String> {
    let (runtime_ptr, handle) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, owner).ok()?;
    let runtime = unsafe { &*runtime_ptr };
    if attribute != "href" {
        return runtime.dom_host().get_attribute(handle, attribute);
    }
    let value = runtime.dom_host().get_attribute_ns(handle, None, attribute);
    if value.is_some() {
        return value;
    }
    runtime.dom_host().get_attribute_ns(
        handle,
        Some(crate::native_bridge::document::XLINK_NS),
        attribute,
    )
}

fn set_svg_owner_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
    value: &str,
) {
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, owner)
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_attribute(scope, runtime_ptr, handle, attribute, value);
}

pub(super) fn parse_svg_length_value(raw: &str) -> Option<SvgParsedLength> {
    svg_geometry::parse_length(raw).map(svg_parsed_length_from_svg_length)
}

pub(super) fn svg_parsed_length_from_svg_length(length: SvgLength) -> SvgParsedLength {
    SvgParsedLength {
        value: length.value,
        unit_type: svg_length_unit_type(length.unit),
        unit: length.unit,
        raw: None,
    }
}

pub(super) fn svg_length_unit_type(unit: SvgLengthUnit) -> u32 {
    match unit {
        SvgLengthUnit::Number => SVG_LENGTH_TYPE_NUMBER,
        SvgLengthUnit::Percentage => SVG_LENGTH_TYPE_PERCENTAGE,
        SvgLengthUnit::Ems => SVG_LENGTH_TYPE_EMS,
        SvgLengthUnit::Exs => SVG_LENGTH_TYPE_EXS,
        SvgLengthUnit::Ch
        | SvgLengthUnit::Rem
        | SvgLengthUnit::Lh
        | SvgLengthUnit::Rlh
        | SvgLengthUnit::Cap
        | SvgLengthUnit::Ic
        | SvgLengthUnit::Q
        | SvgLengthUnit::Vw
        | SvgLengthUnit::Vh
        | SvgLengthUnit::Vmin
        | SvgLengthUnit::Vmax => SVG_LENGTH_TYPE_UNKNOWN,
        SvgLengthUnit::Px => SVG_LENGTH_TYPE_PX,
        SvgLengthUnit::Cm => SVG_LENGTH_TYPE_CM,
        SvgLengthUnit::Mm => SVG_LENGTH_TYPE_MM,
        SvgLengthUnit::In => SVG_LENGTH_TYPE_IN,
        SvgLengthUnit::Pt => SVG_LENGTH_TYPE_PT,
        SvgLengthUnit::Pc => SVG_LENGTH_TYPE_PC,
    }
}

pub(super) fn parse_svg_number_value(raw: &str) -> Option<f64> {
    svg_geometry::parse_number(raw)
}

pub(super) fn serialize_svg_length_value(value: f64, unit_type: u32) -> String {
    SvgLength::new(value, svg_length_unit_from_type(unit_type)).serialize()
}

pub(super) fn svg_length_unit_from_type(unit_type: u32) -> SvgLengthUnit {
    match unit_type {
        SVG_LENGTH_TYPE_PERCENTAGE => SvgLengthUnit::Percentage,
        SVG_LENGTH_TYPE_EMS => SvgLengthUnit::Ems,
        SVG_LENGTH_TYPE_EXS => SvgLengthUnit::Exs,
        SVG_LENGTH_TYPE_PX => SvgLengthUnit::Px,
        SVG_LENGTH_TYPE_CM => SvgLengthUnit::Cm,
        SVG_LENGTH_TYPE_MM => SvgLengthUnit::Mm,
        SVG_LENGTH_TYPE_IN => SvgLengthUnit::In,
        SVG_LENGTH_TYPE_PT => SvgLengthUnit::Pt,
        SVG_LENGTH_TYPE_PC => SvgLengthUnit::Pc,
        _ => SvgLengthUnit::Number,
    }
}

fn svg_length_unit_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> SvgLengthUnit {
    get_private_value(scope, object, SVG_LENGTH_UNIT_SUFFIX_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .and_then(|suffix| svg_geometry::parse_length(&format!("1{suffix}")))
        .map(|length| length.unit)
        .unwrap_or(SvgLengthUnit::Number)
}

pub(super) fn svg_length_value_in_user_units<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> f64 {
    let specified_value =
        svg_length_number_slot(scope, object, SVG_LENGTH_VALUE_IN_SPECIFIED_UNITS_SLOT)
            .unwrap_or(0.0);
    let unit = svg_length_unit_slot(scope, object);
    if resolve_svg_absolute_length(1.0, unit).is_some() {
        return svg_length_number_slot(scope, object, SVG_LENGTH_VALUE_SLOT)
            .unwrap_or(specified_value);
    }
    if let Some(value) =
        resolve_svg_length_user_value_for_unit(scope, object, specified_value, unit)
    {
        set_private_value(
            scope,
            object,
            SVG_LENGTH_VALUE_SLOT,
            v8::Number::new(scope, value).into(),
        );
        return value;
    }
    resolve_svg_length_without_context(specified_value, unit)
        .or_else(|| svg_length_number_slot(scope, object, SVG_LENGTH_VALUE_SLOT))
        .unwrap_or(specified_value)
}

pub(super) fn convert_svg_length_to_unit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    unit_type: u32,
) -> bool {
    let unit = svg_length_unit_from_type(unit_type);
    let value = svg_length_value_in_user_units(scope, object);
    let Some(factor) = svg_length_unit_factor(scope, object, unit).filter(|factor| *factor != 0.0)
    else {
        return false;
    };
    set_private_value(
        scope,
        object,
        SVG_LENGTH_UNIT_TYPE_SLOT,
        v8::Number::new(scope, unit_type as f64).into(),
    );
    set_private_value(
        scope,
        object,
        SVG_LENGTH_UNIT_SUFFIX_SLOT,
        v8_string(scope, unit.suffix())
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
    set_svg_length_specified_value(scope, object, value / factor, value, unit);
    true
}

fn svg_length_unit_factor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    unit: SvgLengthUnit,
) -> Option<f64> {
    resolve_svg_length_user_value_for_unit(scope, object, 1.0, unit)
        .or_else(|| resolve_svg_length_without_context(1.0, unit))
}

fn resolve_svg_length_without_context(value: f64, unit: SvgLengthUnit) -> Option<f64> {
    if let Some(value) = resolve_svg_absolute_length(value, unit) {
        return Some(value);
    }
    matches!(unit, SvgLengthUnit::Percentage).then_some(value)
}

fn resolve_svg_length_user_value_for_unit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: f64,
    unit: SvgLengthUnit,
) -> Option<f64> {
    if let Some(value) = resolve_svg_absolute_length(value, unit) {
        return Some(value);
    }
    let owner = svg_length_owner_element(scope, object)?;
    let attribute = svg_length_owner_attribute(scope, object).unwrap_or_default();
    let (runtime_ptr, handle) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, owner).ok()?;
    let runtime = unsafe { &*runtime_ptr };
    let connected = runtime
        .dom_host()
        .node(handle)
        .is_some_and(|node| node.is_connected());
    let basis = if connected {
        svg_length_percentage_basis(runtime, handle, &attribute)
    } else {
        100.0
    };
    let context = svg_length_numeric_context(runtime, handle, unit, connected);
    match unit {
        SvgLengthUnit::Percentage => Some(value * basis / 100.0),
        SvgLengthUnit::Ems => context.font_size_px.map(|basis| value * basis),
        SvgLengthUnit::Exs | SvgLengthUnit::Ch => {
            context.font_size_px.map(|basis| value * basis * 0.5)
        }
        SvgLengthUnit::Rem => context.root_font_size_px.map(|basis| value * basis),
        SvgLengthUnit::Lh | SvgLengthUnit::Rlh => context.line_height_px.map(|basis| value * basis),
        SvgLengthUnit::Cap => context.font_size_px.map(|basis| value * basis * 0.7),
        SvgLengthUnit::Ic => context.font_size_px.map(|basis| value * basis),
        SvgLengthUnit::Vw => context.viewport_width_px.map(|basis| value * basis / 100.0),
        SvgLengthUnit::Vh => context
            .viewport_height_px
            .map(|basis| value * basis / 100.0),
        SvgLengthUnit::Vmin => {
            Some(value * context.viewport_width_px?.min(context.viewport_height_px?) / 100.0)
        }
        SvgLengthUnit::Vmax => {
            Some(value * context.viewport_width_px?.max(context.viewport_height_px?) / 100.0)
        }
        SvgLengthUnit::Number
        | SvgLengthUnit::Px
        | SvgLengthUnit::Cm
        | SvgLengthUnit::Mm
        | SvgLengthUnit::Q
        | SvgLengthUnit::In
        | SvgLengthUnit::Pt
        | SvgLengthUnit::Pc => unreachable!("absolute SVG length handled above"),
    }
}

fn resolve_svg_absolute_length(value: f64, unit: SvgLengthUnit) -> Option<f64> {
    let pixels = match unit {
        SvgLengthUnit::Number | SvgLengthUnit::Px => value,
        SvgLengthUnit::Cm => value * 96.0 / 2.54,
        SvgLengthUnit::Mm => value * 96.0 / 25.4,
        SvgLengthUnit::Q => value * 96.0 / 101.6,
        SvgLengthUnit::In => value * 96.0,
        SvgLengthUnit::Pt => value * 96.0 / 72.0,
        SvgLengthUnit::Pc => value * 16.0,
        _ => return None,
    };
    Some(pixels)
}

fn svg_length_owner_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, object, SVG_LENGTH_OWNER_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .or_else(|| {
            let list = get_private_value(scope, object, SVG_VALUE_LIST_ITEM_OWNER_LIST_SLOT)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
            get_private_value(scope, list, SVG_VALUE_LIST_OWNER_ELEMENT_SLOT)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        })
}

fn svg_length_owner_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    get_private_value(scope, object, SVG_LENGTH_OWNER_ATTRIBUTE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .or_else(|| {
            let list = get_private_value(scope, object, SVG_VALUE_LIST_ITEM_OWNER_LIST_SLOT)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
            get_private_value(scope, list, SVG_VALUE_LIST_OWNER_ATTRIBUTE_SLOT)
                .and_then(|value| value.to_string(scope))
                .map(|value| value.to_rust_string_lossy(scope))
        })
}

fn svg_length_numeric_context(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    unit: SvgLengthUnit,
    connected: bool,
) -> moli_css_parse::CssNumericContext {
    if !connected {
        return moli_css_parse::CssNumericContext::default();
    }
    let font_size_px = svg_computed_pixel_value(runtime, handle, "font-size");
    let line_height_px = svg_computed_pixel_value(runtime, handle, "line-height")
        .or_else(|| font_size_px.map(|font_size| font_size * 1.2));
    let document = runtime.dom_host().owner_document_handle(handle);
    let root = document.and_then(|document| {
        runtime
            .dom_host()
            .document_element_handle_for_document(document)
    });
    let root_font_size_px = root
        .and_then(|root| svg_computed_pixel_value(runtime, root, "font-size"))
        .or(font_size_px);
    let root_line_height_px = root
        .and_then(|root| svg_computed_pixel_value(runtime, root, "line-height"))
        .or_else(|| root_font_size_px.map(|font_size| font_size * 1.2));
    let viewport = runtime.style_viewport();
    moli_css_parse::CssNumericContext {
        font_size_px,
        root_font_size_px,
        line_height_px: if matches!(unit, SvgLengthUnit::Rlh) {
            root_line_height_px
        } else {
            line_height_px
        },
        viewport_width_px: viewport.width,
        viewport_height_px: viewport.height,
        ..moli_css_parse::CssNumericContext::default()
    }
}

fn svg_computed_pixel_value(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    property: &str,
) -> Option<f64> {
    let value = crate::native_bridge::element::computed_style_property_for_handle(
        runtime, handle, property,
    );
    moli_css_parse::parse_px_length(&value, moli_css_parse::UnitlessLength::Any)
}

fn svg_length_percentage_basis(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    attribute: &str,
) -> f64 {
    let (width, height) = svg_nearest_viewport_dimensions(runtime, handle).unwrap_or_else(|| {
        let viewport = runtime.style_viewport();
        (
            viewport.width.unwrap_or(100.0),
            viewport.height.unwrap_or(100.0),
        )
    });
    match attribute {
        "x" | "x1" | "x2" | "cx" | "rx" | "width" | "markerWidth" | "refX" => width,
        "y" | "y1" | "y2" | "cy" | "ry" | "height" | "markerHeight" | "refY" => height,
        _ => ((width * width + height * height) / 2.0).sqrt(),
    }
}

fn svg_nearest_viewport_dimensions(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> Option<(f64, f64)> {
    let dom = runtime.dom_host();
    let mut current = Some(handle);
    while let Some(candidate) = current {
        let node = dom.node(candidate)?;
        if node
            .as_element()
            .is_some_and(|element| element.is_svg_element("svg"))
        {
            if let Some(view_box) = dom.get_attribute(candidate, "viewBox")
                && let Some([_, _, width, height]) = parse_svg_view_box_value(&view_box)
                && width > 0.0
                && height > 0.0
            {
                return Some((width, height));
            }
            let width = dom
                .get_attribute(candidate, "width")
                .as_deref()
                .and_then(|value| {
                    moli_css_parse::parse_px_length(value, moli_css_parse::UnitlessLength::Any)
                });
            let height = dom
                .get_attribute(candidate, "height")
                .as_deref()
                .and_then(|value| {
                    moli_css_parse::parse_px_length(value, moli_css_parse::UnitlessLength::Any)
                });
            if let (Some(width), Some(height)) = (width, height) {
                return Some((width, height));
            }
        }
        current = node.parent_node();
    }
    None
}
