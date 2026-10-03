use crate::util::v8_string;

use super::super::{
    has_reflected_attribute, html_element_getter_receiver, html_element_setter_receiver,
    property_string_value, reflected_attribute, set_reflected_attribute,
    set_reflected_boolean_attribute,
};
use super::parse_i32_attribute_or;

pub(in crate::native_bridge::element) fn li_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    i32_attribute_getter(
        scope,
        args.this(),
        "HTMLLIElement",
        "value",
        "li",
        "value",
        0,
        rv,
    );
}

pub(in crate::native_bridge::element) fn li_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    i32_attribute_setter(
        scope,
        args.this(),
        "HTMLLIElement",
        "value",
        "li",
        "value",
        args.get(0),
        0,
        rv,
    );
}

pub(in crate::native_bridge::element) fn ol_start_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    i32_attribute_getter(
        scope,
        args.this(),
        "HTMLOListElement",
        "start",
        "ol",
        "start",
        1,
        rv,
    );
}

pub(in crate::native_bridge::element) fn ol_start_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    i32_attribute_setter(
        scope,
        args.this(),
        "HTMLOListElement",
        "start",
        "ol",
        "start",
        args.get(0),
        1,
        rv,
    );
}

pub(in crate::native_bridge::element) fn ol_reversed_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_getter(
        scope,
        args.this(),
        "HTMLOListElement",
        "reversed",
        "ol",
        "reversed",
        rv,
    );
}

pub(in crate::native_bridge::element) fn ol_reversed_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_setter(
        scope,
        args.this(),
        "HTMLOListElement",
        "reversed",
        "ol",
        "reversed",
        args.get(0),
        rv,
    );
}

pub(in crate::native_bridge::element) fn ol_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_getter_receiver(scope, args.this(), "HTMLOListElement", "type", "ol")
    else {
        rv.set_empty_string();
        return;
    };
    let value = reflected_attribute(unsafe { &*runtime_ptr }, handle, "type").unwrap_or_default();
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge::element) fn ol_type_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    string_attribute_setter(
        scope,
        args.this(),
        "HTMLOListElement",
        "type",
        "ol",
        "type",
        args.get(0),
        rv,
    );
}

pub(in crate::native_bridge::element) fn optgroup_disabled_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_getter(
        scope,
        args.this(),
        "HTMLOptGroupElement",
        "disabled",
        "optgroup",
        "disabled",
        rv,
    );
}

pub(in crate::native_bridge::element) fn optgroup_disabled_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_setter(
        scope,
        args.this(),
        "HTMLOptGroupElement",
        "disabled",
        "optgroup",
        "disabled",
        args.get(0),
        rv,
    );
}

fn i32_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    interface: &'static str,
    member: &'static str,
    local_name: &'static str,
    attribute: &str,
    default: i32,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_getter_receiver(scope, object, interface, member, local_name)
    else {
        rv.set_int32(default);
        return;
    };
    rv.set_int32(parse_i32_attribute_or(
        unsafe { &*runtime_ptr },
        handle,
        attribute,
        default,
    ));
}

fn i32_attribute_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    interface: &'static str,
    member: &'static str,
    local_name: &'static str,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    default: i32,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_setter_receiver(scope, object, interface, member, local_name)
    else {
        rv.set_undefined();
        return;
    };
    let number = value.int32_value(scope).unwrap_or(default);
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &number.to_string());
    rv.set_undefined();
}

fn string_attribute_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    interface: &'static str,
    member: &'static str,
    local_name: &'static str,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_setter_receiver(scope, object, interface, member, local_name)
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) = property_string_value(scope, value) else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &value);
    rv.set_undefined();
}

fn boolean_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    interface: &'static str,
    member: &'static str,
    local_name: &'static str,
    attribute: &str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_getter_receiver(scope, object, interface, member, local_name)
    else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(has_reflected_attribute(
        unsafe { &*runtime_ptr },
        handle,
        attribute,
    ));
}

fn boolean_attribute_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    interface: &'static str,
    member: &'static str,
    local_name: &'static str,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_setter_receiver(scope, object, interface, member, local_name)
    else {
        rv.set_undefined();
        return;
    };
    set_reflected_boolean_attribute(
        scope,
        runtime_ptr,
        handle,
        attribute,
        value.boolean_value(scope),
    );
    rv.set_undefined();
}
