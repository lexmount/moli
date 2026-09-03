use super::super::*;
use crate::native_bridge::element::{html_element_getter_receiver, html_element_setter_receiver};
use crate::webidl;
use moli_dom::forms::parse_non_negative_length_attribute;

pub(in crate::native_bridge) fn input_max_length_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    text_control_length_getter_from_object(
        scope,
        args.this(),
        rv,
        "HTMLInputElement",
        "input",
        "maxLength",
        "maxlength",
    );
}

pub(in crate::native_bridge) fn input_max_length_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    text_control_length_setter_on_object(
        scope,
        args.this(),
        args.get(0),
        "HTMLInputElement",
        "input",
        "maxLength",
        "maxlength",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_min_length_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    text_control_length_getter_from_object(
        scope,
        args.this(),
        rv,
        "HTMLInputElement",
        "input",
        "minLength",
        "minlength",
    );
}

pub(in crate::native_bridge) fn input_min_length_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    text_control_length_setter_on_object(
        scope,
        args.this(),
        args.get(0),
        "HTMLInputElement",
        "input",
        "minLength",
        "minlength",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn textarea_max_length_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    text_control_length_getter_from_object(
        scope,
        args.this(),
        rv,
        "HTMLTextAreaElement",
        "textarea",
        "maxLength",
        "maxlength",
    );
}

pub(in crate::native_bridge) fn textarea_max_length_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    text_control_length_setter_on_object(
        scope,
        args.this(),
        args.get(0),
        "HTMLTextAreaElement",
        "textarea",
        "maxLength",
        "maxlength",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn textarea_min_length_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    text_control_length_getter_from_object(
        scope,
        args.this(),
        rv,
        "HTMLTextAreaElement",
        "textarea",
        "minLength",
        "minlength",
    );
}

pub(in crate::native_bridge) fn textarea_min_length_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    text_control_length_setter_on_object(
        scope,
        args.this(),
        args.get(0),
        "HTMLTextAreaElement",
        "textarea",
        "minLength",
        "minlength",
    );
    rv.set_undefined();
}

fn text_control_length_getter_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    owner: &'static str,
    local_name: &'static str,
    property: &'static str,
    attribute: &str,
) {
    let Some((runtime_ptr, handle)) =
        html_element_getter_receiver(scope, object, owner, property, local_name)
    else {
        rv.set_int32(-1);
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, attribute)
        .and_then(|value| parse_non_negative_length_attribute(&value))
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or(-1);
    rv.set_int32(value);
}

fn text_control_length_setter_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    local_name: &'static str,
    property: &'static str,
    attribute: &str,
) {
    let Some((runtime_ptr, handle)) =
        html_element_setter_receiver(scope, object, owner, property, local_name)
    else {
        return;
    };
    let number = match webidl::convert::<webidl::Long>(
        scope,
        value,
        webidl::Context::member(owner, property),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if number < 0 {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "Index or size is negative or greater than the allowed amount",
        );
        return;
    }
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &number.to_string());
}

pub(in crate::native_bridge) fn input_size_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    input_size_getter_from_object(scope, args.this(), &mut rv);
}

fn input_size_getter_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_uint32(20);
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "size")
        .and_then(|value| parse_positive_integer_prefix(&value))
        .unwrap_or(20);
    rv.set_uint32(value);
}

fn parse_positive_integer_prefix(value: &str) -> Option<u32> {
    let value = value.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
    let value = value.strip_prefix('+').unwrap_or(value);
    let digits = value
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits
        .parse::<i32>()
        .ok()
        .filter(|value| *value > 0)
        .map(|value| value as u32)
}

pub(in crate::native_bridge) fn input_size_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let normalized = match webidl::convert::<webidl::Long>(
        scope,
        args.get(0),
        webidl::Context::member("HTMLInputElement", "size"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if normalized == 0 {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "Index or size is negative or greater than the allowed amount",
        );
        return;
    }
    let normalized = if normalized < 0 { 20 } else { normalized };
    set_reflected_attribute(scope, runtime_ptr, handle, "size", &normalized.to_string());
    rv.set_undefined();
}
