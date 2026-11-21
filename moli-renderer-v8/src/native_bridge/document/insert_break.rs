use super::PageScriptHost;
use super::insert_text::{
    EditingInsertionTarget, editing_host_at_boundary, editing_insertion_target,
};
use crate::context_bootstrap::{
    replace_editable_selection_line_break, replace_editable_selection_paragraph,
};
use crate::document_runtime::DomHandle;
use crate::dom::native::Node;
use crate::native_bridge::{
    element::{
        TextEditInputType, construct_input_event, dispatch_public_event,
        replace_text_control_selection_with_input_event,
    },
    node_owner_document_relevant_context,
};

pub(super) fn exec_command_insert_break(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    input_type: TextEditInputType,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let Some(target) = editing_insertion_target(runtime, document) else {
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
    if let EditingInsertionTarget::TextControl(_) = target {
        let multiline = unsafe { &*runtime_ptr }
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| element.is_html_textarea());
        // A single-line input accepts the command without inserting a newline.
        return !multiline
            || replace_text_control_selection_with_input_event(
                scope,
                runtime_ptr,
                handle,
                "\n",
                input_type,
                None,
            );
    }
    insert_break_in_editing_host(scope, runtime_ptr, document, handle, input_type)
}

pub(crate) fn insert_break_in_editing_host(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    host: DomHandle,
    input_type: TextEditInputType,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    if !matches!(editing_insertion_target(runtime, document), Some(EditingInsertionTarget::EditingHost(target)) if target == host)
    {
        return false;
    }
    let is_editable = |runtime: &PageScriptHost, node| {
        runtime.document_is_fully_active(document)
            && editing_host_at_boundary(runtime, document, node) == Some(host)
    };
    let changed = match input_type {
        TextEditInputType::InsertLineBreak => {
            replace_editable_selection_line_break(scope, runtime_ptr, document, host, is_editable)
        }
        TextEditInputType::InsertParagraph => {
            replace_editable_selection_paragraph(scope, runtime_ptr, document, host, is_editable)
        }
        _ => None,
    };
    let Some(changed) = changed else {
        return false;
    };
    if changed && let Some(event) = construct_input_event(scope, "input", input_type, None) {
        let _ = dispatch_public_event(scope, runtime_ptr, host, event);
    }
    true
}
