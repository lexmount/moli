use super::callbacks::*;
use super::*;
use crate::util::serialize_v8_iter_array;
use moli_webapi_declare::WebApiObject;

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
#[webapi(interface = "SVGLength", fallback_to_string_tag = "SVGLength")]
struct SvgLengthObjectDeclaration {
    #[webapi(slot = SVG_LENGTH_UNIT_TYPE_SLOT)]
    unit_type: u32,
    #[webapi(slot = SVG_LENGTH_VALUE_SLOT)]
    value: f64,
    #[webapi(slot = SVG_LENGTH_VALUE_AS_STRING_SLOT)]
    value_as_string: String,
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
        length = 1
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
    #[webapi(method, callback = svg_transform_set_matrix_callback, length = 1)]
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

pub(super) fn build_svg_animated_length_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let base_val = build_svg_length_list(scope);
    let anim_val = build_svg_length_list(scope);
    SvgAnimatedLengthListObjectDeclaration::new(base_val, anim_val)
        .bind(scope)
        .expect("SVGAnimatedLengthList declaration should bind")
}

pub(super) fn build_svg_animated_number_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let base_val = build_svg_number_list(scope);
    let anim_val = build_svg_number_list(scope);
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

pub(super) fn build_svg_length_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    SvgLengthListObjectDeclaration::new(Vec::new())
        .bind(scope)
        .expect("SVGLengthList declaration should bind")
}

pub(super) fn build_svg_number_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    SvgNumberListObjectDeclaration::new(Vec::new())
        .bind(scope)
        .expect("SVGNumberList declaration should bind")
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

pub(super) fn build_svg_animated_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: f64,
) -> v8::Local<'s, v8::Object> {
    let base_val = build_svg_length(scope, value);
    let anim_val = build_svg_length(scope, value);
    SvgAnimatedLengthObjectDeclaration::new(base_val, anim_val)
        .bind(scope)
        .expect("SVGAnimatedLength declaration should bind")
}

pub(super) fn build_svg_animated_length_for_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    attribute: &str,
) -> v8::Local<'s, v8::Object> {
    let parsed = svg_owner_attribute_value(scope, owner, attribute)
        .as_deref()
        .and_then(parse_svg_length_value)
        .unwrap_or_default();
    let base_val = build_svg_length_from_parsed(scope, parsed);
    set_svg_length_owner_attribute(scope, base_val, owner, attribute);
    let anim_val = build_svg_length_from_parsed(scope, parsed);
    SvgAnimatedLengthObjectDeclaration::new(base_val, anim_val)
        .bind(scope)
        .expect("SVGAnimatedLength declaration should bind")
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
        .or_else(|| Some(serialize_svg_length_value(parsed.value, parsed.unit_type)))
        .unwrap_or_else(|| "0".to_owned());
    SvgLengthObjectDeclaration::new(parsed.unit_type, parsed.value, value_as_string)
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
    let Some((intrinsic_width, intrinsic_height)) = runtime.image_resource_concrete_size(handle)
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

pub(super) fn cloned_svg_matrix_value_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let matrix = svg_matrix_value_or_throw(scope, value)?;
    let components = svg_matrix_components(scope, matrix);
    Some(build_svg_matrix(scope, components))
}

pub(super) fn svg_value_list_item_or_default<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    kind: SvgListKind,
) -> v8::Local<'s, v8::Object> {
    v8::Local::<v8::Object>::try_from(value).unwrap_or_else(|_| match kind {
        SvgListKind::Length => build_svg_length(scope, 0.0),
        SvgListKind::Number => build_svg_number(scope, 0.0),
    })
}

pub(super) fn svg_value_list_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    kind: SvgListKind,
) -> v8::Local<'s, v8::Array> {
    let slot = match kind {
        SvgListKind::Length => SVG_LENGTH_LIST_ITEMS_SLOT,
        SvgListKind::Number => SVG_NUMBER_LIST_ITEMS_SLOT,
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
    };
    if let Some(current) = get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        detach_svg_value_list_items(scope, current);
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
    if let Some(base_val) = svg_animated_value_list_member(scope, animated, "baseVal", kind)
        && svg_value_list_synced_attribute_value(scope, base_val)
            .as_deref()
            .is_some_and(|synced| synced == raw_value)
    {
        set_svg_value_list_owner_attribute(scope, base_val, owner, attribute);
        return;
    }
    let base_items = build_svg_value_list_items_from_attribute(scope, raw.as_deref(), kind);
    let anim_items = build_svg_value_list_items_from_attribute(scope, raw.as_deref(), kind);
    if let Some(base_val) = svg_animated_value_list_member(scope, animated, "baseVal", kind) {
        set_svg_value_list_items(scope, base_val, base_items, kind);
        set_svg_value_list_owner_attribute(scope, base_val, owner, attribute);
        set_svg_value_list_synced_attribute_value(scope, base_val, &raw_value);
    }
    if let Some(anim_val) = svg_animated_value_list_member(scope, animated, "animVal", kind) {
        set_svg_value_list_items(scope, anim_val, anim_items, kind);
        set_svg_value_list_synced_attribute_value(scope, anim_val, &raw_value);
    }
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
        _ => "__moliSvgAnimatedUnknown",
    }
}

#[derive(Clone, Copy)]
pub(super) struct SvgParsedLength {
    value: f64,
    unit_type: u32,
    raw: Option<&'static str>,
}

impl Default for SvgParsedLength {
    fn default() -> Self {
        Self {
            value: 0.0,
            unit_type: SVG_LENGTH_TYPE_NUMBER,
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

pub(super) fn set_svg_length_numeric_value(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
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
            raw: None,
        },
    );
    set_svg_length_value_string(scope, object, &value_as_string);
}

pub(super) fn set_svg_length_parsed_value(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    parsed: SvgParsedLength,
) {
    set_private_value(
        scope,
        object,
        SVG_LENGTH_UNIT_TYPE_SLOT,
        v8::Number::new(scope, parsed.unit_type as f64).into(),
    );
    let raw = parsed
        .raw
        .map(str::to_owned)
        .or_else(|| Some(serialize_svg_length_value(parsed.value, parsed.unit_type)))
        .unwrap_or_else(|| "0".to_owned());
    set_svg_length_value(scope, object, parsed.value, &raw);
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
) {
    let parsed = svg_owner_attribute_value(scope, owner, attribute)
        .as_deref()
        .and_then(parse_svg_length_value)
        .unwrap_or_default();
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
        crate::native_bridge::node_runtime_and_handle_from_object(scope, owner).ok()?;
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.dom_host().get_attribute(handle, attribute)
}

pub(super) fn parse_svg_length_value(raw: &str) -> Option<SvgParsedLength> {
    svg_geometry::parse_length(raw).map(svg_parsed_length_from_svg_length)
}

pub(super) fn svg_parsed_length_from_svg_length(length: SvgLength) -> SvgParsedLength {
    SvgParsedLength {
        value: length.value,
        unit_type: svg_length_unit_type(length.unit),
        raw: None,
    }
}

pub(super) fn svg_length_unit_type(unit: SvgLengthUnit) -> u32 {
    match unit {
        SvgLengthUnit::Number => SVG_LENGTH_TYPE_NUMBER,
        SvgLengthUnit::Percentage => SVG_LENGTH_TYPE_PERCENTAGE,
        SvgLengthUnit::Ems => SVG_LENGTH_TYPE_EMS,
        SvgLengthUnit::Exs => SVG_LENGTH_TYPE_EXS,
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
