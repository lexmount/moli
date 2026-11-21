use super::PageScriptHost;
use super::insert_text::{
    EditingInsertionTarget, editing_host_at_boundary, editing_insertion_target,
};
use crate::context_bootstrap::delete_editable_selection;
use crate::document_runtime::DomHandle;
use crate::native_bridge::{
    element::{
        TextEditInputType, construct_input_event, dispatch_public_event,
        dispatch_text_control_event, queue_text_control_document_selection_change_event,
        restore_focused_text_control_selection, text_control_value_dom_string,
    },
    node_owner_document_relevant_context,
};
use crate::util::{
    utf16_next_scalar_boundary, utf16_previous_scalar_boundary, utf16_scalar_boundary_at_or_after,
};

pub(super) fn exec_command_delete(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    forward: bool,
) -> bool {
    let Some(target) = editing_insertion_target(unsafe { &*runtime_ptr }, document) else {
        return false;
    };
    let handle = match target {
        EditingInsertionTarget::TextControl(handle)
        | EditingInsertionTarget::EditingHost(handle) => handle,
    };
    let Some(context) = node_owner_document_relevant_context(scope, runtime_ptr, handle) else {
        return false;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    match target {
        EditingInsertionTarget::TextControl(control) => {
            exec_command_delete_text_control(scope, runtime_ptr, control, forward)
        }
        EditingInsertionTarget::EditingHost(host) => {
            delete_in_editing_host(scope, runtime_ptr, document, host, forward)
        }
    }
}

pub(crate) fn delete_in_editing_host(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    host: DomHandle,
    forward: bool,
) -> bool {
    let Some(changed) = delete_editable_selection(
        scope,
        runtime_ptr,
        document,
        host,
        forward,
        |runtime, node| {
            runtime.document_is_fully_active(document)
                && editing_host_at_boundary(runtime, document, node) == Some(host)
        },
    ) else {
        return false;
    };
    let input_type = if forward {
        TextEditInputType::DeleteContentForward
    } else {
        TextEditInputType::DeleteContentBackward
    };
    let runtime = unsafe { &*runtime_ptr };
    if changed
        && runtime.document_is_fully_active(document)
        && editing_host_at_boundary(runtime, document, host) == Some(host)
        && let Some(event) = construct_input_event(scope, "input", input_type, None)
    {
        let _ = dispatch_public_event(scope, runtime_ptr, host, event);
    }
    true
}

fn exec_command_delete_text_control(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
    forward: bool,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let value = text_control_value_dom_string(runtime, handle);
    let value_units = value.utf16_units();
    let value_len = value_units.len() as u32;
    let (start, end) = runtime
        .text_control_selection(handle)
        .map(|selection| {
            let start = selection.start.min(value_len);
            let end = selection.end.min(value_len);
            if start <= end {
                (start, end)
            } else {
                (end, start)
            }
        })
        .unwrap_or((value_len, value_len));
    // Selection APIs expose code units, but native editing first resolves both
    // endpoints to caret positions that cannot split a surrogate pair.
    let start = utf16_scalar_boundary_at_or_after(&value_units, start as usize) as u32;
    let end = utf16_scalar_boundary_at_or_after(&value_units, end as usize) as u32;
    let (from, to) = if start != end {
        (start, end)
    } else if forward {
        (
            start,
            utf16_next_scalar_boundary(&value_units, start as usize) as u32,
        )
    } else {
        (
            utf16_previous_scalar_boundary(&value_units, start as usize) as u32,
            start,
        )
    };
    if from == to {
        return true;
    }
    let caret = from;

    let next_value =
        value.replace_utf16_range(from as usize, to.saturating_sub(from) as usize, &[]);
    let runtime = unsafe { &mut *runtime_ptr };
    let changed = runtime.set_input_value_from_user_edit(handle, &next_value);
    if changed {
        runtime.mark_text_control_change_pending(handle, &value);
    }
    let direction = runtime.text_control_selection_direction(handle, "none");
    let selection_changed =
        runtime.set_selection_range_with_direction(handle, caret, caret, direction);
    if changed || selection_changed {
        restore_focused_text_control_selection(scope, runtime_ptr, handle);
        dispatch_text_control_event(scope, runtime_ptr, handle, "input");
        queue_text_control_document_selection_change_event(scope, runtime_ptr, handle);
    }
    changed || selection_changed
}
