use super::builders::*;
use super::*;
use crate::util::serialize_v8_array;
use crate::web_api_interfaces;
use moli_webapi_declare::DataPropertyDescriptorDeclaration;

const SVG_RECT_ANIMATED_LENGTH_ATTRIBUTES: &[&str] = &["x", "y", "width", "height", "rx", "ry"];
const SVG_CIRCLE_ANIMATED_LENGTH_ATTRIBUTES: &[&str] = &["cx", "cy", "r"];
const SVG_ELLIPSE_ANIMATED_LENGTH_ATTRIBUTES: &[&str] = &["cx", "cy", "rx", "ry"];
const SVG_LINE_ANIMATED_LENGTH_ATTRIBUTES: &[&str] = &["x1", "y1", "x2", "y2"];
const SVG_BOX_ANIMATED_LENGTH_ATTRIBUTES: &[&str] = &["x", "y", "width", "height"];
const SVG_LENGTH_ACCESSOR_NAMES: &[&str] = &[
    "unitType",
    "value",
    "valueInSpecifiedUnits",
    "valueAsString",
];
const SVG_ANIMATED_ACCESSOR_NAMES: &[&str] = &["baseVal", "animVal"];
const SVG_TRANSFORM_ACCESSOR_NAMES: &[&str] = &["type", "matrix", "angle"];
const SVG_MATRIX_ACCESSOR_NAMES: &[&str] = &["a", "b", "c", "d", "e", "f"];

pub(super) fn configure_svg_string_list_indexed_property_handler(
    template: v8::Local<'_, v8::ObjectTemplate>,
) {
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(svg_string_list_indexed_getter)
            .setter(svg_string_list_indexed_setter)
            .query(svg_string_list_indexed_query)
            .deleter(svg_string_list_indexed_deleter)
            .enumerator(svg_string_list_indexed_enumerator)
            .definer(svg_string_list_indexed_definer)
            .descriptor(svg_string_list_indexed_descriptor),
    );
}

fn svg_dom_matrix_2d_init_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    prefix: &'static str,
) -> Option<SvgMatrixComponents> {
    let [a, b, c, d, e, f] =
        super::super::geometry_runtime::dom_matrix_2d_init_arg(scope, args, 0, prefix)?;
    Some(SvgMatrixComponents { a, b, c, d, e, f })
}

const SVG_TEXT_POSITIONING_LIST_ATTRIBUTES: &[(&str, &str, SvgListKind)] = &[
    ("x", SVG_TEXT_POSITIONING_X_SLOT, SvgListKind::Length),
    ("y", SVG_TEXT_POSITIONING_Y_SLOT, SvgListKind::Length),
    ("dx", SVG_TEXT_POSITIONING_DX_SLOT, SvgListKind::Length),
    ("dy", SVG_TEXT_POSITIONING_DY_SLOT, SvgListKind::Length),
    (
        "rotate",
        SVG_TEXT_POSITIONING_ROTATE_SLOT,
        SvgListKind::Number,
    ),
];

fn require_svg_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    interface: &str,
    member: &str,
) -> bool {
    if web_api_interfaces::descriptor(interface)
        .is_some_and(|interface| interface.is_instance(scope, receiver))
    {
        return true;
    }
    webidl::throw_type_error(
        scope,
        &format!("{interface}.{member} called on incompatible receiver."),
    );
    false
}

pub(super) fn svg_element_class_name_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_animated_string_attribute_getter(scope, args, rv, SVG_ELEMENT_CLASS_NAME_SLOT, "class");
}

pub(super) fn svg_element_owner_svg_element_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        webidl::throw_type_error(
            scope,
            "SVGElement.ownerSVGElement called on incompatible receiver.",
        );
        return;
    };
    let owner_handle = {
        let runtime = unsafe { &*runtime_ptr };
        let Some(node) = runtime.dom_host().node(handle) else {
            rv.set_null();
            return;
        };
        if node.namespace() != Some(crate::native_bridge::document::SVG_NS) {
            webidl::throw_type_error(
                scope,
                "SVGElement.ownerSVGElement called on incompatible receiver.",
            );
            return;
        }

        let mut current = node.parent_node_id();
        let mut owner = None;
        while let Some(candidate) = current {
            let Some(ancestor) = runtime.dom_host().node(candidate) else {
                break;
            };
            if ancestor.namespace() != Some(crate::native_bridge::document::SVG_NS)
                || ancestor.local_name() == Some("foreignObject")
            {
                break;
            }
            if ancestor.local_name() == Some("svg") {
                owner = Some(candidate);
                break;
            }
            current = ancestor.parent_node_id();
        }
        owner
    };

    let Some(owner) = owner_handle.and_then(|owner| {
        crate::native_bridge::document::detached_native_object_for_handle(scope, runtime_ptr, owner)
    }) else {
        rv.set_null();
        return;
    };
    rv.set(owner.into());
}

pub(super) fn svg_uri_href_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_animated_string_attribute_getter(scope, args, rv, SVG_URI_HREF_SLOT, "href");
}

pub(super) fn svg_fe_convolve_matrix_preserve_alpha_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner = args.this();
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, owner)
    else {
        webidl::throw_type_error(
            scope,
            "SVGFEConvolveMatrixElement.preserveAlpha called on incompatible receiver.",
        );
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !runtime.dom_host().node(handle).is_some_and(|node| {
        node.namespace() == Some(crate::native_bridge::document::SVG_NS)
            && node.local_name() == Some("feConvolveMatrix")
    }) {
        webidl::throw_type_error(
            scope,
            "SVGFEConvolveMatrixElement.preserveAlpha called on incompatible receiver.",
        );
        return;
    }
    svg_animated_boolean_attribute_getter(
        scope,
        owner,
        rv,
        SVG_FE_CONVOLVE_MATRIX_PRESERVE_ALPHA_SLOT,
        "preserveAlpha",
        false,
    );
}

fn svg_animated_boolean_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    slot: &'static str,
    attribute: &'static str,
    initial_value: bool,
) {
    if let Some(value) = get_private_value(scope, owner, slot) {
        if let Ok(animated) = v8::Local::<v8::Object>::try_from(value) {
            sync_svg_animated_boolean_from_owner_attribute(scope, animated);
        }
        rv.set(value);
        return;
    }
    let value = build_svg_animated_boolean_for_attribute(scope, owner, attribute, initial_value);
    set_private_value(scope, owner, slot, value.into());
    rv.set(value.into());
}

fn svg_animated_string_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    slot: &str,
    attribute: &str,
) {
    let owner = args.this();
    if let Some(value) = get_private_value(scope, owner, slot) {
        if let Ok(animated) = v8::Local::<v8::Object>::try_from(value) {
            sync_svg_animated_string_from_owner_attribute(scope, animated);
        }
        rv.set(value);
        return;
    }
    let value = build_svg_animated_string_for_attribute(scope, owner, attribute);
    set_private_value(scope, owner, slot, value.into());
    rv.set(value.into());
}

pub(super) fn svg_rect_animated_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_element_animated_length_getter(
        scope,
        &args,
        rv,
        SVG_RECT_ANIMATED_LENGTH_ATTRIBUTES,
        "SVGRectElement animated length attributes",
    );
}

pub(super) fn svg_circle_animated_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_element_animated_length_getter(
        scope,
        &args,
        rv,
        SVG_CIRCLE_ANIMATED_LENGTH_ATTRIBUTES,
        "SVGCircleElement animated length attributes",
    );
}

pub(super) fn svg_ellipse_animated_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_element_animated_length_getter(
        scope,
        &args,
        rv,
        SVG_ELLIPSE_ANIMATED_LENGTH_ATTRIBUTES,
        "SVGEllipseElement animated length attributes",
    );
}

pub(super) fn svg_line_animated_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_element_animated_length_getter(
        scope,
        &args,
        rv,
        SVG_LINE_ANIMATED_LENGTH_ATTRIBUTES,
        "SVGLineElement animated length attributes",
    );
}

pub(super) fn svg_box_animated_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_element_animated_length_getter(
        scope,
        &args,
        rv,
        SVG_BOX_ANIMATED_LENGTH_ATTRIBUTES,
        "SVG graphics box animated length attributes",
    );
}

fn svg_element_animated_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    attributes: &'static [&'static str],
    label: &'static str,
) {
    let Some(name) = callback_data_item(scope, args, attributes, label) else {
        rv.set_undefined();
        return;
    };
    let slot = svg_animated_length_attribute_slot(name);
    let owner = args.this();
    if let Some(value) = get_private_value(scope, owner, slot) {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
            sync_svg_animated_length_from_owner_attribute(scope, object, owner, name);
        }
        rv.set(value);
        return;
    }
    let value = build_svg_animated_length_for_attribute(scope, owner, name);
    set_private_value(scope, owner, slot, value.into());
    rv.set(value.into());
}

fn svg_marker_runtime_and_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &str,
) -> Option<(
    *mut crate::native_bridge::JsContextHost,
    crate::document_runtime::DomHandle,
)> {
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        webidl::throw_type_error(
            scope,
            &format!("SVGMarkerElement.{member} called on incompatible receiver."),
        );
        return None;
    };
    let is_marker = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .is_some_and(|element| element.is_svg_element("marker"));
    if !is_marker {
        webidl::throw_type_error(
            scope,
            &format!("SVGMarkerElement.{member} called on incompatible receiver."),
        );
        return None;
    }
    Some((runtime_ptr, handle))
}

pub(super) fn svg_marker_orient_angle_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner = args.this();
    if svg_marker_runtime_and_handle(scope, owner, "orientAngle").is_none() {
        return;
    }
    if let Some(value) = get_private_value(scope, owner, SVG_MARKER_ORIENT_ANGLE_SLOT) {
        if let Ok(animated) = v8::Local::<v8::Object>::try_from(value) {
            sync_svg_animated_angle_from_owner_attribute(scope, animated, owner, "orient");
        }
        rv.set(value);
        return;
    }
    let value = build_svg_animated_angle_for_attribute(scope, owner, "orient");
    set_private_value(scope, owner, SVG_MARKER_ORIENT_ANGLE_SLOT, value.into());
    rv.set(value.into());
}

fn set_svg_marker_orient_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    marker: v8::Local<'s, v8::Object>,
    member: &str,
    value: &str,
) -> bool {
    let Some((runtime_ptr, handle)) = svg_marker_runtime_and_handle(scope, marker, member) else {
        return false;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_attribute(scope, runtime_ptr, handle, "orient", value);
    true
}

pub(super) fn svg_marker_set_orient_to_auto_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if set_svg_marker_orient_attribute(scope, args.this(), "setOrientToAuto", "auto") {
        rv.set_undefined();
    }
}

pub(super) fn svg_marker_set_orient_to_angle_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(angle) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        webidl::throw_type_error(
            scope,
            "SVGMarkerElement.setOrientToAngle requires an SVGAngle.",
        );
        return;
    };
    if !require_svg_receiver(
        scope,
        angle,
        SVG_ANGLE_UNIT_TYPE_SLOT,
        "SVGAngle",
        "setOrientToAngle argument",
    ) {
        return;
    }
    sync_svg_angle_from_owner_attribute(scope, angle);
    let value = svg_angle_string_slot(scope, angle, SVG_ANGLE_VALUE_AS_STRING_SLOT)
        .unwrap_or_else(|| "0".to_owned());
    if set_svg_marker_orient_attribute(scope, args.this(), "setOrientToAngle", &value) {
        rv.set_undefined();
    }
}

pub(super) fn svg_graphics_transform_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_transform_attribute_getter(scope, args, rv, SVG_GRAPHICS_TRANSFORM_SLOT, "transform");
}

pub(super) fn svg_graphics_test_string_list_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((attribute, slot)) = callback_data_item(
        scope,
        &args,
        SVG_TEST_STRING_LIST_ATTRIBUTES,
        "SVGTests string list attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    let receiver = args.this();
    if !require_svg_graphics_element_receiver(scope, receiver, attribute) {
        return;
    }
    if let Some(value) = get_private_value(scope, receiver, slot) {
        if let Ok(list) = v8::Local::<v8::Object>::try_from(value) {
            sync_svg_string_list_from_owner_attribute(scope, list);
        }
        rv.set(value);
        return;
    }
    let list = build_svg_string_list_for_attribute(scope, receiver, attribute);
    set_private_value(scope, receiver, slot, list.into());
    rv.set(list.into());
}

fn require_svg_graphics_element_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &str,
) -> bool {
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        webidl::throw_type_error(
            scope,
            &format!("SVGGraphicsElement.{member} called on incompatible receiver."),
        );
        return false;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(mut interface_name) = runtime
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .map(|element| element.wrapper_prototype_name())
    else {
        webidl::throw_type_error(
            scope,
            &format!("SVGGraphicsElement.{member} called on incompatible receiver."),
        );
        return false;
    };
    loop {
        if interface_name == "SVGGraphicsElement" {
            return true;
        }
        let Some(parent) =
            crate::context_bootstrap::bridge_descriptor::node_bridge_descriptor(interface_name)
                .and_then(|descriptor| descriptor.parent_constructor)
        else {
            break;
        };
        interface_name = parent;
    }
    webidl::throw_type_error(
        scope,
        &format!("SVGGraphicsElement.{member} called on incompatible receiver."),
    );
    false
}

pub(super) fn svg_pattern_transform_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_transform_attribute_getter(
        scope,
        args,
        rv,
        SVG_PATTERN_TRANSFORM_SLOT,
        "patternTransform",
    );
}

pub(super) fn svg_gradient_transform_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_transform_attribute_getter(
        scope,
        args,
        rv,
        SVG_GRADIENT_TRANSFORM_SLOT,
        "gradientTransform",
    );
}

pub(super) fn svg_transform_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    slot: &str,
    attribute: &str,
) {
    let owner = args.this();
    if let Some(value) = get_private_value(scope, owner, slot) {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
            sync_svg_animated_transform_list_from_owner_attribute(scope, object, owner, attribute);
        }
        rv.set(value);
        return;
    }
    let value = build_svg_animated_transform_list_for_attribute(scope, owner, attribute);
    set_private_value(scope, owner, slot, value.into());
    rv.set(value.into());
}

pub(super) fn svg_geometry_path_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner = args.this();
    if let Some(value) = get_private_value(scope, owner, SVG_GEOMETRY_PATH_LENGTH_SLOT) {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
            sync_svg_animated_number_from_owner_attribute(scope, object, owner, "pathLength");
        }
        rv.set(value);
        return;
    }
    let value = build_svg_animated_number_for_attribute(scope, owner, "pathLength");
    set_private_value(scope, owner, SVG_GEOMETRY_PATH_LENGTH_SLOT, value.into());
    rv.set(value.into());
}

pub(super) fn svg_text_content_text_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    if let Some(value) = get_private_value(scope, holder, SVG_TEXT_CONTENT_TEXT_LENGTH_SLOT) {
        rv.set(value);
        return;
    }
    let value = build_svg_animated_length(scope, 0.0);
    set_private_value(
        scope,
        holder,
        SVG_TEXT_CONTENT_TEXT_LENGTH_SLOT,
        value.into(),
    );
    rv.set(value.into());
}

pub(super) fn svg_element_animated_enumeration_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(property) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ENUMERATION_PROPERTIES,
        "SVG animated enumeration properties",
    ) else {
        rv.set_undefined();
        return;
    };
    let holder = args.this();
    if let Some(value) = get_private_value(scope, holder, property.cache_slot) {
        if let Ok(animated) = v8::Local::<v8::Object>::try_from(value) {
            sync_svg_animated_enumeration_from_owner_attribute(scope, animated);
        }
        rv.set(value);
        return;
    }
    let value = build_svg_animated_enumeration_for_property(scope, holder, property);
    set_private_value(scope, holder, property.cache_slot, value.into());
    rv.set(value.into());
}

pub(super) fn svg_element_animated_integer_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(property) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_INTEGER_PROPERTIES,
        "SVG animated integer properties",
    ) else {
        rv.set_undefined();
        return;
    };
    let holder = args.this();
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, holder)
    else {
        webidl::throw_type_error(
            scope,
            &format!(
                "{}.{} called on incompatible receiver.",
                property.interface, property.name
            ),
        );
        return;
    };
    let is_expected_element = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .is_some_and(|element| element.is_svg_element(property.local_name));
    if !is_expected_element {
        webidl::throw_type_error(
            scope,
            &format!(
                "{}.{} called on incompatible receiver.",
                property.interface, property.name
            ),
        );
        return;
    }
    if let Some(value) = get_private_value(scope, holder, property.cache_slot) {
        if let Ok(animated) = v8::Local::<v8::Object>::try_from(value) {
            sync_svg_animated_integer_from_owner_attribute(scope, animated);
        }
        rv.set(value);
        return;
    }
    let value = build_svg_animated_integer_for_property(scope, holder, property);
    set_private_value(scope, holder, property.cache_slot, value.into());
    rv.set(value.into());
}

pub(super) fn svg_text_positioning_list_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((name, slot, kind)) = callback_data_item(
        scope,
        &args,
        SVG_TEXT_POSITIONING_LIST_ATTRIBUTES,
        "SVGTextPositioningElement list attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    let receiver = args.this();
    if let Some(value) = get_private_value(scope, receiver, slot) {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
            sync_svg_animated_value_list_from_owner_attribute(scope, object, receiver, name, kind);
        }
        rv.set(value);
        return;
    }
    let value = build_svg_animated_value_list_for_attribute(scope, receiver, name, kind);
    set_private_value(scope, receiver, slot, value);
    rv.set(value);
}

pub(super) fn svg_animated_string_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ACCESSOR_NAMES,
        "SVGAnimatedString attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        SVG_ANIMATED_STRING_BASE_VAL_SLOT,
        "SVGAnimatedString",
        &format!("{name} getter"),
    ) {
        return;
    }
    let slot = match name {
        "baseVal" => SVG_ANIMATED_STRING_BASE_VAL_SLOT,
        "animVal" => SVG_ANIMATED_STRING_ANIM_VAL_SLOT,
        _ => {
            rv.set_undefined();
            return;
        }
    };
    let animated = args.this();
    sync_svg_animated_string_from_owner_attribute(scope, animated);
    rv.set(
        get_private_value(scope, animated, slot).unwrap_or_else(|| v8::String::empty(scope).into()),
    );
}

pub(super) fn svg_animated_boolean_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ACCESSOR_NAMES,
        "SVGAnimatedBoolean attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        SVG_ANIMATED_BOOLEAN_BASE_VAL_SLOT,
        "SVGAnimatedBoolean",
        &format!("{name} getter"),
    ) {
        return;
    }
    let slot = match name {
        "baseVal" => SVG_ANIMATED_BOOLEAN_BASE_VAL_SLOT,
        "animVal" => SVG_ANIMATED_BOOLEAN_ANIM_VAL_SLOT,
        _ => {
            rv.set_undefined();
            return;
        }
    };
    sync_svg_animated_boolean_from_owner_attribute(scope, args.this());
    let value = get_private_value(scope, args.this(), slot).is_some_and(|value| value.is_true());
    rv.set(v8::Boolean::new(scope, value).into());
}

pub(super) fn svg_animated_boolean_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ACCESSOR_NAMES,
        "SVGAnimatedBoolean attributes",
    ) else {
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        SVG_ANIMATED_BOOLEAN_BASE_VAL_SLOT,
        "SVGAnimatedBoolean",
        &format!("{name} setter"),
    ) || name != "baseVal"
    {
        return;
    }
    let value = match webidl::convert::<webidl::Boolean>(
        scope,
        args.get(0),
        webidl::Context::member("SVGAnimatedBoolean", "baseVal"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    set_svg_animated_boolean_values(scope, args.this(), value);
    reflect_svg_animated_boolean_to_owner_attribute(scope, args.this());
}

pub(super) fn svg_animated_string_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_svg_receiver(
        scope,
        args.this(),
        SVG_ANIMATED_STRING_BASE_VAL_SLOT,
        "SVGAnimatedString",
        "baseVal setter",
    ) {
        return;
    }
    let animated = args.this();
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
    let Some(value) = crate::native_bridge::element::set_svg_animated_string_base_value(
        scope,
        owner,
        &attribute,
        args.get(0),
    ) else {
        return;
    };
    set_svg_animated_string_values(scope, animated, &value);
}

pub(super) fn svg_animated_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ACCESSOR_NAMES,
        "SVGAnimatedLength attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        "SVGAnimatedLength",
        &format!("{name} getter"),
    ) {
        return;
    }
    let slot = match name {
        "baseVal" => SVG_ANIMATED_LENGTH_BASE_VAL_SLOT,
        "animVal" => SVG_ANIMATED_LENGTH_ANIM_VAL_SLOT,
        _ => {
            rv.set_undefined();
            return;
        }
    };
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

pub(super) fn svg_animated_angle_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ACCESSOR_NAMES,
        "SVGAnimatedAngle attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        SVG_ANIMATED_ANGLE_BASE_VAL_SLOT,
        "SVGAnimatedAngle",
        &format!("{name} getter"),
    ) {
        return;
    }
    let slot = match name {
        "baseVal" => SVG_ANIMATED_ANGLE_BASE_VAL_SLOT,
        "animVal" => SVG_ANIMATED_ANGLE_ANIM_VAL_SLOT,
        _ => {
            rv.set_undefined();
            return;
        }
    };
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

pub(super) fn svg_animated_length_list_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ACCESSOR_NAMES,
        "SVGAnimatedLengthList attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        "SVGAnimatedLengthList",
        &format!("{name} getter"),
    ) {
        return;
    }
    let slot = match name {
        "baseVal" => SVG_ANIMATED_LENGTH_LIST_BASE_VAL_SLOT,
        "animVal" => SVG_ANIMATED_LENGTH_LIST_ANIM_VAL_SLOT,
        _ => {
            rv.set_undefined();
            return;
        }
    };
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

pub(super) fn svg_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_LENGTH_ACCESSOR_NAMES,
        "SVGLength attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !require_svg_receiver(scope, args.this(), "SVGLength", &format!("{name} getter")) {
        return;
    }
    match name {
        "unitType" => {
            let value = svg_length_number_slot(scope, args.this(), SVG_LENGTH_UNIT_TYPE_SLOT)
                .unwrap_or(SVG_LENGTH_TYPE_NUMBER as f64);
            rv.set(v8::Integer::new_from_unsigned(scope, value as u32).into());
        }
        "value" | "valueInSpecifiedUnits" => {
            let value =
                svg_length_number_slot(scope, args.this(), SVG_LENGTH_VALUE_SLOT).unwrap_or(0.0);
            rv.set(v8::Number::new(scope, value).into());
        }
        "valueAsString" => {
            rv.set(
                get_private_value(scope, args.this(), SVG_LENGTH_VALUE_AS_STRING_SLOT)
                    .unwrap_or_else(|| v8str(scope, "0").into()),
            );
        }
        _ => rv.set_undefined(),
    }
}

pub(super) fn svg_angle_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_LENGTH_ACCESSOR_NAMES,
        "SVGAngle attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        SVG_ANGLE_UNIT_TYPE_SLOT,
        "SVGAngle",
        &format!("{name} getter"),
    ) {
        return;
    }
    sync_svg_angle_from_owner_attribute(scope, args.this());
    match name {
        "unitType" => {
            let value = svg_angle_number_slot(scope, args.this(), SVG_ANGLE_UNIT_TYPE_SLOT)
                .unwrap_or(SVG_ANGLE_TYPE_UNSPECIFIED as f64);
            rv.set(v8::Integer::new_from_unsigned(scope, value as u32).into());
        }
        "value" => {
            let value =
                svg_angle_number_slot(scope, args.this(), SVG_ANGLE_VALUE_SLOT).unwrap_or(0.0);
            rv.set(v8::Number::new(scope, value).into());
        }
        "valueInSpecifiedUnits" => {
            let value =
                svg_angle_number_slot(scope, args.this(), SVG_ANGLE_VALUE_IN_SPECIFIED_UNITS_SLOT)
                    .unwrap_or(0.0);
            rv.set(v8::Number::new(scope, value).into());
        }
        "valueAsString" => rv.set(
            get_private_value(scope, args.this(), SVG_ANGLE_VALUE_AS_STRING_SLOT)
                .unwrap_or_else(|| v8str(scope, "0").into()),
        ),
        _ => rv.set_undefined(),
    }
}

pub(super) fn svg_angle_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_LENGTH_ACCESSOR_NAMES,
        "SVGAngle attributes",
    ) else {
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        SVG_ANGLE_UNIT_TYPE_SLOT,
        "SVGAngle",
        &format!("{name} setter"),
    ) {
        return;
    }
    if svg_angle_is_read_only(scope, args.this()) {
        throw_dom_exception(
            scope,
            "NoModificationAllowedError",
            7,
            "The SVG angle is read-only.",
        );
        return;
    }
    sync_svg_angle_from_owner_attribute(scope, args.this());
    match name {
        "value" | "valueInSpecifiedUnits" => {
            let value = match webidl::convert::<webidl::Double>(
                scope,
                args.get(0),
                webidl::Context::member("SVGAngle", name),
            ) {
                Ok(value) => value.0,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return;
                }
            };
            if name == "value" {
                set_svg_angle_value_degrees(scope, args.this(), value);
            } else {
                set_svg_angle_value_in_specified_units(scope, args.this(), value);
            }
        }
        "valueAsString" => {
            let value = match webidl::convert::<webidl::DomString>(
                scope,
                args.get(0),
                webidl::Context::member("SVGAngle", "valueAsString"),
            ) {
                Ok(value) => value.0,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return;
                }
            };
            let Some(parsed) = parse_svg_angle_value(&value) else {
                throw_dom_exception(scope, "SyntaxError", 12, "Invalid SVG angle value.");
                return;
            };
            set_svg_angle_parsed_value(scope, args.this(), &parsed);
        }
        _ => return,
    }
    reflect_svg_angle_to_owner_attribute(scope, args.this());
}

pub(super) fn svg_length_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_LENGTH_ACCESSOR_NAMES,
        "SVGLength attributes",
    ) else {
        return;
    };
    if !require_svg_receiver(scope, args.this(), "SVGLength", &format!("{name} setter")) {
        return;
    }
    match name {
        "value" | "valueInSpecifiedUnits" => {
            let value = match webidl::convert::<webidl::Double>(
                scope,
                args.get(0),
                webidl::Context::member("SVGLength", "value"),
            ) {
                Ok(value) => value.0,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return;
                }
            };
            set_svg_length_numeric_value(scope, args.this(), value, SVG_LENGTH_TYPE_NUMBER);
            reflect_svg_length_to_owner_attribute(scope, args.this());
            reflect_svg_value_list_item_to_owner_list(scope, args.this(), SvgListKind::Length);
        }
        "valueAsString" => {
            let string_value = match webidl::convert::<webidl::DomString>(
                scope,
                args.get(0),
                webidl::Context::member("SVGLength", "valueAsString"),
            ) {
                Ok(value) => value.0,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return;
                }
            };
            let Some(parsed) = parse_svg_length_value(&string_value) else {
                throw_dom_exception(scope, "SyntaxError", 12, "Invalid SVG length value.");
                return;
            };
            set_svg_length_parsed_value(scope, args.this(), parsed);
            reflect_svg_length_to_owner_attribute(scope, args.this());
            reflect_svg_value_list_item_to_owner_list(scope, args.this(), SvgListKind::Length);
        }
        _ => {}
    }
}

pub(super) fn svg_number_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_svg_receiver(scope, args.this(), "SVGNumber", "value getter") {
        return;
    }
    let value = svg_number_slot(scope, args.this(), SVG_NUMBER_VALUE_SLOT).unwrap_or(0.0);
    rv.set(v8::Number::new(scope, value).into());
}

pub(super) fn svg_number_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_svg_receiver(scope, args.this(), "SVGNumber", "value setter") {
        return;
    }
    let value = match webidl::convert::<webidl::Double>(
        scope,
        args.get(0),
        webidl::Context::member("SVGNumber", "value"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    set_private_value(
        scope,
        args.this(),
        SVG_NUMBER_VALUE_SLOT,
        v8::Number::new(scope, value).into(),
    );
    reflect_svg_value_list_item_to_owner_list(scope, args.this(), SvgListKind::Number);
}

pub(super) fn svg_animated_number_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ACCESSOR_NAMES,
        "SVGAnimatedNumber attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        "SVGAnimatedNumber",
        &format!("{name} getter"),
    ) {
        return;
    }
    let slot = match name {
        "baseVal" => SVG_ANIMATED_NUMBER_BASE_VAL_SLOT,
        "animVal" => SVG_ANIMATED_NUMBER_ANIM_VAL_SLOT,
        _ => {
            rv.set_undefined();
            return;
        }
    };
    let value = svg_number_slot(scope, args.this(), slot).unwrap_or(0.0);
    rv.set(v8::Number::new(scope, value).into());
}

pub(super) fn svg_animated_integer_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ACCESSOR_NAMES,
        "SVGAnimatedInteger attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        SVG_ANIMATED_INTEGER_BASE_VAL_SLOT,
        "SVGAnimatedInteger",
        &format!("{name} getter"),
    ) {
        return;
    }
    let slot = match name {
        "baseVal" => SVG_ANIMATED_INTEGER_BASE_VAL_SLOT,
        "animVal" => SVG_ANIMATED_INTEGER_ANIM_VAL_SLOT,
        _ => {
            rv.set_undefined();
            return;
        }
    };
    sync_svg_animated_integer_from_owner_attribute(scope, args.this());
    let initial_value = svg_animated_integer_property(scope, args.this())
        .map(|property| property.initial_value)
        .unwrap_or_default();
    let value = get_private_value(scope, args.this(), slot)
        .and_then(|value| value.int32_value(scope))
        .unwrap_or(initial_value);
    rv.set(v8::Integer::new(scope, value).into());
}

pub(super) fn svg_animated_number_list_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ACCESSOR_NAMES,
        "SVGAnimatedNumberList attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        "SVGAnimatedNumberList",
        &format!("{name} getter"),
    ) {
        return;
    }
    let slot = match name {
        "baseVal" => SVG_ANIMATED_NUMBER_LIST_BASE_VAL_SLOT,
        "animVal" => SVG_ANIMATED_NUMBER_LIST_ANIM_VAL_SLOT,
        _ => {
            rv.set_undefined();
            return;
        }
    };
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

pub(super) fn svg_animated_enumeration_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ACCESSOR_NAMES,
        "SVGAnimatedEnumeration attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        "SVGAnimatedEnumeration",
        &format!("{name} getter"),
    ) {
        return;
    }
    let slot = match name {
        "baseVal" => SVG_ANIMATED_ENUMERATION_BASE_VAL_SLOT,
        "animVal" => SVG_ANIMATED_ENUMERATION_ANIM_VAL_SLOT,
        _ => {
            rv.set_undefined();
            return;
        }
    };
    sync_svg_animated_enumeration_from_owner_attribute(scope, args.this());
    let initial_value = svg_animated_enumeration_property(scope, args.this())
        .map(|property| property.initial_value)
        .unwrap_or(SVG_LENGTH_ADJUST_SPACING);
    let value = svg_length_number_slot(scope, args.this(), slot).unwrap_or(initial_value as f64);
    rv.set(v8::Integer::new_from_unsigned(scope, value as u32).into());
}

pub(super) fn svg_animated_enumeration_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ACCESSOR_NAMES,
        "SVGAnimatedEnumeration attributes",
    ) else {
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        "SVGAnimatedEnumeration",
        &format!("{name} setter"),
    ) || name != "baseVal"
    {
        return;
    }
    let value = match webidl::convert::<webidl::UnsignedShort>(
        scope,
        args.get(0),
        webidl::Context::member("SVGAnimatedEnumeration", "baseVal"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let value = u32::from(value);
    let Some(property) = svg_animated_enumeration_property(scope, args.this()) else {
        webidl::throw_type_error(
            scope,
            "SVGAnimatedEnumeration has no enumeration definition.",
        );
        return;
    };
    if serialize_svg_animated_enumeration(property.kind, value).is_none() {
        webidl::throw_type_error(scope, "Invalid SVGAnimatedEnumeration value.");
        return;
    }
    set_svg_animated_enumeration_values(scope, args.this(), value);
    reflect_svg_animated_enumeration_to_owner_attribute(scope, args.this());
}

pub(super) fn svg_animated_number_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ACCESSOR_NAMES,
        "SVGAnimatedNumber attributes",
    ) else {
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        "SVGAnimatedNumber",
        &format!("{name} setter"),
    ) || name != "baseVal"
    {
        return;
    }
    let value = match webidl::convert::<webidl::UnrestrictedDouble>(
        scope,
        args.get(0),
        webidl::Context::member("SVGAnimatedNumber", "baseVal"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    set_private_value(
        scope,
        args.this(),
        SVG_ANIMATED_NUMBER_BASE_VAL_SLOT,
        v8::Number::new(scope, value).into(),
    );
    set_private_value(
        scope,
        args.this(),
        SVG_ANIMATED_NUMBER_ANIM_VAL_SLOT,
        v8::Number::new(scope, value).into(),
    );
    reflect_svg_animated_number_to_owner_attribute(scope, args.this());
}

pub(super) fn svg_animated_integer_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ACCESSOR_NAMES,
        "SVGAnimatedInteger attributes",
    ) else {
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        SVG_ANIMATED_INTEGER_BASE_VAL_SLOT,
        "SVGAnimatedInteger",
        &format!("{name} setter"),
    ) || name != "baseVal"
    {
        return;
    }
    let value = match webidl::convert::<webidl::Long>(
        scope,
        args.get(0),
        webidl::Context::member("SVGAnimatedInteger", "baseVal"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    set_svg_animated_integer_values(scope, args.this(), value);
    reflect_svg_animated_integer_to_owner_attribute(scope, args.this());
}

pub(super) fn svg_animated_transform_list_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_ANIMATED_ACCESSOR_NAMES,
        "SVGAnimatedTransformList attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        "SVGAnimatedTransformList",
        &format!("{name} getter"),
    ) {
        return;
    }
    let slot = match name {
        "baseVal" => SVG_ANIMATED_TRANSFORM_LIST_BASE_VAL_SLOT,
        "animVal" => SVG_ANIMATED_TRANSFORM_LIST_ANIM_VAL_SLOT,
        _ => {
            rv.set_undefined();
            return;
        }
    };
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

pub(super) fn svg_string_list_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(items) = require_svg_string_list_items(scope, args.this(), "length getter") else {
        return;
    };
    rv.set_uint32(items.length());
}

pub(super) fn svg_string_list_clear_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if require_svg_string_list_items(scope, args.this(), "clear").is_none() {
        return;
    }
    set_svg_string_list_items(scope, args.this(), v8::Array::new(scope, 0));
    reflect_svg_string_list_to_owner_attribute(scope, args.this());
    rv.set_undefined();
}

pub(super) fn svg_string_list_initialize_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if require_svg_string_list_items(scope, args.this(), "initialize").is_none() {
        return;
    }
    let Some(parsed) = webidl::parse_args::<SvgListItemArgs>(scope, &args) else {
        return;
    };
    let Some(item) = svg_string_list_dom_string(scope, parsed.item, "SVGStringList.initialize", 1)
    else {
        return;
    };
    let items = v8::Array::new(scope, 1);
    let _ = items.set_index(scope, 0, item);
    set_svg_string_list_items(scope, args.this(), items);
    reflect_svg_string_list_to_owner_attribute(scope, args.this());
    rv.set(item);
}

pub(super) fn svg_string_list_get_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(items) = require_svg_string_list_items(scope, args.this(), "getItem") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<SvgListIndexArgs>(scope, &args) else {
        return;
    };
    let Some(item) = svg_list_item_or_throw(scope, items, parsed.index) else {
        return;
    };
    rv.set(item);
}

pub(super) fn svg_string_list_insert_item_before_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(items) = require_svg_string_list_items(scope, args.this(), "insertItemBefore") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<SvgListItemIndexArgs>(scope, &args) else {
        return;
    };
    let Some(item) =
        svg_string_list_dom_string(scope, parsed.item, "SVGStringList.insertItemBefore", 1)
    else {
        return;
    };
    let length = items.length();
    let index = parsed.index.min(length);
    let array_length = i32::try_from(length.saturating_add(1)).unwrap_or(i32::MAX);
    let next = v8::Array::new(scope, array_length);
    for old_index in 0..length {
        let new_index = if old_index < index {
            old_index
        } else {
            old_index + 1
        };
        if let Some(value) = items.get_index(scope, old_index) {
            let _ = next.set_index(scope, new_index, value);
        }
    }
    let _ = next.set_index(scope, index, item);
    set_svg_string_list_items(scope, args.this(), next);
    reflect_svg_string_list_to_owner_attribute(scope, args.this());
    rv.set(item);
}

pub(super) fn svg_string_list_replace_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(items) = require_svg_string_list_items(scope, args.this(), "replaceItem") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<SvgListItemIndexArgs>(scope, &args) else {
        return;
    };
    let Some(item) = svg_string_list_dom_string(scope, parsed.item, "SVGStringList.replaceItem", 1)
    else {
        return;
    };
    if parsed.index >= items.length() {
        webidl::throw_index_size_error(scope);
        return;
    }
    let _ = items.set_index(scope, parsed.index, item);
    reflect_svg_string_list_to_owner_attribute(scope, args.this());
    rv.set(item);
}

pub(super) fn svg_string_list_remove_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(items) = require_svg_string_list_items(scope, args.this(), "removeItem") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<SvgListIndexArgs>(scope, &args) else {
        return;
    };
    let length = items.length();
    let Some(removed) = svg_list_item_or_throw(scope, items, parsed.index) else {
        return;
    };
    let next_length = i32::try_from(length.saturating_sub(1)).unwrap_or(i32::MAX);
    let next = v8::Array::new(scope, next_length);
    for old_index in 0..length {
        if old_index == parsed.index {
            continue;
        }
        let new_index = if old_index < parsed.index {
            old_index
        } else {
            old_index - 1
        };
        if let Some(value) = items.get_index(scope, old_index) {
            let _ = next.set_index(scope, new_index, value);
        }
    }
    set_svg_string_list_items(scope, args.this(), next);
    reflect_svg_string_list_to_owner_attribute(scope, args.this());
    rv.set(removed);
}

pub(super) fn svg_string_list_append_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(items) = require_svg_string_list_items(scope, args.this(), "appendItem") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<SvgListItemArgs>(scope, &args) else {
        return;
    };
    let Some(item) = svg_string_list_dom_string(scope, parsed.item, "SVGStringList.appendItem", 1)
    else {
        return;
    };
    let _ = items.set_index(scope, items.length(), item);
    reflect_svg_string_list_to_owner_attribute(scope, args.this());
    rv.set(item);
}

fn require_svg_string_list_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    member: &str,
) -> Option<v8::Local<'s, v8::Array>> {
    if svg_string_list_items(scope, list).is_none() {
        webidl::throw_type_error(
            scope,
            &format!("SVGStringList.{member} called on incompatible receiver."),
        );
        return None;
    }
    sync_svg_string_list_from_owner_attribute(scope, list);
    svg_string_list_items(scope, list)
}

fn svg_string_list_dom_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    operation: &'static str,
    argument_index: usize,
) -> Option<v8::Local<'s, v8::Value>> {
    let value = match webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::argument(operation, argument_index),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    v8_string(scope, &value).map(Into::into)
}

fn svg_string_list_intercepted_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    svg_string_list_items(scope, list)?;
    sync_svg_string_list_from_owner_attribute(scope, list);
    svg_string_list_items(scope, list)
}

fn svg_string_list_indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = svg_string_list_intercepted_items(scope, args.holder())
        .filter(|items| index < items.length())
        .and_then(|items| items.get_index(scope, index))
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(value);
    v8::Intercepted::kYes
}

fn svg_string_list_indexed_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let list = args.holder();
    let Some(items) = svg_string_list_intercepted_items(scope, list) else {
        return v8::Intercepted::kNo;
    };
    let Some(value) = svg_string_list_dom_string(scope, value, "SVGStringList indexed setter", 2)
    else {
        return v8::Intercepted::kYes;
    };
    if index >= items.length() {
        webidl::throw_index_size_error(scope);
        return v8::Intercepted::kYes;
    }
    let _ = items.set_index(scope, index, value);
    reflect_svg_string_list_to_owner_attribute(scope, list);
    v8::Intercepted::kYes
}

fn svg_string_list_indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Some(items) = svg_string_list_intercepted_items(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    if index >= items.length() {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::NONE.as_u32() as i32);
    v8::Intercepted::kYes
}

fn svg_string_list_indexed_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Some(items) = svg_string_list_intercepted_items(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    if index >= items.length() {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn svg_string_list_indexed_definer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    descriptor: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    if descriptor.has_get() || descriptor.has_set() {
        rv.set_bool(false);
        return v8::Intercepted::kYes;
    }
    let value = if descriptor.has_value() {
        v8::Local::new(scope, descriptor.value())
    } else {
        v8::undefined(scope).into()
    };
    svg_string_list_indexed_setter(scope, index, value, args, rv)
}

fn svg_string_list_indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let length = svg_string_list_intercepted_items(scope, args.holder())
        .map(|items| items.length())
        .unwrap_or(0);
    let keys = (0..length)
        .map(|index| v8::Integer::new_from_unsigned(scope, index).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

fn svg_string_list_indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = svg_string_list_intercepted_items(scope, args.holder())
        .filter(|items| index < items.length())
        .and_then(|items| items.get_index(scope, index))
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) = DataPropertyDescriptorDeclaration::new(value, true, true).bind(scope)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

pub(super) fn svg_length_list_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_length_getter(scope, args, rv, SvgListKind::Length, "SVGLengthList");
}

pub(super) fn svg_number_list_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_length_getter(scope, args, rv, SvgListKind::Number, "SVGNumberList");
}

pub(super) fn svg_value_list_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: SvgListKind,
    interface: &'static str,
) {
    if !require_svg_receiver(scope, args.this(), interface, "length getter") {
        return;
    }
    let length = svg_value_list_items(scope, args.this(), kind).length();
    rv.set(v8::Integer::new_from_unsigned(scope, length).into());
}

pub(super) fn svg_length_list_clear_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_clear_callback(scope, args, rv, SvgListKind::Length);
}

pub(super) fn svg_number_list_clear_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_clear_callback(scope, args, rv, SvgListKind::Number);
}

pub(super) fn svg_value_list_clear_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: SvgListKind,
) {
    set_svg_value_list_items(scope, args.this(), v8::Array::new(scope, 0), kind);
    reflect_svg_value_list_to_owner_attribute(scope, args.this(), kind);
    rv.set_undefined();
}

pub(super) fn svg_length_list_initialize_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_initialize_callback(scope, args, rv, SvgListKind::Length);
}

pub(super) fn svg_number_list_initialize_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_initialize_callback(scope, args, rv, SvgListKind::Number);
}

pub(super) fn svg_value_list_initialize_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: SvgListKind,
) {
    let Some(parsed) = webidl::parse_args::<SvgListItemArgs>(scope, &args) else {
        return;
    };
    let item = svg_value_list_item_or_default(scope, parsed.item, kind);
    let Some(items) = serialize_v8_array(scope, [item]) else {
        return;
    };
    set_svg_value_list_items(scope, args.this(), items, kind);
    reflect_svg_value_list_to_owner_attribute(scope, args.this(), kind);
    rv.set(item.into());
}

pub(super) fn svg_length_list_get_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_get_item_callback(scope, args, rv, SvgListKind::Length);
}

pub(super) fn svg_number_list_get_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_get_item_callback(scope, args, rv, SvgListKind::Number);
}

pub(super) fn svg_value_list_get_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: SvgListKind,
) {
    let Some(parsed) = webidl::parse_args::<SvgListIndexArgs>(scope, &args) else {
        return;
    };
    let items = svg_value_list_items(scope, args.this(), kind);
    let Some(item) = svg_list_item_or_throw(scope, items, parsed.index) else {
        return;
    };
    rv.set(item);
}

pub(super) fn svg_length_list_insert_item_before_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_insert_item_before_callback(scope, args, rv, SvgListKind::Length);
}

pub(super) fn svg_number_list_insert_item_before_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_insert_item_before_callback(scope, args, rv, SvgListKind::Number);
}

pub(super) fn svg_value_list_insert_item_before_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: SvgListKind,
) {
    let Some(parsed) = webidl::parse_args::<SvgListItemIndexArgs>(scope, &args) else {
        return;
    };
    let item = svg_value_list_item_or_default(scope, parsed.item, kind);
    let items = svg_value_list_items(scope, args.this(), kind);
    let length = items.length();
    let index = parsed.index.min(length);
    let next = v8::Array::new(scope, (length + 1) as i32);
    for old_index in 0..length {
        let new_index = if old_index < index {
            old_index
        } else {
            old_index + 1
        };
        if let Some(value) = items.get_index(scope, old_index) {
            let _ = next.set_index(scope, new_index, value);
        }
    }
    let _ = next.set_index(scope, index, item.into());
    set_svg_value_list_items(scope, args.this(), next, kind);
    reflect_svg_value_list_to_owner_attribute(scope, args.this(), kind);
    rv.set(item.into());
}

pub(super) fn svg_length_list_replace_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_replace_item_callback(scope, args, rv, SvgListKind::Length);
}

pub(super) fn svg_number_list_replace_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_replace_item_callback(scope, args, rv, SvgListKind::Number);
}

pub(super) fn svg_value_list_replace_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: SvgListKind,
) {
    let Some(parsed) = webidl::parse_args::<SvgListItemIndexArgs>(scope, &args) else {
        return;
    };
    let items = svg_value_list_items(scope, args.this(), kind);
    if parsed.index >= items.length() {
        webidl::throw_index_size_error(scope);
        return;
    }
    let item = svg_value_list_item_or_default(scope, parsed.item, kind);
    if let Some(replaced) = items
        .get_index(scope, parsed.index)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        clear_svg_value_list_item_owner_list(scope, replaced);
    }
    if parsed.index < items.length() {
        let _ = items.set_index(scope, parsed.index, item.into());
        set_svg_value_list_item_owner_list(scope, item, args.this());
    }
    reflect_svg_value_list_to_owner_attribute(scope, args.this(), kind);
    rv.set(item.into());
}

pub(super) fn svg_length_list_remove_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_remove_item_callback(scope, args, rv, SvgListKind::Length);
}

pub(super) fn svg_number_list_remove_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_remove_item_callback(scope, args, rv, SvgListKind::Number);
}

pub(super) fn svg_value_list_remove_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: SvgListKind,
) {
    let Some(parsed) = webidl::parse_args::<SvgListIndexArgs>(scope, &args) else {
        return;
    };
    let index = parsed.index;
    let items = svg_value_list_items(scope, args.this(), kind);
    let length = items.length();
    if index >= length {
        webidl::throw_index_size_error(scope);
        return;
    }
    let Some(removed) = svg_list_item_or_throw(scope, items, index) else {
        return;
    };
    let next = v8::Array::new(scope, length.saturating_sub(1) as i32);
    for old_index in 0..length {
        if old_index == index {
            continue;
        }
        let new_index = if old_index < index {
            old_index
        } else {
            old_index - 1
        };
        if let Some(value) = items.get_index(scope, old_index) {
            let _ = next.set_index(scope, new_index, value);
        }
    }
    set_svg_value_list_items(scope, args.this(), next, kind);
    reflect_svg_value_list_to_owner_attribute(scope, args.this(), kind);
    rv.set(removed);
}

pub(super) fn svg_length_list_append_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_append_item_callback(scope, args, rv, SvgListKind::Length);
}

pub(super) fn svg_number_list_append_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_value_list_append_item_callback(scope, args, rv, SvgListKind::Number);
}

pub(super) fn svg_value_list_append_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: SvgListKind,
) {
    let Some(parsed) = webidl::parse_args::<SvgListItemArgs>(scope, &args) else {
        return;
    };
    let item = svg_value_list_item_or_default(scope, parsed.item, kind);
    let items = svg_value_list_items(scope, args.this(), kind);
    set_svg_value_list_item_owner_list(scope, item, args.this());
    let _ = items.set_index(scope, items.length(), item.into());
    reflect_svg_value_list_to_owner_attribute(scope, args.this(), kind);
    rv.set(item.into());
}

pub(super) fn svg_transform_list_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_svg_receiver(scope, args.this(), "SVGTransformList", "length getter") {
        return;
    }
    let length = svg_transform_list_items(scope, args.this()).length();
    rv.set(v8::Integer::new_from_unsigned(scope, length).into());
}

pub(super) fn svg_transform_list_clear_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_svg_transform_list_items(scope, args.this(), v8::Array::new(scope, 0));
    reflect_svg_transform_list_to_owner_attribute(scope, args.this());
    rv.set_undefined();
}

pub(super) fn svg_transform_list_initialize_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgListItemArgs>(scope, &args) else {
        return;
    };
    let Some(item) = svg_transform_value_or_throw(scope, parsed.item) else {
        return;
    };
    let Some(items) = serialize_v8_array(scope, [item]) else {
        return;
    };
    set_svg_transform_list_items(scope, args.this(), items);
    reflect_svg_transform_list_to_owner_attribute(scope, args.this());
    rv.set(item.into());
}

pub(super) fn svg_transform_list_get_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgListIndexArgs>(scope, &args) else {
        return;
    };
    let items = svg_transform_list_items(scope, args.this());
    let Some(item) = svg_list_item_or_throw(scope, items, parsed.index) else {
        return;
    };
    rv.set(item);
}

pub(super) fn svg_transform_list_insert_item_before_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgListItemIndexArgs>(scope, &args) else {
        return;
    };
    let Some(item) = svg_transform_value_or_throw(scope, parsed.item) else {
        return;
    };
    let items = svg_transform_list_items(scope, args.this());
    let length = items.length();
    let index = parsed.index.min(length);
    let next = v8::Array::new(scope, (length + 1) as i32);
    for old_index in 0..length {
        let new_index = if old_index < index {
            old_index
        } else {
            old_index + 1
        };
        if let Some(value) = items.get_index(scope, old_index) {
            let _ = next.set_index(scope, new_index, value);
        }
    }
    let _ = next.set_index(scope, index, item.into());
    set_svg_transform_list_items(scope, args.this(), next);
    reflect_svg_transform_list_to_owner_attribute(scope, args.this());
    rv.set(item.into());
}

pub(super) fn svg_transform_list_replace_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgListItemIndexArgs>(scope, &args) else {
        return;
    };
    let items = svg_transform_list_items(scope, args.this());
    if parsed.index >= items.length() {
        webidl::throw_index_size_error(scope);
        return;
    }
    let Some(item) = svg_transform_value_or_throw(scope, parsed.item) else {
        return;
    };
    if let Some(replaced) = items
        .get_index(scope, parsed.index)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        clear_svg_transform_item_owner_list(scope, replaced);
    }
    if parsed.index < items.length() {
        let _ = items.set_index(scope, parsed.index, item.into());
        set_svg_transform_item_owner_list(scope, item, args.this());
    }
    reflect_svg_transform_list_to_owner_attribute(scope, args.this());
    rv.set(item.into());
}

pub(super) fn svg_transform_list_remove_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgListIndexArgs>(scope, &args) else {
        return;
    };
    let index = parsed.index;
    let items = svg_transform_list_items(scope, args.this());
    let length = items.length();
    if index >= length {
        webidl::throw_index_size_error(scope);
        return;
    }
    let Some(removed) = svg_list_item_or_throw(scope, items, index) else {
        return;
    };
    let next = v8::Array::new(scope, length.saturating_sub(1) as i32);
    for old_index in 0..length {
        if old_index == index {
            continue;
        }
        let new_index = if old_index < index {
            old_index
        } else {
            old_index - 1
        };
        if let Some(value) = items.get_index(scope, old_index) {
            let _ = next.set_index(scope, new_index, value);
        }
    }
    set_svg_transform_list_items(scope, args.this(), next);
    reflect_svg_transform_list_to_owner_attribute(scope, args.this());
    rv.set(removed);
}

pub(super) fn svg_transform_list_append_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgListItemArgs>(scope, &args) else {
        return;
    };
    let Some(item) = svg_transform_value_or_throw(scope, parsed.item) else {
        return;
    };
    let items = svg_transform_list_items(scope, args.this());
    set_svg_transform_item_owner_list(scope, item, args.this());
    let _ = items.set_index(scope, items.length(), item.into());
    reflect_svg_transform_list_to_owner_attribute(scope, args.this());
    rv.set(item.into());
}

pub(super) fn svg_transform_list_create_transform_from_matrix_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(components) = svg_dom_matrix_2d_init_arg(
        scope,
        &args,
        "SVGTransformList.createSVGTransformFromMatrix",
    ) else {
        return;
    };
    rv.set(build_svg_transform(scope, SvgTransform::matrix(components)).into());
}

pub(super) fn svg_transform_list_consolidate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let items = svg_transform_list_items(scope, args.this());
    if items.length() == 0 {
        rv.set(v8::null(scope).into());
        return;
    }
    let product =
        svg_geometry::consolidate_transform_matrices(svg_transform_list_components(scope, items))
            .unwrap_or_else(SvgMatrixComponents::identity);
    let transform = build_svg_transform(scope, SvgTransform::matrix(product));
    let Some(consolidated_items) = serialize_v8_array(scope, [transform]) else {
        return;
    };
    set_svg_transform_list_items(scope, args.this(), consolidated_items);
    reflect_svg_transform_list_to_owner_attribute(scope, args.this());
    rv.set(transform.into());
}

pub(super) fn svg_transform_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_TRANSFORM_ACCESSOR_NAMES,
        "SVGTransform attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !require_svg_receiver(
        scope,
        args.this(),
        "SVGTransform",
        &format!("{name} getter"),
    ) {
        return;
    }
    match name {
        "type" => {
            let value = svg_number_slot(scope, args.this(), SVG_TRANSFORM_TYPE_SLOT)
                .unwrap_or(SVG_TRANSFORM_TYPE_MATRIX as f64);
            rv.set(v8::Integer::new_from_unsigned(scope, value as u32).into());
        }
        "angle" => {
            let value =
                svg_number_slot(scope, args.this(), SVG_TRANSFORM_ANGLE_SLOT).unwrap_or(0.0);
            rv.set(v8::Number::new(scope, value).into());
        }
        "matrix" => rv.set(
            get_private_value(scope, args.this(), SVG_TRANSFORM_MATRIX_SLOT)
                .unwrap_or_else(|| build_svg_matrix(scope, SvgMatrixComponents::identity()).into()),
        ),
        _ => rv.set_undefined(),
    }
}

pub(super) fn svg_transform_set_matrix_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(components) = svg_dom_matrix_2d_init_arg(scope, &args, "SVGTransform.setMatrix")
    else {
        return;
    };
    set_svg_transform_state(scope, args.this(), SvgTransform::matrix(components));
    reflect_svg_transform_item_to_owner_list(scope, args.this());
    rv.set_undefined();
}

fn require_svg_svg_element_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &str,
) -> bool {
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        webidl::throw_type_error(
            scope,
            &format!("SVGSVGElement.{member} called on incompatible receiver."),
        );
        return false;
    };
    let is_svg_element = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .is_some_and(|element| element.is_svg_element("svg"));
    if !is_svg_element {
        webidl::throw_type_error(
            scope,
            &format!("SVGSVGElement.{member} called on incompatible receiver."),
        );
    }
    is_svg_element
}

pub(super) fn svg_svg_element_create_number_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if require_svg_svg_element_receiver(scope, args.this(), "createSVGNumber") {
        rv.set(build_svg_number(scope, 0.0).into());
    }
}

pub(super) fn svg_svg_element_create_length_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if require_svg_svg_element_receiver(scope, args.this(), "createSVGLength") {
        rv.set(build_svg_length(scope, 0.0).into());
    }
}

pub(super) fn svg_svg_element_create_matrix_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if require_svg_svg_element_receiver(scope, args.this(), "createSVGMatrix") {
        rv.set(super::super::geometry_runtime::build_dom_matrix_identity_object(scope).into());
    }
}

pub(super) fn svg_svg_element_create_angle_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if require_svg_svg_element_receiver(scope, args.this(), "createSVGAngle") {
        rv.set(build_svg_angle(scope).into());
    }
}

pub(super) fn svg_svg_element_create_point_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if require_svg_svg_element_receiver(scope, args.this(), "createSVGPoint") {
        rv.set(super::super::geometry_runtime::build_svg_point_object(scope).into());
    }
}

pub(super) fn svg_svg_element_deselect_all_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        webidl::throw_type_error(
            scope,
            "SVGSVGElement.deselectAll called on incompatible receiver.",
        );
        return;
    };
    let (is_svg_root, owner_document) = {
        let runtime = unsafe { &*runtime_ptr };
        let is_svg_root = runtime
            .dom_host()
            .node(handle)
            .and_then(|node| node.as_element())
            .is_some_and(|element| element.is_svg_element("svg"));
        let owner_document = runtime.dom_host().owner_document_handle(handle);
        (is_svg_root, owner_document)
    };
    if !is_svg_root {
        webidl::throw_type_error(
            scope,
            "SVGSVGElement.deselectAll called on incompatible receiver.",
        );
        return;
    }
    let Some(owner_document) = owner_document else {
        return;
    };
    let Some(window) = crate::native_bridge::document::document_associated_window_for_handle(
        scope,
        runtime_ptr,
        owner_document,
    ) else {
        return;
    };
    let Some(selection) = selection_value_for_window(scope, window) else {
        return;
    };
    if selection_has_range(scope, selection) {
        selection_clear(scope, selection);
        selection_dispatch_change(scope);
    }
}

pub(super) fn svg_svg_element_create_transform_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_svg_svg_element_receiver(scope, args.this(), "createSVGTransform") {
        return;
    }
    rv.set(
        build_svg_transform(scope, SvgTransform::matrix(SvgMatrixComponents::identity())).into(),
    );
}

pub(super) fn svg_svg_element_create_transform_from_matrix_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_svg_svg_element_receiver(scope, args.this(), "createSVGTransformFromMatrix") {
        return;
    }
    let Some(components) =
        svg_dom_matrix_2d_init_arg(scope, &args, "SVGSVGElement.createSVGTransformFromMatrix")
    else {
        return;
    };
    rv.set(build_svg_transform(scope, SvgTransform::matrix(components)).into());
}

pub(super) fn svg_transform_set_translate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgTransformTranslateArgs>(scope, &args) else {
        return;
    };
    set_svg_transform_state(
        scope,
        args.this(),
        SvgTransform::translate(parsed.tx, parsed.ty),
    );
    reflect_svg_transform_item_to_owner_list(scope, args.this());
    rv.set_undefined();
}

pub(super) fn svg_transform_set_scale_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgTransformScaleArgs>(scope, &args) else {
        return;
    };
    set_svg_transform_state(
        scope,
        args.this(),
        SvgTransform::scale(parsed.sx, parsed.sy),
    );
    reflect_svg_transform_item_to_owner_list(scope, args.this());
    rv.set_undefined();
}

pub(super) fn svg_transform_set_rotate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgTransformRotateArgs>(scope, &args) else {
        return;
    };
    set_svg_transform_state(
        scope,
        args.this(),
        SvgTransform::rotate(parsed.angle, parsed.cx, parsed.cy),
    );
    reflect_svg_transform_item_to_owner_list(scope, args.this());
    rv.set_undefined();
}

pub(super) fn svg_transform_set_skew_x_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgAngleArg>(scope, &args) else {
        return;
    };
    set_svg_transform_state(scope, args.this(), SvgTransform::skew_x(parsed.angle));
    reflect_svg_transform_item_to_owner_list(scope, args.this());
    rv.set_undefined();
}

pub(super) fn svg_transform_set_skew_y_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgAngleArg>(scope, &args) else {
        return;
    };
    set_svg_transform_state(scope, args.this(), SvgTransform::skew_y(parsed.angle));
    reflect_svg_transform_item_to_owner_list(scope, args.this());
    rv.set_undefined();
}

pub(super) fn svg_matrix_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_MATRIX_ACCESSOR_NAMES,
        "SVGMatrix attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !require_svg_receiver(scope, args.this(), "SVGMatrix", &format!("{name} getter")) {
        return;
    }
    let slot = svg_matrix_slot(name).expect("SVGMatrix callback data must name a component");
    let value = svg_number_slot(scope, args.this(), slot).unwrap_or(svg_matrix_default(slot));
    rv.set(v8::Number::new(scope, value).into());
}

pub(super) fn svg_matrix_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        SVG_MATRIX_ACCESSOR_NAMES,
        "SVGMatrix attributes",
    ) else {
        return;
    };
    if !require_svg_receiver(scope, args.this(), "SVGMatrix", &format!("{name} setter")) {
        return;
    }
    let slot = svg_matrix_slot(name).expect("SVGMatrix callback data must name a component");
    let value = match webidl::convert::<webidl::Double>(
        scope,
        args.get(0),
        webidl::Context::member("SVGMatrix", "component"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    set_private_value(
        scope,
        args.this(),
        slot,
        v8::Number::new(scope, value).into(),
    );
}

pub(super) fn svg_matrix_multiply_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgMatrixArg>(scope, &args) else {
        return;
    };
    let current = svg_matrix_components(scope, args.this());
    let Some(other) = svg_matrix_value_or_throw(scope, parsed.matrix) else {
        return;
    };
    let other = svg_matrix_components(scope, other);
    rv.set(build_svg_matrix(scope, current.multiply(other)).into());
}

pub(super) fn svg_matrix_inverse_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let current = svg_matrix_components(scope, args.this());
    if !current.is_invertible() {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The matrix is not invertible.",
        );
        return;
    }
    let components = current.inverse();
    rv.set(build_svg_matrix(scope, components).into());
}

pub(super) fn svg_matrix_translate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgMatrixTranslateArgs>(scope, &args) else {
        return;
    };
    let matrix = svg_matrix_components(scope, args.this()).then_translate(parsed.x, parsed.y);
    rv.set(build_svg_matrix(scope, matrix).into());
}

pub(super) fn svg_matrix_scale_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgMatrixScaleArg>(scope, &args) else {
        return;
    };
    let matrix = svg_matrix_components(scope, args.this()).then_scale(parsed.scale_factor);
    rv.set(build_svg_matrix(scope, matrix).into());
}

pub(super) fn svg_matrix_scale_non_uniform_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgMatrixScaleNonUniformArgs>(scope, &args) else {
        return;
    };
    let matrix = svg_matrix_components(scope, args.this())
        .then_scale_non_uniform(parsed.scale_factor_x, parsed.scale_factor_y);
    rv.set(build_svg_matrix(scope, matrix).into());
}

pub(super) fn svg_matrix_rotate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgAngleArg>(scope, &args) else {
        return;
    };
    let matrix = svg_matrix_components(scope, args.this()).then_rotate(parsed.angle);
    rv.set(build_svg_matrix(scope, matrix).into());
}

pub(super) fn svg_matrix_rotate_from_vector_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgMatrixRotateFromVectorArgs>(scope, &args) else {
        return;
    };
    let Some(matrix) =
        svg_matrix_components(scope, args.this()).then_rotate_from_vector(parsed.x, parsed.y)
    else {
        throw_dom_exception(scope, "InvalidAccessError", 15, "Arguments cannot be zero.");
        return;
    };
    rv.set(build_svg_matrix(scope, matrix).into());
}

pub(super) fn svg_matrix_flip_x_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let matrix = svg_matrix_components(scope, args.this()).then_flip_x();
    rv.set(build_svg_matrix(scope, matrix).into());
}

pub(super) fn svg_matrix_flip_y_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let matrix = svg_matrix_components(scope, args.this()).then_flip_y();
    rv.set(build_svg_matrix(scope, matrix).into());
}

pub(super) fn svg_matrix_skew_x_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgAngleArg>(scope, &args) else {
        return;
    };
    let matrix = svg_matrix_components(scope, args.this()).then_skew_x(parsed.angle);
    rv.set(build_svg_matrix(scope, matrix).into());
}

pub(super) fn svg_matrix_skew_y_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgAngleArg>(scope, &args) else {
        return;
    };
    let matrix = svg_matrix_components(scope, args.this()).then_skew_y(parsed.angle);
    rv.set(build_svg_matrix(scope, matrix).into());
}

pub(super) fn svg_length_new_value_specified_units_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_svg_receiver(
        scope,
        args.this(),
        SVG_LENGTH_UNIT_TYPE_SLOT,
        "SVGLength",
        "newValueSpecifiedUnits",
    ) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<SvgLengthNewValueSpecifiedUnitsArgs>(scope, &args)
    else {
        return;
    };
    if !svg_length_unit_type_is_supported(parsed.unit_type as u32) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "The SVG length unit type is not supported.",
        );
        return;
    }
    set_svg_length_numeric_value(scope, args.this(), parsed.value, parsed.unit_type as u32);
    reflect_svg_length_to_owner_attribute(scope, args.this());
    reflect_svg_value_list_item_to_owner_list(scope, args.this(), SvgListKind::Length);
    rv.set_undefined();
}

pub(super) fn svg_angle_new_value_specified_units_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_svg_receiver(
        scope,
        args.this(),
        SVG_ANGLE_UNIT_TYPE_SLOT,
        "SVGAngle",
        "newValueSpecifiedUnits",
    ) {
        return;
    }
    if svg_angle_is_read_only(scope, args.this()) {
        throw_dom_exception(
            scope,
            "NoModificationAllowedError",
            7,
            "The SVG angle is read-only.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<SvgAngleNewValueSpecifiedUnitsArgs>(scope, &args)
    else {
        return;
    };
    if !set_svg_angle_new_value(scope, args.this(), parsed.unit_type as u32, parsed.value) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "The SVG angle unit type is not supported.",
        );
        return;
    }
    reflect_svg_angle_to_owner_attribute(scope, args.this());
    rv.set_undefined();
}

pub(super) fn svg_length_convert_to_specified_units_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_svg_receiver(
        scope,
        args.this(),
        SVG_LENGTH_UNIT_TYPE_SLOT,
        "SVGLength",
        "convertToSpecifiedUnits",
    ) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<SvgLengthConvertToSpecifiedUnitsArgs>(scope, &args)
    else {
        return;
    };
    if !svg_length_unit_type_is_supported(parsed.unit_type as u32) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "The SVG length unit type is not supported.",
        );
        return;
    }
    let value = svg_length_number_slot(scope, args.this(), SVG_LENGTH_VALUE_SLOT).unwrap_or(0.0);
    set_svg_length_numeric_value(scope, args.this(), value, parsed.unit_type as u32);
    reflect_svg_length_to_owner_attribute(scope, args.this());
    reflect_svg_value_list_item_to_owner_list(scope, args.this(), SvgListKind::Length);
    rv.set_undefined();
}

fn svg_length_unit_type_is_supported(unit_type: u32) -> bool {
    (SVG_LENGTH_TYPE_NUMBER..=SVG_LENGTH_TYPE_PC).contains(&unit_type)
}

pub(super) fn svg_angle_convert_to_specified_units_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_svg_receiver(
        scope,
        args.this(),
        SVG_ANGLE_UNIT_TYPE_SLOT,
        "SVGAngle",
        "convertToSpecifiedUnits",
    ) {
        return;
    }
    if svg_angle_is_read_only(scope, args.this()) {
        throw_dom_exception(
            scope,
            "NoModificationAllowedError",
            7,
            "The SVG angle is read-only.",
        );
        return;
    }
    sync_svg_angle_from_owner_attribute(scope, args.this());
    let Some(parsed) = webidl::parse_args::<SvgAngleConvertToSpecifiedUnitsArgs>(scope, &args)
    else {
        return;
    };
    if !convert_svg_angle_to_unit(scope, args.this(), parsed.unit_type as u32) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "The SVG angle unit type is not supported.",
        );
        return;
    }
    reflect_svg_angle_to_owner_attribute(scope, args.this());
    rv.set_undefined();
}

pub(super) fn svg_graphics_get_bbox_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let bbox = svg_graphics_bounding_box(scope, args.this()).unwrap_or(SvgGeometryBox {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    });
    rv.set(build_dom_rect_like(scope, bbox.x, bbox.y, bbox.width, bbox.height).into());
}

pub(super) fn svg_graphics_get_ctm_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::null(scope).into());
}

pub(super) fn svg_graphics_get_screen_ctm_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::null(scope).into());
}

pub(super) fn svg_geometry_is_point_in_fill_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(point) =
        optional_dom_point_init_arg(scope, &args, 0, "SVGGeometryElement.isPointInFill")
    else {
        return;
    };
    let contains = svg_fill_allows_paint(scope, args.this())
        && svg_geometry_element(scope, args.this()).is_some_and(|element| {
            svg_geometry::is_point_in_fill(&element, SvgGeometryPoint::new(point.x, point.y))
        });
    rv.set(v8::Boolean::new(scope, contains).into());
}

pub(super) fn svg_geometry_is_point_in_stroke_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(_point) =
        optional_dom_point_init_arg(scope, &args, 0, "SVGGeometryElement.isPointInStroke")
    else {
        return;
    };
    rv.set(v8::Boolean::new(scope, false).into());
}

pub(super) fn svg_geometry_get_total_length_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let length = svg_geometry_segments(scope, args.this())
        .iter()
        .fold(0.0, |total, segment| total + segment.length());
    rv.set(v8::Number::new(scope, length).into());
}

pub(super) fn svg_geometry_get_point_at_length_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgGeometryPointAtLengthArgs>(scope, &args) else {
        return;
    };
    let segments = svg_geometry_segments(scope, args.this());
    let point = svg_geometry::point_at_length(&segments, parsed.distance);
    rv.set(build_dom_point_like(scope, point.x, point.y).into());
}

pub(super) fn svg_text_content_get_number_of_chars_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Integer::new(scope, 0).into());
}

pub(super) fn svg_text_content_get_computed_text_length_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Number::new(scope, 0.0).into());
}

pub(super) fn svg_text_content_get_substring_length_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgTextSubstringArgs>(scope, &args) else {
        return;
    };
    let _ = (parsed.charnum, parsed.nchars);
    rv.set(v8::Number::new(scope, 0.0).into());
}

pub(super) fn svg_text_content_get_start_position_of_char_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgTextCharacterIndexArgs>(scope, &args) else {
        return;
    };
    let _ = parsed.charnum;
    rv.set(build_dom_point_like(scope, 0.0, 0.0).into());
}

pub(super) fn svg_text_content_get_end_position_of_char_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgTextCharacterIndexArgs>(scope, &args) else {
        return;
    };
    let _ = parsed.charnum;
    rv.set(build_dom_point_like(scope, 0.0, 0.0).into());
}

pub(super) fn svg_text_content_get_extent_of_char_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgTextCharacterIndexArgs>(scope, &args) else {
        return;
    };
    let _ = parsed.charnum;
    rv.set(build_zero_dom_rect_like(scope).into());
}

pub(super) fn svg_text_content_get_rotation_of_char_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgTextCharacterIndexArgs>(scope, &args) else {
        return;
    };
    let _ = parsed.charnum;
    rv.set(v8::Number::new(scope, 0.0).into());
}

pub(super) fn svg_text_content_get_char_num_at_position_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(_point) = optional_dom_point_init_arg(
        scope,
        &args,
        0,
        "SVGTextContentElement.getCharNumAtPosition",
    ) else {
        return;
    };
    rv.set(v8::Integer::new(scope, -1).into());
}

pub(super) fn svg_text_content_select_substring_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgTextSubstringArgs>(scope, &args) else {
        return;
    };
    let _ = (parsed.charnum, parsed.nchars);
    rv.set_undefined();
}
