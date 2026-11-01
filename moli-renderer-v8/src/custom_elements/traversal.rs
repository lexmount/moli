use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(super) fn shadow_including_subtree_handles(
    host_ptr: *mut JsContextHost,
    root: DomHandle,
) -> Vec<DomHandle> {
    unsafe { &*host_ptr }
        .dom_host()
        .shadow_including_subtree(root)
        .collect()
}

pub(super) fn collect_shadow_including_subtree_handles(
    host_ptr: *mut JsContextHost,
    root: DomHandle,
    handles: &mut Vec<DomHandle>,
) {
    handles.extend(
        unsafe { &*host_ptr }
            .dom_host()
            .shadow_including_subtree(root),
    );
}

pub(super) fn shadow_including_child_handles(
    host_ptr: *mut JsContextHost,
    root: DomHandle,
) -> Vec<DomHandle> {
    let dom_host = unsafe { &*host_ptr }.dom_host();
    let mut children = Vec::new();
    if let Some(shadow_root) = dom_host.shadow_root_handle(root) {
        children.push(shadow_root);
    }
    children.extend(dom_host.child_handles(root));
    children
}
