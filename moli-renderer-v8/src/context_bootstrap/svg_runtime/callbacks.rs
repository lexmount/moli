use super::builders::*;
use super::*;
use crate::util::serialize_v8_array;
use crate::web_api_interfaces;

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

pub(super) fn svg_uri_href_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_animated_string_attribute_getter(scope, args, rv, SVG_URI_HREF_SLOT, "href");
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

pub(super) fn svg_graphics_transform_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_transform_attribute_getter(scope, args, rv, SVG_GRAPHICS_TRANSFORM_SLOT, "transform");
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

pub(super) fn svg_text_content_length_adjust_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    if let Some(value) = get_private_value(scope, holder, SVG_TEXT_CONTENT_LENGTH_ADJUST_SLOT) {
        rv.set(value);
        return;
    }
    let value = build_svg_animated_enumeration(scope, SVG_LENGTH_ADJUST_SPACING);
    set_private_value(
        scope,
        holder,
        SVG_TEXT_CONTENT_LENGTH_ADJUST_SLOT,
        value.into(),
    );
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
            let value = match webidl::convert::<webidl::UnrestrictedDouble>(
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
            let parsed = parse_svg_length_value(&string_value).unwrap_or_default();
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
    let value = match webidl::convert::<webidl::UnrestrictedDouble>(
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
    let value = svg_length_number_slot(scope, args.this(), slot)
        .unwrap_or(SVG_LENGTH_ADJUST_SPACING as f64);
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
    let value = v8::Integer::new_from_unsigned(scope, value.into());
    set_private_value(
        scope,
        args.this(),
        SVG_ANIMATED_ENUMERATION_BASE_VAL_SLOT,
        value.into(),
    );
    set_private_value(
        scope,
        args.this(),
        SVG_ANIMATED_ENUMERATION_ANIM_VAL_SLOT,
        value.into(),
    );
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

pub(super) fn svg_svg_element_create_matrix_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(super::super::geometry_runtime::build_dom_matrix_identity_object(scope).into());
}

pub(super) fn svg_svg_element_create_transform_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(
        build_svg_transform(scope, SvgTransform::matrix(SvgMatrixComponents::identity())).into(),
    );
}

pub(super) fn svg_svg_element_create_transform_from_matrix_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
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
    let Some(parsed) = webidl::parse_args::<SvgLengthNewValueSpecifiedUnitsArgs>(scope, &args)
    else {
        return;
    };
    set_svg_length_numeric_value(scope, args.this(), parsed.value, parsed.unit_type as u32);
    reflect_svg_length_to_owner_attribute(scope, args.this());
    rv.set_undefined();
}

pub(super) fn svg_length_convert_to_specified_units_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SvgLengthConvertToSpecifiedUnitsArgs>(scope, &args)
    else {
        return;
    };
    let value = svg_length_number_slot(scope, args.this(), SVG_LENGTH_VALUE_SLOT).unwrap_or(0.0);
    set_svg_length_numeric_value(scope, args.this(), value, parsed.unit_type as u32);
    reflect_svg_length_to_owner_attribute(scope, args.this());
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
        .map(SvgGeometrySegment::length)
        .sum::<f64>();
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
