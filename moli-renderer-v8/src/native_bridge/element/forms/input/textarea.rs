use super::super::*;
use crate::util::v8_string_from_utf16_units;

fn textarea_string_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &str,
    property: &'static str,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if textarea_getter_receiver(scope, receiver, property).is_none() {
        return;
    }
    attribute_property_getter_from_object_or_detached(scope, receiver, attribute, rv);
}

fn set_textarea_dom_string_attribute_on_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    property: &'static str,
) {
    if textarea_setter_receiver(scope, receiver, property).is_none() {
        return;
    }
    set_dom_string_attribute_property_on_object(
        scope,
        receiver,
        attribute,
        value,
        "HTMLTextAreaElement",
        property,
    );
}

fn textarea_boolean_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &str,
    property: &'static str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = textarea_getter_receiver(scope, receiver, property) else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(element_has_attribute(
        unsafe { &*runtime_ptr },
        handle,
        attribute,
    ));
}

fn set_textarea_boolean_attribute_on_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    property: &'static str,
) {
    let Some((runtime_ptr, handle)) = textarea_setter_receiver(scope, receiver, property) else {
        return;
    };
    set_reflected_boolean_attribute(
        scope,
        runtime_ptr,
        handle,
        attribute,
        value.boolean_value(scope),
    );
}

pub(in crate::native_bridge) fn textarea_default_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = textarea_getter_receiver(scope, args.this(), "defaultValue")
    else {
        rv.set_empty_string();
        return;
    };
    let value =
        node_direct_text_content_dom_string(unsafe { &*runtime_ptr }, handle).unwrap_or_default();
    if let Some(value) = v8_string_from_utf16_units(scope, &value.utf16_units()) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn textarea_default_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = textarea_setter_receiver(scope, args.this(), "defaultValue")
    else {
        rv.set_undefined();
        return;
    };
    let Some(next) = form_dom_string_property_utf16_value(
        scope,
        args.get(0),
        "HTMLTextAreaElement",
        "defaultValue",
        false,
    ) else {
        return;
    };
    let _ = set_text_content_in_reaction_scope(scope, runtime_ptr, handle, &next);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn textarea_text_length_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = textarea_getter_receiver(scope, args.this(), "textLength")
    else {
        rv.set_uint32(0);
        return;
    };
    let value = text_control_value_dom_string(unsafe { &*runtime_ptr }, handle);
    rv.set_uint32(value.utf16_units().len() as u32);
}

pub(in crate::native_bridge) fn textarea_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if textarea_getter_receiver(scope, args.this(), "type").is_none() {
        rv.set_empty_string();
        return;
    }
    match v8_string(scope, "textarea") {
        Some(value) => rv.set(value.into()),
        None => rv.set_empty_string(),
    }
}

pub(in crate::native_bridge) fn textarea_disabled_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    textarea_boolean_attribute_getter(scope, args.this(), "disabled", "disabled", rv);
}

pub(in crate::native_bridge) fn textarea_disabled_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_textarea_boolean_attribute_on_receiver(
        scope,
        args.this(),
        "disabled",
        args.get(0),
        "disabled",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn textarea_required_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    textarea_boolean_attribute_getter(scope, args.this(), "required", "required", rv);
}

pub(in crate::native_bridge) fn textarea_required_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_textarea_boolean_attribute_on_receiver(
        scope,
        args.this(),
        "required",
        args.get(0),
        "required",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn textarea_dir_name_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    textarea_string_attribute_getter(scope, args.this(), "dirname", "dirName", rv);
}

pub(in crate::native_bridge) fn textarea_dir_name_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_textarea_dom_string_attribute_on_receiver(
        scope,
        args.this(),
        "dirname",
        args.get(0),
        "dirName",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn textarea_cols_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    textarea_limited_unsigned_getter(scope, args.this(), rv, "cols", "cols", 20);
}

pub(in crate::native_bridge) fn textarea_cols_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    textarea_limited_unsigned_setter(scope, args.this(), args.get(0), "cols", "cols", 20);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn textarea_rows_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    textarea_limited_unsigned_getter(scope, args.this(), rv, "rows", "rows", 2);
}

pub(in crate::native_bridge) fn textarea_rows_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    textarea_limited_unsigned_setter(scope, args.this(), args.get(0), "rows", "rows", 2);
    rv.set_undefined();
}

fn textarea_limited_unsigned_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    attribute: &str,
    property: &'static str,
    default_value: u32,
) {
    let Some((runtime_ptr, handle)) = textarea_getter_receiver(scope, object, property) else {
        rv.set_uint32(default_value);
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, attribute)
        .and_then(|value| parse_positive_integer_prefix(&value))
        .filter(|value| *value <= i32::MAX as u32)
        .unwrap_or(default_value);
    rv.set_uint32(value);
}

fn textarea_limited_unsigned_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    attribute: &str,
    property: &'static str,
    default_value: u32,
) {
    let Some((runtime_ptr, handle)) = textarea_setter_receiver(scope, object, property) else {
        return;
    };
    let value = match webidl::convert::<webidl::UnsignedLong>(
        scope,
        value,
        webidl::Context::member("HTMLTextAreaElement", property),
    ) {
        Ok(value) if (1..=i32::MAX as u32).contains(&value.0) => value.0,
        Ok(_) => default_value,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &value.to_string());
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

pub(in crate::native_bridge) fn textarea_wrap_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    textarea_string_attribute_getter(scope, args.this(), "wrap", "wrap", rv);
}

pub(in crate::native_bridge) fn textarea_wrap_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_textarea_dom_string_attribute_on_receiver(scope, args.this(), "wrap", args.get(0), "wrap");
    rv.set_undefined();
}

pub(in crate::native_bridge) fn textarea_placeholder_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    textarea_string_attribute_getter(scope, args.this(), "placeholder", "placeholder", rv);
}

pub(in crate::native_bridge) fn textarea_placeholder_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_textarea_dom_string_attribute_on_receiver(
        scope,
        args.this(),
        "placeholder",
        args.get(0),
        "placeholder",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn textarea_read_only_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    textarea_boolean_attribute_getter(scope, args.this(), "readonly", "readOnly", rv);
}

pub(in crate::native_bridge) fn textarea_read_only_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_textarea_boolean_attribute_on_receiver(
        scope,
        args.this(),
        "readonly",
        args.get(0),
        "readOnly",
    );
    rv.set_undefined();
}
