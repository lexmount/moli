use super::PageScriptHost;
use crate::context_bootstrap::replace_editable_selection_text;
use crate::document_runtime::DomHandle;
use crate::dom::native::{ContentEditableState, NodeType};
use crate::native_bridge::element::{
    TextEditInputType, construct_input_event, dispatch_public_event, is_text_control_user_editable,
    replace_text_control_selection_with_input_event,
};

pub(super) enum EditingInsertionTarget {
    TextControl(DomHandle),
    EditingHost(DomHandle),
}

pub(super) fn editing_insertion_target(
    runtime: &PageScriptHost,
    document: DomHandle,
) -> Option<EditingInsertionTarget> {
    if !runtime.document_is_fully_active(document) {
        return None;
    }
    if let Some(control) = runtime.document_selected_text_control(document) {
        return (runtime.active_element_handle() == Some(control)
            && is_text_control_user_editable(runtime, control))
        .then_some(EditingInsertionTarget::TextControl(control));
    }

    let selection = runtime.document_editing_selection(document)?;
    let dom = runtime.dom_host();
    let start = dom.node(selection.start.container)?;
    if !matches!(
        start.node_type(),
        NodeType::Element | NodeType::Text | NodeType::CDataSection
    ) {
        return None;
    }
    let host = editing_host_at_boundary(runtime, document, selection.start.container)?;
    (editing_host_at_boundary(runtime, document, selection.end.container) == Some(host))
        .then_some(EditingInsertionTarget::EditingHost(host))
}

pub(super) fn editing_host_at_boundary(
    runtime: &PageScriptHost,
    document: DomHandle,
    node: DomHandle,
) -> Option<DomHandle> {
    let dom = runtime.dom_host();
    if dom.owner_document_handle(node) != Some(document) || !dom.is_connected(node) {
        return None;
    }
    if runtime.document_design_mode_enabled(document) {
        let document_node = dom.node(document)?.as_document()?;
        let body = document_node
            .body_handle(dom.dom(), document)
            .or_else(|| document_node.document_element_handle(dom.dom(), document));
        let mut current = Some(node);
        while let Some(handle) = current {
            let ancestor = dom.node(handle)?;
            if ancestor.as_element().is_some_and(|element| {
                element.content_editable_state() == ContentEditableState::False
            }) {
                break;
            }
            if Some(handle) == body {
                return Some(handle);
            }
            current = ancestor.parent_node_id();
        }
    }
    dom.dom().contenteditable_editing_host(node)
}

pub(super) fn exec_command_insert_text(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    replacement: &str,
) -> bool {
    let Some(target) = editing_insertion_target(unsafe { &*runtime_ptr }, document) else {
        return false;
    };
    let handle = match target {
        EditingInsertionTarget::TextControl(handle)
        | EditingInsertionTarget::EditingHost(handle) => handle,
    };
    let Some(context) =
        crate::native_bridge::node_owner_document_relevant_context(scope, runtime_ptr, handle)
    else {
        return false;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let input_type = TextEditInputType::InsertText;
    let data = input_type.data(replacement);

    if let EditingInsertionTarget::TextControl(_) = target {
        return replace_text_control_selection_with_input_event(
            scope,
            runtime_ptr,
            handle,
            replacement,
            input_type,
            None,
        );
    }
    let Some(changed) = replace_editable_selection_text(
        scope,
        runtime_ptr,
        document,
        replacement,
        |runtime, node| {
            runtime.document_is_fully_active(document)
                && editing_host_at_boundary(runtime, document, node) == Some(handle)
        },
    ) else {
        return false;
    };
    if changed && let Some(event) = construct_input_event(scope, "input", input_type, data) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    }
    true
}
