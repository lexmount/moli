use super::*;
use crate::util::v8_value_to_dom_string_u16;
use widestring::U16String;

pub(super) enum NodeOrString<'s> {
    Node(v8::Local<'s, v8::Value>),
    String(U16String),
}

pub(super) fn convert_node_or_string_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[v8::Local<'s, v8::Value>],
) -> Option<Vec<NodeOrString<'s>>> {
    // Web IDL converts every (Node or DOMString) argument before the DOM
    // algorithm runs, including strings after a node that will fail insertion.
    let mut inputs = Vec::with_capacity(values.len());
    for &value in values {
        if v8::Local::<v8::Object>::try_from(value)
            .is_ok_and(|object| web_api_interfaces::Node::is_instance(scope, object))
        {
            inputs.push(NodeOrString::Node(value));
        } else {
            let value = v8_value_to_dom_string_u16(scope, value, false)?;
            inputs.push(NodeOrString::String(value));
        }
    }
    Some(inputs)
}

fn converted_input_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document: Option<DomHandle>,
    input: NodeOrString<'_>,
) -> Option<DomHandle> {
    match input {
        NodeOrString::Node(value) => {
            if !crate::native_bridge::document::is_attr_node_value(scope, value)
                && let Some(handle) =
                    node_or_foreign_arg_handle_allow_detached(scope, runtime_ptr, document, value)
            {
                return Some(handle);
            }
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
            None
        }
        NodeOrString::String(value) => {
            Some(unsafe { &mut *runtime_ptr }.create_text_node_from_utf16_units(document, value))
        }
    }
}

pub(super) fn convert_nodes_into_node(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document: Option<DomHandle>,
    mut inputs: Vec<NodeOrString<'_>>,
) -> Option<DomHandle> {
    if inputs.len() == 1 {
        return converted_input_handle(scope, runtime_ptr, document, inputs.pop()?);
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let fragment = match document {
        Some(document) => runtime.create_document_fragment_for_document(document),
        None => runtime.create_document_fragment(),
    };
    for input in inputs {
        let child = converted_input_handle(scope, runtime_ptr, document, input)?;
        if !validate_pre_insert_handles(scope, unsafe { &*runtime_ptr }, fragment, child, None, &[])
        {
            return None;
        }
        if !unsafe { &mut *runtime_ptr }.append_child_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            fragment,
            child,
        ) {
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
            return None;
        }
    }
    Some(fragment)
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
