use super::PageScriptHost;
use crate::context_bootstrap::replace_editable_selection_text;
use crate::document_runtime::DomHandle;
use crate::dom::native::NodeType;
use crate::native_bridge::element::{
    TextEditInputType, construct_input_event, dispatch_public_event, is_text_control_user_editable,
    replace_text_control_selection_with_input_event,
};

pub(super) enum InsertTextTarget {
    TextControl(DomHandle),
    EditingHost(DomHandle),
}

pub(super) fn insert_text_command_target(
    runtime: &PageScriptHost,
    document: DomHandle,
) -> Option<InsertTextTarget> {
    if !runtime.document_is_fully_active(document) {
        return None;
    }
    if let Some(control) = runtime.document_selected_text_control(document) {
        return (runtime.active_element_handle() == Some(control)
            && is_text_control_user_editable(runtime, control))
        .then_some(InsertTextTarget::TextControl(control));
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
        .then_some(InsertTextTarget::EditingHost(host))
}

fn editing_host_at_boundary(
    runtime: &PageScriptHost,
    document: DomHandle,
    node: DomHandle,
) -> Option<DomHandle> {
    let dom = runtime.dom_host();
    if dom.owner_document_handle(node) != Some(document) || !dom.is_connected(node) {
        return None;
    }
    dom.dom().contenteditable_editing_host(node)
}

pub(super) fn exec_command_insert_text(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    replacement: &str,
) -> bool {
    let Some(target) = insert_text_command_target(unsafe { &*runtime_ptr }, document) else {
        return false;
    };
    let handle = match target {
        InsertTextTarget::TextControl(handle) | InsertTextTarget::EditingHost(handle) => handle,
    };
    let Some(context) =
        crate::native_bridge::node_owner_document_relevant_context(scope, runtime_ptr, handle)
    else {
        return false;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let input_type = TextEditInputType::InsertText;
    let data = input_type.data(replacement);

    if let InsertTextTarget::TextControl(_) = target {
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
