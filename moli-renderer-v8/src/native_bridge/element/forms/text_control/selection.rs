use super::events::{queue_text_control_select_event, queue_text_control_selection_change_event};
use super::value::{
    char_offset_to_byte_index, clamp_text_control_offset, is_text_control,
    supports_variable_length_selection,
};
use super::*;
use crate::context_bootstrap::{TextInputType, construct_original_input_event};
use crate::util::v8str;
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
    input_type: TextInputType,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    if !is_text_control(runtime, handle) {
        return false;
    }

    let Some(before_input) =
        construct_original_input_event(scope, "beforeinput", input_type, replacement_text)
    else {
        return false;
    };
    let _ = dispatch_public_event(scope, runtime_ptr, handle, before_input);
    if event_default_prevented(scope, before_input) {
        return false;
    }

    let value = text_control_value(runtime, handle);
    let (mut start, mut end) = runtime
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
            let value_len = value.chars().count() as u32;
            (value_len, value_len)
        });

    // Delete relative to the selection only after beforeinput has run. A
    // canceled deletion must not move the caret or extend the selection.
    if start == end {
        match input_type {
            TextInputType::DeleteContentBackward => start = start.saturating_sub(1),
            TextInputType::DeleteContentForward => {
                end = end.saturating_add(1).min(value.chars().count() as u32);
            }
            _ => {}
        }
    }

    let next_value = format!(
        "{}{}{}",
        &value[..char_offset_to_byte_index(&value, start)],
        replacement_text,
        &value[char_offset_to_byte_index(&value, end)..]
    );

    let runtime = unsafe { &mut *runtime_ptr };
    let changed = runtime.set_input_value_from_user_edit(handle, &next_value);
    if changed {
        runtime.mark_text_control_change_pending(handle, &value);
    }
    let caret = start + replacement_text.chars().count() as u32;
    let selection_changed =
        text_control_set_selection_range_internal(scope, runtime_ptr, handle, caret, caret);
    if (changed || selection_changed)
        && let Some(event) =
            construct_original_input_event(scope, "input", input_type, replacement_text)
    {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    changed || selection_changed
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
    let value_len = value.chars().count() as u32;
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

    let replacement_len = parsed.replacement.chars().count() as u32;
    let start_byte = char_offset_to_byte_index(&value, start);
    let end_byte = char_offset_to_byte_index(&value, end);
    let mut next_value = String::with_capacity(value.len() + parsed.replacement.len());
    next_value.push_str(&value[..start_byte]);
    next_value.push_str(&parsed.replacement);
    next_value.push_str(&value[end_byte..]);

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
