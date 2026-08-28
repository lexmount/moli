use crate::{document_runtime::DomHandle, dom::native::DomHost, native_bridge::JsContextHost};

pub(crate) fn is_shadow_including_rooted_in_document(host: &DomHost, handle: DomHandle) -> bool {
    shadow_including_root_document_handle(host, handle).is_some()
}

pub(super) fn shadow_including_root_document_handle(
    host: &DomHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    let mut current = Some(handle);
    while let Some(candidate) = current {
        let node = host.node(candidate)?;
        if node.is_document() {
            return Some(candidate);
        }
        current = node
            .parent_node()
            .or_else(|| host.shadow_root_host(candidate));
    }
    None
}

pub(crate) fn is_shadow_including_rooted_in_browsing_context_document(
    host: &JsContextHost,
    handle: DomHandle,
) -> bool {
    let Some(document_handle) = shadow_including_root_document_handle(host.dom_host(), handle)
    else {
        return false;
    };
    document_handle == host.dom_host().document_handle()
        || host
            .child_browsing_context_host_for_document_handle(document_handle)
            .is_some()
}
