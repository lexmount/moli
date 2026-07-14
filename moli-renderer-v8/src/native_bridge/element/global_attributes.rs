use crate::document_runtime::DomHandle;
use crate::dom::native::Node;
use crate::util::v8_string;
use crate::webidl;

use super::super::{
    JsContextHost,
    node::{
        node_is_element, node_runtime_and_handle_from_object_or_detached,
        throw_incompatible_getter_receiver, throw_incompatible_setter_receiver,
    },
    throw_dom_exception,
};

use super::reflection::{
    DomStringReflection, ElementReflectionInterface, NullToEmptyDomStringReflection,
    UnsignedLongReflection, UsvStringReflection, remove_reflected_attribute,
};
use super::{
    attribute_property_getter_from_object_or_detached,
    boolean_attribute_property_getter_from_object_or_detached, element_attribute,
    form_associated_form_owner, html_element_getter_receiver, html_element_setter_receiver,
    property_dom_string_value, resolve_url_like_attribute,
    set_attribute_property_on_object_or_detached,
    set_boolean_attribute_property_on_object_or_detached,
    set_dom_string_attribute_property_on_object, set_reflected_attribute,
    set_reflected_boolean_attribute, set_usv_string_attribute_property_on_object,
};

pub(in crate::native_bridge) fn html_name_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "name", rv);
}

fn set_html_name_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
) {
    set_dom_string_attribute_property_on_object(scope, receiver, "name", value, owner, "name");
}

pub(in crate::native_bridge) fn html_name_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(interface) = ElementReflectionInterface::from_callback_data(scope, args.data()) {
        set_html_name_for_receiver(scope, args.this(), args.get(0), interface.name());
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn html_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "value", rv);
}

pub(in crate::native_bridge) fn html_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "type", rv);
}

pub(in crate::native_bridge) fn html_value_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "valuetype", rv);
}

pub(in crate::native_bridge) fn html_label_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "label", rv);
}

pub(in crate::native_bridge) fn html_alt_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "alt", rv);
}

pub(in crate::native_bridge) fn html_use_map_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "usemap", rv);
}

pub(in crate::native_bridge) fn html_scrolling_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "scrolling", rv);
}

pub(in crate::native_bridge) fn html_frame_border_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "frameborder", rv);
}

pub(in crate::native_bridge) fn html_long_desc_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_url_attribute_getter(scope, args, "longdesc", rv);
}

pub(in crate::native_bridge) fn html_lowsrc_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_url_attribute_getter(scope, args, "lowsrc", rv);
}

pub(in crate::native_bridge) fn html_margin_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "marginheight", rv);
}

pub(in crate::native_bridge) fn html_margin_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "marginwidth", rv);
}

pub(in crate::native_bridge) fn html_version_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "version", rv);
}

pub(in crate::native_bridge) fn html_date_time_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "datetime", rv);
}

pub(in crate::native_bridge) fn html_cite_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_url_attribute_getter(scope, args, "cite", rv);
}

pub(in crate::native_bridge) fn dom_string_reflection_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(descriptor) = DomStringReflection::descriptor_from_callback_data(scope, args.data())
    else {
        rv.set_undefined();
        return;
    };
    let Some(local_name) = descriptor.html_local_name else {
        attribute_property_getter_from_object_or_detached(
            scope,
            args.this(),
            descriptor.attribute,
            rv,
        );
        return;
    };
    let Some((runtime_ptr, handle)) = html_element_getter_receiver(
        scope,
        args.this(),
        descriptor.interface,
        descriptor.member,
        local_name,
    ) else {
        rv.set_empty_string();
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, descriptor.attribute)
        .unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn html_dom_string_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    attr_name: &'static str,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), attr_name, rv);
}

fn set_html_dom_string_attribute_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attr_name: &'static str,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) {
    set_dom_string_attribute_property_on_object(scope, receiver, attr_name, value, owner, property);
}

pub(in crate::native_bridge) fn dom_string_reflection_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(descriptor) = DomStringReflection::descriptor_from_callback_data(scope, args.data())
    {
        if let Some(local_name) = descriptor.html_local_name {
            if let Some((runtime_ptr, handle)) = html_element_setter_receiver(
                scope,
                args.this(),
                descriptor.interface,
                descriptor.member,
                local_name,
            ) && let Some(value) = property_dom_string_value(
                scope,
                args.get(0),
                descriptor.interface,
                descriptor.member,
            ) {
                set_reflected_attribute(scope, runtime_ptr, handle, descriptor.attribute, &value);
            }
        } else {
            set_html_dom_string_attribute_for_receiver(
                scope,
                args.this(),
                descriptor.attribute,
                args.get(0),
                descriptor.interface,
                descriptor.member,
            );
        }
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn usv_string_reflection_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(descriptor) = UsvStringReflection::descriptor_from_callback_data(scope, args.data())
    {
        set_html_url_attribute_for_receiver(
            scope,
            args.this(),
            descriptor.attribute,
            args.get(0),
            descriptor.interface,
            descriptor.member,
        );
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn html_as_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "as");
    let Some(value) = v8_string(
        scope,
        crate::link_as::link_as_destination(value.as_deref()).reflected_value(),
    ) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn html_url_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    attr_name: &'static str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_empty_string();
        return;
    };
    let value = resolve_url_like_attribute(unsafe { &*runtime_ptr }, handle, attr_name);
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

fn set_html_url_attribute_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attr_name: &'static str,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) {
    set_usv_string_attribute_property_on_object(scope, receiver, attr_name, value, owner, property);
}

pub(in crate::native_bridge) fn object_data_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_url_attribute_getter(scope, args, "data", rv);
}

pub(in crate::native_bridge) fn object_code_base_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_url_attribute_getter(scope, args, "codebase", rv);
}

pub(in crate::native_bridge) fn image_long_desc_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "longdesc",
        args.get(0),
        "HTMLImageElement",
        "longDesc",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn image_lowsrc_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "lowsrc",
        args.get(0),
        "HTMLImageElement",
        "lowsrc",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn object_declare_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_property_getter_from_object_or_detached(scope, args.this(), "declare", rv);
}

pub(in crate::native_bridge) fn object_archive_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "archive", rv);
}

pub(in crate::native_bridge) fn object_code_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "code", rv);
}

pub(in crate::native_bridge) fn object_code_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "codetype", rv);
}

pub(in crate::native_bridge) fn object_standby_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "standby", rv);
}

pub(in crate::native_bridge) fn object_declare_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_html_boolean_attribute_for_receiver(
        scope,
        args.this(),
        "declare",
        args.get(0).boolean_value(scope),
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn html_align_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "align", rv);
}

pub(in crate::native_bridge) fn html_align_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(interface) = ElementReflectionInterface::from_callback_data(scope, args.data()) {
        set_html_dom_string_attribute_for_receiver(
            scope,
            args.this(),
            "align",
            args.get(0),
            interface.name(),
            "align",
        );
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn html_coords_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "coords", rv);
}

pub(in crate::native_bridge) fn html_charset_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "charset", rv);
}

pub(in crate::native_bridge) fn html_shape_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "shape", rv);
}

pub(in crate::native_bridge) fn html_no_href_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_property_getter_from_object_or_detached(scope, args.this(), "nohref", rv);
}

fn set_html_boolean_attribute_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attr_name: &'static str,
    enabled: bool,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        return;
    }
    set_reflected_boolean_attribute(scope, runtime_ptr, handle, attr_name, enabled);
}

pub(in crate::native_bridge) fn area_no_href_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_html_boolean_attribute_for_receiver(
        scope,
        args.this(),
        "nohref",
        args.get(0).boolean_value(scope),
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn table_cell_headers_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "headers", rv);
}

pub(in crate::native_bridge) fn table_cell_abbr_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "abbr", rv);
}

pub(in crate::native_bridge) fn table_cell_axis_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "axis", rv);
}

pub(in crate::native_bridge) fn table_cell_scope_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "scope").unwrap_or_default();
    let canonical = canonical_scope_value(&value);
    let Some(out) = v8_string(scope, canonical) else {
        rv.set_null();
        return;
    };
    rv.set(out.into());
}

pub(in crate::native_bridge) fn table_cell_no_wrap_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_property_getter_from_object_or_detached(scope, args.this(), "nowrap", rv);
}

pub(in crate::native_bridge) fn table_cell_no_wrap_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_html_boolean_attribute_for_receiver(
        scope,
        args.this(),
        "nowrap",
        args.get(0).boolean_value(scope),
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn table_ch_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "char", rv);
}

pub(in crate::native_bridge) fn table_ch_off_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "charoff", rv);
}

pub(in crate::native_bridge) fn table_v_align_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "valign", rv);
}

fn html_unsigned_long_attribute_getter_for_receiver<'s, F>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    content_attr: &str,
    default_value: u32,
    coerce: F,
) where
    F: FnOnce(u32) -> u32,
{
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        rv.set_uint32(default_value);
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let raw = element_attribute(unsafe { &*runtime_ptr }, handle, content_attr).unwrap_or_default();
    let parsed = parse_non_negative_integer(&raw)
        .filter(|value| *value <= i32::MAX as u32)
        .map(coerce)
        .unwrap_or(default_value);
    rv.set_uint32(parsed);
}

fn set_html_unsigned_long_attribute_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    content_attr: &str,
    value: v8::Local<'s, v8::Value>,
    interface: &'static str,
    member: &'static str,
) {
    let value = match webidl::convert::<webidl::UnsignedLong>(
        scope,
        value,
        webidl::Context::member(interface, member),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let value = if value <= i32::MAX as u32 { value } else { 0 };
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, content_attr, &value.to_string());
}

pub(in crate::native_bridge) fn table_col_span_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_unsigned_long_attribute_getter_for_receiver(scope, args.this(), rv, "span", 1, |value| {
        if value == 0 { 1 } else { value.min(1000) }
    });
}

pub(in crate::native_bridge) fn table_col_span_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_html_unsigned_long_attribute_for_receiver(
        scope,
        args.this(),
        "span",
        args.get(0),
        "HTMLTableColElement",
        "span",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn html_download_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_dom_string_attribute_getter(scope, args, "download", rv);
}

pub(in crate::native_bridge) fn html_ping_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "ping", rv);
}

pub(in crate::native_bridge) fn html_hreflang_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "hreflang", rv);
}

pub(in crate::native_bridge) fn node_dir_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    canonical_attribute_getter_from_object(scope, args.this(), "dir", canonical_dir_value, rv);
}

fn canonical_dir_value(raw: &str) -> &'static str {
    if raw.eq_ignore_ascii_case("ltr") {
        "ltr"
    } else if raw.eq_ignore_ascii_case("rtl") {
        "rtl"
    } else if raw.eq_ignore_ascii_case("auto") {
        "auto"
    } else {
        ""
    }
}

pub(in crate::native_bridge) fn node_input_mode_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    canonical_attribute_getter_from_object(
        scope,
        args.this(),
        "inputmode",
        canonical_input_mode_value,
        rv,
    );
}

pub(in crate::native_bridge) fn node_input_mode_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "inputmode",
        args.get(0),
        "HTMLElement",
        "inputMode",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_enter_key_hint_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    canonical_attribute_getter_from_object(
        scope,
        args.this(),
        "enterkeyhint",
        canonical_enter_key_hint_value,
        rv,
    );
}

pub(in crate::native_bridge) fn node_enter_key_hint_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "enterkeyhint",
        args.get(0),
        "HTMLElement",
        "enterKeyHint",
    );
    rv.set_undefined();
}

fn canonical_content_editable_value(raw: &str) -> &'static str {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized == "true" {
        "true"
    } else if normalized == "false" {
        "false"
    } else if normalized == "plaintext-only" {
        "plaintext-only"
    } else {
        "inherit"
    }
}

fn content_editable_state_from_attr(raw: &str) -> Option<bool> {
    match canonical_content_editable_value(raw) {
        "true" | "plaintext-only" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn element_content_editable_state(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let mut current = Some(handle);
    while let Some(candidate) = current {
        let Some(node) = runtime.dom_host().node(candidate) else {
            return false;
        };
        if node.is_document() {
            return runtime.document_design_mode_enabled(candidate);
        }
        let Some(element) = node.as_element() else {
            return false;
        };
        if let Some(value) = element
            .attribute_ns("", "contenteditable")
            .and_then(content_editable_state_from_attr)
        {
            return value;
        }
        current = runtime.dom_host().parent_node(candidate);
    }
    false
}

pub(in crate::native_bridge) fn node_content_editable_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        throw_incompatible_getter_receiver(scope, "HTMLElement", "contentEditable");
        return;
    };
    let Some(element) = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .filter(|element| element.namespace() == "http://www.w3.org/1999/xhtml")
    else {
        throw_incompatible_getter_receiver(scope, "HTMLElement", "contentEditable");
        return;
    };
    let value = element
        .attribute_ns("", "contenteditable")
        .map(canonical_content_editable_value)
        .unwrap_or("inherit");
    let Some(out) = v8_string(scope, value) else {
        rv.set_null();
        return;
    };
    rv.set(out.into());
}

pub(in crate::native_bridge) fn node_content_editable_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        throw_incompatible_setter_receiver(scope, "HTMLElement", "contentEditable");
        return;
    };
    let is_html_element = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.namespace() == "http://www.w3.org/1999/xhtml");
    if !is_html_element {
        throw_incompatible_setter_receiver(scope, "HTMLElement", "contentEditable");
        return;
    }
    let Some(value) =
        property_dom_string_value(scope, args.get(0), "HTMLElement", "contentEditable")
    else {
        return;
    };
    let value = value.to_ascii_lowercase();
    match value.as_str() {
        "true" | "false" | "plaintext-only" => {
            set_reflected_attribute(scope, runtime_ptr, handle, "contenteditable", &value);
        }
        "inherit" => {
            remove_reflected_attribute(scope, runtime_ptr, handle, "contenteditable");
        }
        _ => {
            throw_dom_exception(
                scope,
                "SyntaxError",
                12,
                "The value provided is not one of 'true', 'false', 'plaintext-only', or 'inherit'.",
            );
            return;
        }
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_is_content_editable_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_bool(false);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    rv.set_bool(
        node_is_element(runtime, handle) && element_content_editable_state(runtime, handle),
    );
}

fn canonical_input_mode_value(raw: &str) -> &'static str {
    const KEYWORDS: &[&str] = &[
        "none", "text", "tel", "url", "email", "numeric", "decimal", "search",
    ];
    for kw in KEYWORDS {
        if raw.eq_ignore_ascii_case(kw) {
            return kw;
        }
    }
    ""
}

fn canonical_enter_key_hint_value(raw: &str) -> &'static str {
    const KEYWORDS: &[&str] = &["enter", "done", "go", "next", "previous", "search", "send"];
    for kw in KEYWORDS {
        if raw.eq_ignore_ascii_case(kw) {
            return kw;
        }
    }
    ""
}

pub(in crate::native_bridge) fn node_dir_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "dir",
        args.get(0),
        "HTMLElement",
        "dir",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn canonical_referrer_policy_value(raw: &str) -> &'static str {
    const KEYWORDS: &[&str] = &[
        "no-referrer",
        "no-referrer-when-downgrade",
        "same-origin",
        "origin",
        "strict-origin",
        "origin-when-cross-origin",
        "strict-origin-when-cross-origin",
        "unsafe-url",
    ];
    for kw in KEYWORDS {
        if raw.eq_ignore_ascii_case(kw) {
            return kw;
        }
    }
    ""
}

pub(in crate::native_bridge) fn canonical_cross_origin_value(raw: &str) -> &'static str {
    // "Limited to only known values" with the special invalid-value default
    // "anonymous" (per HTML spec, an unrecognised content attribute maps to
    // CORS-anonymous mode, not the missing-value default). The IDL getter
    // therefore returns "anonymous" for any non-empty, non-keyword value.
    if raw.eq_ignore_ascii_case("use-credentials") {
        "use-credentials"
    } else {
        "anonymous"
    }
}

pub(in crate::native_bridge) fn canonical_loading_value(raw: &str) -> &'static str {
    if raw.eq_ignore_ascii_case("lazy") {
        "lazy"
    } else {
        "eager"
    }
}

pub(in crate::native_bridge) fn html_bg_color_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "bgcolor", rv);
}

pub(in crate::native_bridge) fn html_border_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "border", rv);
}

pub(in crate::native_bridge) fn html_color_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "color", rv);
}

pub(in crate::native_bridge) fn node_sandbox_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "sandbox", rv);
}

pub(in crate::native_bridge) fn node_sandbox_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "sandbox",
        args.get(0),
        "HTMLIFrameElement",
        "sandbox",
    );
    rv.set_undefined();
}

fn set_dom_string_treat_null_as_empty_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) {
    let options = webidl::StringOptions {
        treat_null_as_empty_string: true,
    };
    let value = match webidl::convert_with_options::<webidl::DomString>(
        scope,
        value,
        webidl::Context::member(owner, property),
        &options,
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &value);
}

pub(in crate::native_bridge) fn null_to_empty_dom_string_reflection_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(descriptor) =
        NullToEmptyDomStringReflection::descriptor_from_callback_data(scope, args.data())
    {
        set_dom_string_treat_null_as_empty_on_object(
            scope,
            args.this(),
            descriptor.attribute,
            args.get(0),
            descriptor.interface,
            descriptor.member,
        );
    }
    rv.set_undefined();
}

// size/height/width/sizes still use the legacy broad reflection path and need
// a separate owner/type split. hspace/vspace are installed below as
// unsigned-long reflections on their concrete owner prototypes.

pub(in crate::native_bridge) fn html_rel_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "rel", rv);
}

fn set_html_rel_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
) {
    set_dom_string_attribute_property_on_object(scope, receiver, "rel", value, owner, "rel");
}

pub(in crate::native_bridge) fn html_rel_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(interface) = ElementReflectionInterface::from_callback_data(scope, args.data()) {
        set_html_rel_for_receiver(scope, args.this(), args.get(0), interface.name());
    }
    rv.set_undefined();
}

// formAction / formMethod / formEnctype are intentionally NOT exposed on
// HTMLElement: their global presence regressed
// wpt_compat_case_..._the_select_element_select_remove (select.remove(-1)
// took the no-arg ChildNode.remove fallback once these accessors were on
// the template). The per-element installers in accessors_forms/ still wire
// these for HTMLInputElement and HTMLButtonElement.

fn set_boolean_attribute_on_object_or_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        return;
    }
    set_reflected_boolean_attribute(scope, runtime_ptr, handle, name, value.boolean_value(scope));
}

pub(in crate::native_bridge) fn html_compact_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_property_getter_from_object_or_detached(scope, args.this(), "compact", rv);
}

pub(in crate::native_bridge) fn html_compact_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_boolean_attribute_on_object_or_detached(scope, args.this(), "compact", args.get(0));
    rv.set_undefined();
}

pub(in crate::native_bridge) fn html_no_shade_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_property_getter_from_object_or_detached(scope, args.this(), "noshade", rv);
}

pub(in crate::native_bridge) fn html_no_shade_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_boolean_attribute_on_object_or_detached(scope, args.this(), "noshade", args.get(0));
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_allow_fullscreen_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_property_getter_from_object_or_detached(
        scope,
        args.this(),
        "allowfullscreen",
        rv,
    );
}

pub(in crate::native_bridge) fn node_allow_fullscreen_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_boolean_attribute_on_object_or_detached(scope, args.this(), "allowfullscreen", args.get(0));
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_credentialless_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_property_getter_from_object_or_detached(
        scope,
        args.this(),
        "credentialless",
        rv,
    );
}

pub(in crate::native_bridge) fn node_credentialless_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_boolean_attribute_on_object_or_detached(scope, args.this(), "credentialless", args.get(0));
    rv.set_undefined();
}

pub(in crate::native_bridge) fn html_size_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "size", rv);
}

pub(in crate::native_bridge) fn html_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "height", rv);
}

pub(in crate::native_bridge) fn html_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "width", rv);
}

pub(in crate::native_bridge) fn html_sizes_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "sizes", rv);
}

pub(in crate::native_bridge) fn pre_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_int32(0);
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "width")
        .and_then(|value| parse_tab_index_attribute(&value))
        .unwrap_or(0);
    rv.set_int32(value);
}

pub(in crate::native_bridge) fn pre_width_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = match webidl::convert::<webidl::Long>(
        scope,
        args.get(0),
        webidl::Context::member("HTMLPreElement", "width"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "width", &value.to_string());
    rv.set_undefined();
}

pub(in crate::native_bridge) fn source_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_unsigned_long_attribute_getter_for_receiver(scope, args.this(), rv, "width", 0, |value| {
        value
    });
}

pub(in crate::native_bridge) fn source_width_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_html_unsigned_long_attribute_for_receiver(
        scope,
        args.this(),
        "width",
        args.get(0),
        "HTMLSourceElement",
        "width",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn source_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_unsigned_long_attribute_getter_for_receiver(
        scope,
        args.this(),
        rv,
        "height",
        0,
        |value| value,
    );
}

pub(in crate::native_bridge) fn source_height_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_html_unsigned_long_attribute_for_receiver(
        scope,
        args.this(),
        "height",
        args.get(0),
        "HTMLSourceElement",
        "height",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn html_media_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "media", rv);
}

fn html_target_getter_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    owner: &'static str,
    local_name: &'static str,
) {
    let Some((runtime_ptr, handle)) =
        html_element_getter_receiver(scope, args.this(), owner, "target", local_name)
    else {
        rv.set_empty_string();
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "target").unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn anchor_target_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_target_getter_for_receiver(scope, args, rv, "HTMLAnchorElement", "a");
}

pub(in crate::native_bridge) fn area_target_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_target_getter_for_receiver(scope, args, rv, "HTMLAreaElement", "area");
}

pub(in crate::native_bridge) fn base_target_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_target_getter_for_receiver(scope, args, rv, "HTMLBaseElement", "base");
}

pub(in crate::native_bridge) fn link_target_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    html_target_getter_for_receiver(scope, args, rv, "HTMLLinkElement", "link");
}

fn set_html_target_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    local_name: &'static str,
) {
    let Some((runtime_ptr, handle)) =
        html_element_setter_receiver(scope, receiver, owner, "target", local_name)
    else {
        return;
    };
    let Some(value) = property_dom_string_value(scope, value, owner, "target") else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "target", &value);
}

pub(in crate::native_bridge) fn anchor_target_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_html_target_for_receiver(scope, args.this(), args.get(0), "HTMLAnchorElement", "a");
    rv.set_undefined();
}

pub(in crate::native_bridge) fn area_target_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_html_target_for_receiver(scope, args.this(), args.get(0), "HTMLAreaElement", "area");
    rv.set_undefined();
}

pub(in crate::native_bridge) fn base_target_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_html_target_for_receiver(scope, args.this(), args.get(0), "HTMLBaseElement", "base");
    rv.set_undefined();
}

pub(in crate::native_bridge) fn link_target_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_html_target_for_receiver(scope, args.this(), args.get(0), "HTMLLinkElement", "link");
    rv.set_undefined();
}

fn canonical_scope_value(raw: &str) -> &'static str {
    for kw in ["row", "col", "rowgroup", "colgroup"] {
        if raw.eq_ignore_ascii_case(kw) {
            return kw;
        }
    }
    ""
}

// ---------- Unsigned-long content attribute reflections ----------
//
// Each parses the content attribute via the HTML "non-negative integer
// parsing rules" (leading whitespace skipped, leading digits consumed) and
// returns the parsed value (clamped to [0, u32::MAX] then narrowed to i32 for
// the IDL `unsigned long`). When the attribute is missing or unparseable the
// attribute-specific default applies.

fn parse_non_negative_integer(value: &str) -> Option<u32> {
    // Per HTML "rules for parsing non-negative integers": skip leading ASCII
    // whitespace, then consume leading ASCII digits. Out-of-range results
    // saturate at u32::MAX rather than falling back to the attribute default
    // — the spec's reflection algorithm clamps to the unsigned-long range.
    let mut chars = value.chars().skip_while(|ch| ch.is_ascii_whitespace());
    if matches!(chars.clone().next(), Some('+')) {
        chars.next();
    }
    let mut acc: u64 = 0;
    let mut had_digit = false;
    for ch in chars.by_ref() {
        if let Some(digit) = ch.to_digit(10) {
            had_digit = true;
            acc = acc.saturating_mul(10).saturating_add(digit as u64);
            if acc > u32::MAX as u64 {
                acc = u32::MAX as u64;
            }
        } else {
            break;
        }
    }
    if had_digit { Some(acc as u32) } else { None }
}

fn unsigned_long_attribute_getter_from_object_or_detached<'s, F>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
    content_attr: &str,
    default_value: u32,
    coerce: F,
) where
    F: FnOnce(u32) -> u32,
{
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_uint32(default_value);
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let raw = element_attribute(unsafe { &*runtime_ptr }, handle, content_attr).unwrap_or_default();
    let parsed = parse_non_negative_integer(&raw)
        .filter(|value| *value <= i32::MAX as u32)
        .map(coerce)
        .unwrap_or(default_value);
    rv.set_uint32(parsed);
}

pub(in crate::native_bridge) fn html_hspace_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    unsigned_long_attribute_getter_from_object_or_detached(
        scope,
        args.this(),
        &mut rv,
        "hspace",
        0,
        |v| v,
    );
}

pub(in crate::native_bridge) fn html_vspace_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    unsigned_long_attribute_getter_from_object_or_detached(
        scope,
        args.this(),
        &mut rv,
        "vspace",
        0,
        |v| v,
    );
}

fn set_unsigned_long_attribute_on_object_or_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    content_attr: &str,
    value: v8::Local<'s, v8::Value>,
    interface: &'static str,
    member: &'static str,
) {
    // Per HTML reflection rules, the unsigned-long setter coerces via the
    // [EnforceRange]-free path and serialises with String(value).
    let value = match webidl::convert::<webidl::UnsignedLong>(
        scope,
        value,
        webidl::Context::member(interface, member),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let value = if value <= i32::MAX as u32 { value } else { 0 };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        return;
    }
    set_reflected_attribute(scope, runtime_ptr, handle, content_attr, &value.to_string());
}

pub(in crate::native_bridge) fn unsigned_long_reflection_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(descriptor) =
        UnsignedLongReflection::descriptor_from_callback_data(scope, args.data())
    {
        set_unsigned_long_attribute_on_object_or_detached(
            scope,
            args.this(),
            descriptor.attribute,
            args.get(0),
            descriptor.interface,
            descriptor.member,
        );
    }
    rv.set_undefined();
}

// ---------- "Limited to only known values" enumerations ----------

pub(in crate::native_bridge) fn canonical_preload_value(raw: &str) -> &'static str {
    if raw.eq_ignore_ascii_case("none") {
        "none"
    } else if raw.eq_ignore_ascii_case("metadata") {
        "metadata"
    } else {
        "auto"
    }
}

fn canonical_fetch_priority_value(raw: &str) -> &'static str {
    if raw.eq_ignore_ascii_case("low") {
        "low"
    } else if raw.eq_ignore_ascii_case("high") {
        "high"
    } else {
        "auto"
    }
}

pub(in crate::native_bridge) fn html_fetch_priority_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(descriptor) = DomStringReflection::descriptor_from_callback_data(scope, args.data())
    else {
        return;
    };
    let Some(local_name) = descriptor.html_local_name else {
        return;
    };
    let Some((runtime_ptr, handle)) = html_element_getter_receiver(
        scope,
        args.this(),
        descriptor.interface,
        descriptor.member,
        local_name,
    ) else {
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, descriptor.attribute)
        .unwrap_or_default();
    let Some(value) = v8_string(scope, canonical_fetch_priority_value(&value)) else {
        rv.set_empty_string();
        return;
    };
    rv.set(value.into());
}

// HTMLImageElement.decoding: sync / async / auto. Missing/invalid -> "auto".
pub(in crate::native_bridge) fn html_decoding_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_empty_string();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "decoding").unwrap_or_default();
    let Some(out) = v8_string(scope, canonical_decoding_value(&value)) else {
        rv.set_empty_string();
        return;
    };
    rv.set(out.into());
}

pub(in crate::native_bridge) fn image_decoding_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "decoding",
        args.get(0),
        "HTMLImageElement",
        "decoding",
    );
    rv.set_undefined();
}

fn canonical_decoding_value(raw: &str) -> &'static str {
    if raw.eq_ignore_ascii_case("sync") {
        "sync"
    } else if raw.eq_ignore_ascii_case("async") {
        "async"
    } else {
        "auto"
    }
}

pub(in crate::native_bridge) fn node_title_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "title", rv);
}

pub(in crate::native_bridge) fn node_title_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "title",
        args.get(0),
        "HTMLElement",
        "title",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_lang_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "lang", rv);
}

pub(in crate::native_bridge) fn node_lang_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "lang",
        args.get(0),
        "HTMLElement",
        "lang",
    );
    rv.set_undefined();
}

fn canonical_autocapitalize_value(raw: Option<&str>) -> &'static str {
    let Some(raw) = raw.filter(|raw| !raw.is_empty()) else {
        return "";
    };
    if raw.eq_ignore_ascii_case("none") || raw.eq_ignore_ascii_case("off") {
        "none"
    } else if raw.eq_ignore_ascii_case("characters") {
        "characters"
    } else if raw.eq_ignore_ascii_case("words") {
        "words"
    } else {
        "sentences"
    }
}

fn autocapitalize_inherits_from_form(local_name: &str) -> bool {
    matches!(
        local_name,
        "button" | "fieldset" | "input" | "output" | "select" | "textarea"
    )
}

pub(in crate::native_bridge) fn node_autocapitalize_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        throw_incompatible_getter_receiver(scope, "HTMLElement", "autocapitalize");
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(element) = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .filter(|element| element.namespace() == "http://www.w3.org/1999/xhtml")
    else {
        throw_incompatible_getter_receiver(scope, "HTMLElement", "autocapitalize");
        return;
    };

    let own_value = element.attribute_ns("", "autocapitalize");
    let raw = if own_value.is_some_and(|value| !value.is_empty())
        || !autocapitalize_inherits_from_form(element.local_name())
    {
        own_value
    } else {
        form_associated_form_owner(runtime, handle)
            .and_then(|form| runtime.dom_host().node(form))
            .and_then(Node::as_element)
            .and_then(|form| form.attribute_ns("", "autocapitalize"))
    };
    let Some(value) = v8_string(scope, canonical_autocapitalize_value(raw)) else {
        rv.set_empty_string();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn node_autocapitalize_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        throw_incompatible_setter_receiver(scope, "HTMLElement", "autocapitalize");
        return;
    };
    let is_html_element = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.namespace() == "http://www.w3.org/1999/xhtml");
    if !is_html_element {
        throw_incompatible_setter_receiver(scope, "HTMLElement", "autocapitalize");
        return;
    }
    let Some(value) =
        property_dom_string_value(scope, args.get(0), "HTMLElement", "autocapitalize")
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "autocapitalize", &value);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_translate_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_bool(true);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !node_is_element(runtime, handle) {
        rv.set_bool(true);
        return;
    }
    rv.set_bool(element_translate_mode(runtime, handle));
}

fn element_translate_mode(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let mut current = Some(handle);
    while let Some(candidate) = current {
        if let Some(value) = element_attribute(runtime, candidate, "translate") {
            if value.eq_ignore_ascii_case("no") {
                return false;
            }
            if value.is_empty() || value.eq_ignore_ascii_case("yes") {
                return true;
            }
        }
        current = runtime.dom_host().parent_node(candidate);
    }
    true
}

pub(in crate::native_bridge) fn node_translate_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_keyword_attribute_from_boolean_on_object(
        scope,
        args.this(),
        "translate",
        args.get(0),
        "yes",
        "no",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_access_key_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "accesskey", rv);
}

pub(in crate::native_bridge) fn node_access_key_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "accesskey",
        args.get(0),
        "HTMLElement",
        "accessKey",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_access_key_label_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        throw_incompatible_getter_receiver(scope, "HTMLElement", "accessKeyLabel");
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let is_html_element = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.namespace() == "http://www.w3.org/1999/xhtml");
    if !is_html_element {
        throw_incompatible_getter_receiver(scope, "HTMLElement", "accessKeyLabel");
        return;
    }

    let access_key = element_attribute(runtime, handle, "accesskey").unwrap_or_default();
    if access_key.is_empty() || access_key.encode_utf16().count() != 1 {
        rv.set_empty_string();
        return;
    }
    let Some(label) = v8_string(scope, &format!("Alt+{access_key}")) else {
        rv.set_empty_string();
        return;
    };
    rv.set(label.into());
}

pub(in crate::native_bridge) fn node_draggable_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_bool(false);
        return;
    };
    let Some(element) = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
    else {
        rv.set_bool(false);
        return;
    };
    let draggable = match element.attribute_ns("", "draggable") {
        Some(value) if value.eq_ignore_ascii_case("true") => true,
        Some(value) if value.eq_ignore_ascii_case("false") => false,
        _ => {
            element.is_html_element("img")
                || (element.is_html_element("a") && element.has_attribute_ns("", "href"))
        }
    };
    rv.set_bool(draggable);
}

pub(in crate::native_bridge) fn node_draggable_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_keyword_attribute_from_boolean_on_object(
        scope,
        args.this(),
        "draggable",
        args.get(0),
        "true",
        "false",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_spellcheck_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_bool(true);
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_bool(true);
        return;
    }
    rv.set_bool(
        !element_attribute(unsafe { &*runtime_ptr }, handle, "spellcheck")
            .is_some_and(|value| value.eq_ignore_ascii_case("false")),
    );
}

pub(in crate::native_bridge) fn node_spellcheck_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_keyword_attribute_from_boolean_on_object(
        scope,
        args.this(),
        "spellcheck",
        args.get(0),
        "true",
        "false",
    );
    rv.set_undefined();
}

fn set_keyword_attribute_from_boolean_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
    true_value: &str,
    false_value: &str,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        return;
    }
    let value = if value.boolean_value(scope) {
        true_value
    } else {
        false_value
    };
    set_reflected_attribute(scope, runtime_ptr, handle, name, value);
}

fn canonical_attribute_getter_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    canonical: fn(&str) -> &'static str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_null();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, name).unwrap_or_default();
    let Some(value) = v8_string(scope, canonical(&value)) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn node_autofocus_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_property_getter_from_object_or_detached(scope, args.this(), "autofocus", rv);
}

pub(in crate::native_bridge) fn node_autofocus_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_boolean_attribute_property_on_object_or_detached(
        scope,
        args.this(),
        "autofocus",
        args.get(0),
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_hidden_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    match element_attribute(unsafe { &*runtime_ptr }, handle, "hidden") {
        Some(value) if value.eq_ignore_ascii_case("until-found") => {
            if let Some(value) = v8_string(scope, "until-found") {
                rv.set(value.into());
            }
        }
        Some(_) => rv.set_bool(true),
        None => rv.set_bool(false),
    }
}

pub(in crate::native_bridge) fn node_hidden_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let value = args.get(0);
    if value.is_null_or_undefined() || value.is_boolean() || value.is_number() {
        set_reflected_boolean_attribute(
            scope,
            runtime_ptr,
            handle,
            "hidden",
            !value.is_null_or_undefined() && value.boolean_value(scope),
        );
    } else if let Some(value) = value.to_string(scope) {
        let value = value.to_rust_string_lossy(scope);
        if value.eq_ignore_ascii_case("until-found") {
            set_reflected_attribute(scope, runtime_ptr, handle, "hidden", "until-found");
        } else {
            set_reflected_boolean_attribute(
                scope,
                runtime_ptr,
                handle,
                "hidden",
                !value.is_empty(),
            );
        }
    }
    rv.set_undefined();
}

fn default_tab_index_for(runtime: &JsContextHost, handle: DomHandle) -> i32 {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return -1;
    };
    if element.namespace() != "http://www.w3.org/1999/xhtml" {
        return -1;
    }
    match element.local_name() {
        "a" | "button" | "input" | "select" | "textarea" => 0,
        _ => -1,
    }
}

pub(in crate::native_bridge) fn node_tab_index_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_int32(-1);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let value = element_attribute(runtime, handle, "tabindex")
        .and_then(|value| parse_tab_index_attribute(&value))
        .unwrap_or_else(|| default_tab_index_for(runtime, handle));
    rv.set_int32(value);
}

pub(in crate::native_bridge) fn node_tab_index_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = match webidl::convert::<webidl::Long>(
        scope,
        args.get(0),
        webidl::Context::member("HTMLElement", "tabIndex"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    set_attribute_property_on_object_or_detached(
        scope,
        args.this(),
        "tabindex",
        v8::Integer::new(scope, value).into(),
    );
    rv.set_undefined();
}

fn parse_tab_index_attribute(value: &str) -> Option<i32> {
    let value = value.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
    let mut chars = value.chars();
    let (sign, rest) = match chars.next() {
        Some('+') => (1_i64, chars.as_str()),
        Some('-') => (-1_i64, chars.as_str()),
        Some(_) => (1_i64, value),
        None => return None,
    };
    let digits = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    let magnitude = digits.parse::<i64>().ok()?;
    i32::try_from(sign * magnitude).ok()
}

#[cfg(test)]
mod tests {
    use super::{canonical_dir_value, parse_tab_index_attribute};

    #[test]
    fn parses_tab_index_attribute_like_html_signed_integer() {
        assert_eq!(parse_tab_index_attribute(""), None);
        assert_eq!(parse_tab_index_attribute(" 5abc"), Some(5));
        assert_eq!(parse_tab_index_attribute("+5"), Some(5));
        assert_eq!(parse_tab_index_attribute("-5"), Some(-5));
        assert_eq!(parse_tab_index_attribute("2147483647"), Some(2_147_483_647));
        assert_eq!(parse_tab_index_attribute("2147483648"), None);
        assert_eq!(
            parse_tab_index_attribute("-2147483648"),
            Some(-2_147_483_648)
        );
        assert_eq!(parse_tab_index_attribute("-2147483649"), None);
    }

    #[test]
    fn canonical_scope_value_matches_table_header_enumeration() {
        use super::canonical_scope_value;
        for kw in ["row", "col", "rowgroup", "colgroup"] {
            assert_eq!(canonical_scope_value(kw), kw);
            assert_eq!(canonical_scope_value(&kw.to_ascii_uppercase()), kw);
        }
        assert_eq!(canonical_scope_value(""), "");
        assert_eq!(canonical_scope_value("invalid"), "");
    }

    #[test]
    fn canonical_referrer_policy_value_matches_html_enumeration() {
        use super::canonical_referrer_policy_value;
        for kw in [
            "no-referrer",
            "no-referrer-when-downgrade",
            "same-origin",
            "origin",
            "strict-origin",
            "origin-when-cross-origin",
            "strict-origin-when-cross-origin",
            "unsafe-url",
        ] {
            assert_eq!(canonical_referrer_policy_value(kw), kw);
            assert_eq!(
                canonical_referrer_policy_value(&kw.to_ascii_uppercase()),
                kw
            );
        }
        assert_eq!(canonical_referrer_policy_value(""), "");
        assert_eq!(canonical_referrer_policy_value("bogus"), "");
    }

    #[test]
    fn canonical_cross_origin_value_uses_anonymous_as_invalid_default() {
        use super::canonical_cross_origin_value;
        assert_eq!(
            canonical_cross_origin_value("use-credentials"),
            "use-credentials"
        );
        assert_eq!(
            canonical_cross_origin_value("USE-credentials"),
            "use-credentials"
        );
        // anonymous and any non-keyword (including empty string) -> "anonymous".
        assert_eq!(canonical_cross_origin_value(""), "anonymous");
        assert_eq!(canonical_cross_origin_value("anonymous"), "anonymous");
        assert_eq!(canonical_cross_origin_value("ANONYMOUS"), "anonymous");
        assert_eq!(canonical_cross_origin_value("invalid-value"), "anonymous");
    }

    #[test]
    fn canonical_loading_value_matches_html_enumeration() {
        use super::canonical_loading_value;
        assert_eq!(canonical_loading_value("lazy"), "lazy");
        assert_eq!(canonical_loading_value("LAZY"), "lazy");
        // eager and any non-keyword (including empty string) -> "eager".
        assert_eq!(canonical_loading_value(""), "eager");
        assert_eq!(canonical_loading_value("eager"), "eager");
        assert_eq!(canonical_loading_value("xyz"), "eager");
    }

    #[test]
    fn canonical_input_mode_value_matches_html_known_value_enumeration() {
        use super::canonical_input_mode_value;
        for kw in [
            "none", "text", "tel", "url", "email", "numeric", "decimal", "search",
        ] {
            assert_eq!(canonical_input_mode_value(kw), kw);
            assert_eq!(canonical_input_mode_value(&kw.to_ascii_uppercase()), kw);
        }
        assert_eq!(canonical_input_mode_value(""), "");
        assert_eq!(canonical_input_mode_value("invalid"), "");
        assert_eq!(canonical_input_mode_value(" text "), "");
    }

    #[test]
    fn canonical_enter_key_hint_value_matches_html_known_value_enumeration() {
        use super::canonical_enter_key_hint_value;
        for kw in ["enter", "done", "go", "next", "previous", "search", "send"] {
            assert_eq!(canonical_enter_key_hint_value(kw), kw);
            assert_eq!(canonical_enter_key_hint_value(&kw.to_ascii_uppercase()), kw);
        }
        assert_eq!(canonical_enter_key_hint_value(""), "");
        assert_eq!(canonical_enter_key_hint_value("invalid"), "");
    }

    #[test]
    fn canonical_dir_value_matches_html_known_value_enumeration() {
        // Spec: "limited to only known values" with the three ASCII-case-
        // insensitive keywords ltr / rtl / auto. Anything else canonicalises
        // to the empty string on the IDL getter.
        assert_eq!(canonical_dir_value("ltr"), "ltr");
        assert_eq!(canonical_dir_value("RTL"), "rtl");
        assert_eq!(canonical_dir_value("Auto"), "auto");
        // Empty content attribute -> empty IDL value (not the keyword).
        assert_eq!(canonical_dir_value(""), "");
        // Non-keyword content attribute -> empty IDL value.
        assert_eq!(canonical_dir_value("7"), "");
        assert_eq!(canonical_dir_value(" ltr "), "");
        assert_eq!(canonical_dir_value("undefined"), "");
        assert_eq!(canonical_dir_value("xyz"), "");
    }
}
