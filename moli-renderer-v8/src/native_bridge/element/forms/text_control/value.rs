use super::*;
use crate::dom::forms::normalize_textarea_dom_string_value;
use crate::dom::native::DomStringValue;
use crate::util::v8_string_from_utf16_units;
pub(in crate::native_bridge) use moli_dom::forms::normalize_textarea_api_value;

pub(crate) fn text_control_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    text_control_value_dom_string(runtime, handle)
        .as_str_lossy()
        .to_owned()
}

pub(crate) fn text_control_value_dom_string(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> DomStringValue {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return DomStringValue::default();
    };
    if element.is_html_input() {
        return element.input_value_dom_string();
    }
    if element.is_html_textarea() {
        if element.input_value_dirty() {
            return normalize_textarea_dom_string_value(&element.input_value_dom_string());
        }
        let default_value =
            node_direct_text_content_dom_string(runtime, handle).unwrap_or_default();
        return normalize_textarea_dom_string_value(&default_value);
    }
    DomStringValue::default()
}

pub(super) fn clamp_text_control_offset(
    runtime: &JsContextHost,
    handle: DomHandle,
    offset: u32,
) -> u32 {
    let len = text_control_value_dom_string(runtime, handle)
        .utf16_units()
        .len() as u32;
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
                            | InputType::Color
                            | InputType::File
                            | InputType::Range
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
    let value = text_control_value_dom_string(unsafe { &*runtime_ptr }, handle);
    let Some(value) = v8_string_from_utf16_units(scope, &value.utf16_units()) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
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
    let Some(next_value) = form_dom_string_property_utf16_value(
        scope,
        args.get(0),
        "HTMLTextAreaElement",
        "value",
        true,
    ) else {
        return;
    };
    let previous_value = text_control_value_dom_string(unsafe { &*runtime_ptr }, handle);
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_input_value(handle, &next_value);
    let current_value = text_control_value_dom_string(runtime, handle);
    if current_value != previous_value {
        let end = current_value.utf16_units().len() as u32;
        let selection_changed = runtime.set_text_control_selection(handle, end, end, "none");
        restore_focused_text_control_selection(scope, runtime_ptr, handle);
        if selection_changed || unsafe { &*runtime_ptr }.active_element_handle() == Some(handle) {
            queue_text_control_selection_change_event(scope, runtime_ptr, handle);
        }
    }
    rv.set_undefined();
}
