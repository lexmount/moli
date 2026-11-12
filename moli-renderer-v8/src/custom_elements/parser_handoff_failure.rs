use crate::{document_runtime::DomHandle, dom::native::Node, native_bridge::PageScriptHost};

pub(super) fn reset_parser_failed_custom_element_construction_artifacts(
    host_ptr: *mut PageScriptHost,
    handle: DomHandle,
) {
    let parent = unsafe { &*host_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node);
    if let Some(parent) = parent {
        let _ = unsafe { &mut *host_ptr }
            .dom_host_mut()
            .remove_child(parent, handle);
    }

    let children = unsafe { &*host_ptr }
        .dom_host()
        .child_handles(handle)
        .collect::<Vec<_>>();
    for child in children {
        let _ = unsafe { &mut *host_ptr }
            .dom_host_mut()
            .remove_child(handle, child);
    }

    let attributes = unsafe { &*host_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(|element| element.attributes().to_vec())
        .unwrap_or_default();
    for attribute in attributes {
        if attribute.namespace().is_empty() {
            let _ = unsafe { &mut *host_ptr }
                .dom_host_mut()
                .remove_attribute(handle, &attribute.name());
        } else {
            let _ = unsafe { &mut *host_ptr }
                .dom_host_mut()
                .remove_attribute_ns(handle, Some(attribute.namespace()), attribute.local_name());
        }
    }
}
