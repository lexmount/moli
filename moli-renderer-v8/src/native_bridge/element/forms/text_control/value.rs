use super::*;
use crate::native_bridge::document::detached_native_handle_for_runtime;
use crate::util::utf16_len;

pub(crate) fn text_control_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return String::new();
    };
    if element.is_html_input() {
        return element.input_value();
    }
    if element.is_html_textarea() {
        if element.input_value_dirty() {
            return normalize_textarea_api_value(&element.input_value());
        }
        let default_value = node_direct_text_content(runtime, handle).unwrap_or_default();
        return normalize_textarea_api_value(&default_value);
    }
    String::new()
}

pub(in crate::native_bridge) fn normalize_textarea_api_value(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                let _ = chars.next();
            }
            output.push('\n');
        } else {
            output.push(ch);
        }
    }
    output
}

pub(super) fn clamp_text_control_offset(
    runtime: &JsContextHost,
    handle: DomHandle,
    offset: u32,
) -> u32 {
    let len = utf16_len(&text_control_value(runtime, handle)) as u32;
    offset.min(len)
}

pub(crate) fn is_text_control(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| {
            element.is_html_textarea()
                || (element.is_html_input()
                    && !matches!(
                        element.input_type(),
                        InputType::Hidden
                            | InputType::Checkbox
                            | InputType::Radio
                            | InputType::Button
                            | InputType::Submit
                            | InputType::Reset
                            | InputType::Image
                    ))
        })
}

pub(super) fn supports_variable_length_selection(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| {
            element.is_html_textarea()
                || (element.is_html_input()
                    && element.input_type().supports_variable_length_selection())
        })
}

pub(in crate::native_bridge) fn textarea_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = textarea_getter_receiver(scope, args.this(), "value") else {
        rv.set_null();
        return;
    };
    let value = text_control_value(unsafe { &*runtime_ptr }, handle);
    let value =
        detached_textarea_value_attribute_default(scope, runtime_ptr, args.this(), handle, &value)
            .unwrap_or(value);
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn detached_textarea_value_attribute_default<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    receiver: v8::Local<'s, v8::Object>,
    handle: DomHandle,
    current_value: &str,
) -> Option<String> {
    if !current_value.is_empty()
        || detached_native_handle_for_runtime(scope, runtime_ptr, receiver).is_none()
    {
        return None;
    }
    let runtime = unsafe { &*runtime_ptr };
    let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    if !element.is_html_textarea() || element.input_value_dirty() {
        return None;
    }
    element.attribute_ns("", "value").map(str::to_owned)
}

pub(in crate::native_bridge) fn textarea_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = textarea_setter_receiver(scope, args.this(), "value") else {
        rv.set_undefined();
        return;
    };
    let Some(next_value) =
        form_dom_string_property_value(scope, args.get(0), "HTMLTextAreaElement", "value", true)
    else {
        return;
    };
    let next_value = normalize_textarea_api_value(&next_value);
    let previous_value = text_control_value(unsafe { &*runtime_ptr }, handle);
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_input_value(handle, &next_value);
    let current_value = text_control_value(runtime, handle);
    if current_value != previous_value {
        let end = utf16_len(&current_value) as u32;
        let _ = runtime.set_selection_range(handle, end, end);
    }
    rv.set_undefined();
}
