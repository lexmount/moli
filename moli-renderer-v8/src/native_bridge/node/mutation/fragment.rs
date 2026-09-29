use super::*;

pub(super) fn value_to_inserted_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: Option<DomHandle>,
    value: v8::Local<'_, v8::Value>,
) -> Option<DomHandle> {
    if crate::native_bridge::document::is_attr_node_value(scope, value) {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return None;
    }
    if let Some(handle) =
        node_or_foreign_arg_handle_allow_detached(scope, runtime_ptr, document_handle, value)
    {
        return Some(handle);
    }
    let text = value.to_string(scope)?.to_rust_string_lossy(scope);
    let runtime = unsafe { &mut *runtime_ptr };
    Some(match document_handle {
        Some(document_handle) => runtime.create_text_node_for_document(document_handle, &text),
        None => runtime.create_text_node(&text),
    })
}

pub(super) fn build_insertion_fragment_from_handles(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: Option<DomHandle>,
    handles: &[DomHandle],
) -> Option<DomHandle> {
    blur_focused_inserted_handles(scope, runtime_ptr, handles);
    let runtime = unsafe { &mut *runtime_ptr };
    let fragment = match document_handle {
        Some(document_handle) => runtime.create_document_fragment_for_document(document_handle),
        None => runtime.create_document_fragment(),
    };
    for handle in handles {
        if !runtime.append_child_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            fragment,
            *handle,
        ) {
            return None;
        }
    }
    Some(fragment)
}

fn blur_focused_inserted_handles(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handles: &[DomHandle],
) {
    let runtime = unsafe { &*runtime_ptr };
    let Some(active) = runtime.active_element_handle() else {
        return;
    };
    if handles
        .iter()
        .any(|handle| *handle == active || handle_contains(runtime, *handle, active))
    {
        crate::native_bridge::element::update_focus(scope, runtime_ptr, None);
    }
}

fn handle_contains(runtime: &JsContextHost, root: DomHandle, handle: DomHandle) -> bool {
    let mut current = Some(handle);
    while let Some(candidate) = current {
        if candidate == root {
            return true;
        }
        current = runtime
            .dom_host()
            .node(candidate)
            .and_then(Node::parent_node);
    }
    false
}

pub(super) fn validate_document_sequence(
    runtime: &JsContextHost,
    parent: DomHandle,
    reference_child: Option<DomHandle>,
    inserted: &[DomHandle],
    skipped: &[DomHandle],
) -> bool {
    if !node_is_document(runtime, parent) {
        return true;
    }

    let mut sequence = Vec::new();
    let children = runtime
        .dom_host()
        .node(parent)
        .map(|node| node.child_ids(runtime.dom_host().dom()).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut inserted_emitted = false;
    for child in children {
        if Some(child) == reference_child {
            sequence.extend_from_slice(inserted);
            inserted_emitted = true;
        }
        if skipped.contains(&child) {
            continue;
        }
        sequence.push(child);
    }
    if !inserted_emitted {
        sequence.extend_from_slice(inserted);
    }

    let mut saw_element = false;
    let mut saw_doctype = false;
    for handle in sequence {
        let Some(node) = runtime.dom_host().node(handle) else {
            return false;
        };
        match node.node_type() {
            NodeType::Text | NodeType::CDataSection => return false,
            NodeType::Element => {
                if saw_element {
                    return false;
                }
                saw_element = true;
            }
            NodeType::DocumentType => {
                if saw_doctype || saw_element {
                    return false;
                }
                saw_doctype = true;
            }
            _ => {}
        }
    }
    true
}

pub(super) fn insert_fragment_with_validation(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    parent: DomHandle,
    reference_child: Option<DomHandle>,
    skipped: &[DomHandle],
    fragment: DomHandle,
    inserted: &[DomHandle],
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    if !validate_document_sequence(runtime, parent, reference_child, inserted, skipped) {
        return false;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.insert_before_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        parent,
        fragment,
        reference_child,
    )
}
