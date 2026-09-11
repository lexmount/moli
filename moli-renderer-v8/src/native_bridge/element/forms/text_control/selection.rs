use super::events::{
    dispatch_text_control_event, queue_text_control_select_event,
    queue_text_control_selection_change_event,
};
use super::value::{
    clamp_text_control_offset, is_text_control, supports_variable_length_selection,
};
use super::*;
use crate::dom::forms::parse_non_negative_length_attribute;
use crate::util::{utf16_replace_units_range_lossy, utf16_units, v8str};
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "setSelectionRange")]
struct TextControlSetSelectionRangeArgs {
    #[webidl(required)]
    start: u32,
    #[webidl(required)]
    end: u32,
    direction: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "setRangeText")]
struct TextControlSetRangeTextArgs {
    #[webidl(required)]
    replacement: String,
    start: Option<u32>,
    end: Option<u32>,
    selection_mode: Option<String>,
}

fn throw_invalid_selection_state(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "This input element does not support text selection.",
    );
}

fn text_control_selection_offset_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    member: &'static str,
) -> Option<u32> {
    match webidl::convert::<webidl::UnsignedLong>(
        scope,
        value,
        webidl::Context::member(owner, member),
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn text_control_selection_idl_owner(runtime: &JsContextHost, handle: DomHandle) -> &'static str {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(|element| match element.local_name() {
            "textarea" => "HTMLTextAreaElement",
            _ => "HTMLInputElement",
        })
        .unwrap_or("HTMLInputElement")
}

fn event_default_prevented(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
) -> bool {
    event
        .get(scope, v8str(scope, "defaultPrevented").into())
        .is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn text_control_set_selection_range_internal(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    start: u32,
    end: u32,
) -> bool {
    text_control_set_selection_range_with_direction_internal(
        scope,
        runtime_ptr,
        handle,
        start,
        end,
        "none",
    )
}

pub(crate) fn text_control_set_selection_range_with_direction_internal(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    start: u32,
    end: u32,
    direction: &str,
) -> bool {
    let runtime = unsafe { &mut *runtime_ptr };
    let start = clamp_text_control_offset(runtime, handle, start);
    let end = clamp_text_control_offset(runtime, handle, end);
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, end)
    };
    let changed = runtime.set_selection_range_with_direction(handle, start, end, direction);
    if changed {
        queue_text_control_select_event(scope, runtime_ptr, handle);
        queue_text_control_selection_change_event(scope, runtime_ptr, handle);
    }
    changed
}

pub(crate) fn replace_text_control_selection(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    replacement_text: &str,
) -> bool {
    if !is_text_control(unsafe { &*runtime_ptr }, handle) {
        return false;
    }

    let Some(before_input) = construct_simple_event(scope, "beforeinput", true, true, true) else {
        return false;
    };
    let _ = dispatch_public_event(scope, runtime_ptr, handle, before_input);
    if event_default_prevented(scope, before_input) {
        return false;
    }

    let runtime = unsafe { &*runtime_ptr };
    if !is_text_control(runtime, handle) {
        return false;
    }
    let value = text_control_value(runtime, handle);
    let value_units = utf16_units(&value);
    let (start, end) = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(|element| {
            let start = element.selection_start();
            let end = element.selection_end();
            if start <= end {
                (start, end)
            } else {
                (end, end)
            }
        })
        .unwrap_or_else(|| {
            let value_len = value_units.len() as u32;
            (value_len, value_len)
        });
    let start = (start as usize).min(value_units.len());
    let end = (end as usize).min(value_units.len()).max(start);
    let replacement_units = text_control_user_edit_replacement_units(
        runtime,
        handle,
        value_units.len(),
        start,
        end,
        replacement_text,
    );
    let next_value = utf16_replace_units_range_lossy(
        &value_units,
        start,
        end.saturating_sub(start),
        &replacement_units,
    );

    let runtime = unsafe { &mut *runtime_ptr };
    let changed = runtime.set_input_value_from_user_edit(handle, &next_value);
    if changed {
        runtime.mark_text_control_change_pending(handle, &value);
    }
    let caret = u32::try_from(start.saturating_add(replacement_units.len())).unwrap_or(u32::MAX);
    let selection_changed =
        text_control_set_selection_range_internal(scope, runtime_ptr, handle, caret, caret);
    if changed || selection_changed {
        dispatch_text_control_event(scope, runtime_ptr, handle, "input");
    }
    changed || selection_changed
}

fn text_control_user_edit_replacement_units(
    runtime: &JsContextHost,
    handle: DomHandle,
    current_value_len: usize,
    selection_start: usize,
    selection_end: usize,
    replacement_text: &str,
) -> Vec<u16> {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return Vec::new();
    };
    let replacement_text = if element.is_html_input() {
        normalize_single_line_text_insertion(replacement_text)
    } else {
        replacement_text.to_owned()
    };
    let mut replacement_units = utf16_units(&replacement_text);

    let max_length = (element.is_html_textarea()
        || (element.is_html_input() && element.input_type().supports_text_length_validation()))
    .then(|| element.attribute("maxlength"))
    .flatten()
    .and_then(parse_non_negative_length_attribute);
    let Some(max_length) = max_length else {
        return replacement_units;
    };

    let selection_len = if runtime.active_element_handle() == Some(handle) {
        selection_end.saturating_sub(selection_start)
    } else {
        0
    };
    let base_len = current_value_len.saturating_sub(selection_len);
    let appendable_len = max_length.saturating_sub(base_len);
    if replacement_units.len() <= appendable_len {
        return replacement_units;
    }
    replacement_units.truncate(appendable_len);
    if replacement_units
        .last()
        .is_some_and(|unit| (0xD800..=0xDBFF).contains(unit))
    {
        let _ = replacement_units.pop();
    }
    replacement_units
}

fn normalize_single_line_text_insertion(value: &str) -> String {
    let value = value.trim_end_matches(['\r', '\n']);
    let mut normalized = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    let _ = chars.next();
                }
                normalized.push(' ');
            }
            '\n' => normalized.push(' '),
            _ => normalized.push(ch),
        }
    }
    normalized
}

fn current_selection_or_end(
    runtime: &JsContextHost,
    handle: DomHandle,
    value_len: u32,
) -> (u32, u32) {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(|element| {
            let start = element.selection_start().min(value_len);
            let end = element.selection_end().min(value_len);
            if start <= end {
                (start, end)
            } else {
                (end, start)
            }
        })
        .unwrap_or((value_len, value_len))
}

pub(in crate::native_bridge) fn text_control_selection_start_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_uint32(0);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !supports_variable_length_selection(runtime, handle) {
        rv.set_null();
        return;
    }
    let value = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(Element::selection_start)
        .unwrap_or(0);
    rv.set_uint32(value);
}

pub(in crate::native_bridge) fn text_control_selection_start_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !supports_variable_length_selection(runtime, handle) {
        throw_invalid_selection_state(scope);
        return;
    }
    let owner = text_control_selection_idl_owner(runtime, handle);
    let value = args.get(0);
    let Some(next) = text_control_selection_offset_value(scope, value, owner, "selectionStart")
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let next = clamp_text_control_offset(runtime, handle, next);
    if runtime.set_selection_start(handle, next) {
        queue_text_control_select_event(scope, runtime_ptr, handle);
        queue_text_control_selection_change_event(scope, runtime_ptr, handle);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn text_control_selection_end_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_uint32(0);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !supports_variable_length_selection(runtime, handle) {
        rv.set_null();
        return;
    }
    let value = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(Element::selection_end)
        .unwrap_or(0);
    rv.set_uint32(value);
}

pub(in crate::native_bridge) fn text_control_selection_end_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !supports_variable_length_selection(runtime, handle) {
        throw_invalid_selection_state(scope);
        return;
    }
    let owner = text_control_selection_idl_owner(runtime, handle);
    let value = args.get(0);
    let Some(next) = text_control_selection_offset_value(scope, value, owner, "selectionEnd")
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let next = clamp_text_control_offset(runtime, handle, next);
    if runtime.set_selection_end(handle, next) {
        queue_text_control_select_event(scope, runtime_ptr, handle);
        queue_text_control_selection_change_event(scope, runtime_ptr, handle);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn text_control_selection_direction_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    if !supports_variable_length_selection(unsafe { &*runtime_ptr }, handle) {
        rv.set_null();
        return;
    }
    let direction = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(Element::selection_direction)
        .unwrap_or("none");
    rv.set(v8str(scope, direction).into());
}

pub(in crate::native_bridge) fn text_control_selection_direction_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !supports_variable_length_selection(runtime, handle) {
        throw_invalid_selection_state(scope);
        return;
    }
    let owner = text_control_selection_idl_owner(runtime, handle);
    let value = args.get(0);
    let Some(direction) =
        form_dom_string_property_value(scope, value, owner, "selectionDirection", false)
    else {
        return;
    };
    if unsafe { &mut *runtime_ptr }.set_selection_direction(handle, &direction) {
        queue_text_control_select_event(scope, runtime_ptr, handle);
        queue_text_control_selection_change_event(scope, runtime_ptr, handle);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn text_control_set_selection_range_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    let Some(parsed) = webidl::parse_args::<TextControlSetSelectionRangeArgs>(scope, &args) else {
        return;
    };
    if !supports_variable_length_selection(unsafe { &*runtime_ptr }, handle) {
        throw_invalid_selection_state(scope);
        return;
    }
    text_control_set_selection_range_with_direction_internal(
        scope,
        runtime_ptr,
        handle,
        parsed.start,
        parsed.end,
        parsed.direction.as_deref().unwrap_or("none"),
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn text_control_set_range_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    let Some(parsed) = webidl::parse_args::<TextControlSetRangeTextArgs>(scope, &args) else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !supports_variable_length_selection(runtime, handle) {
        throw_invalid_selection_state(scope);
        return;
    }

    let value = text_control_value(runtime, handle);
    let value_units = utf16_units(&value);
    let value_len = value_units.len() as u32;
    let (current_start, current_end) = current_selection_or_end(runtime, handle, value_len);
    let start = parsed.start.unwrap_or(current_start).min(value_len);
    let end = parsed.end.unwrap_or(current_end).min(value_len);
    if end < start {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "The end offset is less than the start offset.",
        );
        return;
    }

    let replacement_units = utf16_units(&parsed.replacement);
    let replacement_len = replacement_units.len() as u32;
    let next_value = utf16_replace_units_range_lossy(
        &value_units,
        start as usize,
        end.saturating_sub(start) as usize,
        &replacement_units,
    );

    let mode = parsed.selection_mode.as_deref().unwrap_or("preserve");
    if !matches!(mode, "select" | "start" | "end" | "preserve") {
        throw_type_error(scope, "Invalid selectionMode.");
        return;
    }

    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_input_value(handle, &next_value);
    let replacement_end = start + replacement_len;
    let (next_start, next_end) = match mode {
        "select" => (start, replacement_end),
        "start" => (start, start),
        "end" => (replacement_end, replacement_end),
        _ => (
            preserve_selection_position(
                current_start,
                start,
                end,
                replacement_end,
                replacement_len,
            ),
            preserve_selection_position(current_end, start, end, replacement_end, replacement_len),
        ),
    };
    let _ =
        text_control_set_selection_range_internal(scope, runtime_ptr, handle, next_start, next_end);
    rv.set_undefined();
}

fn preserve_selection_position(
    position: u32,
    start: u32,
    end: u32,
    replacement_end: u32,
    replacement_len: u32,
) -> u32 {
    if position <= start {
        return position;
    }
    if position >= end {
        let replaced_len = end - start;
        return if replacement_len >= replaced_len {
            position.saturating_add(replacement_len - replaced_len)
        } else {
            position.saturating_sub(replaced_len - replacement_len)
        };
    }
    replacement_end
}

pub(in crate::native_bridge) fn text_control_select_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    let len = text_control_value(unsafe { &*runtime_ptr }, handle)
        .chars()
        .count() as u32;
    let _ = text_control_set_selection_range_internal(scope, runtime_ptr, handle, 0, len);
    rv.set_undefined();
}
