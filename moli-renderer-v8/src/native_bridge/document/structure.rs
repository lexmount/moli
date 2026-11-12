use crate::dom::native::Node;
use crate::native_bridge::node::node_or_foreign_arg_handle_allow_detached;
use crate::native_bridge::throw_dom_exception;
use crate::util::throw_type_error;

fn throw_document_body_hierarchy_request_error(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "HierarchyRequestError",
        3,
        "Document.body must be set to a body or frameset element.",
    );
}

fn throw_document_body_type_error(scope: &mut v8::PinScope<'_, '_>) {
    throw_type_error(
        scope,
        "Failed to set 'body' on 'Document': The provided value is not of type 'HTMLElement'.",
    );
}

pub(in crate::native_bridge::document) fn set_document_body_for_native_handle_appending_to_current_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut super::PageScriptHost,
    document_handle: super::DomHandle,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    let Some(new_body) =
        node_or_foreign_arg_handle_allow_detached(scope, runtime_ptr, Some(document_handle), value)
    else {
        if value.is_null_or_undefined() {
            throw_document_body_hierarchy_request_error(scope);
        } else {
            throw_document_body_type_error(scope);
        }
        return false;
    };

    let runtime = unsafe { &mut *runtime_ptr };
    let Some(new_body_node) = runtime.dom_host().node(new_body) else {
        throw_document_body_type_error(scope);
        return false;
    };
    let Some(new_body_element) = new_body_node.as_element() else {
        throw_document_body_type_error(scope);
        return false;
    };
    if new_body_element.namespace() != super::XHTML_NS {
        throw_document_body_type_error(scope);
        return false;
    }
    if !new_body_element.is_html_element("body") && !new_body_element.is_html_element("frameset") {
        throw_document_body_hierarchy_request_error(scope);
        return false;
    }

    let dom = runtime.dom_host().dom();
    let Some(document_element) = dom
        .node(document_handle)
        .and_then(Node::as_document)
        .and_then(|document| document.document_element_handle(dom, document_handle))
    else {
        throw_dom_exception(
            scope,
            "HierarchyRequestError",
            3,
            "Document has no documentElement.",
        );
        return false;
    };
    let current_body = dom
        .node(document_handle)
        .and_then(Node::as_document)
        .and_then(|document| document.body_or_frameset_handle(dom, document_handle));
    if current_body == Some(new_body) {
        return true;
    }
    if let Some(current_body) = current_body {
        let parent = runtime
            .dom_host()
            .node(current_body)
            .and_then(Node::parent_node)
            .unwrap_or(document_element);
        runtime.replace_child_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            parent,
            new_body,
            current_body,
        )
    } else {
        runtime.append_child_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            document_element,
            new_body,
        )
    }
}
