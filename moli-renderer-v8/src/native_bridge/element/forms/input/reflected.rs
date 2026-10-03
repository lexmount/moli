use super::super::*;
use crate::native_bridge::element::{html_element_getter_receiver, html_element_setter_receiver};

fn input_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    html_element_getter_receiver(scope, receiver, "HTMLInputElement", member, "input")
}

fn input_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    html_element_setter_receiver(scope, receiver, "HTMLInputElement", member, "input")
}

fn input_string_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &str,
    property: &'static str,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if input_getter_receiver(scope, receiver, property).is_none() {
        return;
    }
    attribute_property_getter_from_object_or_detached(scope, receiver, attribute, rv);
}

fn set_input_dom_string_attribute_on_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    property: &'static str,
) {
    if input_setter_receiver(scope, object, property).is_none() {
        return;
    }
    set_dom_string_attribute_property_on_object(
        scope,
        object,
        attribute,
        value,
        "HTMLInputElement",
        property,
    );
}

fn input_boolean_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &str,
    property: &'static str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = input_getter_receiver(scope, receiver, property) else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(has_reflected_attribute(
        unsafe { &*runtime_ptr },
        handle,
        attribute,
    ));
}

fn set_input_boolean_attribute_on_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    property: &'static str,
) {
    let Some((runtime_ptr, handle)) = input_setter_receiver(scope, object, property) else {
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

pub(in crate::native_bridge) fn input_src_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_src_getter_from_receiver(scope, args.this(), &mut rv);
}

fn input_src_getter_from_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = input_getter_receiver(scope, receiver, "src") else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let value = match reflected_attribute(runtime, handle, "src") {
        None => String::new(),
        // `HTMLInputElement.src` is a reflected URL attribute, so present-but-empty resolves
        // against the document URL while a missing attribute stays the empty string.
        Some(src) if src.is_empty() => runtime.host_document().url().to_string(),
        Some(src) => runtime.host_document().resolve_url(&src),
    };
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn input_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = input_setter_receiver(scope, args.this(), "src") else {
        rv.set_undefined();
        return;
    };
    let Some(value) = property_usv_string_value(scope, args.get(0), "HTMLInputElement", "src")
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "src", &value);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_accept_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_string_attribute_getter(scope, args.this(), "accept", "accept", rv);
}

pub(in crate::native_bridge) fn input_accept_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_dom_string_attribute_on_receiver(scope, args.this(), "accept", args.get(0), "accept");
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_alt_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_string_attribute_getter(scope, args.this(), "alt", "alt", rv);
}

pub(in crate::native_bridge) fn input_alt_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_dom_string_attribute_on_receiver(scope, args.this(), "alt", args.get(0), "alt");
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_disabled_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_boolean_attribute_getter(scope, args.this(), "disabled", "disabled", rv);
}

pub(in crate::native_bridge) fn input_disabled_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_boolean_attribute_on_receiver(
        scope,
        args.this(),
        "disabled",
        args.get(0),
        "disabled",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_dir_name_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_string_attribute_getter(scope, args.this(), "dirname", "dirName", rv);
}

pub(in crate::native_bridge) fn input_dir_name_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_dom_string_attribute_on_receiver(
        scope,
        args.this(),
        "dirname",
        args.get(0),
        "dirName",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_form_action_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = input_getter_receiver(scope, args.this(), "formAction")
    else {
        rv.set_null();
        return;
    };
    input_form_action_getter_from_handle(scope, runtime_ptr, handle, &mut rv);
}

fn input_form_action_getter_from_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let runtime = unsafe { &*runtime_ptr };
    let value = reflected_attribute(runtime, handle, "formaction")
        .filter(|value| !value.is_empty())
        .map(|_| resolve_url_like_attribute(runtime, handle, "formaction"))
        .unwrap_or_else(|| runtime.host_document().url().to_string());
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_null();
    }
}

pub(in crate::native_bridge) fn input_form_action_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_dom_string_attribute_on_receiver(
        scope,
        args.this(),
        "formaction",
        args.get(0),
        "formAction",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_form_enctype_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = input_getter_receiver(scope, args.this(), "formEnctype")
    else {
        rv.set_empty_string();
        return;
    };
    input_form_enctype_getter_from_handle(scope, runtime_ptr, handle, &mut rv);
}

fn input_form_enctype_getter_from_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let value = reflected_attribute(unsafe { &*runtime_ptr }, handle, "formenctype")
        .map(|value| normalized_form_enctype(&value))
        .unwrap_or("");
    if let Some(value) = v8_string(scope, value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn input_form_enctype_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_dom_string_attribute_on_receiver(
        scope,
        args.this(),
        "formenctype",
        args.get(0),
        "formEnctype",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_form_method_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = input_getter_receiver(scope, args.this(), "formMethod")
    else {
        rv.set_empty_string();
        return;
    };
    input_form_method_getter_from_handle(scope, runtime_ptr, handle, &mut rv);
}

fn input_form_method_getter_from_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let method = reflected_attribute(unsafe { &*runtime_ptr }, handle, "formmethod")
        .map(|value| {
            let normalized = normalized_form_method(&value);
            if normalized == "dialog" {
                "get"
            } else {
                normalized
            }
        })
        .unwrap_or("");
    if let Some(value) = v8_string(scope, method) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn input_form_method_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_dom_string_attribute_on_receiver(
        scope,
        args.this(),
        "formmethod",
        args.get(0),
        "formMethod",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_form_target_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = input_getter_receiver(scope, args.this(), "formTarget")
    else {
        rv.set_empty_string();
        return;
    };
    let value =
        reflected_attribute(unsafe { &*runtime_ptr }, handle, "formtarget").unwrap_or_default();
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn input_form_target_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_dom_string_attribute_on_receiver(
        scope,
        args.this(),
        "formtarget",
        args.get(0),
        "formTarget",
    );
    rv.set_undefined();
}

fn input_unsigned_long_attribute_getter_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    attribute: &str,
    member: &'static str,
) {
    let Some((runtime_ptr, handle)) = input_getter_receiver(scope, receiver, member) else {
        rv.set_uint32(0);
        return;
    };
    let value = reflected_attribute(unsafe { &*runtime_ptr }, handle, attribute)
        .map(|value| parse_non_negative_dimension(Some(value)))
        .filter(|value| *value <= i32::MAX as u32)
        .unwrap_or(0);
    rv.set_uint32(value);
}

fn input_unsigned_long_attribute_setter_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    member: &'static str,
) {
    let Some((runtime_ptr, handle)) = input_setter_receiver(scope, receiver, member) else {
        return;
    };
    let value = match webidl::convert::<webidl::UnsignedLong>(
        scope,
        value,
        webidl::Context::member("HTMLInputElement", member),
    ) {
        Ok(value) if value.0 <= i32::MAX as u32 => value.0,
        Ok(_) => 0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &value.to_string());
}

pub(in crate::native_bridge) fn input_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_unsigned_long_attribute_getter_from_object(scope, args.this(), rv, "width", "width");
}

pub(in crate::native_bridge) fn input_width_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    input_unsigned_long_attribute_setter_on_object(
        scope,
        args.this(),
        "width",
        args.get(0),
        "width",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_unsigned_long_attribute_getter_from_object(scope, args.this(), rv, "height", "height");
}

pub(in crate::native_bridge) fn input_height_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    input_unsigned_long_attribute_setter_on_object(
        scope,
        args.this(),
        "height",
        args.get(0),
        "height",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_max_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_string_attribute_getter(scope, args.this(), "max", "max", rv);
}

pub(in crate::native_bridge) fn input_max_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_dom_string_attribute_on_receiver(scope, args.this(), "max", args.get(0), "max");
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_min_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_string_attribute_getter(scope, args.this(), "min", "min", rv);
}

pub(in crate::native_bridge) fn input_min_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_dom_string_attribute_on_receiver(scope, args.this(), "min", args.get(0), "min");
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_multiple_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_boolean_attribute_getter(scope, args.this(), "multiple", "multiple", rv);
}

pub(in crate::native_bridge) fn input_multiple_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_boolean_attribute_on_receiver(
        scope,
        args.this(),
        "multiple",
        args.get(0),
        "multiple",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_form_no_validate_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = input_getter_receiver(scope, args.this(), "formNoValidate")
    else {
        rv.set_undefined();
        return;
    };
    rv.set_bool(has_reflected_attribute(
        unsafe { &*runtime_ptr },
        handle,
        "formnovalidate",
    ));
}

pub(in crate::native_bridge) fn input_form_no_validate_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_boolean_attribute_on_receiver(
        scope,
        args.this(),
        "formnovalidate",
        args.get(0),
        "formNoValidate",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_placeholder_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_string_attribute_getter(scope, args.this(), "placeholder", "placeholder", rv);
}

pub(in crate::native_bridge) fn input_placeholder_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_dom_string_attribute_on_receiver(
        scope,
        args.this(),
        "placeholder",
        args.get(0),
        "placeholder",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_pattern_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_string_attribute_getter(scope, args.this(), "pattern", "pattern", rv);
}

pub(in crate::native_bridge) fn input_pattern_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_dom_string_attribute_on_receiver(
        scope,
        args.this(),
        "pattern",
        args.get(0),
        "pattern",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_read_only_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_boolean_attribute_getter(scope, args.this(), "readonly", "readOnly", rv);
}

pub(in crate::native_bridge) fn input_read_only_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_boolean_attribute_on_receiver(
        scope,
        args.this(),
        "readonly",
        args.get(0),
        "readOnly",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_step_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_string_attribute_getter(scope, args.this(), "step", "step", rv);
}

pub(in crate::native_bridge) fn input_step_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_dom_string_attribute_on_receiver(scope, args.this(), "step", args.get(0), "step");
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_required_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    input_boolean_attribute_getter(scope, args.this(), "required", "required", rv);
}

pub(in crate::native_bridge) fn input_required_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_input_boolean_attribute_on_receiver(
        scope,
        args.this(),
        "required",
        args.get(0),
        "required",
    );
    rv.set_undefined();
}
